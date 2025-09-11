use crate::persistence::PgPool;
use crate::persistence::{Device, Participant};
use crate::proto::ProtocolType;
use meesign_crypto::auth::verify_broadcast;
use meesign_crypto::proto::{ClientMessage, Message, ServerMessage};
use rand::prelude::SliceRandom;
use rand::thread_rng;
use std::collections::HashMap;
use tonic::codegen::Arc;

/// Communication state of a Task
pub struct Communicator {
    /// The minimal number of parties needed to successfully complete the task
    threshold: u32,
    /// Ordered list of devices
    device_list: Vec<Device>,
    /// Ordered list of active devices (participating in the protocol)
    active_devices: Option<Vec<Vec<u8>>>,
    /// A mapping of device identifiers to their Task decision weight (0 - no decision, positive - accept, negative - reject)
    decisions: HashMap<Vec<u8>, i8>,
    /// A mapping of protocol indices to incoming messages
    input: HashMap<u32, ClientMessage>,
    /// A mapping of protocol indices to outgoing messages
    output: HashMap<u32, Vec<u8>>,
    /// Relayed protocol type
    protocol_type: ProtocolType,
}

impl Communicator {
    /// Constructs a new Communicator instance with given Participants, threshold, ProtocolType, decisions and acknowledgements
    ///
    /// # Arguments
    /// * `participants` - List of distinct participants sorted by device id
    /// * `threshold` - The minimal number of devices to successfully complete the task
    pub fn new(
        participants: Vec<Participant>,
        threshold: u32,
        protocol_type: ProtocolType,
        decisions: HashMap<Vec<u8>, i8>,
    ) -> Self {
        let device_list: Vec<Device> = participants
            .into_iter()
            .flat_map(|p| std::iter::repeat(p.device).take(p.shares as usize))
            .collect();

        assert!(device_list.len() > 1);
        assert!(threshold <= device_list.len() as u32);
        // TODO uncomment once is_sorted is stabilized
        // assert!(devices.is_sorted());

        let mut communicator = Communicator {
            threshold,
            device_list,
            active_devices: None,
            decisions,
            input: HashMap::new(),
            output: HashMap::new(),
            protocol_type,
        };
        communicator.clear_input();
        communicator
    }

    /// Clears incoming message buffers
    pub fn clear_input(&mut self) {
        self.input.clear();
    }

    /// Receive messages from a given device
    ///
    /// # Arguments
    ///
    /// * `from_identifier` - identifier of the sender device
    /// * `messages` - vector containing messages from each of the sender device's shares
    pub fn receive_messages(
        &mut self,
        from_identifier: &[u8],
        messages: Vec<ClientMessage>,
    ) -> bool {
        let from_indices = self.identifier_to_indices(from_identifier);
        if messages.is_empty() || from_indices.len() != messages.len() {
            return false;
        }

        for msg in &messages {
            assert!(msg.broadcast.is_some() || msg.unicasts.len() == self.threshold as usize - 1);
        }

        self.input.extend(from_indices.into_iter().zip(messages));

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
        if self.active_devices.is_none() {
            return false;
        }

        self.input.len() == self.active_devices.as_ref().unwrap().len()
    }

    /// Get all messages for a given device
    pub fn get_messages(&self, device_id: &[u8]) -> Vec<Vec<u8>> {
        self.identifier_to_indices(device_id)
            .iter()
            .map(|idx| self.output.get(idx).map(Vec::clone).unwrap_or_default())
            .collect()
    }

    /// Get the final message
    pub fn get_final_message(&self) -> Option<Vec<u8>> {
        if self.input.len() == 0 {
            return None;
        }

        let active_devices = self.get_active_devices()?;
        let protocol_indices = self.get_protocol_indices();

        let mut final_message = None;
        for (&sender, msg) in &self.input {
            let device_index = protocol_indices
                .iter()
                .position(|&idx| idx == sender)
                .unwrap();
            let device = &active_devices[device_index];
            let cert_der = &self
                .device_list
                .iter()
                .find(|dev| dev.identifier() == device)
                .unwrap()
                .certificate;

            // NOTE: Verify all signed broadcasts and check that the messages are all equal
            let msg = verify_broadcast(msg.broadcast.as_ref().unwrap(), cert_der).ok()?;
            if let Some(prev) = final_message {
                assert_eq!(prev, msg);
            }
            final_message = Some(msg);
        }

        final_message
    }

    /// Sets the active devices
    ///
    /// Picks which devices shall participate in the protocol
    /// Considers only those devices which accepted participation
    /// If enough devices are available, additionaly filters by response latency
    pub fn set_active_devices(&mut self, pg_pool: Option<Arc<PgPool>>) -> Vec<Vec<u8>> {
        assert!(self.accept_count() >= self.threshold);
        let agreeing_devices = self
            .device_list
            .iter()
            .filter(|device| self.decisions.get(device.identifier()) > Some(&0))
            .collect::<Vec<_>>();

        let connected_devices: Vec<_> = match pg_pool {
            Some(_pg_pool) => {
                todo!();
                //let latest_acceptable_time = Local::now() - Duration::seconds(5);
                // agreeing_devices
                //     .iter()
                //     .filter(|device| device.last_active() > &latest_acceptable_time)
                //     .map(Deref::deref)
                //     .collect()
            }
            None => agreeing_devices.clone(),
        };

        let (devices, indices): (&Vec<&Device>, Vec<_>) =
            if connected_devices.len() >= self.threshold as usize {
                (&connected_devices, (0..connected_devices.len()).collect())
            } else {
                (&agreeing_devices, (0..agreeing_devices.len()).collect())
            };
        let mut indices = indices
            .choose_multiple(&mut thread_rng(), self.threshold as usize)
            .cloned()
            .collect::<Vec<_>>();
        indices.sort();

        self.active_devices = Some(
            devices
                .iter()
                .enumerate()
                .filter(|(idx, _)| indices.contains(idx))
                .map(|(_, device)| device.identifier().to_vec())
                .collect(),
        );
        assert_eq!(
            self.active_devices.as_ref().unwrap().len(),
            self.threshold as usize
        );

        self.active_devices.as_ref().unwrap().clone()
    }

    /// Get the active devices
    pub fn get_active_devices(&self) -> Option<Vec<Vec<u8>>> {
        self.active_devices.clone()
    }

    /// Save a decision by the given device
    ///
    /// # Returns
    /// `false` if the `device_id` is invalid or has already decided
    /// `true` otherwise
    pub fn decide(&mut self, device_id: &[u8], decision: bool) -> bool {
        if !self.decisions.contains_key(device_id) || self.decisions[device_id] != 0 {
            return false;
        }
        let votes = self
            .device_list
            .iter()
            .filter(|x| x.identifier() == device_id)
            .count() as i8;
        self.decisions
            .insert(device_id.to_vec(), if decision { votes } else { -votes });
        true
    }

    /// Get the number of Task accepts
    pub fn accept_count(&self) -> u32 {
        self.decisions
            .iter()
            .filter(|x| *x.1 > 0)
            .map(|x| *x.1 as i32)
            .sum::<i32>()
            .abs() as u32
    }

    /// Get the number of Task rejects
    pub fn reject_count(&self) -> u32 {
        self.decisions
            .iter()
            .filter(|x| *x.1 < 0)
            .map(|x| *x.1 as i32)
            .sum::<i32>()
            .abs() as u32
    }

    /// Check whether a device submitted its decision
    pub fn device_decided(&self, device_id: &[u8]) -> bool {
        if let Some(d) = self.decisions.get(device_id) {
            *d != 0
        } else {
            false
        }
    }

    /// Get the protocol indices of active devices
    pub fn get_protocol_indices(&self) -> Vec<u32> {
        assert!(self.active_devices.is_some());

        let active_devices = self.get_active_devices().unwrap();
        let mut devices_iter = self.device_list.iter().enumerate();
        let mut indices: Vec<u32> = Vec::new();

        for device in &active_devices {
            while let Some((idx, dev)) = devices_iter.next() {
                if dev.identifier() == device {
                    indices.push(idx as u32 + self.protocol_type.index_offset());
                    break;
                }
            }
        }

        indices
    }

    /// Get the protocol indices of an active device
    pub fn identifier_to_indices(&self, device_id: &[u8]) -> Vec<u32> {
        if self.active_devices.is_none() {
            return Vec::new();
        }

        let mut devices_iter = self.device_list.iter().enumerate();
        let mut indices = Vec::new();

        for device in self.get_active_devices().unwrap() {
            if device == device_id {
                let (idx, _) = devices_iter
                    .find(|(_, dev)| dev.identifier() == &device)
                    .unwrap();

                indices.push(idx as u32 + self.protocol_type.index_offset());
            }
        }

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
        new_communicator(vec![], 0, ProtocolType::Gg18);
    }

    #[test]
    #[should_panic]
    fn communicator_too_large_threshold() {
        new_communicator(prepare_participants(2), 3, ProtocolType::Gg18);
    }

    #[test]
    fn empty_communicator() {
        let participants = prepare_participants(5);
        let d0 = participants[0].device.identifier().clone();
        let communicator = new_communicator(participants, 3, ProtocolType::Gg18);
        assert_eq!(communicator.accept_count(), 0);
        assert_eq!(communicator.reject_count(), 0);
        assert_eq!(communicator.round_received(), false);
        assert_eq!(communicator.get_messages(&d0), Vec::<Vec<u8>>::new());
        assert_eq!(
            communicator.get_messages(&[0x00, 0x00]),
            Vec::<Vec<u8>>::new()
        );
        assert_eq!(communicator.device_decided(&d0), false);
        assert_eq!(communicator.device_decided(&[0x00, 0x00]), false);
        assert_eq!(communicator.waiting_for(&d0), false);
        assert_eq!(communicator.waiting_for(&[0x00, 0x00]), false);
        assert_eq!(communicator.get_active_devices(), None);
        assert_eq!(communicator.get_final_message(), None);
    }

    #[test]
    fn valid_communicator() {
        let participants = prepare_participants(5);
        let mut communicator = new_communicator(participants.clone(), 3, ProtocolType::Gg18);
        assert_eq!(
            communicator.device_decided(participants[0].device.identifier()),
            false
        );
        communicator.decide(participants[0].device.identifier(), true);
        assert_eq!(communicator.accept_count(), 1);
        assert_eq!(communicator.reject_count(), 0);
        assert_eq!(
            communicator.device_decided(participants[0].device.identifier()),
            true
        );
        assert_eq!(
            communicator.device_decided(participants[2].device.identifier()),
            false
        );
        communicator.decide(participants[2].device.identifier(), false);
        assert_eq!(communicator.accept_count(), 1);
        assert_eq!(communicator.reject_count(), 1);
        assert_eq!(
            communicator.device_decided(participants[2].device.identifier()),
            true
        );
        assert_eq!(
            communicator.device_decided(participants[4].device.identifier()),
            false
        );
        communicator.decide(participants[4].device.identifier(), true);
        assert_eq!(communicator.accept_count(), 2);
        assert_eq!(communicator.reject_count(), 1);
        assert_eq!(
            communicator.device_decided(participants[4].device.identifier()),
            true
        );
        assert_eq!(
            communicator.device_decided(participants[1].device.identifier()),
            false
        );
        communicator.decide(participants[1].device.identifier(), true);
        assert_eq!(communicator.accept_count(), 3);
        assert_eq!(communicator.reject_count(), 1);
        assert_eq!(
            communicator.device_decided(participants[1].device.identifier()),
            true
        );
        assert_eq!(
            communicator.device_decided(participants[3].device.identifier()),
            false
        );
        assert_eq!(communicator.get_active_devices(), None);
        communicator.set_active_devices(None);
        let active_indices = [0, 1, 4];
        assert_eq!(
            communicator.get_active_devices(),
            Some(
                active_indices
                    .iter()
                    .map(|idx| participants[*idx].device.identifier().to_vec())
                    .collect()
            )
        );

        assert_eq!(
            &communicator
                .get_protocol_indices()
                .into_iter()
                .map(|x| x as usize)
                .collect::<Vec<_>>(),
            &active_indices
        );

        for idx in 0..participants.len() {
            assert_eq!(
                communicator.waiting_for(participants[idx].device.identifier()),
                active_indices.contains(&idx)
            );
        }
        assert_eq!(communicator.round_received(), false);
        for idx in 0..participants.len() {
            assert_eq!(
                communicator.receive_messages(
                    participants[idx].device.identifier(),
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

        for idx in 0..participants.len() {
            let msgs = communicator.get_messages(participants[idx].device.identifier());
            let expected: Vec<Vec<u8>> = if active_indices.contains(&idx) {
                vec![vec![]]
            } else {
                vec![]
            };
            assert_eq!(msgs, expected);
        }
        communicator.relay();
        for idx in 0..participants.len() {
            assert_eq!(
                !communicator
                    .get_messages(participants[idx].device.identifier())
                    .is_empty(),
                active_indices.contains(&idx)
            );
        }
        assert_eq!(communicator.round_received(), false);
    }

    #[test]
    fn unknown_device_decide() {
        let participants = prepare_participants(3);
        let mut communicator = new_communicator(
            participants.iter().cloned().take(2).collect(),
            2,
            ProtocolType::Gg18,
        );
        assert_eq!(
            communicator.decide(participants[2].device.identifier(), true),
            false
        );
    }

    #[test]
    fn repeated_device_decide() {
        let participants = prepare_participants(2);
        let mut communicator = new_communicator(participants.clone(), 2, ProtocolType::Gg18);
        assert_eq!(
            communicator.decide(participants[0].device.identifier(), true),
            true
        );
        assert_eq!(
            communicator.decide(participants[0].device.identifier(), true),
            false
        );
    }

    #[test]
    fn repeated_devices() {
        let participants = prepare_participants(1);
        let participants = vec![participants[0].clone(), participants[0].clone()];
        let mut communicator = new_communicator(participants.clone(), 2, ProtocolType::Gg18);
        assert_eq!(
            communicator.decide(participants[0].device.identifier(), true),
            true
        );
        communicator.set_active_devices(None);
        assert_eq!(communicator.get_protocol_indices(), vec![0, 1]);
    }

    #[test]
    #[should_panic]
    fn not_enough_messages() {
        let participants = prepare_participants(3);
        let mut communicator = new_communicator(participants.clone(), 3, ProtocolType::Gg18);
        communicator.decide(participants[0].device.identifier(), true);
        communicator.decide(participants[1].device.identifier(), true);
        communicator.decide(participants[2].device.identifier(), true);
        communicator.set_active_devices(None);
        communicator.receive_messages(
            participants[0].device.identifier(),
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
        let participants = prepare_participants(3);
        let mut communicator = new_communicator(participants.clone(), 3, ProtocolType::Gg18);
        communicator.decide(participants[0].device.identifier(), true);
        communicator.decide(participants[1].device.identifier(), true);
        communicator.decide(participants[2].device.identifier(), true);
        communicator.set_active_devices(None);
        communicator.receive_messages(
            participants[0].device.identifier(),
            vec![ClientMessage {
                protocol_type: 0,
                unicasts: (0..6 as u32).map(|i| (i, vec![])).collect(),
                broadcast: None,
            }],
        );
    }

    #[test]
    #[should_panic]
    fn not_enough_accepts() {
        let participants = prepare_participants(5);
        let mut communicator = new_communicator(participants.clone(), 3, ProtocolType::Gg18);
        communicator.decide(participants[0].device.identifier(), true);
        communicator.decide(participants[2].device.identifier(), false);
        communicator.decide(participants[4].device.identifier(), true);
        communicator.set_active_devices(None);
    }

    #[test]
    fn more_than_threshold_accepts() {
        let threshold = 3;
        let participants = prepare_participants(5);
        let mut communicator =
            new_communicator(participants.clone(), threshold, ProtocolType::Gg18);
        for participant in participants {
            communicator.decide(participant.device.identifier(), true);
        }
        communicator.set_active_devices(None);
        assert_eq!(
            communicator.get_active_devices().as_ref().map(Vec::len),
            Some(threshold as usize)
        );
    }

    #[test]
    fn send_all() {
        let participants = prepare_participants(3);
        let mut communicator = new_communicator(participants.clone(), 2, ProtocolType::Gg18);
        communicator.decide(participants[0].device.identifier(), true);
        communicator.decide(participants[2].device.identifier(), true);
        communicator.set_active_devices(None);
        assert_eq!(
            communicator.get_active_devices(),
            Some(vec![
                participants[0].device.identifier().to_vec(),
                participants[2].device.identifier().to_vec()
            ])
        );
        communicator.send_all(|idx| vec![idx as u8]);
        assert_eq!(
            communicator.get_messages(participants[0].device.identifier()),
            vec![vec![0]]
        );
        assert_eq!(
            communicator.get_messages(participants[1].device.identifier()),
            Vec::<Vec<u8>>::new()
        );
        assert_eq!(
            communicator.get_messages(participants[2].device.identifier()),
            vec![vec![2]]
        );
    }

    #[test]
    fn protocol_init() {
        use meesign_crypto::proto::ProtocolInit;
        let participants = prepare_participants(3);
        let mut communicator = new_communicator(participants.clone(), 2, ProtocolType::Frost);
        communicator.decide(participants[0].device.identifier(), true);
        communicator.decide(participants[2].device.identifier(), true);
        communicator.set_active_devices(None);
        assert_eq!(
            communicator.get_active_devices(),
            Some(vec![
                participants[0].device.identifier().to_vec(),
                participants[2].device.identifier().to_vec()
            ])
        );
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
            communicator.get_messages(participants[0].device.identifier()),
            vec![ProtocolInit {
                protocol_type: ProtocolType::Frost as i32,
                indices: Vec::new(),
                index: 1,
                data: "hello".as_bytes().to_vec(),
            }
            .encode_to_vec()]
        );
        assert_eq!(
            communicator.get_messages(participants[1].device.identifier()),
            Vec::new() as Vec<Vec<u8>>
        );
        assert_eq!(
            communicator.get_messages(participants[2].device.identifier()),
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
        let participants = prepare_participants(3);
        let mut communicator = new_communicator(participants.clone(), 2, ProtocolType::Frost);

        communicator.decide(participants[0].device.identifier(), true);
        communicator.decide(participants[1].device.identifier(), true);
        communicator.decide(participants[2].device.identifier(), false);
        communicator.set_active_devices(None);

        assert_eq!(communicator.get_protocol_indices(), vec![1, 2]);

        for i in 0..2 {
            assert_eq!(
                communicator.receive_messages(
                    participants[i].device.identifier(),
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
        eprintln!("input: {:?}", communicator.input);
        communicator.relay();
        eprintln!("output: {:?}", communicator.output);

        assert_eq!(
            communicator.get_messages(participants[0].device.identifier()),
            vec![ServerMessage {
                protocol_type: ProtocolType::Frost.into(),
                unicasts: HashMap::new(),
                broadcasts: HashMap::from([(2, vec![1])]),
            }
            .encode_to_vec()],
        );
        assert_eq!(
            communicator.get_messages(participants[1].device.identifier()),
            vec![ServerMessage {
                protocol_type: ProtocolType::Frost.into(),
                unicasts: HashMap::new(),
                broadcasts: HashMap::from([(1, vec![0])]),
            }
            .encode_to_vec()],
        );
    }

    fn new_communicator(
        participants: Vec<Participant>,
        threshold: u32,
        protocol_type: ProtocolType,
    ) -> Communicator {
        let decisions = participants
            .iter()
            .map(|p| (p.device.identifier().clone(), 0))
            .collect();
        Communicator::new(participants, threshold, protocol_type, decisions)
    }

    fn prepare_participants(n: usize) -> Vec<Participant> {
        assert!(n < u8::MAX as usize);
        (0..n)
            .map(|i| {
                let device = Device::new(
                    vec![i as u8],
                    format!("d{}", i),
                    DeviceKind::User,
                    vec![0xf0 | i as u8],
                );
                Participant { device, shares: 1 }
            })
            .collect()
    }
}
