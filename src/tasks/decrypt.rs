use std::sync::Arc;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::{Participant, PersistenceError, Task as TaskModel};
use crate::protocols::elgamal::ElgamalDecrypt;
use crate::protocols::{create_threshold_protocol, Protocol};
use crate::tasks::{FailedTask, FinishedTask, RoundUpdate, RunningTask, TaskInfo, TaskResult};
use crate::utils;
use async_trait::async_trait;
use log::info;
use meesign_crypto::proto::{ClientMessage, Message as _};
use std::collections::HashMap;
use tokio::sync::RwLock;

pub struct DecryptTask {
    task_info: TaskInfo,
    group: Group,
    communicator: Arc<RwLock<Communicator>>,
    pub(super) data: Vec<u8>,
    pub(super) protocol: Box<dyn Protocol + Send + Sync>,
    pub(super) attempts: u32,
}

impl DecryptTask {
    pub fn try_new(
        task_info: TaskInfo,
        group: Group,
        data: Vec<u8>,
        decisions: HashMap<Vec<u8>, i8>,
    ) -> Result<Self, String> {
        let mut participants: Vec<Participant> = group.participants().to_vec();
        participants.sort_by(|a, b| a.device.identifier().cmp(b.device.identifier()));

        let communicator = Arc::new(RwLock::new(Communicator::new(
            participants,
            group.threshold(),
            group.protocol(),
            decisions,
        )));

        Ok(DecryptTask {
            task_info,
            group,
            communicator,
            data,
            protocol: Box::new(ElgamalDecrypt::new()),
            attempts: 0,
        })
    }

    pub fn from_model(
        task_info: TaskInfo,
        task_model: TaskModel,
        communicator: Arc<RwLock<Communicator>>,
        group: Group,
    ) -> Result<Self, Error> {
        let protocol = create_threshold_protocol(
            group.protocol(),
            group.key_type(),
            task_model.protocol_round as u16,
        )?;
        let data = task_model
            .task_data
            .ok_or(PersistenceError::DataInconsistencyError(
                "Task data not set for a sign task".into(),
            ))?;
        let task = Self {
            task_info,
            group,
            communicator,
            data,
            protocol,
            attempts: task_model.attempt_count as u32,
        };
        Ok(task)
    }

    pub(super) async fn start_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol
            .initialize(&mut *self.communicator.write().await, &self.data);
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    pub(super) async fn advance_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol.advance(&mut *self.communicator.write().await);
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    pub(super) async fn finalize_task(&mut self) -> Result<RoundUpdate, Error> {
        let decrypted = self
            .protocol
            .finalize(&mut *self.communicator.write().await);
        if decrypted.is_none() {
            let reason = "Task failed (data not output)".to_string();
            return Ok(RoundUpdate::Failed(FailedTask { reason }));
        }
        let decrypted = decrypted.unwrap();

        info!(
            "Data decrypted by group_id={}",
            utils::hextrunc(self.group.identifier())
        );

        self.communicator.write().await.clear_input();
        Ok(RoundUpdate::Finished(
            self.protocol.round(),
            FinishedTask::new(TaskResult::Decrypted(decrypted)),
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

    fn increment_attempt_count(&mut self) {
        self.attempts += 1;
    }
}

#[async_trait]
impl RunningTask for DecryptTask {
    async fn get_work(&self, device_id: &[u8]) -> Vec<Vec<u8>> {
        if !self.waiting_for(device_id).await {
            return Vec::new();
        }

        self.communicator.read().await.get_messages(device_id)
    }

    fn get_round(&self) -> u16 {
        self.protocol.round()
    }

    async fn initialize(&mut self) -> Result<RoundUpdate, Error> {
        self.start_task().await
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

    async fn restart(&mut self) -> Result<RoundUpdate, Error> {
        self.increment_attempt_count();
        self.start_task().await
    }

    async fn waiting_for(&self, device: &[u8]) -> bool {
        self.communicator.read().await.waiting_for(device)
    }

    fn get_attempts(&self) -> u32 {
        self.attempts
    }

    fn get_communicator(&self) -> Arc<RwLock<Communicator>> {
        self.communicator.clone()
    }
}
