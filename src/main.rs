use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use device::Role;
use lazy_static::lazy_static;
use openssl::pkey::{PKey, Private};
use openssl::x509::X509;
use tonic::transport::Certificate;

use crate::state::State;
use tokio::{sync::Mutex, try_join};
use tonic::codegen::Arc;

mod communicator;
mod device;
mod group;
mod interfaces;
mod protocols;
mod state;
mod tasks;

mod proto {
    #![allow(clippy::derive_partial_eq_without_eq)]
    tonic::include_proto!("meesign");

    impl From<meesign_crypto::proto::ProtocolType> for ProtocolType {
        fn from(proto: meesign_crypto::proto::ProtocolType) -> Self {
            match proto {
                meesign_crypto::proto::ProtocolType::Gg18 => ProtocolType::Gg18,
                meesign_crypto::proto::ProtocolType::Elgamal => ProtocolType::Elgamal,
                meesign_crypto::proto::ProtocolType::Frost => ProtocolType::Frost,
            }
        }
    }

    impl From<ProtocolType> for meesign_crypto::proto::ProtocolType {
        fn from(proto: ProtocolType) -> Self {
            match proto {
                ProtocolType::Gg18 => meesign_crypto::proto::ProtocolType::Gg18,
                ProtocolType::Elgamal => meesign_crypto::proto::ProtocolType::Elgamal,
                ProtocolType::Frost => meesign_crypto::proto::ProtocolType::Frost,
            }
        }
    }
}

lazy_static! {
    static ref CA_CERT: X509 =
        X509::from_pem(&std::fs::read("keys/meesign-ca-cert.pem").unwrap()).unwrap();
    static ref CA_KEY: PKey<Private> =
        PKey::private_key_from_pem(&std::fs::read("keys/meesign-ca-key.pem").unwrap()).unwrap();
}

const VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub(self) struct Args {
    #[clap(short, long, default_value_t = 1337)]
    port: u16,

    #[clap(short, long, default_value_t = String::from("127.0.0.1"))]
    addr: String,

    #[clap(short, long, default_value_t = String::from("meesign.local"))]
    host: String,
}

pub fn get_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn cert_to_id(cert: impl AsRef<[u8]>) -> Vec<u8> {
    use sha2::Digest;
    sha2::Sha256::digest(cert).to_vec()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::init();
    let args = Args::parse();

    let admin_cert = Certificate::from_pem(std::fs::read("keys/meesign-admin-cert.pem").unwrap());
    let admin_id = cert_to_id(&admin_cert);

    let mut state = State::new();
    state.add_device(&admin_id, "MeeSign Admin", admin_cert.as_ref(), Role::Admin);

    let state = Arc::new(Mutex::new(state));

    let grpc = interfaces::grpc::run_grpc(state.clone(), &args.addr, args.port);
    let timer = interfaces::timer::run_timer(state);

    try_join!(grpc, timer).map(|_| ())
}
