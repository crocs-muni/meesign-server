use std::collections::HashMap;

mod rpc;
mod task;

mod proto {
    tonic::include_proto!("mpcoord");
}

use task::{Task, TaskStatus};

pub struct State {
    devices: HashMap<Vec<u8>, Vec<u8>>,
    tasks: Vec<Box<dyn Task + Send + Sync>>,
}

impl State {
    pub fn new() -> Self {
        State {
            devices: HashMap::new(),
            tasks: Vec::new(),
        }
    }

    pub fn add_device(&mut self, device: Vec<u8>) {
        self.devices.insert(device, Vec::new());
    }

    pub fn add_sign_task(&mut self, devices: &[Vec<u8>], data: &[u8]) {
        self.add_task(Box::new(SignTask::new(devices, data.to_vec())));
    }

    fn add_task(&mut self, task: Box<dyn Task + Send + Sync>) {
        self.tasks.push(task);
    }

    pub fn get_device_tasks(&self, device: &Vec<u8>) -> Vec<u32> {
        let mut tasks = Vec::new();
        for (idx, task) in self.tasks.iter().enumerate() {
            if task.waiting_for(device) {
                tasks.push(idx as u32);
            }
        }
        tasks
    }

    pub fn get_task(&self, task: u32) -> TaskStatus {
        self.tasks.get(task as usize).unwrap().get_status()
    }

    pub fn update_task(&mut self, task: u32, device: &[u8], data: &[u8]) -> TaskStatus {
        self.tasks.get_mut(task as usize).unwrap().update(device, data).unwrap()
    }
}

pub struct SignTask {
    subtasks: HashMap<Vec<u8>, bool>,
    data: Vec<u8>,
    result: Vec<u8>,
}

impl SignTask {
    pub fn new(devices: &[Vec<u8>], data: Vec<u8>) -> Self {
        let mut subtasks = HashMap::new();
        for device in devices.iter() {
            subtasks.insert(device.clone(), false);
        }
        SignTask { subtasks, data, result: Vec::new() }
    }
}

impl Task for SignTask {
    fn get_status(&self) -> TaskStatus {
        let waiting: Vec<_> = self.subtasks.iter()
            .filter(|(_, value)| !*value)
            .map(|(key, _)| key.clone())
            .collect();

        if waiting.is_empty() {
            TaskStatus::Waiting(waiting, self.data.clone())
        } else {
            TaskStatus::Finished(self.result.clone())
        }
    }

    fn update(&mut self, device_id: &[u8], _data: &[u8]) -> Result<TaskStatus, String> {
        if self.subtasks.contains_key(device_id) {
            self.subtasks.insert(device_id.to_vec(), true);
            Ok(self.get_status())
        } else {
            Err("Incompatible device ID".into())
        }
    }

    fn waiting_for(&self, device_id: &[u8]) -> bool {
        self.subtasks.get(device_id).map(|x| !*x).unwrap_or(false)
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    rpc::run_rpc(State::new()).await
}
