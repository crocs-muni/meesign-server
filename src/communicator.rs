use crate::device::Device;
use crate::proto::Gg18Message;
use prost::Message;
use std::collections::HashMap;

/// Communication state of a Task
pub struct Communicator {
    /// The minimal number of parties needed for task computation
    pub threshold: u32,
    /// Ordered list of devices (should be removed when reimplementing communication)
    device_list: Vec<Device>,
    /// A mapping of device identifiers to their Task decision
    devices: HashMap<Vec<u8>, Option<bool>>,
    /// Incoming messages
    pub input: Vec<Vec<Option<Vec<u8>>>>,
    /// Outgoing messages
    output: Vec<Vec<u8>>,
}

impl Communicator {
    /// Constructs a new Communicator instance with given Devices, threshold, and request message
    pub fn new(devices: &[Device], threshold: u32) -> Self {
        let communicator = Communicator {
            threshold,
            device_list: devices.iter().map(Device::clone).collect(),
            devices: devices
                .iter()
                .map(|x| (x.identifier().to_vec(), Some(false)))
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

    /// Sends a message to all active parties
    pub fn broadcast(&mut self, message: Vec<u8>) {
        self.output.clear();

        for _ in 0..self.threshold {
            self.output.push(message.clone());
        }
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

    /// Check whether incoming buffers contain messages from all active parties
    pub fn round_received(&self, ids: &[Vec<u8>]) -> bool {
        ids.into_iter()
            .zip((&self.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .count()
            == 0
    }

    /// Get message for given party index
    pub fn get_message(&self, idx: usize) -> Option<&Vec<u8>> {
        self.output.get(idx)
    }

    /// Get identifiers of active parties
    pub fn get_participants(&mut self) -> Vec<Vec<u8>> {
        assert!(self.accept_count() >= self.threshold);
        self.devices
            .iter()
            .filter(|(_device, confirmation)| **confirmation == Some(true))
            .take(self.threshold as usize)
            .map(|(device, _confirmation)| device.clone())
            .collect()
    }

    /// Save decision by the given device
    pub fn confirmation(&mut self, device: &[u8], agreement: bool) {
        if !self.devices.contains_key(device) || self.devices[device].is_some() {
            panic!();
        }
        self.devices.insert(device.to_vec(), Some(agreement));
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
    pub fn device_confirmed(&self, device: &[u8]) -> bool {
        if let Some(Some(_)) = self.devices.get(device) {
            true
        } else {
            false
        }
    }

    /// Get indices of active participants
    pub fn get_participant_indices(&mut self) -> Vec<u32> {
        let participant_ids = self.get_participants();
        let mut indices: Vec<u32> = Vec::new();
        for i in 0..participant_ids.len() {
            indices.push(
                self.device_list
                    .iter()
                    .position(|x| x.identifier() == &participant_ids[i])
                    .unwrap() as u32,
            );
        }
        indices
    }

    /// Translate device identifier to index
    pub fn identifier_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.device_list
            .iter()
            .position(|x| x.identifier() == device_id)
    }
}
