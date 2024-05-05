use crate::communicator::Communicator;
use crate::proto::ProtocolType;
use crate::protocols::Protocol;
use meesign_crypto::proto::{Message, ProtocolGroupInit, ProtocolInit};
use meesign_crypto::protocol::gg18 as protocol;
use tokio::sync::RwLockWriteGuard;

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
    fn initialize(&mut self, mut communicator: RwLockWriteGuard<'_, Communicator>, _: &[u8]) {
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

        self.round = 1;
    }

    fn advance(&mut self, mut communicator: RwLockWriteGuard<'_, Communicator>) {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        self.round += 1;
    }

    fn finalize(&mut self, communicator: RwLockWriteGuard<'_, Communicator>) -> Option<Vec<u8>> {
        assert_eq!(self.last_round(), self.round);
        self.round += 1;
        communicator.get_final_message()
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
}

impl GG18Sign {
    pub fn new() -> Self {
        Self { round: 0 }
    }
}

impl Protocol for GG18Sign {
    fn initialize(&mut self, mut communicator: RwLockWriteGuard<'_, Communicator>, data: &[u8]) {
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

        self.round = 1;
    }

    fn advance(&mut self, mut communicator: RwLockWriteGuard<'_, Communicator>) {
        assert!((0..self.last_round()).contains(&self.round));

        communicator.relay();
        self.round += 1;
    }

    fn finalize(&mut self, communicator: RwLockWriteGuard<'_, Communicator>) -> Option<Vec<u8>> {
        assert_eq!(self.last_round(), self.round);
        self.round += 1;
        communicator.get_final_message()
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
