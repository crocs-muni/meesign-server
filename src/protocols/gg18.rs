use std::sync::Arc;

use crate::error::Error;
use crate::proto::ProtocolType;
use crate::protocols::Protocol;
use crate::{communicator::Communicator, persistence::Repository};
use async_trait::async_trait;
use meesign_crypto::proto::{Message, ProtocolGroupInit, ProtocolInit};
use meesign_crypto::protocol::gg18 as protocol;
use tokio::sync::RwLockWriteGuard;
use uuid::Uuid;

pub struct GG18Group {
    parties: u32,
    threshold: u32,
    round: u16,
    repository: Arc<Repository>,
    task_id: Uuid,
}

impl GG18Group {
    pub fn new(parties: u32, threshold: u32, repository: Arc<Repository>, task_id: Uuid) -> Self {
        Self {
            parties,
            threshold,
            round: 0,
            repository,
            task_id,
        }
    }

    pub fn from_model(
        parties: u32,
        threshold: u32,
        repository: Arc<Repository>,
        task_id: Uuid,
        round: u16,
    ) -> Self {
        Self {
            parties,
            threshold,
            repository,
            task_id,
            round,
        }
    }

    async fn set_round(&mut self, round: u16) -> Result<(), Error> {
        self.round = round;
        self.repository.set_round(&self.task_id, round).await?;
        Ok(())
    }

    async fn increment_round(&mut self) -> Result<(), Error> {
        self.round += 1;
        self.repository.increment_round(&self.task_id).await?;
        Ok(())
    }
}

#[async_trait]
impl Protocol for GG18Group {
    async fn initialize(
        &mut self,
        mut communicator: RwLockWriteGuard<'_, Communicator>,
        _: &[u8],
    ) -> Result<(), Error> {
        communicator.set_active_devices(None);
        let parties = self.parties;
        let threshold = self.threshold;
        communicator.send_all(|idx| {
            (ProtocolGroupInit {
                protocol_type: ProtocolType::Gg18 as i32,
                index: idx,
                parties,
                threshold,
            })
            .encode_to_vec()
        });

        self.set_round(1).await
    }

    async fn advance(
        &mut self,
        mut communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<(), Error> {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        self.increment_round().await
    }

    async fn finalize(
        &mut self,
        communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<Option<Vec<u8>>, Error> {
        assert_eq!(self.last_round(), self.round);
        self.increment_round().await?;
        Ok(communicator.get_final_message())
    }

    fn round(&self) -> u16 {
        self.round
    }

    fn last_round(&self) -> u16 {
        protocol::KEYGEN_ROUNDS
    }

    fn get_type(&self) -> ProtocolType {
        ProtocolType::Gg18
    }
}

pub struct GG18Sign {
    round: u16,
    repository: Arc<Repository>,
    task_id: Uuid,
}

impl GG18Sign {
    pub fn new(repository: Arc<Repository>, task_id: Uuid) -> Self {
        Self {
            round: 0,
            repository,
            task_id,
        }
    }

    pub fn from_model(repository: Arc<Repository>, task_id: Uuid, round: u16) -> Self {
        Self {
            round,
            repository,
            task_id,
        }
    }

    async fn set_round(&mut self, round: u16) -> Result<(), Error> {
        self.round = round;
        self.repository.set_round(&self.task_id, round).await?;
        Ok(())
    }

    async fn increment_round(&mut self) -> Result<(), Error> {
        self.round += 1;
        self.repository.increment_round(&self.task_id).await?;
        Ok(())
    }
}

#[async_trait]
impl Protocol for GG18Sign {
    async fn initialize(
        &mut self,
        mut communicator: RwLockWriteGuard<'_, Communicator>,
        data: &[u8],
    ) -> Result<(), Error> {
        communicator.set_active_devices(None);
        let participant_indices = communicator.get_protocol_indices();
        communicator.send_all(|idx| {
            (ProtocolInit {
                protocol_type: ProtocolType::Gg18 as i32,
                indices: participant_indices.clone(),
                index: idx,
                data: Vec::from(data),
            })
            .encode_to_vec()
        });

        self.set_round(1).await
    }

    async fn advance(
        &mut self,
        mut communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<(), Error> {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        self.increment_round().await
    }

    async fn finalize(
        &mut self,
        communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<Option<Vec<u8>>, Error> {
        assert_eq!(self.last_round(), self.round);
        self.increment_round().await?;
        Ok(communicator.get_final_message())
    }

    fn round(&self) -> u16 {
        self.round
    }

    fn last_round(&self) -> u16 {
        protocol::SIGN_ROUNDS
    }

    fn get_type(&self) -> ProtocolType {
        ProtocolType::Gg18
    }
}
