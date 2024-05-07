use std::sync::Arc;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::{Device, Repository};
use crate::proto::{ProtocolType, SignRequest, TaskType};
use crate::protocols::frost::FROSTSign;
use crate::protocols::gg18::GG18Sign;
use crate::protocols::Protocol;
use crate::tasks::{Task, TaskResult, TaskStatus};
use crate::{get_timestamp, utils};
use async_trait::async_trait;
use log::{info, warn};
use meesign_crypto::proto::{ClientMessage, Message as _};
use prost::Message as _;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct SignTask {
    id: Uuid,
    group: Group,
    communicator: Arc<RwLock<Communicator>>,
    result: Option<Result<Vec<u8>, String>>,
    pub(super) data: Vec<u8>,
    preprocessed: Option<Vec<u8>>,
    pub(super) protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
    pub(super) last_update: u64,
    pub(super) attempts: u32,
}

impl SignTask {
    pub fn try_new(group: Group, name: String, data: Vec<u8>) -> Result<Self, String> {
        // let mut devices: Vec<Arc<Device>> = group.devices().to_vec();
        let mut devices: Vec<Device> = todo!();
        devices.sort_by_key(|x| x.identifier().to_vec());
        let protocol_type = group.protocol();

        let communicator = Arc::new(RwLock::new(Communicator::new(
            devices,
            group.threshold(),
            protocol_type,
        )));

        let request = (SignRequest {
            group_id: group.identifier().to_vec(),
            name,
            data: data.clone(),
        })
        .encode_to_vec();

        Ok(SignTask {
            id: Uuid::new_v4(),
            group,
            communicator,
            result: None,
            data,
            preprocessed: None,
            protocol: match protocol_type {
                ProtocolType::Gg18 => Box::new(GG18Sign::new(todo!(), todo!())),
                ProtocolType::Frost => Box::new(FROSTSign::new()),
                _ => {
                    warn!("Protocol type {:?} does not support signing", protocol_type);
                    return Err("Unsupported protocol type for signing".into());
                }
            },
            request,
            last_update: get_timestamp(),
            attempts: 0,
        })
    }

    pub fn get_group(&self) -> &Group {
        &self.group
    }

    /// Use this method to change data to be used for signing
    pub(super) fn set_preprocessed(&mut self, preprocessed: Vec<u8>) {
        self.preprocessed = Some(preprocessed);
    }

    pub(super) async fn start_task(&mut self, repository: Arc<Repository>) {
        assert!(self.communicator.read().await.accept_count() >= self.group.threshold());
        todo!()
        // self.protocol.initialize(
        //     // &mut self.communicator,
        //     todo!(),
        //     self.preprocessed.as_ref().unwrap_or(&self.data),
        //     repository,
        // );
    }

    pub(super) fn advance_task(&mut self) {
        todo!()
        // self.protocol.advance(todo!())
    }

    pub(super) async fn finalize_task(&mut self) {
        let signature = self.protocol.finalize(todo!()).await.unwrap();
        if signature.is_none() {
            self.result = Some(Err("Task failed (signature not output)".to_string()));
            return;
        }
        let signature = signature.unwrap();

        info!(
            "Signature created by group_id={}",
            utils::hextrunc(self.group.identifier())
        );

        self.result = Some(Ok(signature));
        self.communicator.write().await.clear_input();
    }

    pub(super) async fn next_round(&mut self, repository: Arc<Repository>) {
        if self.protocol.round() == 0 {
            self.start_task(repository).await;
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task();
        } else {
            self.finalize_task().await;
        }
    }

    pub(super) async fn update_internal(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
    ) -> Result<bool, Error> {
        if self.communicator.read().await.accept_count() < self.group.threshold() {
            return Err(Error::GeneralProtocolError(
                "Not enough agreements to proceed with the protocol.".into(),
            ));
        }

        if !self.waiting_for(device_id).await {
            return Err(Error::GeneralProtocolError(
                "Wasn't waiting for a message from this ID.".into(),
            ));
        }

        let messages = data
            .iter()
            .map(|d| ClientMessage::decode(d.as_slice()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::GeneralProtocolError("Expected ClientMessage.".into()))?;

        self.communicator
            .write()
            .await
            .receive_messages(device_id, messages);
        self.last_update = get_timestamp();

        if self.communicator.read().await.round_received()
            && self.protocol.round() <= self.protocol.last_round()
        {
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) async fn decide_internal(
        &mut self,
        device_id: &[u8],
        decision: bool,
    ) -> Option<bool> {
        self.communicator.write().await.decide(device_id, decision);
        self.last_update = get_timestamp();
        if self.result.is_none() && self.protocol.round() == 0 {
            if self.communicator.read().await.reject_count() >= self.group.reject_threshold() {
                self.result = Some(Err("Task declined".to_string()));
                return Some(false);
            } else if self.communicator.read().await.accept_count() >= self.group.threshold() {
                return Some(true);
            }
        }
        None
    }
}

#[async_trait]
impl Task for SignTask {
    fn get_status(&self) -> TaskStatus {
        match &self.result {
            Some(Err(e)) => TaskStatus::Failed(e.clone()),
            Some(Ok(_)) => TaskStatus::Finished,
            None => {
                if self.protocol.round() == 0 {
                    TaskStatus::Created
                } else {
                    TaskStatus::Running(self.protocol.round())
                }
            }
        }
    }

    fn get_type(&self) -> TaskType {
        TaskType::SignChallenge
    }

    async fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<Vec<u8>>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()).await {
            return None;
        }

        self.communicator
            .read()
            .await
            .get_message(device_id.unwrap())
    }

    fn get_result(&self) -> Option<TaskResult> {
        if let Some(Ok(signature)) = &self.result {
            Some(TaskResult::Signed(signature.clone()))
        } else {
            None
        }
    }

    async fn get_decisions(&self) -> (u32, u32) {
        (
            self.communicator.read().await.accept_count(),
            self.communicator.read().await.reject_count(),
        )
    }

    async fn update(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
        repository: Arc<Repository>,
    ) -> Result<bool, Error> {
        let result = self.update_internal(device_id, data).await;
        if let Ok(true) = result {
            self.next_round(repository).await;
        };
        result
    }

    async fn restart(&mut self, repository: Arc<Repository>) -> Result<bool, String> {
        self.last_update = get_timestamp();
        if self.result.is_some() {
            return Ok(false);
        }

        if self.is_approved().await {
            self.attempts += 1;
            self.start_task(repository).await;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn last_update(&self) -> u64 {
        self.last_update
    }

    async fn is_approved(&self) -> bool {
        self.communicator.read().await.accept_count() >= self.group.threshold()
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        // self.group.contains(device_id)
        todo!()
    }

    fn get_devices(&self) -> &Vec<Device> {
        // &self.group.devices()
        todo!()
    }

    async fn waiting_for(&self, device: &[u8]) -> bool {
        if self.protocol.round() == 0 {
            return !self.communicator.read().await.device_decided(device);
        } else if self.protocol.round() >= self.protocol.last_round() {
            return !self.communicator.read().await.device_acknowledged(device);
        }

        self.communicator.read().await.waiting_for(device)
    }

    async fn decide(
        &mut self,
        device_id: &[u8],
        decision: bool,
        repository: Arc<Repository>,
    ) -> Option<bool> {
        let result = self.decide_internal(device_id, decision).await;
        if let Some(true) = result {
            self.next_round(repository).await;
        };
        result
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

    async fn from_model(
        model: crate::persistence::Task,
        devices: Vec<Device>,
        communicator: Arc<RwLock<Communicator>>,
        repository: Arc<Repository>,
    ) -> Result<Self, crate::error::Error>
    where
        Self: Sized,
    {
        todo!()
    }

    fn get_id(&self) -> &Uuid {
        &self.id
    }

    fn get_communicator(&self) -> Arc<RwLock<Communicator>> {
        todo!()
    }

    fn get_threshold(&self) -> u32 {
        self.get_group().threshold()
    }
}
