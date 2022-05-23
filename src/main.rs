mod rpc;
mod task;
mod group;
mod device;
mod protocols;
mod state;

mod proto {
    tonic::include_proto!("meesign");
}

use crate::state::State;
use clap::{Parser, Subcommand};
use crate::proto::mpc_client::MpcClient;
use std::time::SystemTime;

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, default_value_t = 1337)]
    port: u16,

    #[clap(short, long, default_value_t = String::from("127.0.0.1"))]
    addr: String,

    #[clap(subcommand)]
    command: Option<Commands>
}

#[derive(Subcommand)]
enum Commands {
    Register {
        identifier: String,
        name: String
    },
    GetDevices,
    GetGroups {
        device_id: Option<String>,
    },
    GetTasks {
        device_id: Option<String>,
    },
}

async fn register(server: String, identifier: &[u8], name: &str) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

    let request = tonic::Request::new(
        crate::proto::RegistrationRequest {
            identifier: identifier.to_vec(),
            name: name.to_string()
        }
    );

    let response = client.register(request)
        .await
        .map_err(|_| String::from("Request failed"))?
        .into_inner();

    let msg = match response.variant {
        Some(crate::proto::resp::Variant::Success(msg)) => msg,
        Some(crate::proto::resp::Variant::Failure(msg)) => msg,
        None => String::from("Unknown error"),
    };

    println!("{}", msg);

    Ok(())
}

async fn get_devices(server: String) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

    let request = tonic::Request::new(crate::proto::DevicesRequest {});

    let mut response = client.get_devices(request)
        .await
        .map_err(|_| String::from("Request failed"))?
        .into_inner();

    let now = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();

    response.devices.sort_by_key(|x| u64::MAX - x.last_active);
    for device in response.devices {
        println!("{} [{}] (seen before {}s)", &device.name, hex::encode(device.identifier), now - device.last_active);
    }

    Ok(())
}

async fn get_groups(server: String, device_id: Option<Vec<u8>>) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

    let request = tonic::Request::new(crate::proto::GroupsRequest { device_id });

    let response = client.get_groups(request)
        .await
        .map_err(|_| String::from("Request failed"))?
        .into_inner();

    for group in response.groups {
        println!("{} [{}] ({}-of-{})", &group.name, hex::encode(group.identifier), group.threshold, group.device_ids.len());
    }

    Ok(())
}

async fn get_tasks(server: String, device_id: Option<Vec<u8>>) -> Result<(), String> {
    let mut client = MpcClient::connect(server)
        .await
        .map_err(|_| String::from("Unable to connect to server"))?;

    let request = tonic::Request::new(crate::proto::TasksRequest { device_id });

    let response = client.get_tasks(request)
        .await
        .map_err(|_| String::from("Request failed"))?
        .into_inner();

    for task in response.tasks {
        let task_type = match task.r#type {
            0 => "Group",
            1 => "Sign",
            _ => "Unknown",
        };
        println!("Task {} [{}] (state {}:{})", task_type, hex::encode(task.id), task.state, task.round);
    }

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
            },
            Commands::GetDevices => get_devices(server).await,
            Commands::GetGroups { device_id } => {
                let device_id = device_id.map(|x| hex::decode(x).unwrap());
                get_groups(server, device_id).await
            },
            Commands::GetTasks { device_id } => {
                let device_id = device_id.map(|x| hex::decode(x).unwrap());
                get_tasks(server, device_id).await
            }
        }
    } else {
        rpc::run_rpc(State::new(), &args.addr, args.port).await
    }
}
