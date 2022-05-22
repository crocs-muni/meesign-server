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
        }
    } else {
        rpc::run_rpc(State::new(), &args.addr, args.port).await
    }
}
