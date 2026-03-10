use crate::persistence::Device;
use crate::proto::ProtocolType;
use meesign_crypto::auth::verify_broadcast;
use meesign_crypto::proto::{ClientMessage, Message, ServerMessage};
use std::collections::HashMap;

/// Communication state of a Task
pub struct Communicator {
    /// The minimal number of parties needed to successfully complete the task
    threshold: u32,
    /// A mapping of protocol indices to the active shares' devices
    active_shares: HashMap<u32, Device>,
    /// A mapping of protocol indices to incoming messages
    input: HashMap<u32, ClientMessage>,
    /// A mapping of protocol indices to outgoing messages
    output: HashMap<u32, Vec<u8>>,
    /// Relayed protocol type
    protocol_type: ProtocolType,
}

impl Communicator {
    /// Constructs a new Communicator instance.
    ///
    /// # Arguments
    /// * `threshold` - The minimal number of devices to successfully complete the task.
    /// * `protocol_type` - The protocol type of the task.
    /// * `active_shares` - A mapping of protocol indices to the active shares' devices.
    pub fn new(
        threshold: u32,
        protocol_type: ProtocolType,
        active_shares: HashMap<u32, Device>,
    ) -> Self {
        assert!(active_shares.len() > 1);
        assert!(threshold <= active_shares.len() as u32);
        // TODO uncomment once is_sorted is stabilized
        // assert!(devices.is_sorted());

        Communicator {
            threshold,
            active_shares,
            input: HashMap::new(),
            output: HashMap::new(),
            protocol_type,
        }
    }

    /// Clears incoming message buffers
    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    /// Receive messages from a given device
    ///
    /// # Arguments
    ///
    /// * `sender_id` - identifier of the sender device
    /// * `messages` - a message from each of the sender device's shares
    pub fn receive_messages(&mut self, sender_id: &[u8], messages: Vec<ClientMessage>) -> bool {
        let sender_indices = self.identifier_to_indices(sender_id);
        if messages.is_empty() || sender_indices.len() != messages.len() {
            return false;
        }

        for msg in &messages {
            assert!(msg.broadcast.is_some() || msg.unicasts.len() == self.threshold as usize - 1);
        }

        self.input.extend(sender_indices.into_iter().zip(messages));

        true
    }

    /// Is waiting for a message from the given device id
    pub fn waiting_for(&self, device_id: &[u8]) -> bool {
        self.identifier_to_indices(device_id)
            .iter()
            .any(|idx| !self.input.contains_key(idx))
    }

    /// Moves messages from incoming buffers to outgoing buffers
    pub fn relay(&mut self) {
        self.output = self
            .get_protocol_indices()
            .into_iter()
            .map(|idx| {
                let mut unicasts = HashMap::new();
                let mut broadcasts = HashMap::new();

                for (&sender, msg) in &self.input {
                    if sender == idx {
                        continue;
                    }
                    if let Some(broadcast) = &msg.broadcast {
                        broadcasts.insert(sender, broadcast.clone());
                    }
                    if let Some(unicast) = msg.unicasts.get(&idx) {
                        unicasts.insert(sender, unicast.clone());
                    }
                }

                let msg = ServerMessage {
                    protocol_type: meesign_crypto::proto::ProtocolType::from(self.protocol_type)
                        .into(),
                    unicasts,
                    broadcasts,
                };

                (idx, msg.encode_to_vec())
            })
            .collect();

        self.clear_input();
    }

    /// Sends a message to all active devices parametrized by their protocol index
    pub fn send_all<F>(&mut self, f: F)
    where
        F: Fn(u32) -> Vec<u8>,
    {
        self.output = self
            .get_protocol_indices()
            .into_iter()
            .map(|idx| (idx, f(idx)))
            .collect();

        self.clear_input();
    }

    /// Check whether incoming buffers contain messages from all active devices
    pub fn round_received(&self) -> bool {
        self.input.len() == self.active_shares.len()
    }

    /// Get all messages for a given device
    pub fn get_messages(&self, device_id: &[u8]) -> Vec<Vec<u8>> {
        self.identifier_to_indices(device_id)
            .iter()
            .map(|idx| self.output.get(idx).cloned().unwrap_or_default())
            .collect()
    }

    /// Get the final message
    pub fn get_final_message(&self) -> Option<Vec<u8>> {
        if self.input.is_empty() {
            return None;
        }

        let mut final_message = None;
        for (sender, msg) in &self.input {
            let sender_index = sender - self.protocol_type.index_offset();
            let cert_der = &self.active_shares[&sender_index].certificate;

            // NOTE: Verify all signed broadcasts and check that the messages are all equal
            let msg = verify_broadcast(msg.broadcast.as_ref().unwrap(), cert_der).ok()?;
            if let Some(prev) = final_message {
                assert_eq!(prev, msg);
            }
            final_message = Some(msg);
        }

        final_message
    }

    /// Get the protocol indices of active devices
    pub fn get_protocol_indices(&self) -> Vec<u32> {
        let mut indices: Vec<u32> = self
            .active_shares
            .keys()
            .map(|idx| *idx + self.protocol_type.index_offset())
            .collect();
        indices.sort();
        indices
    }

    /// Get the protocol indices of an active device
    pub fn identifier_to_indices(&self, device_id: &[u8]) -> Vec<u32> {
        let mut indices: Vec<u32> = self
            .active_shares
            .iter()
            .filter(|(_, device)| device.id == device_id)
            .map(|(idx, _)| *idx + self.protocol_type.index_offset())
            .collect();
        indices.sort();
        indices
    }
}

#[cfg(test)]
mod tests {
    use crate::persistence::DeviceKind;

    use super::*;

    #[test]
    #[should_panic]
    fn communicator_with_no_devices() {
        new_communicator(&[], 0, ProtocolType::Gg18);
    }

    #[test]
    #[should_panic]
    fn communicator_too_large_threshold() {
        new_communicator(&[(true, 1), (true, 1)], 3, ProtocolType::Gg18);
    }

    #[test]
    fn valid_communicator() {
        let mut communicator = new_communicator(
            &[(true, 1), (true, 1), (false, 1), (false, 1), (true, 1)],
            3,
            ProtocolType::Gg18,
        );
        let active_indices = [0, 1, 4];
        assert_eq!(
            &communicator
                .get_protocol_indices()
                .into_iter()
                .map(|x| x as usize)
                .collect::<Vec<_>>(),
            &active_indices
        );
        assert_eq!(communicator.round_received(), false);
        for idx in 0..5 {
            assert_eq!(
                communicator.receive_messages(
                    &[idx as u8],
                    vec![ClientMessage {
                        protocol_type: 0,
                        unicasts: active_indices
                            .iter()
                            .filter(|&&i| i != idx)
                            .map(|&i| (i as u32, vec![]))
                            .collect(),
                        broadcast: None,
                    }]
                ),
                active_indices.contains(&idx)
            );
            assert_eq!(
                communicator.round_received(),
                idx >= active_indices[active_indices.len() - 1]
            );
        }
        assert_eq!(communicator.round_received(), true);

        for idx in 0..5 {
            let msgs = communicator.get_messages(&[idx as u8]);
            let expected: Vec<Vec<u8>> = if active_indices.contains(&idx) {
                vec![vec![]]
            } else {
                vec![]
            };
            assert_eq!(msgs, expected);
        }
        communicator.relay();
        for idx in 0..5 {
            assert_eq!(
                !communicator.get_messages(&[idx as u8]).is_empty(),
                active_indices.contains(&idx)
            );
        }
        assert_eq!(communicator.round_received(), false);
    }

    #[test]
    fn repeated_devices() {
        let communicator = new_communicator(&[(true, 2)], 2, ProtocolType::Gg18);
        assert_eq!(communicator.get_protocol_indices(), vec![0, 1]);
    }

    #[test]
    #[should_panic]
    fn not_enough_messages() {
        let mut communicator =
            new_communicator(&[(true, 1), (true, 1), (true, 1)], 3, ProtocolType::Gg18);
        communicator.receive_messages(
            &[0],
            vec![ClientMessage {
                protocol_type: 0,
                unicasts: HashMap::new(),
                broadcast: None,
            }],
        );
    }

    #[test]
    #[should_panic]
    fn too_many_messages() {
        let mut communicator =
            new_communicator(&[(true, 1), (true, 1), (true, 1)], 3, ProtocolType::Gg18);
        communicator.receive_messages(
            &[0],
            vec![ClientMessage {
                protocol_type: 0,
                unicasts: (0..6 as u32).map(|i| (i, vec![])).collect(),
                broadcast: None,
            }],
        );
    }

    #[test]
    fn send_all() {
        let mut communicator =
            new_communicator(&[(true, 1), (false, 1), (true, 1)], 2, ProtocolType::Gg18);
        communicator.send_all(|idx| vec![idx as u8]);
        assert_eq!(communicator.get_messages(&[0]), vec![vec![0]]);
        assert_eq!(communicator.get_messages(&[1]), Vec::<Vec<u8>>::new());
        assert_eq!(communicator.get_messages(&[2]), vec![vec![2]]);
    }

    #[test]
    fn protocol_init() {
        use meesign_crypto::proto::ProtocolInit;
        let mut communicator =
            new_communicator(&[(true, 1), (false, 1), (true, 1)], 2, ProtocolType::Frost);
        communicator.send_all(|idx| {
            ProtocolInit {
                protocol_type: ProtocolType::Frost as i32,
                indices: Vec::new(),
                index: idx,
                data: "hello".as_bytes().to_vec(),
            }
            .encode_to_vec()
        });
        assert_eq!(
            communicator.get_messages(&[0]),
            vec![ProtocolInit {
                protocol_type: ProtocolType::Frost as i32,
                indices: Vec::new(),
                index: 1,
                data: "hello".as_bytes().to_vec(),
            }
            .encode_to_vec()]
        );
        assert_eq!(communicator.get_messages(&[1]), Vec::new() as Vec<Vec<u8>>);
        assert_eq!(
            communicator.get_messages(&[2]),
            vec![ProtocolInit {
                protocol_type: ProtocolType::Frost as i32,
                indices: Vec::new(),
                index: 3,
                data: "hello".as_bytes().to_vec(),
            }
            .encode_to_vec()]
        );
    }

    #[test]
    fn broadcast_messages() {
        let mut communicator =
            new_communicator(&[(true, 1), (true, 1), (false, 1)], 2, ProtocolType::Frost);

        assert_eq!(communicator.get_protocol_indices(), vec![1, 2]);

        for i in 0..2 {
            assert_eq!(
                communicator.receive_messages(
                    &[i as u8],
                    vec![ClientMessage {
                        protocol_type: ProtocolType::Frost.into(),
                        unicasts: HashMap::new(),
                        broadcast: Some(vec![i as u8]),
                    }]
                ),
                true
            );
        }

        assert_eq!(communicator.round_received(), true);
        communicator.relay();

        assert_eq!(
            communicator.get_messages(&[0]),
            vec![ServerMessage {
                protocol_type: ProtocolType::Frost.into(),
                unicasts: HashMap::new(),
                broadcasts: HashMap::from([(2, vec![1])]),
            }
            .encode_to_vec()],
        );
        assert_eq!(
            communicator.get_messages(&[1]),
            vec![ServerMessage {
                protocol_type: ProtocolType::Frost.into(),
                unicasts: HashMap::new(),
                broadcasts: HashMap::from([(1, vec![0])]),
            }
            .encode_to_vec()],
        );
    }

    fn new_communicator(
        decisions_shares: &[(bool, u32)],
        threshold: u32,
        protocol_type: ProtocolType,
    ) -> Communicator {
        let active_shares = decisions_shares
            .into_iter()
            .enumerate()
            .flat_map(|(idx, &(accept, shares))| {
                let device = Device::new(
                    vec![idx as u8],
                    format!("d{}", idx),
                    DeviceKind::User,
                    vec![0xf0 | idx as u8],
                );
                std::iter::repeat_n((accept, device), shares as usize)
            })
            .enumerate()
            .filter(|(_, (accept, _))| *accept)
            .map(|(idx, (_, device))| (idx as u32, device))
            .collect();
        Communicator::new(threshold, protocol_type, active_shares)
    }
}
