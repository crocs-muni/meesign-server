use crate::communicator::Communicator;
use crate::proto::{Gg18KeyGenInit, Gg18SignInit};
use crate::protocols::Protocol;
use prost::Message;

pub struct GG18Group {
    parties: u32,
    threshold: u32,
    round: u16,
}

impl GG18Group {
    pub fn new(parties: u32, threshold: u32) -> Self {
        GG18Group {
            parties,
            threshold,
            round: 0,
        }
    }
}

impl Protocol for GG18Group {
    fn initialize(&mut self, communicator: &mut Communicator, _: &[u8]) {
        communicator.set_active_devices();
        let parties = self.parties;
        let threshold = self.threshold;
        communicator.send_all(|idx| {
            (Gg18KeyGenInit {
                index: idx as u32,
                parties,
                threshold,
            })
            .encode_to_vec()
        });
        communicator.clear_input();

        self.round += 1;
    }

    fn advance(&mut self, communicator: &mut Communicator) {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        communicator.clear_input();
        self.round += 1;
    }

    fn finalize(&mut self, communicator: &mut Communicator) -> Vec<u8> {
        assert_eq!(self.last_round(), self.round);
        self.round += 1;
        communicator.get_final_message().unwrap()
    }

    fn round(&self) -> u16 {
        self.round
    }

    fn last_round(&self) -> u16 {
        6
    }
}

pub struct GG18Sign {
    round: u16,
}

impl GG18Sign {
    pub fn new() -> Self {
        Self { round: 0 }
    }
}

impl Protocol for GG18Sign {
    fn initialize(&mut self, communicator: &mut Communicator, data: &[u8]) {
        communicator.set_active_devices();
        let participant_indices = communicator.get_protocol_indices();
        communicator.send_all(|idx| {
            (Gg18SignInit {
                indices: participant_indices.clone(),
                index: idx as u32,
                hash: Vec::from(data),
            })
            .encode_to_vec()
        });
        communicator.clear_input();

        self.round += 1;
    }

    fn advance(&mut self, communicator: &mut Communicator) {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        communicator.clear_input();
        self.round += 1;
    }

    fn finalize(&mut self, communicator: &mut Communicator) -> Vec<u8> {
        assert_eq!(self.last_round(), self.round);
        self.round += 1;
        communicator.get_final_message().unwrap()
    }

    fn round(&self) -> u16 {
        self.round
    }

    fn last_round(&self) -> u16 {
        10
    }
}
