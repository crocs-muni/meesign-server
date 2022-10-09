use crate::communicator::Communicator;
use crate::device::Device;
use crate::get_timestamp;
use crate::group::Group;
use crate::proto::{Gg18Message, SignRequest};
use crate::protocols::gg18::GG18Sign;
use crate::protocols::Protocol;
use crate::tasks::{Task, TaskResult, TaskStatus, TaskType};
use log::info;
use prost::Message;
use tonic::codegen::Arc;

pub struct SignChallengeTask {
    group: Group,
    communicator: Communicator,
    result: Option<Result<Vec<u8>, String>>,
    challenge: Vec<u8>,
    protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
    last_update: u64,
    attempts: u32,
}

impl SignChallengeTask {
    pub fn new(group: Group, name: String, data: Vec<u8>) -> Self {
        let mut devices: Vec<Arc<Device>> = group.devices().to_vec();
        devices.sort_by_key(|x| x.identifier().to_vec());

        let communicator = Communicator::new(&devices, group.threshold());

        let request = (SignRequest {
            group_id: group.identifier().to_vec(),
            name,
            data: data.clone(),
        })
        .encode_to_vec();

        SignChallengeTask {
            group,
            communicator,
            result: None,
            challenge: data,
            protocol: Box::new(GG18Sign::new()),
            request,
            last_update: get_timestamp(),
            attempts: 0,
        }
    }

    fn start_task(&mut self) {
        assert!(self.communicator.accept_count() >= self.group.threshold());
        self.protocol
            .initialize(&mut self.communicator, &self.challenge);
    }

    fn advance_task(&mut self) {
        self.protocol.advance(&mut self.communicator)
    }

    fn finalize_task(&mut self) {
        let signature = self.protocol.finalize(&mut self.communicator);
        if signature.is_none() {
            self.result = Some(Err("Task failed (signature not output)".to_string()));
            return;
        }
        let signature = signature.unwrap();

        info!(
            "Challenge signed by group_id={}",
            hex::encode(self.group.identifier())
        );

        self.result = Some(Ok(signature));
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

impl Task for SignChallengeTask {
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
        TaskType::Sign
    }

    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>> {
        if device_id.is_none() || !self.waiting_for(device_id.unwrap()) {
            return None;
        }

        self.communicator.get_message(device_id.unwrap())
    }

    fn get_result(&self) -> Option<TaskResult> {
        if let Some(Ok(signature)) = &self.result {
            Some(TaskResult::Signed(signature.clone()))
        } else {
            None
        }
    }

    fn get_decisions(&self) -> (u32, u32) {
        (
            self.communicator.accept_count(),
            self.communicator.reject_count(),
        )
    }

    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<bool, String> {
        if self.communicator.accept_count() < self.group.threshold() {
            return Err("Not enough agreements to proceed with the protocol.".to_string());
        }

        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string());
        }

        let data: Gg18Message =
            Message::decode(data).map_err(|_| String::from("Expected GG18Message."))?;
        self.communicator.receive_messages(device_id, data.message);
        self.last_update = get_timestamp();

        if self.communicator.round_received() && self.protocol.round() <= self.protocol.last_round()
        {
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
        self.communicator.accept_count() >= self.group.threshold()
    }

    fn has_device(&self, device_id: &[u8]) -> bool {
        self.group.contains(device_id)
    }

    fn get_devices(&self) -> Vec<Arc<Device>> {
        self.group.devices().to_vec()
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        if self.protocol.round() == 0 {
            return !self.communicator.device_decided(device);
        } else if self.protocol.round() >= self.protocol.last_round() {
            return !self.communicator.device_acknowledged(device);
        }

        self.communicator.waiting_for(device)
    }

    fn decide(&mut self, device_id: &[u8], decision: bool) -> bool {
        self.communicator.decide(device_id, decision);
        self.last_update = get_timestamp();
        if self.result.is_none() && self.protocol.round() == 0 {
            if self.communicator.reject_count() >= self.group.reject_threshold() {
                self.result = Some(Err("Task declined".to_string()));
                return true;
            } else if self.communicator.accept_count() >= self.group.threshold() {
                self.next_round();
                return true;
            }
        }
        false
    }

    fn acknowledge(&mut self, device_id: &[u8]) {
        self.communicator.acknowledge(device_id);
    }

    fn device_acknowledged(&self, device_id: &[u8]) -> bool {
        self.communicator.device_acknowledged(device_id)
    }

    fn get_request(&self) -> &[u8] {
        &self.request
    }

    fn get_attempts(&self) -> u32 {
        self.attempts
    }
}
