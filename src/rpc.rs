use crate::proto::*;
use crate::proto::mpc_server::{Mpc, MpcServer};
use tonic::{Request, Status, Response};
use tonic::transport::Server;
use crate::State;
use tokio::sync::Mutex;
use crate::task::{TaskStatus, TaskType, Task};
use uuid::Uuid;
use crate::protocols::ProtocolType;
use log::{debug, info};

pub struct MPCService {
    state: Mutex<State>
}

impl MPCService {
    pub fn new(state: State) -> Self {
        MPCService { state: Mutex::new(state) }
    }
}

#[tonic::async_trait]
impl Mpc for MPCService {
    async fn register(&self, request: Request<RegistrationRequest>) -> Result<Response<Resp>, Status> {
        let request = request.into_inner();
        let id = request.id;
        let name = request.name;
        info!("RegistrationRequest device_id={} name={:?}", hex::encode(&id), name);

        let mut state = self.state.lock().await;
        state.add_device(&id, &name);

        let resp = Resp {
            variant: Some(resp::Variant::Success("OK".into()))
        };

        Ok(Response::new(resp))
    }

    async fn sign(&self, request: Request<SignRequest>) -> Result<Response<crate::proto::Task>, Status> {
        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        info!("SignRequest group_id={} data={}", hex::encode(&group_id), hex::encode(&data));

        let mut state = self.state.lock().await;
        let task_id = state.add_sign_task(&group_id, &name, &data);
        let task = state.get_task(&task_id).unwrap();

        let resp = format_task(&task_id, task, None);
        Ok(Response::new(resp))
    }

    async fn get_task(&self, request: Request<TaskRequest>) -> Result<Response<crate::proto::Task>, Status> {
        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task_id).unwrap();
        let device_id = request.device_id;
        let device_id = if device_id.is_none() { None } else { Some(device_id.as_ref().unwrap().as_slice()) };
        debug!("TaskRequest task_id={} device_id={}", hex::encode(&task_id), hex::encode(&device_id.clone().unwrap_or(&[])));

        let mut state = self.state.lock().await;
        if device_id.is_some() {
            state.device_activated(device_id.as_ref().unwrap());
        }
        let task = state.get_task(&task_id).unwrap();

        let resp = format_task(&task_id, task, device_id);
        Ok(Response::new(resp))
    }

    async fn update_task(&self, request: Request<TaskUpdate>) -> Result<Response<Resp>, Status> {
        let request = request.into_inner();
        let task = Uuid::from_slice(&request.task).unwrap();
        let device_id = request.device_id;
        let data = request.data;
        info!("TaskUpdate task_id={} device_id={} data={}", hex::encode(&task), hex::encode(&device_id), hex::encode(&data));

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        let result = state.update_task(&task, &device_id, &data);

        let resp = Resp {
            variant: Some(match result {
                Ok(_) => resp::Variant::Success(String::from("OK")),
                Err(e) => resp::Variant::Failure(e),
            })
        };
        Ok(Response::new(resp))
    }

    async fn get_tasks(&self, request: Request<TasksRequest>) -> Result<Response<Tasks>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        debug!("TasksRequest device_id={}", hex::encode(&device_id));

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        let tasks = state.get_device_tasks(&device_id).iter()
            .map(|(task_id, task)| format_task(task_id, task, Some(&device_id)))
            .collect();

        let resp = Tasks {
            tasks
        };
        Ok(Response::new(resp))
    }

    async fn get_groups(&self, request: Request<GroupsRequest>) -> Result<Response<Groups>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        debug!("GroupsRequest device_id={}", hex::encode(&device_id));

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        let groups = state.get_device_groups(&device_id).iter().map(|group| {
            Group {
                id: group.identifier().to_vec(),
                name: group.name().to_owned(),
                threshold: group.threshold(),
                device_ids: group.devices().keys().map(Vec::clone).collect(),
            }
        }).collect();

        let resp = Groups {
            groups
        };
        Ok(Response::new(resp))
    }

    async fn group(&self, request: Request<GroupRequest>) -> Result<Response<crate::proto::Task>, Status> {
        let request = request.into_inner();
        let name = request.name;
        let device_ids = request.device_ids;
        let threshold = request.threshold.unwrap_or(device_ids.len() as u32);
        let protocol = match request.protocol.unwrap_or(0) {
            0 => ProtocolType::GG18,
            _ => unreachable!(),
        };
        info!("GroupRequest name={:?} device_ids={:?} threshold={}", &name, device_ids.iter().map(hex::encode).collect::<Vec<String>>(), threshold);

        let mut state = self.state.lock().await;
        let task_id = state.add_group_task(&name, &device_ids, threshold, protocol).unwrap();
        let task = state.get_task(&task_id).unwrap();

        let resp = format_task(&task_id, task, None);
        Ok(Response::new(resp))
    }

    async fn get_devices(&self, _request: Request<DevicesRequest>) -> Result<Response<Devices>, Status> {
        debug!("DevicesRequest");
        let resp = Devices {
            devices: self.state.lock().await.get_devices().values().map(|device|
                crate::proto::Device {
                    id: device.identifier().to_vec(),
                    name: device.name().to_string(),
                    last_active: device.last_active()
                }
            ).collect()
        };
        Ok(Response::new(resp))
    }

    async fn log(&self, request: Request<LogRequest>) -> Result<Response<Resp>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id.as_ref().map(|x| hex::encode(&x)).unwrap_or("unknown".to_string());
        let message = request.message;
        info!("LogRequest device_id={} message={}", device_str, message);

        if device_id.is_some() {
            self.state.lock().await.device_activated(device_id.as_ref().unwrap());
        }

        let resp = Resp {
            variant: Some(resp::Variant::Success("OK".into()))
        };
        Ok(Response::new(resp))
    }
}

fn format_task(task_id: &Uuid, task: &Box<dyn Task + Send + Sync>, device_id: Option<&[u8]>) -> crate::proto::Task {
    let task_status = task.get_status();

    let (task_status, round, data) = match task_status {
        TaskStatus::Created => (task::TaskState::Created, 0, task.get_work(device_id)),
        TaskStatus::Running(round) => (task::TaskState::Running, round, task.get_work(device_id)),
        TaskStatus::Finished => (task::TaskState::Finished, u16::MAX, Some(task.get_result().unwrap().as_bytes().to_vec())),
        TaskStatus::Failed(data) => (task::TaskState::Failed, u16::MAX, Some(data.as_bytes().to_vec())),
    };

    crate::proto::Task {
        id: task_id.as_bytes().to_vec(),
        r#type: match task.get_type() {
            TaskType::Group => task::TaskType::Group as i32,
            TaskType::Sign => task::TaskType::Sign as i32,
        },
        state: task_status as i32,
        data,
        round: round.into(),
    }
}

pub async fn run_rpc(state: State) -> Result<(), String> {
    let addr = "127.0.0.1:1337".parse().unwrap();
    let node = MPCService::new(state);

    Server::builder()
        .add_service(MpcServer::new(node))
        .serve(addr)
        .await
        .unwrap();

    Ok(())
}
