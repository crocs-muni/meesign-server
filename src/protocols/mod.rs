pub mod gg18;
use prost::Message;
use crate::proto::Gg18Message;
use std::collections::HashMap;
use crate::device::Device;

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum ProtocolType {
    GG18,
}

struct Communicator {
    parties: usize,
    devices: HashMap<Vec<u8>, Option<bool>>,
    request: Vec<u8>,
    input: Vec<Vec<Option<Vec<u8>>>>,
    output: Vec<Vec<u8>>,
}

impl Communicator {
    pub fn new(devices: &[Device], request: Vec<u8>) -> Self {
        let mut communicator = Communicator {
            parties: devices.len(),
            devices: devices.iter().map(|x| (x.identifier().to_vec(), Some(false))).collect(),
            request,
            input: Vec::new(),
            output: Vec::new(),
        };
        communicator.clear_input();
        communicator
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        for _ in 0..self.parties {
            self.input.push(vec![None; self.parties]);
        }
    }

    pub fn broadcast(&mut self, message: Vec<u8>) {
        self.output.clear();

        for _ in 0..self.parties {
            self.output.push(message.clone());
        }
    }

    pub fn relay(&mut self) {
        self.output.clear();

        for i in 0..self.parties {
            let mut out: Vec<Vec<u8>> = Vec::new();
            for j in 0..self.parties {
                if i != j {
                    out.push(self.input[j][i].clone().unwrap());
                }
            }
            let message = Gg18Message { message: out };
            self.output.push(message.encode_to_vec());
        }
    }

    pub fn send_all<F>(&mut self, f: F) where F: Fn(usize) -> Vec<u8> {
        self.output.clear();

        for i in 0..self.parties {
            self.output.push(f(i));
        }
    }

    pub fn round_received(&self, ids: &[Vec<u8>]) -> bool {
        ids.into_iter()
            .zip((&self.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .count() == 0
    }

    pub fn get_message(&self, idx: usize) -> Option<&Vec<u8>> {
        self.output.get(idx)
    }

    pub fn confirmation(&mut self, device: &[u8], agreement: bool) {
        if !self.devices.contains_key(device) || self.devices[device].is_some() {
            panic!();
        }
        self.devices.insert(device.to_vec(), Some(agreement));
    }

    pub fn accept_count(&self) -> usize {
        self.devices.iter().map(|x| if x.1.unwrap_or(false) { 1 } else { 0 }).sum()
    }

    pub fn reject_count(&self) -> usize {
        self.devices.iter().map(|x| if x.1.unwrap_or(true) { 0 } else { 1 }).sum()
    }
}
