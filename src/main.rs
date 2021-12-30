use std::collections::{HashMap, HashSet};

mod rpc;
mod task;
mod group;
mod device;

mod proto {
    tonic::include_proto!("meesign");
}

use crate::task::{Task, TaskStatus, TaskType};
use crate::group::Group;
use crate::device::Device;

pub struct State {
    devices: HashSet<Device>,
    groups: HashSet<Group>,
    tasks: Vec<Box<dyn Task + Send + Sync>>,
}

impl State {
    pub fn new() -> Self {
        State {
            devices: HashSet::new(),
            groups: HashSet::new(),
            tasks: Vec::new(),
        }
    }

    pub fn add_device(&mut self, id: &[u8], name: &str) {
        let device = Device::new(id.to_vec(), name.to_owned());
        self.devices.insert(device);
    }

    pub fn add_group_task(&mut self, name: &str, devices: &[Vec<u8>], threshold: u32) -> Option<u32> {
        if threshold > devices.len() as u32 {
            return None
        }
        for device in devices {
            if !self.devices.contains(device.as_slice()) {
                return None
            }
        }
        self.add_task(Box::new(GroupTask::new(name, devices, threshold)));
        Some((self.tasks.len() - 1) as u32)
    }

    pub fn add_sign_task(&mut self, group: &[u8], data: &[u8]) -> u32 {
        let devices = self.groups.get(group).unwrap().devices().clone();
        self.add_task(Box::new(SignTask::new(&devices, data.to_vec())))
    }

    fn add_task(&mut self, task: Box<dyn Task + Send + Sync>) -> u32 {
        self.tasks.push(task);
        (self.tasks.len() - 1) as u32
    }

    pub fn get_device_tasks(&self, device: &Vec<u8>) -> Vec<(u32, (TaskType, TaskStatus))> {
        let mut tasks = Vec::new();
        for (idx, task) in self.tasks.iter().enumerate() {
            if task.waiting_for(device) {
                tasks.push((idx as u32, task.get_status()));
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

    pub fn get_task(&self, task: u32) -> (TaskType, TaskStatus) {
        self.tasks.get(task as usize).unwrap().get_status()
    }

    pub fn get_work(&self, task: u32, device: &[u8]) -> Option<Vec<u8>> {
        self.tasks.get(task as usize).unwrap().get_work(device)
    }

    pub fn update_task(&mut self, task: u32, device: &[u8], data: &[u8]) -> TaskStatus {
        let task = self.tasks.get_mut(task as usize).unwrap();
        let status = task.update(device, data).unwrap();
        match &status {
            TaskStatus::GroupEstablished(group) => {
                self.groups.insert(group.clone());
            },
            _ => ()
        }
        status
    }

    pub fn get_devices(&self) -> Vec<Device> {
        self.devices.iter().map(Device::clone).collect()
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
    fn get_status(&self) -> (TaskType, TaskStatus) {
        let waiting: Vec<_> = self.subtasks.iter()
            .filter(|(_, value)| !*value)
            .map(|(key, _)| key.clone())
            .collect();

        if !waiting.is_empty() {
            (TaskType::Sign, TaskStatus::Waiting(waiting))
        } else {
            (TaskType::Sign, TaskStatus::Signed(self.result.clone()))
        }
    }

    fn update(&mut self, device_id: &[u8], _data: &[u8]) -> Result<TaskStatus, String> {
        if self.subtasks.contains_key(device_id) {
            self.subtasks.insert(device_id.to_vec(), true);
            Ok(self.get_status().1)
        } else {
            Err("Incompatible device ID".into())
        }
    }

    fn get_work(&self, device_id: &[u8]) -> Option<Vec<u8>> {
        if *self.subtasks.get(device_id).unwrap_or(&false) {
            None
        } else {
            Some(self.data.clone())
        }
    }
}

pub struct GroupTask {
    name: String,
    subtasks: HashMap<Vec<u8>, bool>,
    threshold: u32,
    result: Option<Group>,
}

impl GroupTask {
    pub fn new(name: &str, devices: &[Vec<u8>], threshold: u32) -> Self {
        assert!(threshold <= devices.len() as u32);

        let mut subtasks = HashMap::new();
        for device in devices.iter() {
            subtasks.insert(device.clone(), false);
        }

        GroupTask { name: name.to_owned(), subtasks, threshold, result: None }
    }

    fn try_advance(&mut self) -> bool {
        if self.result.is_none() && self.subtasks.values().all(|x| *x) {
            let mut identifier = Vec::new();
            for device in self.subtasks.keys() {
                identifier.extend_from_slice(device);
            }
            self.result = Some(Group::new(identifier,  self.name.clone(), self.subtasks.keys().map(Vec::clone).collect(), self.threshold));
            true
        } else {
            false
        }
    }
}

impl Task for GroupTask {
    fn get_status(&self) -> (TaskType, TaskStatus) {
        let waiting: Vec<_> = self.subtasks.iter()
            .filter(|(_, value)| !*value)
            .map(|(key, _)| key.clone())
            .collect();

        if !waiting.is_empty() {
            (TaskType::Group, TaskStatus::Waiting(waiting))
        } else {
            (TaskType::Group, TaskStatus::GroupEstablished(self.result.as_ref().unwrap().clone()))
        }
    }

    fn update(&mut self, device_id: &[u8], _data: &[u8]) -> Result<TaskStatus, String> {
        if self.subtasks.contains_key(device_id) {
            self.subtasks.insert(device_id.to_vec(), true);
            self.try_advance();
            Ok(self.get_status().1)
        } else {
            Err("Incompatible device ID".into())
        }
    }

    fn get_work(&self, device_id: &[u8]) -> Option<Vec<u8>> {
        if *self.subtasks.get(device_id).unwrap_or(&false) {
            None
        } else {
            let mut data = Vec::new();
            data.extend(self.name.as_bytes());
            data.push(0x00);
            for device in self.subtasks.keys() {
                data.extend(device);
            }
            Some(data)
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), String> {
    rpc::run_rpc(State::new()).await
}
