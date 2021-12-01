mod rpc;

mod proto {
    tonic::include_proto!("mpcoord");
}

struct State {

}

#[tokio::main]
async fn main() -> Result<(), String> {
    rpc::run_rpc().await
}
