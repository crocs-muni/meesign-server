use crate::communicator::Communicator;
use crate::proto::{KeyType, ProtocolType};

pub mod gg18;

impl ProtocolType {
    pub fn check_key_type(self, key_type: KeyType) -> bool {
        match (key_type, self) {
            (KeyType::SignPdf, ProtocolType::Gg18) => true,
            (KeyType::SignDigest, _) => true,
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
