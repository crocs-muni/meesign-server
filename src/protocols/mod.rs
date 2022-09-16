use std::collections::HashMap;

use prost::Message;

use crate::device::Device;
use crate::proto::{Gg18Message, KeyType, Protocol};

pub mod gg18;

impl Protocol {
    pub fn check_key_type(self, key_type: KeyType) -> bool {
        match (key_type, self) {
            (KeyType::SignPdf, Protocol::Gg18) => true,
            (KeyType::SignDigest, _) => true,
        }
    }
}

struct Communicator {
    threshold: u32,
    devices: HashMap<Vec<u8>, Option<bool>>,
    request: Vec<u8>,
    input: Vec<Vec<Option<Vec<u8>>>>,
    output: Vec<Vec<u8>>,
}

impl Communicator {
    pub fn new(devices: &[Device], threshold: u32, request: Vec<u8>) -> Self {
        let mut communicator = Communicator {
            threshold,
            devices: devices
                .iter()
                .map(|x| (x.identifier().to_vec(), Some(false)))
                .collect(),
            request,
            input: Vec::new(),
            output: Vec::new(),
        };
        communicator.clear_input();
        communicator
    }

    pub fn clear_input(&mut self) {
        self.input.clear();
        for _ in 0..self.threshold {
            self.input.push(vec![None; self.threshold as usize]);
        }
    }

    pub fn broadcast(&mut self, message: Vec<u8>) {
        self.output.clear();

        for _ in 0..self.threshold {
            self.output.push(message.clone());
        }
    }

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
    }

    pub fn send_all<F>(&mut self, f: F)
    where
        F: Fn(u32) -> Vec<u8>,
    {
        self.output.clear();

        for i in 0..self.threshold {
            self.output.push(f(i));
        }
    }

    pub fn round_received(&self, ids: &[Vec<u8>]) -> bool {
        ids.into_iter()
            .zip((&self.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .count()
            == 0
    }

    pub fn get_message(&self, idx: usize) -> Option<&Vec<u8>> {
        self.output.get(idx)
    }

    pub fn get_participants(&mut self) -> Vec<Vec<u8>> {
        assert!(self.accept_count() >= self.threshold);
        self.devices
            .iter()
            .filter(|(_device, confirmation)| **confirmation == Some(true))
            .take(self.threshold as usize)
            .map(|(device, _confirmation)| device.clone())
            .collect()
    }

    pub fn confirmation(&mut self, device: &[u8], agreement: bool) {
        if !self.devices.contains_key(device) || self.devices[device].is_some() {
            panic!();
        }
        self.devices.insert(device.to_vec(), Some(agreement));
    }

    pub fn accept_count(&self) -> u32 {
        self.devices
            .iter()
            .map(|x| if x.1.unwrap_or(false) { 1 } else { 0 })
            .sum()
    }

    pub fn reject_count(&self) -> u32 {
        self.devices
            .iter()
            .map(|x| if x.1.unwrap_or(true) { 0 } else { 1 })
            .sum()
    }

    pub fn device_confirmed(&self, device: &[u8]) -> bool {
        if let Some(Some(_)) = self.devices.get(device) {
            true
        } else {
            false
        }
    }
}
