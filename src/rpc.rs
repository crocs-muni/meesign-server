use log::{debug, info};
use tokio::sync::{mpsc, Mutex};
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use uuid::Uuid;

use crate::device::Device;
use crate::proto as msg;
use crate::proto::mpc_server::{Mpc, MpcServer};
use crate::proto::{KeyType, ProtocolType};
use crate::state::State;
use crate::tasks::{Task, TaskStatus, TaskType};
use std::collections::HashMap;
use std::pin::Pin;
use tokio::sync::mpsc::Sender;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

pub struct MPCService {
    state: Mutex<State>,
    subscribers: Mutex<HashMap<Vec<u8>, Sender<Result<msg::Task, Status>>>>,
}

impl MPCService {
    pub fn new(state: State) -> Self {
        MPCService {
            state: Mutex::new(state),
            subscribers: Mutex::new(HashMap::new()),
        }
    }

    pub async fn send_updates(&self, task_id: &Uuid, task: &Box<dyn Task + Send + Sync>) {
        let mut subscribers = self.subscribers.lock().await;
        for device_id in task.get_devices().iter().map(Device::identifier) {
            if let Some(tx) = subscribers.get(device_id) {
                let result = tx.try_send(Ok(format_task(task_id, task, Some(device_id), None)));

                if result.is_err() {
                    info!("Channel with device_id={} closed", hex::encode(device_id));
                    subscribers.remove(device_id);
                }
            }
        }
    }
}

#[tonic::async_trait]
impl Mpc for MPCService {
    type SubscribeUpdatesStream =
        Pin<Box<dyn Stream<Item = Result<msg::Task, Status>> + Send + 'static>>;

    async fn register(
        &self,
        request: Request<msg::RegistrationRequest>,
    ) -> Result<Response<msg::Resp>, Status> {
        let request = request.into_inner();
        let identifier = request.identifier;
        let name = request.name;
        info!(
            "RegistrationRequest device_id={} name={:?}",
            hex::encode(&identifier),
            name
        );

        let mut state = self.state.lock().await;

        if state.add_device(&identifier, &name) {
            Ok(Response::new(msg::Resp {
                message: "OK".into(),
            }))
        } else {
            Err(Status::failed_precondition("Request failed"))
        }
    }

    async fn sign(
        &self,
        request: Request<msg::SignRequest>,
    ) -> Result<Response<crate::proto::Task>, Status> {
        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        info!("SignRequest group_id={}", hex::encode(&group_id));

        let mut state = self.state.lock().await;
        if let Some(task_id) = state.add_sign_task(&group_id, &name, &data) {
            let task = state.get_task(&task_id).unwrap();
            self.send_updates(&task_id, &task).await;
            Ok(Response::new(format_task(&task_id, task, None, None)))
        } else {
            Err(Status::failed_precondition("Request failed"))
        }
    }

    async fn get_task(
        &self,
        request: Request<msg::TaskRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task_id).unwrap();
        let device_id = request.device_id;
        let device_id = if device_id.is_none() {
            None
        } else {
            Some(device_id.as_ref().unwrap().as_slice())
        };
        debug!(
            "TaskRequest task_id={} device_id={}",
            hex::encode(&task_id),
            hex::encode(&device_id.clone().unwrap_or(&[]))
        );

        let mut state = self.state.lock().await;
        if device_id.is_some() {
            state.device_activated(device_id.as_ref().unwrap());
        }
        let task = state.get_task(&task_id).unwrap();
        let request = Some(task.get_request());

        let resp = format_task(&task_id, task, device_id, request);
        Ok(Response::new(resp))
    }

    async fn update_task(
        &self,
        request: Request<msg::TaskUpdate>,
    ) -> Result<Response<msg::Resp>, Status> {
        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task).unwrap();
        let device_id = request.device_id;
        let data = request.data;
        info!(
            "TaskUpdate task_id={} device_id={}",
            hex::encode(&task_id),
            hex::encode(&device_id)
        );

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        let result = state.update_task(&task_id, &device_id, &data);

        match result {
            Ok(next_round) => {
                if next_round {
                    self.send_updates(&task_id, &state.get_task(&task_id).unwrap())
                        .await;
                }
                Ok(Response::new(msg::Resp {
                    message: "OK".into(),
                }))
            }
            Err(e) => Err(Status::failed_precondition(e)),
        }
    }

    async fn get_tasks(
        &self,
        request: Request<msg::TasksRequest>,
    ) -> Result<Response<msg::Tasks>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(|x| hex::encode(&x))
            .unwrap_or("unknown".to_string());
        debug!("TasksRequest device_id={}", device_str);

        let mut state = self.state.lock().await;
        let tasks = if let Some(device_id) = device_id {
            state.device_activated(&device_id);
            state
                .get_device_tasks(&device_id)
                .iter()
                .map(|(task_id, task)| format_task(task_id, task, Some(&device_id), None))
                .collect()
        } else {
            state
                .get_tasks()
                .iter()
                .map(|(task_id, task)| format_task(task_id, task, None, None))
                .collect()
        };

        Ok(Response::new(msg::Tasks { tasks }))
    }

    async fn get_groups(
        &self,
        request: Request<msg::GroupsRequest>,
    ) -> Result<Response<msg::Groups>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(|x| hex::encode(&x))
            .unwrap_or("unknown".to_string());
        debug!("GroupsRequest device_id={}", device_str);

        let mut state = self.state.lock().await;
        let groups = if let Some(device_id) = device_id {
            state.device_activated(&device_id);
            state
                .get_device_groups(&device_id)
                .iter()
                .map(|group| group.into())
                .collect()
        } else {
            state
                .get_groups()
                .values()
                .map(|group| group.into())
                .collect()
        };

        Ok(Response::new(msg::Groups { groups }))
    }

    async fn group(
        &self,
        request: Request<msg::GroupRequest>,
    ) -> Result<Response<msg::Task>, Status> {
        let request = request.into_inner();
        let name = request.name;
        let device_ids = request.device_ids;
        let threshold = request.threshold;
        let protocol = ProtocolType::from_i32(request.protocol).unwrap();
        let key_type = KeyType::from_i32(request.key_type).unwrap();

        info!(
            "GroupRequest name={:?} device_ids={:?} threshold={}",
            &name,
            device_ids.iter().map(hex::encode).collect::<Vec<String>>(),
            threshold
        );

        let mut state = self.state.lock().await;
        if let Some(task_id) =
            state.add_group_task(&name, &device_ids, threshold, protocol, key_type)
        {
            let task = state.get_task(&task_id).unwrap();
            self.send_updates(&task_id, task).await;
            Ok(Response::new(format_task(&task_id, task, None, None)))
        } else {
            Err(Status::failed_precondition("Request failed"))
        }
    }

    async fn get_devices(
        &self,
        _request: Request<msg::DevicesRequest>,
    ) -> Result<Response<msg::Devices>, Status> {
        debug!("DevicesRequest");

        let resp = msg::Devices {
            devices: self
                .state
                .lock()
                .await
                .get_devices()
                .values()
                .map(|device| msg::Device::from(device))
                .collect(),
        };
        Ok(Response::new(resp))
    }

    async fn log(&self, request: Request<msg::LogRequest>) -> Result<Response<msg::Resp>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id
            .as_ref()
            .map(|x| hex::encode(&x))
            .unwrap_or("unknown".to_string());
        let message = request.message;
        info!("LogRequest device_id={} message={}", device_str, message);

        if device_id.is_some() {
            self.state
                .lock()
                .await
                .device_activated(device_id.as_ref().unwrap());
        }

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn decide_task(
        &self,
        request: Request<msg::TaskDecision>,
    ) -> Result<Response<msg::Resp>, Status> {
        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task).unwrap();
        let device_id = request.device;
        let accept = request.accept;

        info!(
            "TaskDecision task_id={} device_id={} accept={}",
            hex::encode(&task_id),
            hex::encode(&device_id),
            accept
        );

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        if state.decide_task(&task_id, &device_id, accept) {
            self.send_updates(&task_id, state.get_task(&task_id).unwrap())
                .await;
        }

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn acknowledge_task(
        &self,
        request: Request<msg::TaskAcknowledgement>,
    ) -> Result<Response<msg::Resp>, Status> {
        let request = request.into_inner();
        let task_id = request.task_id;
        let device_id = request.device_id;

        info!(
            "TaskAcknowledgement task_id={} device_id={}",
            hex::encode(&task_id),
            hex::encode(&device_id)
        );

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        state.acknowledge_task(&Uuid::from_slice(&task_id).unwrap(), &device_id);

        Ok(Response::new(msg::Resp {
            message: "OK".into(),
        }))
    }

    async fn subscribe_updates(
        &self,
        request: Request<msg::SubscribeRequest>,
    ) -> Result<Response<Self::SubscribeUpdatesStream>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;

        let (tx, rx) = mpsc::channel(8);

        self.subscribers.lock().await.insert(device_id, tx);

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))))
    }
}

fn format_task(
    task_id: &Uuid,
    task: &Box<dyn Task + Send + Sync>,
    device_id: Option<&[u8]>,
    request: Option<&[u8]>,
) -> crate::proto::Task {
    let task_status = task.get_status();

    let (task_status, round, data) = match task_status {
        TaskStatus::Created => (msg::task::TaskState::Created, 0, task.get_work(device_id)),
        TaskStatus::Running(round) => (
            msg::task::TaskState::Running,
            round,
            task.get_work(device_id),
        ),
        TaskStatus::Finished => (
            msg::task::TaskState::Finished,
            u16::MAX,
            Some(task.get_result().unwrap().as_bytes().to_vec()),
        ),
        TaskStatus::Failed(data) => (
            msg::task::TaskState::Failed,
            u16::MAX,
            Some(data.as_bytes().to_vec()),
        ),
    };

    let (accept, reject) = task.get_decisions();

    msg::Task {
        id: task_id.as_bytes().to_vec(),
        r#type: match task.get_type() {
            TaskType::Group => msg::task::TaskType::Group as i32,
            TaskType::Sign => msg::task::TaskType::Sign as i32,
        },
        state: task_status as i32,
        round: round.into(),
        accept: accept as u32,
        reject: reject as u32,
        data,
        request: request.map(Vec::from),
    }
}

pub async fn run_rpc(state: State, addr: &str, port: u16) -> Result<(), String> {
    let addr = format!("{}:{}", addr, port)
        .parse()
        .map_err(|_| String::from("Unable to parse server address"))?;
    let node = MPCService::new(state);

    Server::builder()
        .add_service(MpcServer::new(node))
        .serve(addr)
        .await
        .map_err(|_| String::from("Unable to run server"))?;

    Ok(())
}
