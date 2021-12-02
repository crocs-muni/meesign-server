use std::collections::{HashMap, VecDeque};

mod rpc;

mod proto {
    tonic::include_proto!("mpcoord");
}

pub struct State {
    devices: HashMap<Vec<u8>, Vec<u8>>,
    tasks: Vec<Task>,
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

    pub fn add_task(&mut self, devices: &[Vec<u8>]) {
        self.tasks.push(Task::new(devices));
    }

    pub fn get_device_tasks(&self, device: &Vec<u8>) -> Vec<u32> {
        let mut tasks = Vec::new();
        for (idx, task) in self.tasks.iter().enumerate() {
            if task.subtasks.contains_key(device) {
                tasks.push(idx as u32);
            }
        }
        tasks
    }

    pub fn update_task(&mut self, task: u32, device: &Vec<u8>) {
        self.tasks.get_mut(task as usize).unwrap().subtasks.insert(device.clone(), true);
    }
}

pub struct Task {
    subtasks: HashMap<Vec<u8>, bool>
}

impl Task {
    pub fn new(devices: &[Vec<u8>]) -> Self {
        let mut subtasks = HashMap::new();
        for device in devices.iter() {
            subtasks.insert(device.clone(), false);
        }
        Task { subtasks }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    rpc::run_rpc(State::new()).await
}
