use std::fs::File;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};

use prost::Message;

use crate::device::Device;
use crate::group::Group;
use crate::proto::*;
use crate::protocols::{Communicator, ProtocolType};
use crate::task::TaskResult;
use crate::task::{Task, TaskStatus, TaskType};

const LAST_ROUND_GROUP: u16 = 6;
const LAST_ROUND_SIGN: u16 = 10;

// TODO remove once #![feature(exclusive_range_pattern)] gets stabilized
const LAST_ROUND_GROUP_MINUS_ONE: u16 = LAST_ROUND_GROUP - 1;
const LAST_ROUND_SIGN_MINUS_ONE: u16 = LAST_ROUND_SIGN - 1;

pub struct GG18Group {
    name: String,
    threshold: u32,
    devices: Vec<Device>,
    communicator: Communicator,
    result: Option<Group>,
    round: u16,
    failed: Option<String>,
}

impl GG18Group {
    pub fn new(name: &str, devices: &[Device], threshold: u32) -> Self {
        assert!(threshold <= devices.len() as u32);

        let mut devices = devices.to_vec();
        devices.sort_by_key(|x| x.identifier().to_vec());

        let message = GroupRequest {
            device_ids: devices.iter().map(|x| x.identifier().to_vec()).collect(),
            name: String::from(name),
            threshold: Some(threshold),
            protocol: Some(0),
        };

        let mut communicator =
            Communicator::new(&devices, devices.len() as u32, message.encode_to_vec());
        communicator.broadcast(message.encode_to_vec());

        GG18Group {
            name: name.into(),
            threshold,
            devices,
            communicator,
            result: None,
            round: 0,
            failed: None,
        }
    }

    fn id_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.devices
            .iter()
            .position(|x| x.identifier() == device_id)
    }

    fn start_task(&mut self) {
        assert_eq!(self.round, 0);

        let parties = self.devices.len() as u32;
        let threshold = self.threshold;
        self.communicator.send_all(|idx| {
            (Gg18KeyGenInit {
                index: idx as u32,
                parties,
                threshold,
            })
            .encode_to_vec()
        });
        self.communicator.clear_input();
    }

    fn advance_task(&mut self) {
        assert!((0..LAST_ROUND_GROUP).contains(&self.round));

        self.communicator.relay();
        self.communicator.clear_input();
    }

    fn finalize_task(&mut self) {
        assert_eq!(self.round, LAST_ROUND_GROUP);

        let identifier = self.communicator.input[0][1].clone().unwrap();
        let group_name = if self.devices.len() <= 5 {
            format!(
                "{} ({})",
                &self.name,
                &self
                    .devices
                    .iter()
                    .map(|x| x.name())
                    .collect::<Vec<_>>()
                    .join(" & ")
            )
        } else {
            format!("{} ({} devices)", &self.name, self.devices.len())
        };
        let certificate = issue_certificate(&group_name, &identifier);

        self.result = Some(Group::new(
            identifier,
            self.name.clone(),
            self.devices.iter().map(Device::clone).collect(),
            self.threshold,
            ProtocolType::GG18,
            certificate,
        ));

        self.communicator.clear_input();
    }

    fn next_round(&mut self) {
        assert!(self.round <= LAST_ROUND_GROUP);

        match self.round {
            0 => self.start_task(),                                // Created -> Running
            1..=LAST_ROUND_GROUP_MINUS_ONE => self.advance_task(), // Running -> Running
            LAST_ROUND_GROUP => self.finalize_task(),              // Running -> Finished
            _ => unreachable!(),
        }
        self.round += 1;
    }
}

impl Task for GG18Group {
    fn get_status(&self) -> TaskStatus {
        if self.failed.is_some() {
            return TaskStatus::Failed(self.failed.clone().unwrap());
        }

        match self.round {
            0 => TaskStatus::Created,
            1..=LAST_ROUND_GROUP => TaskStatus::Running(self.round),
            _ => self
                .result
                .as_ref()
                .map(|_| TaskStatus::Finished)
                .unwrap_or(TaskStatus::Failed(String::from(
                    "The group was not established.",
                ))), // TODO Is it reachable?
        }
    }

    fn get_type(&self) -> TaskType {
        TaskType::Group
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return None;
        }

        self.id_to_index(device_id.unwrap())
            .map(|idx| self.communicator.get_message(idx).unwrap().clone())
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
            .id_to_index(device_id)
            .ok_or("Device ID not found.".to_string())?;

        match self.round {
            1..=LAST_ROUND_GROUP => {
                let data: Gg18Message =
                    Message::decode(data).map_err(|_| String::from("Expected GG18Message."))?;
                self.communicator.input[idx] = data.message.into_iter().map(Some).collect();
                self.communicator.input[idx].insert(idx, None);
            }
            _ => {
                let _data: TaskAcknowledgement = Message::decode(data)
                    .map_err(|_| String::from("Expected TaskAcknowledgement."))?;
                self.communicator.input[idx] =
                    vec![Some(vec![]); (self.communicator.threshold - 1) as usize];
                self.communicator.input[idx].insert(idx, None);
            }
        }

        if self.communicator.round_received(
            &self
                .devices
                .iter()
                .map(|x| x.identifier().to_vec())
                .collect::<Vec<_>>(),
        ) && self.round <= LAST_ROUND_GROUP
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
        if self.round == 0 {
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
        if self.round == 0 {
            if self.communicator.reject_count() > 0 {
                self.failed = Some("Too many rejections.".to_string());
            } else if self.communicator.accept_count() == self.devices.len() as u32 {
                self.next_round();
            }
        }
    }
}

pub struct GG18Sign {
    group: Group,
    devices: Vec<Device>,
    participant_ids: Option<Vec<Vec<u8>>>,
    communicator: Communicator,
    result: Option<Vec<u8>>,
    round: u16,
    document: Vec<u8>,
    pdfhelper: Option<Child>,
    failed: Option<String>,
}

impl GG18Sign {
    pub fn new(group: Group, name: String, data: Vec<u8>) -> Self {
        let mut devices: Vec<Device> = group.devices().values().map(Device::clone).collect();
        devices.sort_by_key(|x| x.identifier().to_vec());

        let message = SignRequest {
            group_id: group.identifier().to_vec(),
            name: name.clone(),
            data: data.clone(),
        };

        let mut communicator =
            Communicator::new(&devices, group.threshold(), message.encode_to_vec());
        communicator.broadcast(message.encode_to_vec());

        GG18Sign {
            group,
            devices,
            participant_ids: None,
            communicator,
            result: None,
            round: 0,
            document: data.clone(),
            pdfhelper: None,
            failed: None,
        }
    }

    fn id_to_index(&self, device_id: &[u8]) -> Option<usize> {
        self.participant_ids
            .as_ref()
            .and_then(|x| x.iter().position(|x| x == device_id))
    }

    fn start_task(&mut self) {
        assert_eq!(self.round, 0);
        {
            let mut file = File::create("document.pdf").unwrap();
            file.write_all(&self.document).unwrap();
        }

        let mut pdfhelper = Command::new("java")
            .arg("-jar")
            .arg("MeeSignHelper.jar")
            .arg("sign")
            .arg("document.pdf")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();

        let hash = request_hash(&mut pdfhelper, self.group.certificate());
        self.pdfhelper = Some(pdfhelper);
        std::fs::remove_file("document.pdf").unwrap();

        self.participant_ids = Some(self.communicator.get_participants());
        let mut indices: Vec<u32> = Vec::new();
        for i in 0..self.participant_ids.as_ref().unwrap().len() {
            indices.push(
                self.devices
                    .iter()
                    .position(|x| x.identifier() == &self.participant_ids.as_ref().unwrap()[i])
                    .unwrap() as u32,
            );
        }
        self.communicator.send_all(|idx| {
            (Gg18SignInit {
                indices: indices.clone(),
                index: idx as u32,
                hash: hash.clone(),
            })
            .encode_to_vec()
        });
        self.communicator.clear_input();
    }

    fn advance_task(&mut self) {
        assert!((0..LAST_ROUND_SIGN).contains(&self.round));

        self.communicator.relay();
        self.communicator.clear_input();
    }

    fn finalize_task(&mut self) {
        assert_eq!(self.round, LAST_ROUND_SIGN);

        let signature = self.communicator.input[0][1].clone().unwrap();
        let signed = include_signature(self.pdfhelper.as_mut().unwrap(), &signature);
        self.pdfhelper = None;
        self.result = Some(signed);

        self.communicator.clear_input();
    }

    fn next_round(&mut self) {
        assert!(self.round <= LAST_ROUND_SIGN);

        match self.round {
            0 => self.start_task(),                               // Created -> Running
            1..=LAST_ROUND_SIGN_MINUS_ONE => self.advance_task(), // Running -> Running
            LAST_ROUND_SIGN => self.finalize_task(),              // Running -> Finished
            _ => unreachable!(),
        }
        self.round += 1;
    }
}

impl Task for GG18Sign {
    fn get_status(&self) -> TaskStatus {
        if self.failed.is_some() {
            return TaskStatus::Failed(self.failed.clone().unwrap());
        }

        match self.round {
            0 => TaskStatus::Created,
            1..=LAST_ROUND_SIGN => TaskStatus::Running(self.round),
            _ => self
                .result
                .as_ref()
                .map(|_| TaskStatus::Finished)
                .unwrap_or(TaskStatus::Failed(String::from(
                    "Server did not receive a signature.",
                ))), // TODO Is it reachable?
        }
    }

    fn get_type(&self) -> TaskType {
        TaskType::Sign
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return None;
        }

        self.id_to_index(device_id.unwrap())
            .map(|idx| self.communicator.get_message(idx).unwrap().clone())
    }

    fn get_result(&self) -> Option<TaskResult> {
        self.result.as_ref().map(|x| TaskResult::Signed(x.clone()))
    }

    fn get_confirmations(&self) -> (u32, u32) {
        (
            self.communicator.accept_count(),
            self.communicator.reject_count(),
        )
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<(), String> {
        if self.communicator.accept_count() != self.communicator.threshold as u32 {
            return Err("Not enough agreements to proceed with the protocol.".to_string());
        }

        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string());
        }

        let idx = self
            .id_to_index(device_id)
            .ok_or("Device ID not found.".to_string())?;

        match self.round {
            1..=LAST_ROUND_SIGN => {
                let data: Gg18Message =
                    Message::decode(data).map_err(|_| String::from("Expected GG18Message."))?;
                self.communicator.input[idx] = data.message.into_iter().map(Some).collect();
                self.communicator.input[idx].insert(idx, None);
            }
            _ => {
                let _data: TaskAcknowledgement = Message::decode(data)
                    .map_err(|_| String::from("Expected TaskAcknowledgement."))?;
                self.communicator.input[idx] =
                    vec![Some(vec![]); (self.communicator.threshold - 1) as usize];
                self.communicator.input[idx].insert(idx, None);
            }
        }

        if self.participant_ids.is_some()
            && self
                .communicator
                .round_received(&self.participant_ids.as_ref().unwrap())
            && self.round <= LAST_ROUND_SIGN
        {
            self.next_round();
        }
        Ok(())
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        return self.participant_ids.is_some()
            && self
                .participant_ids
                .as_ref()
                .unwrap()
                .contains(&device_id.to_vec());
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        if self.round == 0 {
            return !self.communicator.device_confirmed(device);
        }

        self.participant_ids
            .as_ref()
            .unwrap()
            .iter()
            .zip((&self.communicator.input).into_iter())
            .filter(|(_a, b)| b.iter().all(|x| x.is_none()))
            .map(|(a, _b)| a.as_slice())
            .any(|x| x == device)
    }

    fn confirmation(&mut self, device_id: &[u8], accept: bool) {
        self.communicator.confirmation(device_id, accept);
        if self.round == 0 {
            if self.communicator.reject_count() >= self.group.reject_threshold() {
                self.failed = Some("Too many rejections.".to_string());
            } else if self.communicator.accept_count() >= self.group.threshold() {
                self.next_round();
            }
        }
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

fn request_hash(process: &mut Child, certificate: &[u8]) -> Vec<u8> {
    let process_stdin = process.stdin.as_mut().unwrap();
    let process_stdout = process.stdout.as_mut().unwrap();

    process_stdin.write(certificate).unwrap();
    process_stdin.flush().unwrap();

    let mut in_buffer = [0u8; 65]; // \n
    process_stdout.read_exact(&mut in_buffer).unwrap();

    hex::decode(String::from_utf8(Vec::from(&in_buffer[..64])).unwrap()).unwrap()
}

fn include_signature(process: &mut Child, signature: &[u8]) -> Vec<u8> {
    let process_stdin = process.stdin.as_mut().unwrap();
    let process_stdout = process.stdout.as_mut().unwrap();

    let mut out_buffer = [0u8; 129];
    out_buffer[..128].copy_from_slice(hex::encode(&signature).as_bytes());
    out_buffer[128] = '\n' as u8;

    process_stdin.write(&out_buffer).unwrap();

    let mut result = Vec::new();
    process_stdout.read_to_end(&mut result).unwrap();
    hex::decode(&result).unwrap()
}
