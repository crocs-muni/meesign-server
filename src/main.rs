use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};

use crate::proto::mpc_client::MpcClient;
use crate::state::State;
use tokio::{join, sync::Mutex};
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
struct Args {
    #[clap(short, long, default_value_t = 1337)]
    port: u16,

    #[clap(short, long, default_value_t = String::from("127.0.0.1"))]
    addr: String,

    #[clap(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
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

pub fn get_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn register(server: String, identifier: &[u8], name: &str) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

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

    Ok(())
}

async fn get_devices(server: String) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

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

    Ok(())
}

async fn get_groups(server: String, device_id: Option<Vec<u8>>) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

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

    Ok(())
}

async fn get_tasks(server: String, device_id: Option<Vec<u8>>) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

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

    Ok(())
}

async fn request_group(
    server: String,
    name: String,
    threshold: u32,
    key_type: String,
    device_ids: Vec<Vec<u8>>,
) -> Result<(), String> {
    let device_count = device_ids.len();
    if device_count <= 1 {
        return Err(String::from("Not enough parties to create a group"));
    }

    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

    let key_type = match key_type.as_str() {
        "sign_pdf" => Ok(crate::proto::KeyType::SignPdf as i32),
        "sign_challenge" => Ok(crate::proto::KeyType::SignChallenge as i32),
        _ => Err("Unknown key type".to_string()),
    }?;

    let request = tonic::Request::new(crate::proto::GroupRequest {
        name,
        device_ids,
        threshold,
        protocol: crate::proto::ProtocolType::Gg18 as i32,
        key_type,
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

    Ok(())
}

async fn request_sign(
    server: String,
    name: String,
    group_id: Vec<u8>,
    data: Vec<u8>,
) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

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

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::init();
    let args = Args::parse();
    if let Some(command) = args.command {
        let server = format!("http://{}:{}", &args.addr, args.port);

        match command {
            Commands::Register { identifier, name } => {
                let identifier = hex::decode(identifier).unwrap();
                register(server, &identifier, &name).await
            }
            Commands::GetDevices => get_devices(server).await,
            Commands::GetGroups { device_id } => {
                let device_id = device_id.map(|x| hex::decode(x).unwrap());
                get_groups(server, device_id).await
            }
            Commands::GetTasks { device_id } => {
                let device_id = device_id.map(|x| hex::decode(x).unwrap());
                get_tasks(server, device_id).await
            }
            Commands::RequestGroup {
                name,
                threshold,
                key_type,
                device_ids,
            } => {
                let device_ids = device_ids.iter().map(|x| hex::decode(x).unwrap()).collect();
                request_group(server, name, threshold, key_type, device_ids).await
            }
            Commands::RequestSignPdf {
                name,
                group_id,
                pdf_file,
            } => {
                let group_id = hex::decode(group_id).unwrap();
                let data = std::fs::read(pdf_file).unwrap();
                request_sign(server, name, group_id, data).await
            }
            Commands::RequestSignChallenge {
                name,
                group_id,
                data,
            } => {
                let group_id = hex::decode(group_id).unwrap();
                let data = hex::decode(data).unwrap();
                request_sign(server, name, group_id, data).await
            }
        }
    } else {
        let state = Arc::new(Mutex::new(State::new()));

        let grpc = interfaces::grpc::run_grpc(state.clone(), &args.addr, args.port);
        let timer = interfaces::timer::run_timer(state);

        let (grpc_result, timer_result) = join!(grpc, timer);
        grpc_result.and(timer_result)
    }
}
