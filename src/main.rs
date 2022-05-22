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
use clap::Parser;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, default_value_t = 1337)]
    port: u16,

    #[clap(short, long, default_value_t = String::from("127.0.0.1"))]
    addr: String,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::init();
    let args = Args::parse();
    rpc::run_rpc(State::new(), &args.addr, args.port).await
}
