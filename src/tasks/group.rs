use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::PersistenceError;
use crate::persistence::Task as TaskModel;
use crate::proto::ProtocolType;
use crate::protocols::{create_keygen_protocol, Protocol};
use crate::tasks::{FailedTask, FinishedTask, RoundUpdate, RunningTask, TaskInfo, TaskResult};
use crate::utils;
use log::{info, warn};
use meesign_crypto::proto::{ClientMessage, Message as _, ServerMessage};
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};

pub struct GroupTask {
    task_info: TaskInfo,
    threshold: u32,
    communicator: Communicator,
    protocol: Box<dyn Protocol + Send + Sync>,
    note: Option<String>,
    certificates_sent: bool, // TODO: remove the field completely
}

impl GroupTask {
    pub fn try_new(
        mut task_info: TaskInfo,
        threshold: u32,
        note: Option<String>,
        decisions: HashMap<Vec<u8>, i8>,
    ) -> Result<Self, String> {
        let total_shares = task_info.total_shares();

        let protocol = create_keygen_protocol(
            task_info.protocol_type,
            task_info.key_type,
            total_shares,
            threshold,
            0,
        )?;

        if total_shares < 1 {
            warn!("Invalid number of parties {}", total_shares);
            return Err("Invalid input".into());
        }
        if !protocol.get_type().check_threshold(threshold, total_shares) {
            warn!("Invalid group threshold {}-of-{}", threshold, total_shares);
            return Err("Invalid input".into());
        }

        task_info
            .participants
            .sort_by(|a, b| a.device.identifier().cmp(b.device.identifier()));

        let group_task_threshold = total_shares;

        let communicator = Communicator::new(
            task_info.participants.clone(),
            group_task_threshold,
            task_info.protocol_type,
            decisions,
        );

        Ok(GroupTask {
            task_info,
            threshold,
            communicator,
            protocol,
            note,
            certificates_sent: false,
        })
    }

    pub fn from_model(
        task_info: TaskInfo,
        model: TaskModel,
        communicator: Communicator,
    ) -> Result<Self, Error> {
        let protocol = create_keygen_protocol(
            model.protocol_type.into(),
            model.key_type.clone().into(),
            task_info.total_shares(),
            model.threshold as u32,
            model.protocol_round as u16,
        )?;
        let Some(certificates_sent) = model.group_certificates_sent else {
            return Err(Error::PersistenceError(
                PersistenceError::DataInconsistencyError(
                    "certificates_sent flag missing in group task".into(),
                ),
            ));
        };
        Ok(Self {
            task_info,
            threshold: model.threshold as u32,
            communicator,
            protocol,
            note: model.note,
            certificates_sent, // TODO: remove the field completely
        })
    }

    fn increment_attempt_count(&mut self) {
        self.task_info.attempts += 1;
    }

    fn start_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol.initialize(&mut self.communicator, &[]);
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    fn advance_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol.advance(&mut self.communicator);
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    fn finalize_task(&mut self) -> Result<RoundUpdate, Error> {
        let identifier = self.protocol.finalize(&mut self.communicator);
        let Some(identifier) = identifier else {
            let reason = "Task failed (group key not output)".to_string();
            return Ok(RoundUpdate::Failed(FailedTask {
                task_info: self.task_info.clone(),
                reason,
            }));
        };
        // TODO
        let certificate = if self.protocol.get_type() == ProtocolType::Gg18 {
            Some(issue_certificate(&self.task_info.name, &identifier)?)
        } else {
            None
        };

        info!(
            "Group established group_id={} devices={:?}",
            utils::hextrunc(&identifier),
            self.task_info
                .participants
                .iter()
                .map(|p| (utils::hextrunc(p.device.identifier()), p.shares))
                .collect::<Vec<_>>()
        );

        let group = Group::new(
            identifier.clone(),
            self.task_info.name.clone(),
            self.threshold,
            self.task_info.participants.clone(),
            self.protocol.get_type(),
            self.task_info.key_type,
            certificate,
            self.note.clone(),
        );

        self.communicator.clear_input();
        Ok(RoundUpdate::Finished(
            self.protocol.round(),
            FinishedTask::new(self.task_info.clone(), TaskResult::GroupEstablished(group)),
        ))
    }

    fn next_round(&mut self) -> Result<RoundUpdate, Error> {
        if !self.certificates_sent {
            self.send_certificates()
        } else if self.protocol.round() == 0 {
            self.start_task()
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task()
        } else {
            self.finalize_task()
        }
    }

    fn send_certificates(&mut self) -> Result<RoundUpdate, Error> {
        self.communicator.set_active_devices(None);

        let certs: HashMap<u32, Vec<u8>> = {
            self.task_info
                .participants
                .iter()
                .flat_map(|p| {
                    let cert = &p.device.certificate;
                    self.communicator
                        .identifier_to_indices(p.device.identifier())
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

        self.communicator.send_all(|_| certs.clone());
        self.certificates_sent = true;
        Ok(RoundUpdate::GroupCertificatesSent)
    }
}

impl RunningTask for GroupTask {
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
        if !self.certificates_sent {
            0
        } else {
            self.protocol.round() + 1
        }
    }

    fn initialize(&mut self) -> Result<RoundUpdate, Error> {
        self.send_certificates()
    }

    fn update(&mut self, device_id: &[u8], data: &Vec<Vec<u8>>) -> Result<RoundUpdate, Error> {
        if !self.waiting_for(device_id) {
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

        self.communicator.receive_messages(device_id, messages);

        if self.communicator.round_received() && self.protocol.round() <= self.protocol.last_round()
        {
            return self.next_round();
        }

        Ok(RoundUpdate::Listen)
    }

    fn restart(&mut self) -> Result<RoundUpdate, Error> {
        self.increment_attempt_count();
        // TODO: Should this instead be the certificate exchange round?
        self.start_task()
    }

    fn waiting_for(&self, device: &[u8]) -> bool {
        self.communicator.waiting_for(device)
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
