use crate::device::Device;
use crate::proto::Gg18Message;
use prost::Message;
use std::collections::HashMap;

/// Communication state of a Task
pub struct Communicator {
    /// The minimal number of parties needed for task computation
    threshold: u32,
    /// Ordered list of devices (should be removed when reimplementing communication)
    device_list: Vec<Device>,
    /// Ordered list of active devices (participating in the protocol)
    active_devices: Option<Vec<Vec<u8>>>,
    /// A mapping of device identifiers to their Task decision
    devices: HashMap<Vec<u8>, Option<bool>>,
    /// Incoming messages
    input: Vec<Vec<Option<Vec<u8>>>>,
    /// Outgoing messages
    output: Vec<Vec<u8>>,
}

impl Communicator {
    /// Constructs a new Communicator instance with given Devices, threshold, and request message
    pub fn new(devices: &[Device], threshold: u32) -> Self {
        let communicator = Communicator {
            threshold,
            device_list: devices.iter().map(Device::clone).collect(),
            active_devices: None,
            devices: devices
                .iter()
                .map(|x| (x.identifier().to_vec(), None))
                .collect(),
            input: Vec::new(),
            output: Vec::new(),
        };
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

    /// Was message received from a given device identifier
    ///
    /// In case no message is expected from the given device, returns `true`.
    pub fn device_responded(&self, device_identifier: &[u8]) -> bool {
        let device_index = self.identifier_to_index(device_identifier);
        if device_index.is_none() {
            return true;
        }
        let device_index = device_index.unwrap();

        self.input
            .get(device_index)
            .and_then(|messages| Some(messages.iter().any(Option::is_some)))
            .unwrap_or(false)
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
            let message = Gg18Message { message: out };
            self.output.push(message.encode_to_vec());
        }
        self.clear_input();
    }

    /// Sends a message to all active parties that can be parametrized by their index
    pub fn send_all<F>(&mut self, f: F)
    where
        F: Fn(u32) -> Vec<u8>,
    {
        self.output.clear();

        for i in 0..self.threshold {
            self.output.push(f(i));
        }
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
            .zip((&self.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
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
        self.active_devices = Some(
            self.devices
                .iter()
                .filter(|(_device, decision)| **decision == Some(true))
                .take(self.threshold as usize)
                .map(|(device, _decision)| device.clone())
                .collect(),
        );
        self.active_devices.as_ref().unwrap().clone()
    }

    /// Get active devices
    pub fn get_active_devices(&self) -> Option<Vec<Vec<u8>>> {
        self.active_devices.clone()
    }

    /// Save decision by the given device
    pub fn decide(&mut self, device: &[u8], decision: bool) {
        if !self.devices.contains_key(device) || self.devices[device].is_some() {
            panic!();
        }
        self.devices.insert(device.to_vec(), Some(decision));
    }

    /// Get the number of Task accepts
    pub fn accept_count(&self) -> u32 {
        self.devices
            .iter()
            .map(|x| if x.1.unwrap_or(false) { 1 } else { 0 })
            .sum()
    }

    /// Get the number of Task rejects
    pub fn reject_count(&self) -> u32 {
        self.devices
            .iter()
            .map(|x| if x.1.unwrap_or(true) { 0 } else { 1 })
            .sum()
    }

    /// Check whether a device submitted its decision
    pub fn device_decided(&self, device: &[u8]) -> bool {
        if let Some(Some(_)) = self.devices.get(device) {
            true
        } else {
            false
        }
    }

    /// Get indices of active devices in the corresponding group
    ///
    /// Indices returned by this command are supposed to correspond to key share indices of active.
    pub fn get_protocol_indices(&mut self) -> Vec<u32> {
        assert!(self.active_devices.is_some());

        let active_devices = self.get_active_devices().unwrap();
        let mut indices: Vec<u32> = Vec::new();
        for i in 0..active_devices.len() {
            indices.push(
                self.device_list
                    .iter()
                    .position(|x| x.identifier() == &active_devices[i])
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
