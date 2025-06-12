use async_trait::async_trait;
use log::warn;
use std::sync::Arc;
use tokio::sync::RwLockWriteGuard;
use uuid::Uuid;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::persistence::Repository;
use crate::proto::{KeyType, ProtocolType};

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

/// Instantiates a keygen (group) protocol
pub fn create_keygen_protocol(
    protocol_type: ProtocolType,
    key_type: KeyType,
    devices_len: u32,
    threshold: u32,
    repository: Arc<Repository>,
    task_id: Uuid,
    round: u16,
) -> Result<Box<dyn Protocol + Send + Sync>, String> {

    let protocol: Box<dyn Protocol + Send + Sync> = match (protocol_type, key_type) {
        (ProtocolType::Gg18, KeyType::SignPdf) => Box::new(
            gg18::GG18Group::from_model(devices_len, threshold, repository, task_id, round),
        ),
        (ProtocolType::Gg18, KeyType::SignChallenge) => Box::new(
            gg18::GG18Group::from_model(devices_len, threshold, repository, task_id, round),
        ),
        (ProtocolType::Frost, KeyType::SignChallenge) => Box::new(
            frost::FROSTGroup::from_model(devices_len, threshold, repository, task_id, round),
        ),
        (ProtocolType::Elgamal, KeyType::Decrypt) => Box::new(
            elgamal::ElgamalGroup::from_model(devices_len, threshold, repository, task_id, round),
        ),
        _ => {
            warn!(
                "Protocol {:?} does not support {:?} key type",
                protocol_type, key_type
            );
            return Err("Unsupported protocol type and key type combination".into());
        }
    };
    Ok(protocol)
}

/// Instantiates a threshold protocol
pub fn create_threshold_protocol(
    protocol_type: ProtocolType,
    key_type: KeyType,
    repository: Arc<Repository>,
    task_id: Uuid,
    round: u16,
) -> Result<Box<dyn Protocol + Send + Sync>, String> {
    let protocol: Box<dyn Protocol + Send + Sync> = match (protocol_type, key_type) {
        (ProtocolType::Gg18, KeyType::SignPdf) => Box::new(
            gg18::GG18Sign::from_model(repository, task_id, round),
        ),
        (ProtocolType::Gg18, KeyType::SignChallenge) => Box::new(
            gg18::GG18Sign::from_model(repository, task_id, round),
        ),
        (ProtocolType::Frost, KeyType::SignChallenge) => Box::new(
            frost::FROSTSign::from_model(repository, task_id, round),
        ),
        (ProtocolType::Elgamal, KeyType::Decrypt) => Box::new(
            elgamal::ElgamalDecrypt::from_model(repository, task_id, round),
        ),
        _ => {
            warn!(
                "Protocol {:?} does not support {:?} key type",
                protocol_type, key_type
            );
            return Err("Unsupported protocol type and key type combination".into());
        }
    };
    Ok(protocol)
}
