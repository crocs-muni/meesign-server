use crate::communicator::Communicator;
use crate::error::Error;
use crate::persistence::Repository;
use crate::proto::ProtocolType;
use crate::protocols::Protocol;
use async_trait::async_trait;
use meesign_crypto::proto::{Message, ProtocolGroupInit, ProtocolInit};
use meesign_crypto::protocol::frost as protocol;
use std::sync::Arc;
use uuid::Uuid;

pub struct FROSTGroup {
    parties: u32,
    threshold: u32,
    round: u16,
    repository: Arc<Repository>,
    task_id: Uuid,
}

impl FROSTGroup {
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
impl Protocol for FROSTGroup {
    async fn initialize(&mut self, communicator: &mut Communicator, _: &[u8]) -> Result<(), Error> {
        communicator.set_active_devices(None);
        let parties = self.parties;
        let threshold = self.threshold;
        communicator.send_all(|idx| {
            (ProtocolGroupInit {
                protocol_type: meesign_crypto::proto::ProtocolType::Frost as i32,
                index: idx,
                parties,
                threshold,
            })
            .encode_to_vec()
        });

        self.set_round(1).await
    }

    async fn advance(&mut self, communicator: &mut Communicator) -> Result<(), Error> {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        self.increment_round().await
    }

    async fn finalize(
        &mut self,
        communicator: &mut Communicator,
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
        ProtocolType::Frost
    }
}

pub struct FROSTSign {
    round: u16,
    repository: Arc<Repository>,
    task_id: Uuid,
}

impl FROSTSign {
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
impl Protocol for FROSTSign {
    async fn initialize(
        &mut self,
        communicator: &mut Communicator,
        data: &[u8],
    ) -> Result<(), Error> {
        communicator.set_active_devices(None);
        let participant_indices = communicator.get_protocol_indices();
        communicator.send_all(|idx| {
            (ProtocolInit {
                protocol_type: meesign_crypto::proto::ProtocolType::Frost as i32,
                indices: participant_indices.clone(),
                index: idx,
                data: Vec::from(data),
            })
            .encode_to_vec()
        });

        self.set_round(1).await
    }

    async fn advance(&mut self, communicator: &mut Communicator) -> Result<(), Error> {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        self.increment_round().await
    }

    async fn finalize(
        &mut self,
        communicator: &mut Communicator,
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
        ProtocolType::Frost
    }
}
