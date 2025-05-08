use std::sync::Arc;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::Device;
use crate::persistence::Repository;
use crate::persistence::Task as TaskModel;
use crate::proto::{DecryptRequest, ProtocolType, TaskType};
use crate::protocols::elgamal::ElgamalDecrypt;
use crate::protocols::Protocol;
use crate::tasks::{Task, TaskResult, TaskStatus};
use crate::{get_timestamp, utils};
use async_trait::async_trait;
use log::info;
use meesign_crypto::proto::{ClientMessage, Message as _};
use prost::Message as _;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct DecryptTask {
    id: Uuid,
    group: Group,
    communicator: Communicator,
    result: Option<Result<Vec<u8>, String>>,
    pub(super) data: Vec<u8>,
    pub(super) protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
    pub(super) last_update: u64,
    pub(super) attempts: u32,
}

impl DecryptTask {
    pub fn new(group: Group, name: String, data: Vec<u8>, data_type: String) -> Self {
        // let mut devices: Vec<Arc<Device>> = group.devices().to_vec();
        let mut devices: Vec<Device> = todo!();
        devices.sort_by_key(|x| x.identifier().to_vec());

        let communicator = Communicator::new(devices, group.threshold(), ProtocolType::Elgamal);

        let request = (DecryptRequest {
            group_id: group.identifier().to_vec(),
            name,
            data: data.clone(),
            data_type,
        })
        .encode_to_vec();

        DecryptTask {
            id: Uuid::new_v4(),
            group,
            communicator,
            result: None,
            data,
            protocol: Box::new(ElgamalDecrypt::new()),
            request,
            last_update: get_timestamp(),
            attempts: 0,
        }
    }

    pub(super) async fn start_task(&mut self, repository: Arc<Repository>) {
        assert!(self.communicator.accept_count() >= self.group.threshold());
        self.protocol.initialize(todo!(), &self.data).await;
    }

    pub(super) async fn advance_task(&mut self) {
        self.protocol.advance(todo!()).await;
    }

    pub(super) async fn finalize_task(&mut self) {
        let decrypted = self.protocol.finalize(todo!()).await.unwrap();
        if decrypted.is_none() {
            self.result = Some(Err("Task failed (data not output)".to_string()));
            return;
        }
        let decrypted = decrypted.unwrap();

        info!(
            "Data decrypted by group_id={}",
            utils::hextrunc(self.group.identifier())
        );

        self.result = Some(Ok(decrypted));
        self.communicator.clear_input();
    }

    pub(super) async fn next_round(&mut self, repository: Arc<Repository>) {
        if self.protocol.round() == 0 {
            self.start_task(repository);
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task().await;
        } else {
            self.finalize_task().await;
        }
    }

    pub(super) async fn update_internal(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
    ) -> Result<bool, Error> {
        if self.communicator.accept_count() < self.group.threshold() {
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
            .map_err(|_| Error::GeneralProtocolError("Expected ClientMessage".into()))?;

        self.communicator.receive_messages(device_id, messages);
        self.last_update = get_timestamp();

        if self.communicator.round_received() && self.protocol.round() <= self.protocol.last_round()
        {
            return Ok(true);
        }
        Ok(false)
    }

    pub(super) fn decide_internal(&mut self, device_id: &[u8], decision: bool) -> Option<bool> {
        self.communicator.decide(device_id, decision);
        self.last_update = get_timestamp();
        if self.result.is_none() && self.protocol.round() == 0 {
            if self.communicator.reject_count() >= self.group.reject_threshold() {
                self.result = Some(Err("Task declined".to_string()));
                return Some(false);
            } else if self.communicator.accept_count() >= self.group.threshold() {
                return Some(true);
            }
        }
        None
    }
}

#[async_trait]
impl Task for DecryptTask {
    async fn from_model(
        model: TaskModel,
        devices: Vec<Device>,
        communicator: Arc<RwLock<Communicator>>,
        repository: Arc<Repository>,
    ) -> Result<Self, Error> {
        todo!()
    }

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
        TaskType::Decrypt
    }

    async fn get_work(&self, device_id: Option<&[u8]>) -> Vec<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()).await {
            return Vec::new();
        }

        self.communicator.get_messages(device_id.unwrap())
    }

    fn get_result(&self) -> Option<TaskResult> {
        if let Some(Ok(decrypted)) = &self.result {
            Some(TaskResult::Decrypted(decrypted.clone()))
        } else {
            None
        }
    }

    async fn get_decisions(&self) -> (u32, u32) {
        (
            self.communicator.accept_count(),
            self.communicator.reject_count(),
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
            self.next_round(repository);
        };
        result
    }

    async fn restart(&mut self, repository: Arc<Repository>) -> Result<bool, Error> {
        self.last_update = get_timestamp();
        if self.result.is_some() {
            return Ok(false);
        }

        if self.is_approved().await {
            self.attempts += 1;
            self.start_task(repository);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn last_update(&self) -> u64 {
        self.last_update
    }

    async fn is_approved(&self) -> bool {
        self.communicator.accept_count() >= self.group.threshold()
    }

    fn get_devices(&self) -> &Vec<Device> {
        // self.group.devices().to_vec()
        todo!();
    }

    async fn waiting_for(&self, device: &[u8]) -> bool {
        if self.protocol.round() == 0 {
            return !self.communicator.device_decided(device);
        } else if self.protocol.round() >= self.protocol.last_round() {
            return !self.communicator.device_acknowledged(device);
        }

        self.communicator.waiting_for(device)
    }

    async fn decide(
        &mut self,
        device_id: &[u8],
        decision: bool,
        repository: Arc<Repository>,
    ) -> Option<bool> {
        let result = self.decide_internal(device_id, decision);
        if let Some(true) = result {
            self.next_round(repository);
        };
        result
    }

    async fn acknowledge(&mut self, device_id: &[u8]) {
        self.communicator.acknowledge(device_id);
    }

    async fn device_acknowledged(&self, device_id: &[u8]) -> bool {
        self.communicator.device_acknowledged(device_id)
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
        todo!()
    }

    fn get_threshold(&self) -> u32 {
        self.group.threshold()
    }

    fn get_data(&self) -> Option<&[u8]> {
        Some(&self.data)
    }
}
