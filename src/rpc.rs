use crate::proto::*;
use crate::proto::mpc_server::{Mpc, MpcServer};
use tonic::{Request, Status, Response};
use tonic::transport::Server;
use crate::State;
use tokio::sync::Mutex;
use crate::task::{TaskStatus, TaskType};
use uuid::Uuid;
use crate::protocols::ProtocolType;

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

        let mut state = self.state.lock().await;
        state.add_device(&id, &name);

        let resp = Resp {
            variant: Some(resp::Variant::Success("OK".into()))
        };

        Ok(Response::new(resp))
    }

    async fn sign(&self, request: Request<SignRequest>) -> Result<Response<Task>, Status> {
        let request = request.into_inner();
        let group_id = request.group_id;
        let data = request.data;

        let mut state = self.state.lock().await;
        let task_id = state.add_sign_task(&group_id, &data);
        let (task_type, task_status) = state.get_task(&task_id);

        let resp = format_task(&task_id, task_type, task_status);

        Ok(Response::new(resp))
    }

    async fn get_task(&self, request: Request<TaskRequest>) -> Result<Response<Task>, Status> {
        let request = request.into_inner();
        let task_id = Uuid::from_slice(&request.task_id).unwrap();
        let device_id = request.device_id;


        let state = self.state.lock().await;
        let (task_type, task_state) = state.get_task(&task_id);
        let (task_state, data) = match task_state {
            TaskStatus::Waiting(data) => (task::TaskState::Waiting, data.clone()),
            TaskStatus::Signed(data) => (task::TaskState::Finished, vec![data]),
            TaskStatus::GroupEstablished(data) => (task::TaskState::Finished, vec![data.identifier().to_vec()]),
            TaskStatus::Failed(data) => (task::TaskState::Failed, vec![data]),
        };

        let resp = Task {
            id: task_id.as_bytes().to_vec(),
            r#type: match task_type {
                TaskType::Group => task::TaskType::Group as i32,
                TaskType::Sign => task::TaskType::Sign as i32,
            },
            state: task_state as i32,
            data,
            progress: 0,
            work: device_id.and_then(|device_id| state.get_work(&task_id, &device_id))
        };

        Ok(Response::new(resp))
    }

    async fn update_task(&self, request: Request<TaskUpdate>) -> Result<Response<Resp>, Status> {
        let request = request.into_inner();
        let task = Uuid::from_slice(&request.task).unwrap();
        let device = request.device;
        let data = request.data;

        self.state.lock().await.update_task(&task, &device, &data);

        let resp = Resp {
            variant: Some(resp::Variant::Success("OK".into()))
        };

        Ok(Response::new(resp))
    }

    async fn get_info(&self, request: Request<InfoRequest>) -> Result<Response<Info>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;

        let groups = self.state.lock().await.get_device_groups(&device_id).iter().map(|group| {
            Group {
                id: group.identifier().to_vec(),
                name: group.name().to_owned(),
                threshold: group.threshold(),
                device_ids: group.devices().iter().map(Vec::clone).collect(),
            }
        }).collect();

        let tasks = self.state.lock().await.get_device_tasks(&device_id).iter().map(|(task_id, (task_type, task_status))| {
            Task {
                id: task_id.as_bytes().to_vec(),
                r#type: match task_type {
                    TaskType::Group => task::TaskType::Group as i32,
                    TaskType::Sign => task::TaskType::Sign as i32,
                },
                state: match task_status {
                    TaskStatus::Waiting(_) => task::TaskState::Waiting as i32,
                    TaskStatus::GroupEstablished(_) => task::TaskState::Finished as i32,
                    TaskStatus::Signed(_) => task::TaskState::Finished as i32,
                    TaskStatus::Failed(_) => task::TaskState::Failed as i32,
                },
                data: Vec::new(),
                progress: 0,
                work: None
            }
        }).collect();

        let resp = Info {
            tasks,
            groups
        };

        Ok(Response::new(resp))
    }

    async fn group(&self, request: Request<GroupRequest>) -> Result<Response<Task>, Status> {
        let request = request.into_inner();
        let name = request.name;
        let device_ids = request.device_ids;
        let threshold = request.threshold.unwrap_or(device_ids.len() as u32);
        let protocol = match request.protocol.unwrap_or(0) {
            0 => ProtocolType::GG18,
            _ => unreachable!(),
        };

        let mut state = self.state.lock().await;
        let task_id = state.add_group_task(&name, &device_ids, threshold, protocol).unwrap();
        let (task_type, task_status) = state.get_task(&task_id);

        let resp = format_task(&task_id, task_type, task_status);

        Ok(Response::new(resp))
    }

    async fn get_devices(&self, _request: Request<DevicesRequest>) -> Result<Response<Devices>, Status> {
        let resp = Devices {
            devices: self.state.lock().await.get_devices().iter().map(|device|
                crate::proto::Device {
                    id: device.identifier().to_vec(),
                    name: device.name().to_string()
                }
            ).collect()
        };

        Ok(Response::new(resp))
    }

    async fn get_archived(&self, request: Request<ArchivedRequest>) -> Result<Response<Archived>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;

        let resp = Archived {
            tasks: self.state.lock().await.get_device_archived(&device_id).iter().map(|(task_id, (task_type, task_status))| {
                Task {
                    id: task_id.as_bytes().to_vec(),
                    r#type: match task_type {
                        TaskType::Group => task::TaskType::Group as i32,
                        TaskType::Sign => task::TaskType::Sign as i32,
                    },
                    state: match task_status {
                        TaskStatus::Waiting(_) => task::TaskState::Waiting as i32,
                        TaskStatus::GroupEstablished(_) => task::TaskState::Finished as i32,
                        TaskStatus::Signed(_) => task::TaskState::Finished as i32,
                        TaskStatus::Failed(_) => task::TaskState::Failed as i32,
                    },
                    data: Vec::new(),
                    progress: 0,
                    work: None
                }
            }).collect()
        };

        Ok(Response::new(resp))
    }
}

pub fn format_task(task_id: &Uuid, task_type: TaskType, task_status: TaskStatus) -> Task {
    let (task_status, data) = match task_status {
        TaskStatus::Waiting(data) => (task::TaskState::Waiting, data.clone()),
        TaskStatus::Signed(data) => (task::TaskState::Finished, vec![data]),
        TaskStatus::GroupEstablished(data) => (task::TaskState::Finished, vec![data.identifier().to_vec()]),
        TaskStatus::Failed(data) => (task::TaskState::Failed, vec![data]),
    };

    Task {
        id: task_id.as_bytes().to_vec(),
        r#type: match task_type {
            TaskType::Group => task::TaskType::Group as i32,
            TaskType::Sign => task::TaskType::Sign as i32,
        },
        state: task_status as i32,
        data,
        progress: 0,
        work: None
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
