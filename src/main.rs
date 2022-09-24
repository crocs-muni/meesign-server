use std::time::{SystemTime, UNIX_EPOCH};

use clap::Parser;

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
}

const VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub(self) struct Args {
    #[clap(short, long, default_value_t = 1337)]
    port: u16,

    #[cfg(feature = "web")]
    #[clap(short, long, default_value_t = 1338)]
    web: u16,

    #[clap(short, long, default_value_t = String::from("127.0.0.1"))]
    addr: String,

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

    let state = Arc::new(Mutex::new(State::new()));

    let grpc = interfaces::grpc::run_grpc(state.clone(), &args.addr, args.port);
    let web = tonic::codegen::ok(());
    #[cfg(feature = "web")]
    let web = interfaces::web::run_web(&args.addr, args.port);
    let timer = interfaces::timer::run_timer(state);

    try_join!(grpc, timer, web).map(|_| ())
}

#[cfg(feature = "cli")]
mod cli {
    use crate::proto::mpc_client::MpcClient;
    use crate::proto::KeyType;
    use crate::Args;
    use clap::Subcommand;
    use hyper::client::HttpConnector;
    use hyper::Uri;
    use rustls::ClientConfig;
    use std::str::FromStr;
    use std::time::SystemTime;
    use tonic::codegen::Arc;

    #[derive(Subcommand)]
    pub enum Commands {
        Register {
            identifier: String,
            name: String,
        },
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
    }

    // Skip server certificate validation for testing from CLI
    // Based on https://quinn-rs.github.io/quinn/quinn/certificate.html
    struct SkipServerVerification;

    impl SkipServerVerification {
        fn new() -> Arc<Self> {
            Arc::new(Self)
        }
    }

    impl rustls::client::ServerCertVerifier for SkipServerVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::Certificate,
            _intermediates: &[rustls::Certificate],
            _server_name: &rustls::ServerName,
            _scts: &mut dyn Iterator<Item = &[u8]>,
            _ocsp_response: &[u8],
            _now: SystemTime,
        ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::ServerCertVerified::assertion())
        }
    }

    pub(super) async fn handle_command(args: Args) -> Result<(), String> {
        if let Some(command) = args.command {
            let server = format!("https://{}:{}", &args.addr, args.port);
            let server_move = server.clone();

            // Based on https://github.com/hyperium/tonic/blob/675b3b2e75b896632cc8cbc291133d7a44d790a1/examples/src/tls/client_rustls.rs
            let connector = tower::ServiceBuilder::new()
                .layer_fn(|s| {
                    let tls = ClientConfig::builder()
                        .with_safe_defaults()
                        .with_custom_certificate_verifier(SkipServerVerification::new())
                        .with_no_client_auth();

                    hyper_rustls::HttpsConnectorBuilder::new()
                        .with_tls_config(tls)
                        .https_or_http()
                        .enable_http2()
                        .wrap_connector(s)
                })
                .map_request(move |_| Uri::from_str(&server_move).unwrap())
                .service({
                    let mut http = HttpConnector::new();
                    http.enforce_http(false);
                    http
                });

            let mut client = MpcClient::with_origin(
                hyper::Client::builder().build(connector),
                Uri::from_str(&server).unwrap(),
            );

            // TODO Refactor once MpcClient (GrpcClient) can be passed to functions more ergonomically
            // More info here https://github.com/hyperium/tonic/issues/110
            match command {
                Commands::Register { identifier, name } => {
                    let identifier = hex::decode(identifier).unwrap();
                    let request = tonic::Request::new(crate::proto::RegistrationRequest {
                        identifier: identifier.to_vec(),
                        name: name.to_string(),
                    });

                    let response = client
                        .register(request)
                        .await
                        .map_err(|_| String::from("Request failed"))?
                        .into_inner();

                    println!("{}", response.message);
                }
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
                            "[{}] {} ({}-of-{})",
                            hex::encode(group.identifier),
                            &group.name,
                            group.threshold,
                            group.device_ids.len()
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
