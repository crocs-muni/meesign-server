use crate::communicator::Communicator;
use crate::error::Error;
use crate::proto::ProtocolType;
use crate::protocols::Protocol;
use async_trait::async_trait;
use meesign_crypto::proto::{Message, ProtocolGroupInit, ProtocolInit};
use meesign_crypto::protocol::frost as protocol;
use tokio::sync::RwLockWriteGuard;

pub struct FROSTGroup {
    parties: u32,
    threshold: u32,
    round: u16,
}

impl FROSTGroup {
    pub fn new(parties: u32, threshold: u32) -> Self {
        Self {
            parties,
            threshold,
            round: 0,
        }
    }
}

#[async_trait]
impl Protocol for FROSTGroup {
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
                protocol_type: meesign_crypto::proto::ProtocolType::Frost as i32,
                index: idx,
                parties,
                threshold,
            })
            .encode_to_vec()
        });

        self.round = 1; // TODO
        Ok(())
    }

    async fn advance(
        &mut self,
        mut communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<(), Error> {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        self.round += 1;
        Ok(())
    }

    async fn finalize(
        &mut self,
        communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<Option<Vec<u8>>, Error> {
        assert_eq!(self.last_round(), self.round);
        self.round += 1;
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
}

impl FROSTSign {
    pub fn new() -> Self {
        Self { round: 0 }
    }
}

#[async_trait]
impl Protocol for FROSTSign {
    async fn initialize(
        &mut self,
        mut communicator: RwLockWriteGuard<'_, Communicator>,
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

        self.round = 1;
        Ok(())
    }

    async fn advance(
        &mut self,
        mut communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<(), Error> {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        self.round += 1;
        Ok(())
    }

    async fn finalize(
        &mut self,
        communicator: RwLockWriteGuard<'_, Communicator>,
    ) -> Result<Option<Vec<u8>>, Error> {
        assert_eq!(self.last_round(), self.round);
        self.round += 1;
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
