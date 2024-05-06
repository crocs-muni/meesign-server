use async_trait::async_trait;
use tokio::sync::RwLockWriteGuard;

use crate::communicator::Communicator;
use crate::error::Error;
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

#[async_trait]
pub trait Protocol {
    async fn initialize(
        &mut self,
        communicator: RwLockWriteGuard<'_, Communicator>,
        data: &[u8],
    ) -> Result<(), Error>;
    async fn advance(
        &mut self,
        communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<(), Error>;
    async fn finalize(
        &mut self,
        communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<Option<Vec<u8>>, Error>;
    fn round(&self) -> u16;
    fn last_round(&self) -> u16;
    fn get_type(&self) -> ProtocolType;
}
