use prost::Message;

use crate::communicator::Communicator;
use crate::device::Device;
use crate::group::Group;
use crate::proto::*;
use crate::protocols::gg18::GG18Group;
use crate::protocols::Protocol;
use crate::tasks::{Task, TaskResult, TaskStatus, TaskType};
use std::io::Read;
use std::process::{Command, Stdio};

pub struct GroupTask {
    name: String,
    threshold: u32,
    devices: Vec<Device>,
    communicator: Communicator,
    result: Option<Group>,
    failed: Option<String>,
    protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
}

impl GroupTask {
    pub fn new(name: &str, devices: &[Device], threshold: u32) -> Self {
        let devices_len = devices.len() as u32;
        assert!(threshold <= devices_len);

        let mut devices = devices.to_vec();
        devices.sort_by_key(|x| x.identifier().to_vec());

        let communicator = Communicator::new(&devices, devices.len() as u32);

        let request = (GroupRequest {
            device_ids: devices.iter().map(|x| x.identifier().to_vec()).collect(),
            name: String::from(name),
            threshold,
            protocol: ProtocolType::Gg18 as i32,
            key_type: KeyType::SignPdf as i32,
        })
        .encode_to_vec();

        GroupTask {
            name: name.into(),
            threshold,
            devices,
            communicator,
            result: None,
            failed: None,
            protocol: Box::new(GG18Group::new(devices_len, threshold)),
            request,
        }
    }

    fn start_task(&mut self) {
        self.protocol.initialize(&mut self.communicator, &[]);
    }

    fn advance_task(&mut self) {
        self.protocol.advance(&mut self.communicator);
    }

    fn finalize_task(&mut self) {
        let identifier = self.protocol.finalize(&mut self.communicator);
        let certificate = issue_certificate(&self.name, &identifier);

        self.result = Some(Group::new(
            identifier,
            self.name.clone(),
            self.devices.iter().map(Device::clone).collect(),
            self.threshold,
            ProtocolType::Gg18,
            KeyType::SignPdf,
            certificate,
        ));

        self.communicator.clear_input();
    }

    fn next_round(&mut self) {
        if self.protocol.round() == 0 {
            self.start_task();
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task()
        } else {
            self.finalize_task()
        }
    }
}

impl Task for GroupTask {
    fn get_status(&self) -> TaskStatus {
        if self.failed.is_some() {
            return TaskStatus::Failed(self.failed.clone().unwrap());
        }

        if self.protocol.round() == 0 {
            TaskStatus::Created
        } else if self.protocol.round() <= self.protocol.last_round() {
            TaskStatus::Running(self.protocol.round())
        } else {
            self.result
                .as_ref()
                .map(|_| TaskStatus::Finished)
                .unwrap_or(TaskStatus::Failed(String::from(
                    "The group was not established.",
                )))
        }
    }

    fn get_type(&self) -> TaskType {
        TaskType::Group
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return None;
        }

        self.communicator
            .identifier_to_index(device_id.unwrap())
            .and_then(|idx| self.communicator.get_message(idx).cloned())
    }

    fn get_result(&self) -> Option<TaskResult> {
        self.result
            .as_ref()
            .map(|x| TaskResult::GroupEstablished(x.clone()))
    }

    fn get_confirmations(&self) -> (u32, u32) {
        (
            self.communicator.accept_count(),
            self.communicator.reject_count(),
        )
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<(), String> {
        if self.communicator.accept_count() != self.devices.len() as u32 {
            return Err("Not enough agreements to proceed with the protocol.".to_string());
        }

        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string());
        }

        let idx = self
            .communicator
            .identifier_to_index(device_id)
            .ok_or("Device ID not found.".to_string())?;

        if 0 < self.protocol.round() && self.protocol.round() <= self.protocol.last_round() {
            let data: Gg18Message =
                Message::decode(data).map_err(|_| String::from("Expected GG18Message."))?;
            self.communicator.input[idx] = data.message.into_iter().map(Some).collect();
            self.communicator.input[idx].insert(idx, None);
        } else {
            let _data: TaskAcknowledgement =
                Message::decode(data).map_err(|_| String::from("Expected TaskAcknowledgement."))?;
            self.communicator.input[idx] =
                vec![Some(vec![]); (self.communicator.threshold - 1) as usize];
            self.communicator.input[idx].insert(idx, None);
        }

        if self.communicator.round_received(
            &self
                .devices
                .iter()
                .map(|x| x.identifier().to_vec())
                .collect::<Vec<_>>(),
        ) && self.protocol.round() <= self.protocol.last_round()
        {
            self.next_round();
        }

        Ok(())
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        return self
            .devices
            .iter()
            .map(Device::identifier)
            .any(|x| x == device_id);
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        if self.protocol.round() == 0 {
            return !self.communicator.device_confirmed(device);
        }

        self.devices
            .iter()
            .map(Device::identifier)
            .zip((&self.communicator.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .map(|(a, _b)| a)
            .any(|x| x == device)
    }

    fn confirmation(&mut self, device_id: &[u8], accept: bool) {
        self.communicator.confirmation(device_id, accept);
        if self.protocol.round() == 0 {
            if self.communicator.reject_count() > 0 {
                self.failed = Some("Too many rejections.".to_string());
            } else if self.communicator.accept_count() == self.devices.len() as u32 {
                self.next_round();
            }
        }
    }

    fn get_request(&self) -> &[u8] {
        &self.request
    }
}

fn issue_certificate(name: &str, public_key: &[u8]) -> Vec<u8> {
    assert_eq!(public_key.len(), 65);
    let mut process = Command::new("java")
        .arg("-jar")
        .arg("MeeSignHelper.jar")
        .arg("cert")
        .arg(name)
        .arg(hex::encode(public_key))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let mut result = Vec::new();
    process
        .stdout
        .as_mut()
        .unwrap()
        .read_to_end(&mut result)
        .unwrap();
    result
}
