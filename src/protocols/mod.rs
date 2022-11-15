use crate::communicator::Communicator;
use crate::proto::{KeyType, ProtocolType};

pub mod gg18;

impl ProtocolType {
    pub fn check_key_type(self, key_type: KeyType) -> bool {
        match (self, key_type) {
            (ProtocolType::Gg18, KeyType::SignPdf) => true,
            (_, KeyType::SignChallenge) => true,
        }
    }

    pub fn check_threshold(self, threshold: u32, group_size: u32) -> bool {
        match self {
            ProtocolType::Gg18 => threshold >= 2 && threshold <= group_size,
        }
    }
}

pub trait Protocol {
    fn initialize(&mut self, communicator: &mut Communicator, data: &[u8]);
    fn advance(&mut self, communicator: &mut Communicator);
    fn finalize(&mut self, communicator: &mut Communicator) -> Option<Vec<u8>>;
    fn round(&self) -> u16;
    fn last_round(&self) -> u16;
}
