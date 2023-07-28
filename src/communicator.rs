use crate::device::Device;
use crate::get_timestamp;
use crate::proto::ProtocolType;
use meesign_crypto::proto::ProtocolMessage;
use prost::Message;
use rand::prelude::SliceRandom;
use rand::thread_rng;
use std::collections::HashMap;
use std::ops::Deref;
use tonic::codegen::Arc;

/// Communication state of a Task
pub struct Communicator {
    /// The minimal number of parties needed to successfully complete the task
    threshold: u32,
    /// Ordered list of devices
    device_list: Vec<Arc<Device>>,
    /// Ordered list of active devices (participating in the protocol)
    active_devices: Option<Vec<Vec<u8>>>,
    /// A mapping of device identifiers to their Task decision
    decisions: HashMap<Vec<u8>, Option<bool>>,
    /// A mapping of device identifiers to their Task acknowledgement
    acknowledgements: HashMap<Vec<u8>, bool>,
    /// Incoming messages
    input: Vec<Vec<Option<Vec<u8>>>>,
    /// Outgoing messages
    output: Vec<Vec<u8>>,
    /// Relayed protocol type
    protocol_type: ProtocolType,
}

impl Communicator {
    /// Constructs a new Communicator instance with given Devices, threshold, and request message
    ///
    /// # Arguments
    /// * `devices` - Sorted list of devices; items of the list need to be unique
    /// * `threshold` - The minimal number of devices to successfully complete the task
    pub fn new(devices: &[Arc<Device>], threshold: u32, protocol_type: ProtocolType) -> Self {
        assert!(devices.len() > 1);
        assert!(threshold <= devices.len() as u32);
        // TODO uncomment once is_sorted is stabilized
        // assert!(devices.is_sorted());

        let mut communicator = Communicator {
            threshold,
            device_list: devices.iter().map(Arc::clone).collect(),
            active_devices: None,
            decisions: devices
                .iter()
                .map(|x| (x.identifier().to_vec(), None))
                .collect(),
            acknowledgements: devices
                .iter()
                .map(|x| (x.identifier().to_vec(), false))
                .collect(),
            input: Vec::new(),
            output: Vec::new(),
            protocol_type,
        };
        communicator.clear_input();
        communicator
    }

    /// Clears incoming message buffers
    pub fn clear_input(&mut self) {
        self.input.clear();
        for _ in 0..self.threshold {
            self.input.push(vec![None; self.threshold as usize]);
        }
    }

    /// Receive messages from a given device identifier
    ///
    /// # Arguments
    ///
    /// * `from_identifier` - identifier of device from which is this broadcast received
    /// * `message` - vector of length (threshold - 1) containing messages for other parties, sending party is excluded
    pub fn receive_messages(&mut self, from_identifier: &[u8], message: Vec<Vec<u8>>) -> bool {
        assert_eq!(message.len(), (self.threshold - 1) as usize);

        let from_index = self.identifier_to_index(from_identifier);
        if from_index.is_none() {
            return false;
        }
        let from_index = from_index.unwrap();

        self.input[from_index] = message.into_iter().map(Some).collect();
        self.input[from_index].insert(from_index, None);
        true
    }

    /// Is waiting for a message from the given device identifier
    pub fn waiting_for(&self, device_identifier: &[u8]) -> bool {
        let device_index = self.identifier_to_index(device_identifier);
        if device_index.is_none() {
            return false;
        }
        let device_index = device_index.unwrap();

        self.input
            .get(device_index)
            .map(|messages| !messages.iter().any(Option::is_some))
            .unwrap_or(true)
    }

    /// Moves messages from incoming buffers to outgoing buffers
    pub fn relay(&mut self) {
        self.output.clear();

        for i in 0..self.threshold {
            let mut out: Vec<Vec<u8>> = Vec::new();
            for j in 0..self.threshold {
                if i != j {
                    out.push(self.input[j as usize][i as usize].clone().unwrap());
                }
            }
            let message = ProtocolMessage {
                protocol_type: meesign_crypto::proto::ProtocolType::from(self.protocol_type) as i32,
                message: out,
            };
            self.output.push(Message::encode_to_vec(&message));
        }

        self.clear_input();
    }

    /// Sends a message to all active devices that can be parametrized by their share index
    pub fn send_all<F>(&mut self, f: F)
    where
        F: Fn(u32) -> Vec<u8>,
    {
        self.output.clear();
        let indices = self.get_protocol_indices();

        for i in 0..self.threshold as usize {
            self.output.push(f(indices[i]));
        }

        self.clear_input();
    }

    /// Check whether incoming buffers contain messages from all active devices
    pub fn round_received(&self) -> bool {
        if self.active_devices.is_none() {
            return false;
        }

        self.active_devices
            .as_ref()
            .unwrap()
            .iter()
            .zip(self.input.iter())
            .filter(|(_a, b)| b.iter().all(Option::is_none))
            .count()
            == 0
    }

    /// Get message for given device identifier
    pub fn get_message(&self, device_identifier: &[u8]) -> Option<Vec<u8>> {
        self.identifier_to_index(device_identifier)
            .and_then(|device_identifier| self.output.get(device_identifier))
            .map(Vec::clone)
    }

    /// Get final message
    pub fn get_final_message(&self) -> Option<Vec<u8>> {
        self.input[0][1].clone()
    }

    /// Set active devices
    pub fn set_active_devices(&mut self) -> Vec<Vec<u8>> {
        assert!(self.accept_count() >= self.threshold);
        let agreeing_devices = self
            .device_list
            .iter()
            .filter(|device| self.decisions.get(device.identifier()) == Some(&Some(true)))
            .collect::<Vec<_>>();

        let timestamp = get_timestamp();
        let connected_devices = agreeing_devices
            .iter()
            .filter(|device| device.last_active() > timestamp - 5)
            .map(Deref::deref)
            .collect::<Vec<_>>();

        let (devices, indices): (&Vec<&Arc<Device>>, Vec<_>) =
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

    /// Get active devices
    pub fn get_active_devices(&self) -> Option<Vec<Vec<u8>>> {
        self.active_devices.clone()
    }

    /// Save decision by the given device; return true if successful
    pub fn decide(&mut self, device: &[u8], decision: bool) -> bool {
        if !self.decisions.contains_key(device) || self.decisions[device].is_some() {
            return false;
        }
        self.decisions.insert(device.to_vec(), Some(decision));
        true
    }

    /// Get the number of Task accepts
    pub fn accept_count(&self) -> u32 {
        self.decisions
            .iter()
            .map(|x| u32::from(x.1.unwrap_or(false)))
            .sum()
    }

    /// Get the number of Task rejects
    pub fn reject_count(&self) -> u32 {
        self.decisions
            .iter()
            .map(|x| u32::from(!x.1.unwrap_or(true)))
            .sum()
    }

    /// Check whether a device submitted its decision
    pub fn device_decided(&self, device: &[u8]) -> bool {
        matches!(self.decisions.get(device), Some(Some(_)))
    }

    /// Save acknowledgement by the given device; return true if successful
    pub fn acknowledge(&mut self, device: &[u8]) -> bool {
        if !self.acknowledgements.contains_key(device) || self.acknowledgements[device] {
            return false;
        }
        self.acknowledgements.insert(device.to_vec(), true);
        true
    }

    /// Check whether a device acknowledged task output
    pub fn device_acknowledged(&self, device: &[u8]) -> bool {
        *self.acknowledgements.get(device).unwrap_or(&false)
    }

    /// Get indices of active devices in the corresponding group
    ///
    /// Indices returned by this command correspond to key share indices of `active_devices`.
    pub fn get_protocol_indices(&mut self) -> Vec<u32> {
        assert!(self.active_devices.is_some());

        let active_devices = self.get_active_devices().unwrap();
        let mut indices: Vec<u32> = Vec::new();
        for device in &active_devices {
            indices.push(
                self.device_list
                    .iter()
                    .position(|x| x.identifier() == device)
                    .unwrap() as u32,
            );
        }
        indices
    }

    /// Translate device identifier to `active_devices` index
    fn identifier_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.active_devices
            .as_ref()
            .and_then(|list| list.iter().position(|id| id == device_id))
    }
}

#[cfg(test)]
mod tests {
    use crate::device::Role;

    use super::*;

    #[test]
    #[should_panic]
    fn communicator_with_no_devices() {
        Communicator::new(&[], 0, ProtocolType::Gg18);
    }

    #[test]
    #[should_panic]
    fn communicator_too_large_threshold() {
        Communicator::new(&prepare_devices(2), 3, ProtocolType::Gg18);
    }

    #[test]
    fn empty_communicator() {
        let devices = prepare_devices(5);
        let d0 = devices[0].identifier();
        let communicator = Communicator::new(&devices, 3, ProtocolType::Gg18);
        assert_eq!(communicator.accept_count(), 0);
        assert_eq!(communicator.reject_count(), 0);
        assert_eq!(communicator.round_received(), false);
        assert_eq!(communicator.get_message(d0), None);
        assert_eq!(communicator.get_message(&[0x00, 0x00]), None);
        assert_eq!(communicator.device_decided(d0), false);
        assert_eq!(communicator.device_decided(&[0x00, 0x00]), false);
        assert_eq!(communicator.waiting_for(d0), false);
        assert_eq!(communicator.waiting_for(&[0x00, 0x00]), false);
        assert_eq!(communicator.get_active_devices(), None);
        assert_eq!(communicator.get_final_message(), None);
    }

    #[test]
    fn valid_communicator() {
        let devices = prepare_devices(5);
        let mut communicator = Communicator::new(&devices, 3, ProtocolType::Gg18);
        assert_eq!(communicator.device_decided(devices[0].identifier()), false);
        communicator.decide(devices[0].identifier(), true);
        assert_eq!(communicator.accept_count(), 1);
        assert_eq!(communicator.reject_count(), 0);
        assert_eq!(communicator.device_decided(devices[0].identifier()), true);
        assert_eq!(communicator.device_decided(devices[2].identifier()), false);
        communicator.decide(devices[2].identifier(), false);
        assert_eq!(communicator.accept_count(), 1);
        assert_eq!(communicator.reject_count(), 1);
        assert_eq!(communicator.device_decided(devices[2].identifier()), true);
        assert_eq!(communicator.device_decided(devices[4].identifier()), false);
        communicator.decide(devices[4].identifier(), true);
        assert_eq!(communicator.accept_count(), 2);
        assert_eq!(communicator.reject_count(), 1);
        assert_eq!(communicator.device_decided(devices[4].identifier()), true);
        assert_eq!(communicator.device_decided(devices[1].identifier()), false);
        communicator.decide(devices[1].identifier(), true);
        assert_eq!(communicator.accept_count(), 3);
        assert_eq!(communicator.reject_count(), 1);
        assert_eq!(communicator.device_decided(devices[1].identifier()), true);
        assert_eq!(communicator.device_decided(devices[3].identifier()), false);
        assert_eq!(communicator.get_active_devices(), None);
        communicator.set_active_devices();
        let active_indices = [0, 1, 4];
        assert_eq!(
            communicator.get_active_devices(),
            Some(
                active_indices
                    .iter()
                    .map(|idx| devices[*idx].identifier().to_vec())
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

        for idx in 0..devices.len() {
            assert_eq!(
                communicator.waiting_for(devices[idx].identifier()),
                active_indices.contains(&idx)
            );
        }
        assert_eq!(communicator.round_received(), false);
        for idx in 0..devices.len() {
            assert_eq!(
                communicator.receive_messages(
                    devices[idx].identifier(),
                    vec![vec![]; active_indices.len() - 1]
                ),
                active_indices.contains(&idx)
            );
            assert_eq!(
                communicator.round_received(),
                idx >= active_indices[active_indices.len() - 1]
            );
        }
        assert_eq!(communicator.round_received(), true);

        for idx in 0..devices.len() {
            assert_eq!(communicator.get_message(devices[idx].identifier()), None);
        }
        communicator.relay();
        for idx in 0..devices.len() {
            assert_eq!(
                communicator
                    .get_message(devices[idx].identifier())
                    .is_some(),
                active_indices.contains(&idx)
            );
        }
        assert_eq!(communicator.round_received(), false);
        for device in devices {
            assert_eq!(communicator.device_acknowledged(device.identifier()), false);
            assert_eq!(communicator.acknowledge(device.identifier()), true);
            assert_eq!(communicator.device_acknowledged(device.identifier()), true);
        }
    }

    #[test]
    fn unknown_device_decide() {
        let devices = prepare_devices(3);
        let mut communicator = Communicator::new(&devices[..2], 2, ProtocolType::Gg18);
        assert_eq!(communicator.decide(devices[2].identifier(), true), false);
    }

    #[test]
    fn repeated_device_decide() {
        let devices = prepare_devices(2);
        let mut communicator = Communicator::new(&devices, 2, ProtocolType::Gg18);
        assert_eq!(communicator.decide(devices[0].identifier(), true), true);
        assert_eq!(communicator.decide(devices[0].identifier(), true), false);
    }

    #[test]
    #[should_panic]
    fn not_enough_messages() {
        let devices = prepare_devices(3);
        let mut communicator = Communicator::new(&devices, 3, ProtocolType::Gg18);
        communicator.decide(devices[0].identifier(), true);
        communicator.decide(devices[1].identifier(), true);
        communicator.receive_messages(devices[0].identifier(), vec![vec![]; 1]);
    }

    #[test]
    #[should_panic]
    fn too_many_messages() {
        let devices = prepare_devices(3);
        let mut communicator = Communicator::new(&devices, 3, ProtocolType::Gg18);
        communicator.decide(devices[0].identifier(), true);
        communicator.decide(devices[1].identifier(), true);
        communicator.receive_messages(devices[0].identifier(), vec![vec![]; 3]);
    }

    #[test]
    #[should_panic]
    fn not_enough_accepts() {
        let devices = prepare_devices(5);
        let mut communicator = Communicator::new(&devices, 3, ProtocolType::Gg18);
        communicator.decide(devices[0].identifier(), true);
        communicator.decide(devices[2].identifier(), false);
        communicator.decide(devices[4].identifier(), true);
        communicator.set_active_devices();
    }

    #[test]
    fn more_than_threshold_accepts() {
        let threshold = 3;
        let devices = prepare_devices(5);
        let mut communicator = Communicator::new(&devices, threshold, ProtocolType::Gg18);
        for device in devices {
            communicator.decide(device.identifier(), true);
        }
        communicator.set_active_devices();
        assert_eq!(
            communicator.get_active_devices().as_ref().map(Vec::len),
            Some(threshold as usize)
        );
    }

    #[test]
    fn send_all() {
        let devices = prepare_devices(3);
        let mut communicator = Communicator::new(&devices, 2, ProtocolType::Gg18);
        communicator.decide(devices[0].identifier(), true);
        communicator.decide(devices[2].identifier(), true);
        communicator.set_active_devices();
        assert_eq!(
            communicator.get_active_devices(),
            Some(vec![
                devices[0].identifier().to_vec(),
                devices[2].identifier().to_vec()
            ])
        );
        communicator.send_all(|idx| vec![idx as u8]);
        assert_eq!(
            communicator.get_message(devices[0].identifier()),
            Some(vec![0])
        );
        assert_eq!(communicator.get_message(devices[1].identifier()), None);
        assert_eq!(
            communicator.get_message(devices[2].identifier()),
            Some(vec![2])
        );
    }

    #[test]
    fn unknown_device_acknowledgement() {
        let devices = prepare_devices(3);
        let mut communicator = Communicator::new(&devices[..2], 2, ProtocolType::Gg18);
        assert_eq!(communicator.acknowledge(devices[2].identifier()), false);
    }

    #[test]
    fn repeated_device_acknowledgement() {
        let devices = prepare_devices(2);
        let mut communicator = Communicator::new(&devices, 2, ProtocolType::Gg18);
        assert_eq!(communicator.acknowledge(devices[0].identifier()), true);
        assert_eq!(communicator.acknowledge(devices[0].identifier()), false);
    }

    fn prepare_devices(n: usize) -> Vec<Arc<Device>> {
        assert!(n < u8::MAX as usize);
        (0..n)
            .map(|i| {
                Arc::new(Device::new(
                    vec![i as u8],
                    format!("d{}", i),
                    vec![0xf0 | i as u8],
                    Role::User,
                ))
            })
            .collect()
    }
}
