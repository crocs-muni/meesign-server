use crate::communicator::Communicator;
use crate::device::Device;
use crate::group::Group;
use crate::proto::{ProtocolType, SignRequest, TaskType};
use crate::protocols::frost::FROSTSign;
use crate::protocols::gg18::GG18Sign;
use crate::protocols::Protocol;
use crate::tasks::{Task, TaskResult, TaskStatus};
use crate::{get_timestamp, utils};
use log::{info, warn};
use meesign_crypto::proto::{Message as _, ProtocolMessage};
use prost::Message as _;
use tonic::codegen::Arc;

pub struct SignTask {
    group: Group,
    communicator: Communicator,
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
        let mut devices: Vec<Arc<Device>> = group.devices().to_vec();
        devices.sort_by_key(|x| x.identifier().to_vec());
        let protocol_type = group.protocol();

        let communicator = Communicator::new(&devices, group.threshold(), protocol_type);

        let request = (SignRequest {
            group_id: group.identifier().to_vec(),
            name,
            data: data.clone(),
        })
        .encode_to_vec();

        Ok(SignTask {
            group,
            communicator,
            result: None,
            data,
            preprocessed: None,
            protocol: match protocol_type {
                ProtocolType::Gg18 => Box::new(GG18Sign::new()),
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

    pub(super) fn start_task(&mut self) {
        assert!(self.communicator.accept_count() >= self.group.threshold());
        self.protocol.initialize(
            &mut self.communicator,
            self.preprocessed.as_ref().unwrap_or(&self.data),
        );
    }

    pub(super) fn advance_task(&mut self) {
        self.protocol.advance(&mut self.communicator)
    }

    pub(super) fn finalize_task(&mut self) {
        let signature = self.protocol.finalize(&mut self.communicator);
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
        self.communicator.clear_input();
    }

    pub(super) fn next_round(&mut self) {
        if self.protocol.round() == 0 {
            self.start_task();
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task()
        } else {
            self.finalize_task()
        }
    }

    pub(super) fn update_internal(
        &mut self,
        device_id: &[u8],
        data: &[u8],
    ) -> Result<bool, String> {
        if self.communicator.accept_count() < self.group.threshold() {
            return Err("Not enough agreements to proceed with the protocol.".to_string());
        }

        if !self.waiting_for(device_id) {
            return Err("Wasn't waiting for a message from this ID.".to_string());
        }

        let data =
            ProtocolMessage::decode(data).map_err(|_| String::from("Expected ProtocolMessage."))?;
        self.communicator.receive_messages(device_id, data.message);
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
        let result = self.update_internal(device_id, data);
        if let Ok(true) = result {
            self.next_round();
        };
        result
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

    fn decide(&mut self, device_id: &[u8], decision: bool) -> Option<bool> {
        let result = self.decide_internal(device_id, decision);
        if let Some(true) = result {
            self.next_round();
        };
        result
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
