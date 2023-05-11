use crate::communicator::Communicator;
use crate::proto::{KeyType, ProtocolType};

pub mod elgamal;
pub mod gg18;

impl ProtocolType {
    pub fn check_key_type(self, key_type: KeyType) -> bool {
        match (self, key_type) {
            (ProtocolType::Gg18, KeyType::SignPdf) => true,
            (ProtocolType::Gg18, KeyType::SignChallenge) => true,
            (ProtocolType::Elgamal, KeyType::Decrypt) => true,
            _ => false,
        }
    }

    pub fn check_threshold(self, threshold: u32, group_size: u32) -> bool {
        match self {
            ProtocolType::Gg18 | ProtocolType::Elgamal => threshold >= 2 && threshold <= group_size,
        }
    }
}

pub trait Protocol {
    fn initialize(&mut self, communicator: &mut Communicator, data: &[u8]);
    fn advance(&mut self, communicator: &mut Communicator);
    fn finalize(&mut self, communicator: &mut Communicator) -> Option<Vec<u8>>;
    fn round(&self) -> u16;
    fn last_round(&self) -> u16;
    fn get_type(&self) -> ProtocolType;
}
