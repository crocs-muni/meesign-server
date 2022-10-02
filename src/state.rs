use std::collections::HashMap;

use log::{error, warn};
use uuid::Uuid;

use crate::device::Device;
use crate::group::Group;
use crate::proto::{KeyType, ProtocolType};
use crate::tasks::group::GroupTask;
use crate::tasks::sign_pdf::SignPDFTask;
use crate::tasks::{Task, TaskResult, TaskStatus};

pub struct State {
    devices: HashMap<Vec<u8>, Device>,
    groups: HashMap<Vec<u8>, Group>,
    tasks: HashMap<Uuid, Box<dyn Task + Send + Sync>>,
}

impl State {
    pub fn new() -> Self {
        State {
            devices: HashMap::new(),
            groups: HashMap::new(),
            tasks: HashMap::new(),
        }
    }

    pub fn add_device(&mut self, identifier: &[u8], name: &str) -> bool {
        if name.chars().count() > 64
            || name
                .chars()
                .any(|x| x.is_ascii_punctuation() || x.is_control())
        {
            warn!("Invalid Device name {}", name);
            return false;
        }

        let device = Device::new(identifier.to_vec(), name.to_owned());
        // TODO improve when feature map_try_insert gets stabilized
        if self.devices.contains_key(identifier) {
            warn!(
                "Device identifier already registered {}",
                hex::encode(identifier)
            );
            return false;
        }
        self.devices.insert(identifier.to_vec(), device);
        true
    }

    pub fn add_group_task(
        &mut self,
        name: &str,
        devices: &[Vec<u8>],
        threshold: u32,
        protocol: ProtocolType,
        key_type: KeyType,
    ) -> Option<Uuid> {
        if name.chars().count() > 64
            || name
                .chars()
                .any(|x| x.is_ascii_punctuation() || x.is_control())
        {
            warn!("Invalid Group name {}", name);
            return None;
        }

        if !protocol.check_key_type(key_type) {
            warn!(
                "Protocol {:?} does not support {:?} key type",
                protocol, key_type
            )
        }

        let mut device_list = Vec::new();
        for device in devices {
            if !self.devices.contains_key(device.as_slice()) {
                warn!("Unknown Device ID {}", hex::encode(device));
                return None;
            }
            device_list.push(self.devices.get(device.as_slice()).unwrap().clone());
        }

        let task: Box<dyn Task + Send + Sync + 'static> = match protocol {
            ProtocolType::Gg18 => Box::new(GroupTask::new(name, &device_list, threshold)),
        };

        Some(self.add_task(task))
    }

    pub fn add_sign_task(&mut self, group: &[u8], name: &str, data: &[u8]) -> Option<Uuid> {
        if data.len() > 8 * 1024 * 1024 || name.len() > 256 || name.chars().any(|x| x.is_control())
        {
            warn!("Invalid PDF name {} ({} B)", name, data.len());
            return None;
        }

        self.groups.get(group).cloned().map(|group| {
            let task: Box<dyn Task + Send + Sync + 'static> = match group.protocol() {
                ProtocolType::Gg18 => {
                    Box::new(SignPDFTask::new(group, name.to_string(), data.to_vec()))
                }
            };
            self.add_task(task)
        })
    }

    fn add_task(&mut self, task: Box<dyn Task + Send + Sync>) -> Uuid {
        let uuid = Uuid::new_v4();
        self.tasks.insert(uuid, task);
        uuid
    }

    pub fn get_device_tasks(&self, device: &[u8]) -> Vec<(Uuid, &Box<dyn Task + Send + Sync>)> {
        let mut tasks = Vec::new();
        for (uuid, task) in self.tasks.iter() {
            // TODO refactor
            if task.has_device(device)
                && (task.get_status() != TaskStatus::Finished
                    || (task.get_status() == TaskStatus::Finished
                        && !task.device_acknowledged(device)))
            {
                tasks.push((*uuid, task));
            }
        }
        tasks
    }

    pub fn get_device_groups(&self, device: &Vec<u8>) -> Vec<Group> {
        let mut groups = Vec::new();
        for group in self.groups.values() {
            if group.contains(device) {
                groups.push(group.clone());
            }
        }
        groups
    }

    pub fn get_groups(&self) -> &HashMap<Vec<u8>, Group> {
        &self.groups
    }

    pub fn get_tasks(&self) -> &HashMap<Uuid, Box<dyn Task + Send + Sync>> {
        &self.tasks
    }

    pub fn get_task(&self, task: &Uuid) -> Option<&Box<dyn Task + Send + Sync>> {
        self.tasks.get(task)
    }

    pub fn update_task(&mut self, task: &Uuid, device: &[u8], data: &[u8]) -> Result<bool, String> {
        let task = self.tasks.get_mut(task).unwrap();
        let previous_status = task.get_status();
        let update_result = task.update(device, data);
        if previous_status != TaskStatus::Finished && task.get_status() == TaskStatus::Finished {
            // TODO join if statements once #![feature(let_chains)] gets stabilized
            if let TaskResult::GroupEstablished(group) = task.get_result().unwrap() {
                self.groups.insert(group.identifier().to_vec(), group);
            }
        }
        update_result
    }

    pub fn decide_task(&mut self, task: &Uuid, device: &[u8], decision: bool) -> bool {
        let task = self.tasks.get_mut(task).unwrap();
        task.decide(device, decision)
    }

    pub fn acknowledge_task(&mut self, task: &Uuid, device: &[u8]) {
        let task = self.tasks.get_mut(task).unwrap();
        task.acknowledge(device);
    }

    pub fn get_devices(&self) -> &HashMap<Vec<u8>, Device> {
        &self.devices
    }

    pub fn device_activated(&mut self, device_id: &[u8]) {
        if let Some(device) = self.devices.get_mut(device_id) {
            device.activated();
        } else {
            error!("Unknown Device ID {}", hex::encode(device_id));
        }
    }
}
