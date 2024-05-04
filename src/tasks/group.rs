use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::Device;
use crate::persistence::Task as TaskModel;
use crate::proto;
use crate::proto::{KeyType, ProtocolType, TaskType};
use crate::protocols::elgamal::ElgamalGroup;
use crate::protocols::frost::FROSTGroup;
use crate::protocols::gg18::GG18Group;
use crate::protocols::Protocol;
use crate::tasks::{Task, TaskResult, TaskStatus};
use crate::{get_timestamp, utils};
use log::{info, warn};
use meesign_crypto::proto::{ClientMessage, Message as _, ServerMessage};
use prost::Message as _;
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::RwLock;
use uuid::Uuid;

pub struct GroupTask {
    name: String,
    id: Uuid,
    threshold: u32,
    key_type: KeyType,
    devices: Vec<Device>,
    communicator: Arc<RwLock<Communicator>>, // TODO: consider using tokio RwLock for async
    result: Option<Result<Group, String>>,
    protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
    last_update: u64,
    attempts: u32,
    note: Option<String>,
    certificates_sent: bool,
}

impl GroupTask {
    pub fn try_new(
        name: &str,
        mut devices: Vec<Device>,
        threshold: u32,
        protocol_type: ProtocolType,
        key_type: KeyType,
        note: &Option<String>,
    ) -> Result<Self, String> {
        let devices_len = devices.len() as u32;
        let protocol: Box<dyn Protocol + Send + Sync> =
            create_protocol(protocol_type, key_type, devices_len, threshold)?;

        if devices_len < 1 {
            warn!("Invalid number of devices {}", devices_len);
            return Err("Invalid input".into());
        }
        if !protocol.get_type().check_threshold(threshold, devices_len) {
            warn!("Invalid group threshold {}-of-{}", threshold, devices_len);
            return Err("Invalid input".into());
        }

        devices.sort_by_key(|x| x.identifier().to_vec());

        let threshold = devices.len() as u32;
        let device_ids = devices.iter().map(|x| x.identifier().to_vec()).collect();
        let communicator = Communicator::new(devices.clone(), threshold, protocol.get_type());
        let communicator = Arc::new(RwLock::new(communicator));

        let request = (crate::proto::GroupRequest {
            device_ids,
            name: String::from(name),
            threshold,
            protocol: protocol.get_type() as i32,
            key_type: key_type as i32,
            note: note.to_owned(),
        })
        .encode_to_vec();

        Ok(GroupTask {
            name: name.into(),
            id: Uuid::new_v4(),
            threshold,
            devices: devices.to_vec(),
            key_type,
            communicator,
            result: None,
            protocol,
            request,
            last_update: get_timestamp(),
            attempts: 0,
            note: note.to_owned(),
            certificates_sent: false,
        })
    }

    fn start_task(&mut self) {
        self.protocol
            .initialize(&mut self.communicator.write().unwrap(), &[]);
    }

    fn advance_task(&mut self) {
        self.protocol
            .advance(&mut self.communicator.write().unwrap());
    }

    fn finalize_task(&mut self) {
        let identifier = self
            .protocol
            .finalize(&mut self.communicator.write().unwrap());
        if identifier.is_none() {
            self.result = Some(Err("Task failed (group key not output)".to_string()));
            return;
        }
        let identifier = identifier.unwrap();
        // TODO
        let certificate = if self.protocol.get_type() == ProtocolType::Gg18 {
            Some(issue_certificate(&self.name, &identifier))
        } else {
            None
        };

        info!(
            "Group established group_id={} devices={:?}",
            utils::hextrunc(&identifier),
            self.devices
                .iter()
                .map(|device| utils::hextrunc(device.identifier()))
                .collect::<Vec<_>>()
        );

        self.result = Some(Ok(Group::new(
            identifier,
            self.name.clone(),
            self.threshold,
            self.protocol.get_type(),
            self.key_type,
            certificate,
            self.note.clone(),
        )));

        self.communicator.write().unwrap().clear_input();
    }

    fn next_round(&mut self) {
        if !self.certificates_sent {
            self.send_certificates();
        } else if self.protocol.round() == 0 {
            self.start_task();
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task()
        } else {
            self.finalize_task()
        }
    }

    fn send_certificates(&mut self) {
        self.communicator.set_active_devices();

        let certs: HashMap<u32, Vec<u8>> = self
            .devices
            .iter()
            .flat_map(|dev| {
                let cert = dev.certificate().to_vec();
                self.communicator
                    .identifier_to_indices(dev.identifier())
                    .into_iter()
                    .zip(std::iter::repeat(cert))
            })
            .collect();
        let certs = ServerMessage {
            broadcasts: certs,
            unicasts: HashMap::new(),
            protocol_type: self.protocol.get_type().into(),
        }
        .encode_to_vec();

        self.communicator.send_all(|_| certs.clone());
        self.certificates_sent = true;
    }
}

fn create_protocol(
    protocol_type: proto::ProtocolType,
    key_type: proto::KeyType,
    devices_len: u32,
    threshold: u32,
) -> Result<Box<dyn Protocol + Send + Sync>, String> {
    let protocol: Box<dyn Protocol + Send + Sync> = match (protocol_type, key_type) {
        (ProtocolType::Gg18, KeyType::SignPdf) => Box::new(GG18Group::new(devices_len, threshold)),
        (ProtocolType::Gg18, KeyType::SignChallenge) => {
            Box::new(GG18Group::new(devices_len, threshold))
        }
        (ProtocolType::Frost, KeyType::SignChallenge) => {
            Box::new(FROSTGroup::new(devices_len, threshold))
        }
        (ProtocolType::Elgamal, KeyType::Decrypt) => {
            Box::new(ElgamalGroup::new(devices_len, threshold))
        }
        _ => {
            warn!(
                "Protocol {:?} does not support {:?} key type",
                protocol_type, key_type
            );
            return Err("Unsupported protocol type and key type combination".into());
        }
    };
    Ok(protocol)
}
impl Task for GroupTask {
    fn get_status(&self) -> TaskStatus {
        match &self.result {
            Some(Err(e)) => TaskStatus::Failed(e.clone()),
            Some(Ok(_)) => TaskStatus::Finished,
            None => {
                if self.protocol.round() == 0 && !self.certificates_sent {
                    TaskStatus::Created
                } else {
                    TaskStatus::Running(self.protocol.round() + 1)
                }
            }
        }
    }

    // TODO: unwraps
    fn from_model(
        model: TaskModel,
        devices: Vec<Device>,
        communicator: Arc<RwLock<Communicator>>,
    ) -> Result<Self, Error> {
        let protocol = create_protocol(
            model.protocol_type.unwrap().into(),
            model.key_type.clone().unwrap().into(),
            devices.len() as u32,
            model.threshold as u32,
        )?;
        Ok(Self {
            name: "test group".into(), // TODO: name
            id: model.id,
            threshold: model.threshold as u32,
            key_type: model.key_type.unwrap().into(),
            devices,
            communicator,
            result: None, // TODO: we may need to propagate the created group here
            protocol,
            request: model.request.unwrap(),
            last_update: model.last_update.timestamp_millis() as u64,
            attempts: model.attempt_count as u32,
        })
    }

    fn get_type(&self) -> TaskType {
        TaskType::Group
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Vec<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return Vec::new();
        }

        self.communicator
            .read()
            .unwrap()
            .get_message(device_id.unwrap())
    }

    fn get_result(&self) -> Option<TaskResult> {
        if let Some(Ok(group)) = &self.result {
            Some(TaskResult::GroupEstablished(group.clone()))
        } else {
            None
        }
    }

    fn get_decisions(&self) -> (u32, u32) {
        let communicator = self.communicator.read().unwrap();
        (communicator.accept_count(), communicator.reject_count())
    }

    fn update(&mut self, device_id: &[u8], data: &Vec<Vec<u8>>) -> Result<bool, String> {
        let mut communicator = self.communicator.write().unwrap();
        if communicator.accept_count() != self.devices.len() as u32 {
            return Err("Not enough agreements to proceed with the protocol.".to_string());
        }

        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string());
        }

        assert_eq!(self.certificates_sent, true);

        let messages = data
            .iter()
            .map(|d| ClientMessage::decode(d.as_slice()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Failed to decode messages".to_string())?;

        communicator.receive_messages(device_id, messages);
        self.last_update = get_timestamp();

        if communicator.round_received() && self.protocol.round() <= self.protocol.last_round() {
            drop(communicator);
            self.next_round();
            return Ok(true);
        }

        Ok(false)
    }

    fn restart(&mut self) -> Result<bool, String> {
        self.last_update = get_timestamp();
        if self.result.is_some() {
            return Ok(false);
        }

        if self.is_approved() {
            self.attempts += 1;
            self.start_task();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn last_update(&self) -> u64 {
        self.last_update
    }

    fn is_approved(&self) -> bool {
        self.communicator.read().unwrap().accept_count() == self.devices.len() as u32
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        return self
            .devices
            .iter()
            .map(|device| device.identifier())
            .any(|x| x == device_id);
    }

    fn get_devices(&self) -> &Vec<Device> {
        &self.devices
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        let mut communicator = self.communicator.write().unwrap();
        if !self.certificates_sent && self.protocol.round() == 0 {
            return !communicator.device_decided(device);
        } else if self.protocol.round() >= self.protocol.last_round() {
            return !communicator.device_acknowledged(device);
        }

        communicator.waiting_for(device)
    }

    fn decide(&mut self, device_id: &[u8], decision: bool) -> Option<bool> {
        let mut communicator = self.communicator.write().unwrap();
        communicator.decide(device_id, decision);
        self.last_update = get_timestamp();
        if self.result.is_none() && self.protocol.round() == 0 {
            if communicator.reject_count() > 0 {
                self.result = Some(Err("Task declined".to_string()));
                return Some(false);
            } else if communicator.accept_count() == self.devices.len() as u32 {
                drop(communicator);
                self.next_round();
                return Some(true);
            }
        }
        None
    }

    fn acknowledge(&mut self, device_id: &[u8]) {
        self.communicator.write().unwrap().acknowledge(device_id);
    }

    fn device_acknowledged(&self, device_id: &[u8]) -> bool {
        self.communicator
            .read()
            .unwrap()
            .device_acknowledged(device_id)
    }

    fn get_request(&self) -> &[u8] {
        &self.request
    }

    fn get_attempts(&self) -> u32 {
        self.attempts
    }

    fn get_id(&self) -> &Uuid {
        &self.id
    }

    fn get_communicator(&self) -> Arc<RwLock<Communicator>> {
        self.communicator.clone()
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
