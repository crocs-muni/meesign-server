use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::{Participant, PersistenceError, Task as TaskModel};
use crate::protocols::elgamal::ElgamalDecrypt;
use crate::protocols::{create_threshold_protocol, Protocol};
use crate::tasks::{FailedTask, FinishedTask, RoundUpdate, RunningTask, TaskInfo, TaskResult};
use crate::utils;
use log::info;
use meesign_crypto::proto::{ClientMessage, Message as _};
use std::collections::HashMap;

pub struct DecryptTask {
    task_info: TaskInfo,
    group: Group,
    communicator: Communicator,
    pub(super) data: Vec<u8>,
    pub(super) protocol: Box<dyn Protocol + Send + Sync>,
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

        let communicator =
            Communicator::new(participants, group.threshold(), group.protocol(), decisions);

        Ok(DecryptTask {
            task_info,
            group,
            communicator,
            data,
            protocol: Box::new(ElgamalDecrypt::new()),
        })
    }

    pub fn from_model(
        task_info: TaskInfo,
        task_model: TaskModel,
        communicator: Communicator,
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
        };
        Ok(task)
    }

    pub(super) fn start_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol.initialize(&mut self.communicator, &self.data);
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    pub(super) fn advance_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol.advance(&mut self.communicator);
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    pub(super) fn finalize_task(&mut self) -> Result<RoundUpdate, Error> {
        let decrypted = self.protocol.finalize(&mut self.communicator);
        if decrypted.is_none() {
            let reason = "Task failed (data not output)".to_string();
            return Ok(RoundUpdate::Failed(FailedTask {
                task_info: self.task_info.clone(),
                reason,
            }));
        }
        let decrypted = decrypted.unwrap();

        info!(
            "Data decrypted by group_id={}",
            utils::hextrunc(self.group.identifier())
        );

        self.communicator.clear_input();
        Ok(RoundUpdate::Finished(
            self.protocol.round(),
            FinishedTask::new(self.task_info.clone(), TaskResult::Decrypted(decrypted)),
        ))
    }

    pub(super) fn next_round(&mut self) -> Result<RoundUpdate, Error> {
        if self.protocol.round() == 0 {
            self.start_task()
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task()
        } else {
            self.finalize_task()
        }
    }

    pub(super) fn update_internal(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
    ) -> Result<bool, Error> {
        if !self.waiting_for(device_id) {
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

        if self.communicator.round_received() && self.protocol.round() <= self.protocol.last_round()
        {
            return Ok(true);
        }
        Ok(false)
    }

    fn increment_attempt_count(&mut self) {
        self.task_info.attempts += 1;
    }
}

impl RunningTask for DecryptTask {
    fn task_info(&self) -> &TaskInfo {
        &self.task_info
    }

    fn get_work(&self, device_id: &[u8]) -> Vec<Vec<u8>> {
        if !self.waiting_for(device_id) {
            return Vec::new();
        }

        self.communicator.get_messages(device_id)
    }

    fn get_round(&self) -> u16 {
        self.protocol.round()
    }

    fn initialize(&mut self) -> Result<RoundUpdate, Error> {
        self.start_task()
    }

    fn update(&mut self, device_id: &[u8], data: &Vec<Vec<u8>>) -> Result<RoundUpdate, Error> {
        let round_update = if self.update_internal(device_id, data)? {
            self.next_round()?
        } else {
            RoundUpdate::Listen
        };
        Ok(round_update)
    }

    fn restart(&mut self) -> Result<RoundUpdate, Error> {
        self.increment_attempt_count();
        self.start_task()
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        self.communicator.waiting_for(device)
    }
}
