use std::time::SystemTime;

use clap::{Parser, Subcommand};

use crate::proto::mpc_client::MpcClient;
use crate::state::State;

mod device;
mod group;
mod protocols;
mod rpc;
mod state;
mod tasks;

mod proto {
    tonic::include_proto!("meesign");
}

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
        device_ids: Vec<String>,
    },
    RequestSign {
        name: String,
        group_id: String,
        pdf_file: String,
    },
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
    device_ids: Vec<Vec<u8>>,
) -> Result<(), String> {
    let device_count = device_ids.len();
    if device_count <= 1 {
        return Err(String::from("Not enough parties to create a group"));
    }

    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

    let request = tonic::Request::new(crate::proto::GroupRequest {
        name,
        device_ids,
        threshold: device_count as u32,
        protocol: crate::proto::Protocol::Gg18 as i32,
        key_type: crate::proto::KeyType::SignPdf as i32,
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
            Commands::RequestGroup { name, device_ids } => {
                let device_ids = device_ids.iter().map(|x| hex::decode(x).unwrap()).collect();
                request_group(server, name, device_ids).await
            }
            Commands::RequestSign {
                name,
                group_id,
                pdf_file,
            } => {
                let group_id = hex::decode(group_id).unwrap();
                let data = std::fs::read(pdf_file).unwrap();
                request_sign(server, name, group_id, data).await
            }
        }
    } else {
        rpc::run_rpc(State::new(), &args.addr, args.port).await
    }
}
