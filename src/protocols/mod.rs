use log::warn;

use crate::communicator::Communicator;
use crate::proto::{KeyType, ProtocolType};

pub mod elgamal;
pub mod frost;
pub mod gg18;
pub mod musig2;

impl ProtocolType {
    pub fn check_threshold(self, threshold: u32, group_size: u32) -> bool {
        match self {
            ProtocolType::Gg18 | ProtocolType::Elgamal | ProtocolType::Frost => {
                threshold >= 2 && threshold <= group_size
            }
            ProtocolType::Musig2 => threshold >= 2 && threshold == group_size,
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

/// Instantiates a keygen (group) protocol
pub fn create_keygen_protocol(
    protocol_type: ProtocolType,
    key_type: KeyType,
    total_shares: u32,
    threshold: u32,
    round: u16,
) -> Result<Box<dyn Protocol + Send + Sync>, String> {
    let protocol: Box<dyn Protocol + Send + Sync> =
        match (protocol_type, key_type) {
            (ProtocolType::Gg18, KeyType::SignPdf) => {
                Box::new(gg18::GG18Group::from_model(total_shares, threshold, round))
            }
            (ProtocolType::Gg18, KeyType::SignChallenge) => {
                Box::new(gg18::GG18Group::from_model(total_shares, threshold, round))
            }
            (ProtocolType::Frost, KeyType::SignChallenge) => Box::new(
                frost::FROSTGroup::from_model(total_shares, threshold, round),
            ),
            (ProtocolType::Musig2, KeyType::SignChallenge) => {
                Box::new(musig2::MuSig2Group::from_model(total_shares, round))
            }
            (ProtocolType::Elgamal, KeyType::Decrypt) => Box::new(
                elgamal::ElgamalGroup::from_model(total_shares, threshold, round),
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
    round: u16,
) -> Result<Box<dyn Protocol + Send + Sync>, String> {
    let protocol: Box<dyn Protocol + Send + Sync> = match (protocol_type, key_type) {
        (ProtocolType::Gg18, KeyType::SignPdf) => Box::new(gg18::GG18Sign::from_model(round)),
        (ProtocolType::Gg18, KeyType::SignChallenge) => Box::new(gg18::GG18Sign::from_model(round)),
        (ProtocolType::Frost, KeyType::SignChallenge) => {
            Box::new(frost::FROSTSign::from_model(round))
        }
        (ProtocolType::Musig2, KeyType::SignChallenge) => {
            Box::new(musig2::MuSig2Sign::from_model(round))
        }
        (ProtocolType::Elgamal, KeyType::Decrypt) => {
            Box::new(elgamal::ElgamalDecrypt::from_model(round))
        }
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
