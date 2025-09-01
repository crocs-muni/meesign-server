use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::Device;
use crate::persistence::PersistenceError;
use crate::persistence::Repository;
use crate::persistence::Task as TaskModel;
use crate::proto::{KeyType, ProtocolType, TaskType};
use crate::protocols::{create_keygen_protocol, Protocol};
use crate::tasks::{Task, TaskResult, TaskStatus};
use crate::{get_timestamp, utils};
use async_trait::async_trait;
use log::{info, warn};
use meesign_crypto::proto::{ClientMessage, Message as _, ServerMessage};
use prost::Message as _;
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct GroupTask {
    name: String,
    id: Uuid,
    threshold: u32,
    key_type: KeyType,
    devices: Vec<Device>,
    communicator: Arc<RwLock<Communicator>>,
    result: Option<Result<Group, String>>,
    protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
    last_update: u64,
    attempts: u32,
    note: Option<String>,
    certificates_sent: bool, // TODO: remove the field completely
}

impl GroupTask {
    pub fn try_new(
        name: &str,
        mut devices: Vec<Device>,
        threshold: u32,
        protocol_type: ProtocolType,
        key_type: KeyType,
        note: Option<String>,
        repository: Arc<Repository>,
    ) -> Result<Self, String> {
        let id = Uuid::new_v4();

        let devices_len = devices.len() as u32;
        let protocol = create_keygen_protocol(
            protocol_type,
            key_type,
            devices_len,
            threshold,
            repository,
            id.clone(),
            0,
        )?;

        if devices_len < 1 {
            warn!("Invalid number of devices {}", devices_len);
            return Err("Invalid input".into());
        }
        if !protocol.get_type().check_threshold(threshold, devices_len) {
            warn!("Invalid group threshold {}-of-{}", threshold, devices_len);
            return Err("Invalid input".into());
        }

        devices.sort_by_key(|x| x.identifier().to_vec());

        let group_task_threshold = devices.len() as u32;
        let device_ids = devices.iter().map(|x| x.identifier().to_vec()).collect();

        let decisions = devices
            .iter()
            .map(|x| (x.identifier().clone(), 0))
            .collect();
        let acknowledgements = devices
            .iter()
            .map(|x| (x.identifier().clone(), false))
            .collect();
        let communicator = Arc::new(RwLock::new(Communicator::new(
            devices.clone(),
            group_task_threshold,
            protocol.get_type(),
            decisions,
            acknowledgements,
        )));

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
            id,
            threshold,
            devices: devices.to_vec(),
            key_type,
            communicator,
            result: None,
            protocol,
            request,
            last_update: get_timestamp(),
            attempts: 0,
            note,
            certificates_sent: false,
        })
    }

    async fn set_result(
        &mut self,
        result: Result<Group, String>,
        repository: Arc<Repository>,
    ) -> Result<(), Error> {
        self.result = Some(result.clone());
        let result = result.map(|group| group.identifier().to_vec());
        repository.set_task_result(&self.id, &result).await?;
        Ok(())
    }

    async fn set_last_update(&mut self, repository: Arc<Repository>) -> Result<u64, Error> {
        self.last_update = get_timestamp();
        repository.set_task_last_update(&self.id).await?;
        Ok(self.last_update)
    }

    async fn increment_attempt_count(&mut self, repository: Arc<Repository>) -> Result<u32, Error> {
        self.attempts += 1;
        Ok(repository.increment_task_attempt_count(&self.id).await?)
    }

    async fn start_task(&mut self) -> Result<(), Error> {
        self.protocol
            .initialize(&mut *self.communicator.write().await, &[])
            .await?;
        Ok(())
    }

    async fn advance_task(&mut self) {
        self.protocol
            .advance(&mut *self.communicator.write().await)
            .await
            .unwrap();
    }

    async fn finalize_task(&mut self, repository: Arc<Repository>) -> Result<(), Error> {
        let identifier = self
            .protocol
            .finalize(&mut *self.communicator.write().await)
            .await?;
        let Some(identifier) = identifier else {
            let error_message = "Task failed (group key not output)".to_string();
            self.set_result(Err(error_message.clone()), repository)
                .await?;
            return Err(Error::GeneralProtocolError(error_message));
        };
        // TODO
        let certificate = if self.protocol.get_type() == ProtocolType::Gg18 {
            Some(issue_certificate(&self.name, &identifier)?)
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

        self.set_result(
            Ok(Group::new(
                identifier.clone(),
                self.name.clone(),
                self.threshold,
                self.devices.clone(),
                self.protocol.get_type(),
                self.key_type,
                certificate,
                self.note.clone(),
            )),
            repository.clone(),
        )
        .await?;
        repository
            .set_task_result(&self.id, &Ok(identifier))
            .await?;

        self.communicator.write().await.clear_input();
        Ok(())
    }

    async fn next_round(&mut self, repository: Arc<Repository>) {
        if !self.certificates_sent {
            self.send_certificates(repository).await.unwrap();
        } else if self.protocol.round() == 0 {
            self.start_task().await.unwrap();
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task().await
        } else {
            self.finalize_task(repository).await.unwrap() // TODO
        }
    }

    async fn send_certificates(&mut self, repository: Arc<Repository>) -> Result<(), Error> {
        self.communicator.write().await.set_active_devices(None);

        let certs: HashMap<u32, Vec<u8>> = {
            let communicator_read = self.communicator.read().await;
            self.devices
                .iter()
                .flat_map(|dev| {
                    let cert = &dev.certificate;
                    communicator_read
                        .identifier_to_indices(dev.identifier())
                        .into_iter()
                        .zip(std::iter::repeat(cert).cloned())
                })
                .collect()
        };
        let certs = ServerMessage {
            broadcasts: certs,
            unicasts: HashMap::new(),
            protocol_type: self.protocol.get_type().into(),
        }
        .encode_to_vec();

        self.communicator.write().await.send_all(|_| certs.clone());
        self.certificates_sent = true;
        repository
            .set_task_group_certificates_sent(&self.id, Some(true))
            .await?;
        Ok(())
    }
}

#[async_trait]
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
    async fn from_model(
        model: TaskModel,
        devices: Vec<Device>,
        communicator: Arc<RwLock<Communicator>>,
        repository: Arc<Repository>,
    ) -> Result<Self, Error> {
        if model.key_type.is_none() || model.request.is_none() {
            return Err(PersistenceError::DataInconsistencyError(
                "TaskModel is not data-consistent. KeyType of request is not set".into(),
            )
            .into());
        }
        let key_type = model.key_type.unwrap().into();
        if model.protocol_type.is_none() {
            return Err(PersistenceError::DataInconsistencyError(
                "TaskModel is not data-consistent. ProtocolType of request is not set".into(),
            )
            .into());
        }
        let protocol_type = model.protocol_type.unwrap().into();

        let protocol = create_keygen_protocol(
            protocol_type,
            key_type,
            devices.len() as u32,
            model.threshold as u32,
            repository.clone(),
            model.id,
            model.protocol_round as u16,
        )?;

        // TODO: refactor
        let result = if let Some(result) = model.result {
            let result = result.try_into_option()?;
            result
        } else {
            None
        };
        let result: Option<Result<Group, String>> = match result {
            Some(Ok(group_id)) => {
                let Some(resulting_group) = repository.get_group(&group_id).await? else {
                    return Err(Error::PersistenceError(
                        PersistenceError::DataInconsistencyError(
                            "Established group is missing".into(),
                        ),
                    ));
                };
                Some(Ok(crate::group::Group::try_from_model(
                    resulting_group,
                    devices.clone(),
                )?))
            }
            Some(Err(err)) => Some(Err(err)),
            None => None,
        };
        let name = if let Some(Ok(group)) = &result {
            group.name().into()
        } else {
            "".into() // TODO add field to the task table
        };
        let request = model.request.unwrap().into();
        Ok(Self {
            name,
            id: model.id,
            threshold: model.threshold as u32,
            key_type,
            devices,
            communicator,
            result,
            protocol,
            request,
            last_update: model.last_update.timestamp() as u64,
            attempts: model.attempt_count as u32,
            note: model.note,
            certificates_sent: model.group_certificates_sent.unwrap(), // TODO: remove the field completely
        })
    }

    fn get_type(&self) -> TaskType {
        TaskType::Group
    }

    async fn get_work(&self, device_id: Option<&[u8]>) -> Vec<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()).await {
            return Vec::new();
        }

        self.communicator
            .read()
            .await
            .get_messages(device_id.unwrap())
    }

    fn get_result(&self) -> Option<TaskResult> {
        if let Some(Ok(group)) = &self.result {
            Some(TaskResult::GroupEstablished(group.clone()))
        } else {
            None
        }
    }

    async fn get_decisions(&self) -> (u32, u32) {
        let communicator = self.communicator.read().await;
        (communicator.accept_count(), communicator.reject_count())
    }

    async fn update(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
        repository: Arc<Repository>,
    ) -> Result<bool, Error> {
        if self.communicator.read().await.accept_count() != self.devices.len() as u32 {
            return Err(Error::GeneralProtocolError(
                "Not enough agreements to proceed with the protocol.".into(),
            ));
        }

        if !self.waiting_for(device_id).await {
            return Err(Error::GeneralProtocolError(
                "Wasn't waiting for a message from this ID.".into(),
            ));
        }

        assert_eq!(self.certificates_sent, true);

        let messages = data
            .iter()
            .map(|d| ClientMessage::decode(d.as_slice()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::GeneralProtocolError("Expected ClientMessage.".into()))?;

        self.communicator
            .write()
            .await
            .receive_messages(device_id, messages);
        self.set_last_update(repository.clone()).await?;

        if self.communicator.read().await.round_received()
            && self.protocol.round() <= self.protocol.last_round()
        {
            self.next_round(repository.clone()).await;
            return Ok(true);
        }

        Ok(false)
    }

    async fn restart(&mut self, repository: Arc<Repository>) -> Result<bool, Error> {
        self.set_last_update(repository.clone()).await.unwrap();
        if self.result.is_some() {
            return Ok(false);
        }

        if self.is_approved().await {
            self.increment_attempt_count(repository.clone()).await?;
            self.start_task().await.unwrap();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn last_update(&self) -> u64 {
        self.last_update
    }

    async fn is_approved(&self) -> bool {
        self.communicator.read().await.accept_count() == self.devices.len() as u32
    }

    fn get_devices(&self) -> &Vec<Device> {
        &self.devices
    }

    async fn waiting_for(&self, device: &[u8]) -> bool {
        let communicator = self.communicator.write().await;
        if !self.certificates_sent && self.protocol.round() == 0 {
            return !communicator.device_decided(device);
        } else if self.protocol.round() >= self.protocol.last_round() {
            return !communicator.device_acknowledged(device);
        }

        communicator.waiting_for(device)
    }

    async fn decide(
        &mut self,
        device_id: &[u8],
        decision: bool,
        repository: Arc<Repository>,
    ) -> Option<bool> {
        self.communicator.write().await.decide(device_id, decision);
        self.set_last_update(repository.clone()).await.unwrap();
        if self.result.is_none() && self.protocol.round() == 0 {
            if self.communicator.read().await.reject_count() > 0 {
                self.set_result(Err("Task declined".to_string()), repository)
                    .await
                    .unwrap();
                return Some(false);
            } else if self.communicator.read().await.accept_count() == self.devices.len() as u32 {
                self.next_round(repository).await;
                return Some(true);
            }
        }
        None
    }

    async fn acknowledge(&mut self, device_id: &[u8]) {
        self.communicator.write().await.acknowledge(device_id);
    }

    async fn device_acknowledged(&self, device_id: &[u8]) -> bool {
        self.communicator
            .read()
            .await
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

    fn get_threshold(&self) -> u32 {
        self.threshold
    }

    fn get_data(&self) -> Option<&[u8]> {
        None
    }
}

fn issue_certificate(name: &str, public_key: &[u8]) -> Result<Vec<u8>, Error> {
    assert_eq!(public_key.len(), 65);
    let mut process = Command::new("java")
        .arg("-jar")
        .arg("MeeSignHelper.jar")
        .arg("cert")
        .arg(name)
        .arg(hex::encode(public_key))
        .stdout(Stdio::piped())
        .spawn()?;

    let mut result = Vec::new();
    process.stdout.as_mut().unwrap().read_to_end(&mut result)?;
    Ok(result)
}
