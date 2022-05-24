use crate::proto::mpc_server::{Mpc, MpcServer};
use tonic::{Request, Status, Response};
use tonic::transport::Server;
use crate::state::State;
use tokio::sync::Mutex;
use crate::task::{TaskStatus, TaskType, Task};
use uuid::Uuid;
use crate::protocols::ProtocolType;
use log::{debug, info};
use crate::proto as msg;

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
    async fn register(&self, request: Request<msg::RegistrationRequest>) -> Result<Response<msg::Resp>, Status> {
        let request = request.into_inner();
        let identifier = request.identifier;
        let name = request.name;
        info!("RegistrationRequest device_id={} name={:?}", hex::encode(&identifier), name);

        let mut state = self.state.lock().await;

        if state.add_device(&identifier, &name) {
            Ok(Response::new(msg::Resp { message: "OK".into() }))
        } else {
            Err(Status::failed_precondition("Request failed"))
        }
    }

    async fn sign(&self, request: Request<msg::SignRequest>) -> Result<Response<crate::proto::Task>, Status> {
        let request = request.into_inner();
        let group_id = request.group_id;
        let name = request.name;
        let data = request.data;
        info!("SignRequest group_id={}", hex::encode(&group_id));

        let mut state = self.state.lock().await;
        let task_id = state.add_sign_task(&group_id, &name, &data);
        let task = state.get_task(&task_id).unwrap();

        let resp = format_task(&task_id, task, None);
        Ok(Response::new(resp))
    }

    async fn get_task(&self, request: Request<msg::TaskRequest>) -> Result<Response<msg::Task>, Status> {
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

    async fn update_task(&self, request: Request<msg::TaskUpdate>) -> Result<Response<msg::Resp>, Status> {
        let request = request.into_inner();
        let task = Uuid::from_slice(&request.task).unwrap();
        let device_id = request.device_id;
        let data = request.data;
        info!("TaskUpdate task_id={} device_id={}", hex::encode(&task), hex::encode(&device_id));

        let mut state = self.state.lock().await;
        state.device_activated(&device_id);
        let result = state.update_task(&task, &device_id, &data);

        match result {
            Ok(_) => Ok(Response::new(msg::Resp { message: "OK".into() })),
            Err(e) => Err(Status::failed_precondition(e))
        }
    }

    async fn get_tasks(&self, request: Request<msg::TasksRequest>) -> Result<Response<msg::Tasks>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id.as_ref().map(|x| hex::encode(&x)).unwrap_or("unknown".to_string());
        debug!("TasksRequest device_id={}", device_str);

        let mut state = self.state.lock().await;
        let tasks = if let Some(device_id) = device_id {
            state.device_activated(&device_id);
            state.get_device_tasks(&device_id).iter()
                .map(|(task_id, task)| format_task(task_id, task, Some(&device_id)))
                .collect()
        } else {
            state.get_tasks().iter()
                .map(|(task_id, task)| format_task(task_id, task, None))
                .collect()
        };

        Ok(Response::new(msg::Tasks { tasks }))
    }

    async fn get_groups(&self, request: Request<msg::GroupsRequest>) -> Result<Response<msg::Groups>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id.as_ref().map(|x| hex::encode(&x)).unwrap_or("unknown".to_string());
        debug!("GroupsRequest device_id={}", device_str);

        let mut state = self.state.lock().await;
        let groups = if let Some(device_id) = device_id {
            state.device_activated(&device_id);
            state.get_device_groups(&device_id).iter().map(|group| msg::Group::from(group)).collect()
        } else {
            state.get_groups().values().map(|group| msg::Group::from(group)).collect()
        };

        Ok(Response::new(msg::Groups { groups }))
    }

    async fn group(&self, request: Request<msg::GroupRequest>) -> Result<Response<msg::Task>, Status> {
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
        if let Some(task_id) = state.add_group_task(&name, &device_ids, threshold, protocol) {
            let task = state.get_task(&task_id).unwrap();
            Ok(Response::new(format_task(&task_id, task, None)))
        } else {
            Err(Status::failed_precondition("Request failed"))
        }

    }

    async fn get_devices(&self, _request: Request<msg::DevicesRequest>) -> Result<Response<msg::Devices>, Status> {
        debug!("DevicesRequest");

        let resp = msg::Devices {
            devices: self.state.lock().await.get_devices().values().map(|device| msg::Device::from(device)).collect()
        };
        Ok(Response::new(resp))
    }

    async fn log(&self, request: Request<msg::LogRequest>) -> Result<Response<msg::Resp>, Status> {
        let request = request.into_inner();
        let device_id = request.device_id;
        let device_str = device_id.as_ref().map(|x| hex::encode(&x)).unwrap_or("unknown".to_string());
        let message = request.message;
        info!("LogRequest device_id={} message={}", device_str, message);

        if device_id.is_some() {
            self.state.lock().await.device_activated(device_id.as_ref().unwrap());
        }

        Ok(Response::new(msg::Resp { message: "OK".into() }))
    }
}

fn format_task(task_id: &Uuid, task: &Box<dyn Task + Send + Sync>, device_id: Option<&[u8]>) -> crate::proto::Task {
    let task_status = task.get_status();

    let (task_status, round, data) = match task_status {
        TaskStatus::Created => (msg::task::TaskState::Created, 0, task.get_work(device_id)),
        TaskStatus::Running(round) => (msg::task::TaskState::Running, round, task.get_work(device_id)),
        TaskStatus::Finished => (msg::task::TaskState::Finished, u16::MAX, Some(task.get_result().unwrap().as_bytes().to_vec())),
        TaskStatus::Failed(data) => (msg::task::TaskState::Failed, u16::MAX, Some(data.as_bytes().to_vec())),
    };

    msg::Task {
        id: task_id.as_bytes().to_vec(),
        r#type: match task.get_type() {
            TaskType::Group => msg::task::TaskType::Group as i32,
            TaskType::Sign => msg::task::TaskType::Sign as i32,
        },
        state: task_status as i32,
        data,
        round: round.into(),
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
