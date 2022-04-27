#![feature(exclusive_range_pattern)]

use std::collections::{HashMap, HashSet};

mod rpc;
mod task;
mod group;
mod device;
mod protocols;

mod proto {
    tonic::include_proto!("meesign");
}

use crate::task::{Task, TaskStatus, TaskType, TaskResult};
use crate::group::Group;
use crate::device::Device;
use uuid::Uuid;
use crate::protocols::ProtocolType;
use crate::protocols::gg18::{GG18Group, GG18Sign};

pub struct State {
    devices: HashSet<Device>,
    groups: HashSet<Group>,
    tasks: HashMap<Uuid, Box<dyn Task + Send + Sync>>,
}

impl State {
    pub fn new() -> Self {
        State {
            devices: HashSet::new(),
            groups: HashSet::new(),
            tasks: HashMap::new(),
        }
    }

    pub fn add_device(&mut self, id: &[u8], name: &str) {
        let device = Device::new(id.to_vec(), name.to_owned());
        self.devices.insert(device);
    }

    pub fn add_group_task(&mut self, name: &str, devices: &[Vec<u8>], threshold: u32, protocol: ProtocolType) -> Option<Uuid> {
        if threshold > devices.len() as u32 {
            return None
        }
        for device in devices {
            if !self.devices.contains(device.as_slice()) {
                return None
            }
        }

        let task: Box<dyn Task + Send + Sync + 'static> = match protocol {
            ProtocolType::GG18 => Box::new(GG18Group::new(name, devices, threshold))
        };

        Some(self.add_task(task))
    }

    pub fn add_sign_task(&mut self, group: &[u8], data: &[u8]) -> Uuid {
        let group = self.groups.get(group).unwrap().clone();
        let task: Box<dyn Task + Send + Sync + 'static> = match group.protocol() {
            ProtocolType::GG18 => Box::new(GG18Sign::new(group, data.to_vec())),
        };
        self.add_task(task)
    }

    fn add_task(&mut self, task: Box<dyn Task + Send + Sync>) -> Uuid {
        let uuid = Uuid::new_v4();
        self.tasks.insert(uuid, task);
        uuid
    }

    pub fn get_device_tasks(&self, device: &[u8]) -> Vec<(Uuid, &Box<dyn Task + Send + Sync>)> {
        let mut tasks = Vec::new();
        for (uuid, task) in self.tasks.iter() {
            if task.waiting_for(device) {
                tasks.push((uuid.clone(), task));
            }
        }
        tasks
    }

    pub fn get_device_groups(&self, device: &Vec<u8>) -> Vec<Group> {
        let mut groups = Vec::new();
        for group in self.groups.iter() {
            if group.contains(device) {
                groups.push(group.clone());
            }
        }
        groups
    }

    pub fn get_task(&self, task: &Uuid) -> Option<&Box<dyn Task + Send + Sync>> {
        self.tasks.get(task)
    }

    pub fn get_work(&self, task: &Uuid, device: &[u8]) -> Option<Vec<u8>> {
        self.tasks.get(task).unwrap().get_work(Some(device))
    }

    pub fn update_task(&mut self, task: &Uuid, device: &[u8], data: &[u8]) -> Result<TaskStatus, String> {
        let task = self.tasks.get_mut(task).unwrap();
        let status = task.update(device, data)?;
        match &status {
            TaskStatus::Finished => {
                if let TaskResult::GroupEstablished(group) = task.get_result().unwrap() {
                    self.groups.insert(group);
                }
            },
            _ => ()
        }
        Ok(status)
    }

    pub fn get_devices(&self) -> Vec<Device> {
        self.devices.iter().map(Device::clone).collect()
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    env_logger::init();
    rpc::run_rpc(State::new()).await
}
