use tokio::sync::RwLockWriteGuard;

use crate::communicator::Communicator;
use crate::proto::ProtocolType;

pub mod elgamal;
pub mod frost;
pub mod gg18;

impl ProtocolType {
    pub fn check_threshold(self, threshold: u32, group_size: u32) -> bool {
        match self {
            ProtocolType::Gg18 | ProtocolType::Elgamal | ProtocolType::Frost => {
                threshold >= 2 && threshold <= group_size
            }
        }
    }
}

pub trait Protocol {
    fn initialize(&mut self, communicator: RwLockWriteGuard<'_, Communicator>, data: &[u8]);
    fn advance(&mut self, communicator: RwLockWriteGuard<'_, Communicator>);
    fn finalize(&mut self, communicator: RwLockWriteGuard<'_, Communicator>) -> Option<Vec<u8>>;
    fn round(&self) -> u16;
    fn last_round(&self) -> u16;
    fn get_type(&self) -> ProtocolType;
}
