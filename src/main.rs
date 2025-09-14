use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;
use dotenvy::dotenv;
use lazy_static::lazy_static;
use openssl::pkey::{PKey, Private};
use openssl::x509::X509;
use persistence::Repository;

use crate::state::State;
use tokio::try_join;
use tonic::codegen::Arc;

mod communicator;
mod error;
mod group;
mod interfaces;
mod persistence;
mod protocols;
mod state;
mod task_store;
mod tasks;
mod utils;

mod proto {
    #![allow(clippy::derive_partial_eq_without_eq)]
    tonic::include_proto!("meesign");
    use crate::persistence::Group as GroupModel;
    pub(crate) use mee_sign_client::MeeSignClient;
    pub(crate) use mee_sign_server::MeeSign;
    pub(crate) use mee_sign_server::MeeSignServer;

    impl From<meesign_crypto::proto::ProtocolType> for ProtocolType {
        fn from(proto: meesign_crypto::proto::ProtocolType) -> Self {
            match proto {
                meesign_crypto::proto::ProtocolType::Gg18 => ProtocolType::Gg18,
                meesign_crypto::proto::ProtocolType::Elgamal => ProtocolType::Elgamal,
                meesign_crypto::proto::ProtocolType::Frost => ProtocolType::Frost,
                meesign_crypto::proto::ProtocolType::Musig2 => ProtocolType::Musig2,
            }
        }
    }

    impl From<ProtocolType> for meesign_crypto::proto::ProtocolType {
        fn from(proto: ProtocolType) -> Self {
            match proto {
                ProtocolType::Gg18 => meesign_crypto::proto::ProtocolType::Gg18,
                ProtocolType::Elgamal => meesign_crypto::proto::ProtocolType::Elgamal,
                ProtocolType::Frost => meesign_crypto::proto::ProtocolType::Frost,
                ProtocolType::Musig2 => meesign_crypto::proto::ProtocolType::Musig2,
            }
        }
    }

    impl ProtocolType {
        pub fn index_offset(&self) -> u32 {
            match self {
                ProtocolType::Gg18 | ProtocolType::Elgamal | ProtocolType::Musig2 => 0,
                ProtocolType::Frost => 1,
            }
        }
    }

    impl From<TaskType> for crate::persistence::TaskType {
        fn from(task_type: TaskType) -> Self {
            match task_type {
                TaskType::Group => Self::Group,
                TaskType::SignChallenge => Self::SignChallenge,
                TaskType::SignPdf => Self::SignPdf,
                TaskType::Decrypt => Self::Decrypt,
            }
        }
    }

    impl Into<TaskType> for crate::persistence::TaskType {
        fn into(self) -> TaskType {
            match self {
                Self::Group => TaskType::Group,
                Self::SignChallenge => TaskType::SignChallenge,
                Self::SignPdf => TaskType::SignPdf,
                Self::Decrypt => TaskType::Decrypt,
            }
        }
    }

    impl Into<DeviceKind> for crate::persistence::DeviceKind {
        fn into(self) -> DeviceKind {
            match self {
                Self::User => DeviceKind::User,
                Self::Bot => DeviceKind::Bot,
            }
        }
    }

    impl Task {
        pub fn created(
            id: Vec<u8>,
            r#type: i32,
            accept: u32,
            reject: u32,
            request: Option<Vec<u8>>,
            attempt: u32,
        ) -> Self {
            Self {
                id,
                r#type,
                state: task::TaskState::Created.into(),
                round: 0,
                accept,
                reject,
                data: Vec::new(),
                request,
                attempt,
            }
        }
        pub fn declined(
            id: Vec<u8>,
            r#type: i32,
            accept: u32,
            reject: u32,
            request: Option<Vec<u8>>,
            attempt: u32,
        ) -> Self {
            Self {
                id,
                r#type,
                state: task::TaskState::Failed.into(),
                round: 0,
                accept,
                reject,
                data: vec!["Task declined".to_string().into_bytes()],
                request,
                attempt,
            }
        }
        pub fn running(
            id: Vec<u8>,
            r#type: i32,
            round: u32,
            data: Vec<Vec<u8>>,
            request: Option<Vec<u8>>,
            attempt: u32,
        ) -> Self {
            Self {
                id,
                r#type,
                state: task::TaskState::Running.into(),
                round,
                accept: u16::MAX as u32,
                reject: u16::MAX as u32,
                data,
                request,
                attempt,
            }
        }
        pub fn finished(
            id: Vec<u8>,
            r#type: i32,
            result: Vec<u8>,
            request: Option<Vec<u8>>,
            attempt: u32,
        ) -> Self {
            Self {
                id,
                r#type,
                state: task::TaskState::Finished.into(),
                round: u16::MAX as u32,
                accept: u16::MAX as u32,
                reject: u16::MAX as u32,
                data: vec![result],
                request,
                attempt,
            }
        }
        pub fn failed(
            id: Vec<u8>,
            r#type: i32,
            reason: String,
            request: Option<Vec<u8>>,
            attempt: u32,
        ) -> Self {
            Self {
                id,
                r#type,
                state: task::TaskState::Failed.into(),
                round: u32::MAX,
                accept: u32::MAX,
                reject: 0,
                data: vec![reason.into_bytes()],
                request,
                attempt,
            }
        }
    }

    impl Group {
        pub fn from_model(model: GroupModel) -> Self {
            let protocol: crate::proto::ProtocolType = model.protocol.into();
            let key_type: crate::proto::KeyType = model.key_type.into();
            let device_ids = model
                .participant_ids_shares
                .into_iter()
                .map(|(device_id, _)| device_id)
                .collect();
            Self {
                identifier: model.id,
                name: model.name,
                threshold: model.threshold as u32,
                protocol: protocol.into(),
                key_type: key_type.into(),
                note: model.note,
                device_ids,
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
struct Args {
    #[clap(short, long, default_value_t = 1337)]
    port: u16,

    #[clap(short, long, default_value_t = String::from("127.0.0.1"))]
    addr: String,

    #[clap(short, long, default_value_t = String::from("meesign.local"))]
    host: String,

    #[cfg(feature = "cli")]
    #[clap(subcommand)]
    command: Option<cli::Commands>,
}

pub fn get_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::init();
    let args = Args::parse();

    #[cfg(feature = "cli")]
    if args.command.is_some() {
        return cli::handle_command(args).await;
    }
    let _ = dotenv();
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let repo = Repository::from_url(&database_url)
        .await
        .expect("Coudln't init postgres repo");
    repo.apply_migrations().expect("Couldn't apply migrations");
    let state = State::restore(Arc::new(repo))
        .await
        .expect("Couldn't initialize State");
    // TODO: remove mutex when DB done
    let state = Arc::new(state);

    let grpc = interfaces::grpc::run_grpc(state.clone(), &args.addr, args.port);
    let timer = interfaces::timer::run_timer(state);

    try_join!(grpc, timer).map(|_| ())
}

#[cfg(feature = "cli")]
mod cli {
    use crate::proto::KeyType;
    use crate::proto::MeeSignClient;
    use crate::{Args, CA_CERT};
    use clap::Subcommand;
    use meesign_crypto;
    use std::str::FromStr;
    use std::time::SystemTime;
    use tonic::transport::{Certificate, Channel, ClientTlsConfig, Uri};

    #[derive(Subcommand)]
    pub enum Commands {
        GetDevices,
        GetGroups {
            device_id: Option<String>,
        },
        GetTasks {
            device_id: Option<String>,
        },
        RequestGroup {
            name: String,
            threshold: u32,
            #[clap(help = "sign_pdf or sign_challenge")]
            key_type: String,
            device_ids: Vec<String>,
        },
        RequestSignPdf {
            name: String,
            group_id: String,
            pdf_file: String,
        },
        RequestSignChallenge {
            name: String,
            group_id: String,
            data: String,
        },
        RequestDecrypt {
            name: String,
            group_id: String,
            data: String,
        },
    }

    pub(super) async fn handle_command(args: Args) -> Result<(), String> {
        if let Some(command) = args.command {
            let tls = ClientTlsConfig::new()
                .domain_name(&args.host)
                .ca_certificate(Certificate::from_pem(
                    CA_CERT
                        .to_pem()
                        .map_err(|_| "Unable to load CA certificate".to_string())?,
                ));

            let channel = Channel::builder(
                Uri::from_str(&format!("https://{}:{}", &args.host, args.port))
                    .map_err(|_| "Unable to parse URI".to_string())?,
            )
            .tls_config(tls)
            .map_err(|_| "Unable to configure TLS connection".to_string())?
            .connect()
            .await
            .map_err(|_| "Unable to connect to the server".to_string())?;

            let mut client = MeeSignClient::new(channel);

            // TODO Refactor once MeeSignClient (GrpcClient) can be passed to functions more ergonomically
            // More info here https://github.com/hyperium/tonic/issues/110
            match command {
                Commands::GetDevices => {
                    let request = tonic::Request::new(crate::proto::DevicesRequest {});

                    let mut response = client
                        .get_devices(request)
                        .await
                        .map_err(|_| String::from("Request failed"))?
                        .into_inner();

                    let now = SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();

                    response.devices.sort_by_key(|x| u64::MAX - x.last_active);
                    for device in response.devices {
                        println!(
                            "[{}] {} (seen before {}s)",
                            hex::encode(device.identifier),
                            &device.name,
                            now - device.last_active
                        );
                    }
                }
                Commands::GetGroups { device_id } => {
                    let device_id = device_id.map(|x| hex::decode(x).unwrap());
                    let request = tonic::Request::new(crate::proto::GroupsRequest { device_id });

                    let response = client
                        .get_groups(request)
                        .await
                        .map_err(|_| String::from("Request failed"))?
                        .into_inner();

                    for group in response.groups {
                        println!(
                            "[{}] {} ({}-of-{}; {:?})",
                            hex::encode(&group.identifier),
                            &group.name,
                            &group.threshold,
                            &group.device_ids.len(),
                            &group.key_type()
                        );
                    }
                }
                Commands::GetTasks { device_id } => {
                    let device_id = device_id.map(|x| hex::decode(x).unwrap());
                    let request = tonic::Request::new(crate::proto::TasksRequest { device_id });

                    let response = client
                        .get_tasks(request)
                        .await
                        .map_err(|_| String::from("Request failed"))?
                        .into_inner();

                    for task in response.tasks {
                        let task_type = match task.r#type {
                            0 => "Group",
                            1 => "Sign",
                            2 => "Challenge",
                            3 => "Decrypt",
                            _ => "Unknown",
                        };
                        println!(
                            "Task {} [{}] (state {}:{})",
                            task_type,
                            hex::encode(task.id),
                            task.state,
                            task.round
                        );
                    }
                }
                Commands::RequestGroup {
                    name,
                    threshold,
                    key_type,
                    device_ids,
                } => {
                    let device_ids: Vec<_> =
                        device_ids.iter().map(|x| hex::decode(x).unwrap()).collect();
                    if device_ids.len() <= 1 {
                        return Err(String::from("Not enough parties to create a group"));
                    }

                    let request = tonic::Request::new(crate::proto::GroupRequest {
                        name,
                        device_ids,
                        threshold,
                        protocol: crate::proto::ProtocolType::Gg18 as i32,
                        key_type: match key_type.as_str() {
                            "sign_pdf" => KeyType::SignPdf,
                            "sign_challenge" => KeyType::SignChallenge,
                            _ => panic!("Incorrect key type"),
                        } as i32,
                        note: None,
                    });

                    let response = client
                        .group(request)
                        .await
                        .map_err(|_| String::from("Request failed"))?
                        .into_inner();

                    let task = response;
                    let task_type = match task.r#type {
                        0 => "Group",
                        1 => "Sign",
                        2 => "Challenge",
                        3 => "Decrypt",
                        _ => "Unknown",
                    };
                    println!(
                        "Task {} [{}] (state {}:{})",
                        task_type,
                        hex::encode(task.id),
                        task.state,
                        task.round
                    );
                }
                Commands::RequestSignPdf {
                    name,
                    group_id,
                    pdf_file,
                } => {
                    let group_id = hex::decode(group_id).unwrap();
                    let data = std::fs::read(pdf_file).unwrap();
                    let request = tonic::Request::new(crate::proto::SignRequest {
                        name,
                        group_id,
                        data,
                    });

                    let response = client
                        .sign(request)
                        .await
                        .map_err(|_| String::from("Request failed"))?
                        .into_inner();

                    let task = response;
                    let task_type = match task.r#type {
                        0 => "Group",
                        1 => "Sign",
                        2 => "Challenge",
                        3 => "Decrypt",
                        _ => "Unknown",
                    };
                    println!(
                        "Task {} [{}] (state {}:{})",
                        task_type,
                        hex::encode(task.id),
                        task.state,
                        task.round
                    );
                }
                Commands::RequestSignChallenge {
                    name,
                    group_id,
                    data,
                } => {
                    let group_id = hex::decode(group_id).unwrap();
                    let data = hex::decode(data).unwrap();

                    let request = tonic::Request::new(crate::proto::SignRequest {
                        name,
                        group_id,
                        data,
                    });

                    let response = client
                        .sign(request)
                        .await
                        .map_err(|_| String::from("Request failed"))?
                        .into_inner();

                    let task = response;
                    let task_type = match task.r#type {
                        0 => "Group",
                        1 => "Sign",
                        2 => "Challenge",
                        3 => "Decrypt",
                        _ => "Unknown",
                    };
                    println!(
                        "Task {} [{}] (state {}:{})",
                        task_type,
                        hex::encode(task.id),
                        task.state,
                        task.round
                    );
                }
                Commands::RequestDecrypt {
                    name,
                    group_id,
                    data,
                } => {
                    let group_id = hex::decode(group_id).unwrap();
                    // let data = hex::decode(data).unwrap();
                    let data =
                        meesign_crypto::protocol::elgamal::encrypt(data.as_bytes(), &group_id)
                            .unwrap();

                    let request = tonic::Request::new(crate::proto::DecryptRequest {
                        name,
                        group_id,
                        data,
                        data_type: "text/plain;charset=utf-8".to_string(),
                    });

                    let response = client
                        .decrypt(request)
                        .await
                        .map_err(|_| String::from("Request failed"))?
                        .into_inner();

                    let task = response;
                    let task_type = match task.r#type {
                        0 => "Group",
                        1 => "Sign",
                        2 => "Challenge",
                        3 => "Decrypt",
                        _ => "Unknown",
                    };
                    println!(
                        "Task {} [{}] (state {}:{})",
                        task_type,
                        hex::encode(task.id),
                        task.state,
                        task.round
                    );
                }
            }
        }
        Ok(())
    }
}
