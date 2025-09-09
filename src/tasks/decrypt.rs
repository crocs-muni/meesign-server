use std::sync::Arc;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::{Participant, PersistenceError, Repository, Task as TaskModel};
use crate::proto::{DecryptRequest, TaskType};
use crate::protocols::elgamal::ElgamalDecrypt;
use crate::protocols::{create_threshold_protocol, Protocol};
use crate::tasks::{DecisionUpdate, RoundUpdate, Task, TaskResult, TaskStatus};
use crate::utils;
use async_trait::async_trait;
use log::info;
use meesign_crypto::proto::{ClientMessage, Message as _};
use prost::Message as _;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct DecryptTask {
    id: Uuid,
    group: Group,
    communicator: Arc<RwLock<Communicator>>,
    result: Option<Result<Vec<u8>, String>>,
    pub(super) data: Vec<u8>,
    pub(super) protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
    pub(super) attempts: u32,
}

impl DecryptTask {
    pub fn try_new(
        group: Group,
        name: String,
        data: Vec<u8>,
        data_type: String,
        repository: Arc<Repository>,
    ) -> Result<Self, String> {
        let mut participants: Vec<Participant> = group.participants().to_vec();
        participants.sort_by(|a, b| a.device.identifier().cmp(b.device.identifier()));

        let decisions = participants
            .iter()
            .map(|p| (p.device.identifier().clone(), 0))
            .collect();
        let acknowledgements = participants
            .iter()
            .map(|p| (p.device.identifier().clone(), false))
            .collect();

        let communicator = Arc::new(RwLock::new(Communicator::new(
            participants,
            group.threshold(),
            group.protocol(),
            decisions,
            acknowledgements,
        )));

        let request = (DecryptRequest {
            group_id: group.identifier().to_vec(),
            name,
            data: data.clone(),
            data_type,
        })
        .encode_to_vec();

        let id = Uuid::new_v4();
        Ok(DecryptTask {
            id,
            group,
            communicator,
            result: None,
            data,
            protocol: Box::new(ElgamalDecrypt::new(repository, id)),
            request,
            attempts: 0,
        })
    }

    pub(super) async fn start_task(&mut self) -> Result<RoundUpdate, Error> {
        assert!(self.communicator.read().await.accept_count() >= self.group.threshold());
        self.protocol
            .initialize(&mut *self.communicator.write().await, &self.data)
            .await?;
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    pub(super) async fn advance_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol
            .advance(&mut *self.communicator.write().await)
            .await?;
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    pub(super) async fn finalize_task(&mut self) -> Result<RoundUpdate, Error> {
        let decrypted = self
            .protocol
            .finalize(&mut *self.communicator.write().await)
            .await?;
        if decrypted.is_none() {
            let reason = "Task failed (data not output)".to_string();
            self.set_result(Err(reason.clone()));
            return Ok(RoundUpdate::Failed(reason));
        }
        let decrypted = decrypted.unwrap();

        info!(
            "Data decrypted by group_id={}",
            utils::hextrunc(self.group.identifier())
        );

        self.set_result(Ok(decrypted.clone()));

        self.communicator.write().await.clear_input();
        Ok(RoundUpdate::Finished(
            self.protocol.round(),
            TaskResult::Decrypted(decrypted),
        ))
    }

    pub(super) async fn next_round(&mut self) -> Result<RoundUpdate, Error> {
        if self.protocol.round() == 0 {
            self.start_task().await
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task().await
        } else {
            self.finalize_task().await
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
            .map_err(|_| Error::GeneralProtocolError("Expected ClientMessage".into()))?;

        self.communicator
            .write()
            .await
            .receive_messages(device_id, messages);

        if self.communicator.read().await.round_received()
            && self.protocol.round() <= self.protocol.last_round()
        {
            return Ok(true);
        }
        Ok(false)
    }

    // TODO: deduplicate across sign and decrypt
    pub(super) async fn decide_internal(
        &mut self,
        device_id: &[u8],
        decision: bool,
    ) -> Option<bool> {
        self.communicator.write().await.decide(device_id, decision);

        if self.result.is_none() && self.protocol.round() == 0 {
            if self.communicator.read().await.reject_count() >= self.group.reject_threshold() {
                self.set_result(Err("Task declined".to_string()));
                return Some(false);
            } else if self.communicator.read().await.accept_count() >= self.group.threshold() {
                return Some(true);
            }
        }
        None
    }

    fn set_result(&mut self, result: Result<Vec<u8>, String>) {
        self.result = Some(result);
    }

    async fn increment_attempt_count(&mut self, repository: Arc<Repository>) -> Result<u32, Error> {
        self.attempts += 1;
        Ok(repository.increment_task_attempt_count(&self.id).await?)
    }
}

#[async_trait]
impl Task for DecryptTask {
    async fn from_model(
        task_model: TaskModel,
        participants: Vec<Participant>,
        communicator: Arc<RwLock<Communicator>>,
        repository: Arc<Repository>,
    ) -> Result<Self, Error> {
        let result = match task_model.result {
            Some(val) => val.try_into_option()?,
            None => None,
        };
        let group = repository
            .get_group(&task_model.group_id.unwrap())
            .await?
            .unwrap();
        let group = Group::try_from_model(group, participants)?;
        let request = task_model
            .request
            .ok_or(PersistenceError::DataInconsistencyError(
                "Request not set for a sign task".into(),
            ))?;
        let protocol = create_threshold_protocol(
            group.protocol(),
            group.key_type(),
            repository.clone(),
            task_model.id,
            task_model.protocol_round as u16,
        )?;
        let data = task_model
            .task_data
            .ok_or(PersistenceError::DataInconsistencyError(
                "Task data not set for a sign task".into(),
            ))?;
        let task = Self {
            id: task_model.id,
            group,
            communicator,
            result,
            data,
            protocol,
            request,
            attempts: task_model.attempt_count as u32,
        };
        Ok(task)
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

        self.communicator
            .read()
            .await
            .get_messages(device_id.unwrap())
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
            self.communicator.read().await.accept_count(),
            self.communicator.read().await.reject_count(),
        )
    }

    async fn update(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
    ) -> Result<RoundUpdate, Error> {
        let round_update = if self.update_internal(device_id, data).await? {
            self.next_round().await?
        } else {
            RoundUpdate::Listen
        };
        Ok(round_update)
    }

    async fn restart(&mut self, repository: Arc<Repository>) -> Result<bool, Error> {
        if self.result.is_some() {
            return Ok(false);
        }

        if self.is_approved().await {
            self.increment_attempt_count(repository.clone()).await?;
            self.start_task().await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn is_approved(&self) -> bool {
        self.communicator.read().await.accept_count() >= self.group.threshold()
    }

    fn get_participants(&self) -> &Vec<Participant> {
        &self.group.participants()
    }

    async fn waiting_for(&self, device: &[u8]) -> bool {
        if self.protocol.round() == 0 {
            return !self.communicator.read().await.device_decided(device);
        } else if self.protocol.round() >= self.protocol.last_round() {
            return !self.communicator.read().await.device_acknowledged(device);
        }

        self.communicator.read().await.waiting_for(device)
    }

    async fn decide(&mut self, device_id: &[u8], decision: bool) -> Result<DecisionUpdate, Error> {
        let result = self.decide_internal(device_id, decision).await;
        let decision_update = match result {
            Some(true) => {
                let round_update = self.next_round().await?;
                DecisionUpdate::Accepted(round_update)
            }
            Some(false) => DecisionUpdate::Declined,
            None => DecisionUpdate::Undecided,
        };
        Ok(decision_update)
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
        self.group.threshold()
    }

    fn get_data(&self) -> Option<&[u8]> {
        Some(&self.data)
    }
}
