use std::collections::HashMap;

use log::{error, warn};
use uuid::Uuid;

use crate::device::{Device, Role};
use crate::group::Group;
use crate::interfaces::grpc::format_task;
use crate::proto::{KeyType, ProtocolType};
use crate::tasks::decrypt::DecryptTask;
use crate::tasks::group::GroupTask;
use crate::tasks::sign::SignTask;
use crate::tasks::sign_pdf::SignPDFTask;
use crate::tasks::{Task, TaskResult, TaskStatus};
use log::info;
use tokio::sync::mpsc::Sender;
use tonic::codegen::Arc;
use tonic::Status;

pub struct State {
    devices: HashMap<Vec<u8>, Arc<Device>>,
    groups: HashMap<Vec<u8>, Group>,
    tasks: HashMap<Uuid, Box<dyn Task + Send + Sync>>,
    subscribers: HashMap<Vec<u8>, Sender<Result<crate::proto::Task, Status>>>,
}

impl State {
    pub fn new() -> Self {
        State {
            devices: HashMap::new(),
            groups: HashMap::new(),
            tasks: HashMap::new(),
            subscribers: HashMap::new(),
        }
    }

    pub fn add_device(
        &mut self,
        identifier: &[u8],
        name: &str,
        certificate: &[u8],
        role: Role,
    ) -> bool {
        if name.chars().count() > 64
            || name
                .chars()
                .any(|x| x.is_ascii_punctuation() || x.is_control())
        {
            warn!("Invalid Device name {}", name);
            return false;
        }

        let device = Device::new(
            identifier.to_vec(),
            name.to_owned(),
            certificate.to_vec(),
            role,
        );
        // TODO improve when feature map_try_insert gets stabilized
        if self.devices.contains_key(identifier) {
            warn!(
                "Device identifier already registered {}",
                hex::encode(identifier)
            );
            return false;
        }
        self.devices.insert(identifier.to_vec(), Arc::new(device));
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

        let mut device_list = Vec::new();
        for device in devices {
            if !self.devices.contains_key(device.as_slice()) {
                warn!("Unknown Device ID {}", hex::encode(device));
                return None;
            }
            device_list.push(self.devices.get(device.as_slice()).unwrap().clone());
        }

        let task = GroupTask::try_new(name, &device_list, threshold, protocol, key_type)
            .ok()
            .map(|task| Box::new(task) as Box<dyn Task + Send + Sync>);

        let task_id = task.map(|task| self.add_task(task));
        if let Some(task_id) = &task_id {
            self.send_updates(task_id);
        }
        task_id
    }

    pub fn add_sign_task(&mut self, group_id: &[u8], name: &str, data: &[u8]) -> Option<Uuid> {
        let group = self.groups.get(group_id);
        if group.is_none() {
            warn!(
                "Signing requested from an unknown group group_id={}",
                hex::encode(group_id)
            );
            return None;
        }
        let group = group.unwrap();
        let task = match group.key_type() {
            KeyType::SignPdf => {
                SignPDFTask::try_new(group.clone(), name.to_string(), data.to_vec())
                    .ok()
                    .map(|task| Box::new(task) as Box<dyn Task + Sync + Send>)
            }
            KeyType::SignChallenge => {
                SignTask::try_new(group.clone(), name.to_string(), data.to_vec())
                    .ok()
                    .map(|task| Box::new(task) as Box<dyn Task + Sync + Send>)
            }
            KeyType::Decrypt => {
                warn!(
                    "Signing request made for decryption group group_id={}",
                    hex::encode(group_id)
                );
                return None;
            }
        };

        let task_id = task.map(|task| self.add_task(task));
        if let Some(task_id) = &task_id {
            self.send_updates(task_id);
        }
        task_id
    }

    pub fn add_decrypt_task(&mut self, group_id: &[u8], name: &str, data: &[u8]) -> Option<Uuid> {
        let group = self.groups.get(group_id);
        if group.is_none() {
            warn!(
                "Decryption requested from an unknown group group_id={}",
                hex::encode(group_id)
            );
            return None;
        }
        let group = group.unwrap();
        let task = match group.key_type() {
            KeyType::Decrypt => Some(DecryptTask::new(
                group.clone(),
                name.to_string(),
                data.to_vec(),
            ))
            .map(|task| Box::new(task) as Box<dyn Task + Sync + Send>),
            KeyType::SignPdf | KeyType::SignChallenge => {
                warn!(
                    "Decryption request made for a signing group group_id={}",
                    hex::encode(group_id)
                );
                return None;
            }
        };

        let task_id = task.map(|task| self.add_task(task));
        if let Some(task_id) = &task_id {
            self.send_updates(task_id);
        }
        task_id
    }

    fn add_task(&mut self, task: Box<dyn Task + Sync + Send>) -> Uuid {
        let uuid = Uuid::new_v4();
        self.tasks.insert(uuid, task);
        uuid
    }

    pub fn get_device_tasks(&self, device: &[u8]) -> Vec<(Uuid, &dyn Task)> {
        let mut tasks = Vec::new();
        for (uuid, task) in self.tasks.iter() {
            // TODO refactor
            if task.has_device(device)
                && (task.get_status() != TaskStatus::Finished
                    || (task.get_status() == TaskStatus::Finished
                        && !task.device_acknowledged(device)))
            {
                tasks.push((*uuid, task.as_ref() as &dyn Task));
            }
        }
        tasks
    }

    pub fn get_device_groups(&self, device: &[u8]) -> Vec<Group> {
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

    pub fn get_task(&self, task: &Uuid) -> Option<&dyn Task> {
        self.tasks.get(task).map(|task| task.as_ref() as &dyn Task)
    }

    pub fn update_task(
        &mut self,
        task_id: &Uuid,
        device: &[u8],
        data: &[u8],
        attempt: u32,
    ) -> Result<bool, String> {
        let task = self.tasks.get_mut(task_id).unwrap();
        if attempt != task.get_attempts() {
            warn!(
                "Stale update discarded task_id={} device_id={} attempt={}",
                hex::encode(task_id),
                hex::encode(device),
                attempt
            );
            return Err("Stale update".to_string());
        }

        let previous_status = task.get_status();
        let update_result = task.update(device, data);
        if previous_status != TaskStatus::Finished && task.get_status() == TaskStatus::Finished {
            // TODO join if statements once #![feature(let_chains)] gets stabilized
            if let TaskResult::GroupEstablished(group) = task.get_result().unwrap() {
                self.groups.insert(group.identifier().to_vec(), group);
            }
        }
        if let Ok(true) = update_result {
            self.send_updates(task_id);
        }
        update_result
    }

    pub fn decide_task(&mut self, task_id: &Uuid, device: &[u8], decision: bool) -> bool {
        let task = self.tasks.get_mut(task_id).unwrap();
        let change = task.decide(device, decision);
        if change.is_some() {
            self.send_updates(task_id);
        }
        change.is_some()
    }

    pub fn acknowledge_task(&mut self, task: &Uuid, device: &[u8]) {
        let task = self.tasks.get_mut(task).unwrap();
        task.acknowledge(device);
    }

    pub fn get_devices(&self) -> &HashMap<Vec<u8>, Arc<Device>> {
        &self.devices
    }

    pub fn device_activated(&self, device_id: &[u8]) {
        if let Some(device) = self.devices.get(device_id) {
            device.activated();
        } else {
            error!("Unknown Device ID {}", hex::encode(device_id));
        }
    }

    pub fn restart_task(&mut self, task_id: &Uuid) -> bool {
        if self
            .tasks
            .get_mut(task_id)
            .and_then(|task| task.restart().ok())
            .unwrap_or(false)
        {
            self.send_updates(task_id);
            true
        } else {
            false
        }
    }

    pub fn add_subscriber(
        &mut self,
        device_id: Vec<u8>,
        tx: Sender<Result<crate::proto::Task, Status>>,
    ) {
        self.subscribers.insert(device_id, tx);
    }

    pub fn remove_subscriber(&mut self, device_id: &Vec<u8>) {
        self.subscribers.remove(device_id);
        info!("Removing subscriber device_id={:?}", hex::encode(device_id));
    }

    pub fn get_subscribers(&self) -> &HashMap<Vec<u8>, Sender<Result<crate::proto::Task, Status>>> {
        &self.subscribers
    }

    fn send_updates(&mut self, task_id: &Uuid) {
        let task = self.get_task(task_id).unwrap();
        let mut remove = Vec::new();

        for device_id in task.get_devices().iter().map(|device| device.identifier()) {
            if let Some(tx) = self.subscribers.get(device_id) {
                let result = tx.try_send(Ok(format_task(task_id, task, Some(device_id), None)));

                if result.is_err() {
                    info!(
                        "Closed channel detected device_id={}",
                        hex::encode(device_id)
                    );
                    remove.push(device_id.to_vec());
                }
            }
        }

        for device_id in remove {
            self.remove_subscriber(&device_id);
        }
    }
}
