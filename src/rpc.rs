use crate::proto::*;
use crate::proto::mpc_server::{Mpc, MpcServer};
use tonic::{Request, Status, Response};
use tonic::transport::Server;

pub struct MPCService {}

impl MPCService {
    pub fn new() -> Self {
        MPCService {}
    }
}

#[tonic::async_trait]
impl Mpc for MPCService {
    async fn register(&self, request: Request<RegistrationRequest>) -> Result<Response<Resp>, Status> {
        let resp = Resp {
            variant: Some(resp::Variant::Success("OK".into()))
        };

        Ok(Response::new(resp))
    }

    async fn sign(&self, request: Request<SignRequest>) -> Result<Response<Resp>, Status> {
        let resp = Resp {
            variant: Some(resp::Variant::Success("OK".into()))
        };

        Ok(Response::new(resp))
    }

    async fn get_task(&self, request: Request<TaskRequest>) -> Result<Response<Task>, Status> {
        let resp = Task {
            task_id: vec![0u8; 16]
        };

        Ok(Response::new(resp))
    }

    async fn update_task(&self, request: Request<Task>) -> Result<Response<Resp>, Status> {
        let resp = Resp {
            variant: Some(resp::Variant::Success("OK".into()))
        };

        Ok(Response::new(resp))
    }
}

pub async fn run_rpc() -> Result<(), String> {
    let addr = "127.0.0.1:1337".parse().unwrap();
    let node = MPCService::new();

    Server::builder()
        .add_service(MpcServer::new(node))
        .serve(addr)
        .await
        .unwrap();

    Ok(())
}
