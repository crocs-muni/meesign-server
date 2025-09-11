use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::Participant;
use crate::persistence::PersistenceError;
use crate::persistence::Task as TaskModel;
use crate::proto::{KeyType, ProtocolType, TaskType};
use crate::protocols::{create_keygen_protocol, Protocol};
use crate::tasks::{
    ActiveTask, DecisionUpdate, DeclinedTask, FailedTask, FinishedTask, RestartUpdate, RoundUpdate,
    TaskResult,
};
use crate::utils;
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
    participants: Vec<Participant>,
    communicator: Arc<RwLock<Communicator>>,
    protocol: Box<dyn Protocol + Send + Sync>,
    request: Vec<u8>,
    attempts: u32,
    note: Option<String>,
    certificates_sent: bool, // TODO: remove the field completely
}

impl GroupTask {
    pub fn try_new(
        name: &str,
        mut participants: Vec<Participant>,
        threshold: u32,
        protocol_type: ProtocolType,
        key_type: KeyType,
        note: Option<String>,
    ) -> Result<Self, String> {
        let id = Uuid::new_v4();

        let total_shares: u32 = participants.iter().map(|p| p.shares).sum();

        let protocol = create_keygen_protocol(protocol_type, key_type, total_shares, threshold, 0)?;

        if total_shares < 1 {
            warn!("Invalid number of parties {}", total_shares);
            return Err("Invalid input".into());
        }
        if !protocol.get_type().check_threshold(threshold, total_shares) {
            warn!("Invalid group threshold {}-of-{}", threshold, total_shares);
            return Err("Invalid input".into());
        }

        participants.sort_by(|a, b| a.device.identifier().cmp(b.device.identifier()));

        let group_task_threshold = total_shares;

        let decisions = participants
            .iter()
            .map(|p| (p.device.identifier().clone(), 0))
            .collect();

        let communicator = Arc::new(RwLock::new(Communicator::new(
            participants.clone(),
            group_task_threshold,
            protocol.get_type(),
            decisions,
        )));

        let device_ids = participants
            .iter()
            .flat_map(|p| std::iter::repeat(p.device.identifier().to_vec()).take(p.shares as usize))
            .collect();
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
            participants: participants.to_vec(),
            key_type,
            communicator,
            protocol,
            request,
            attempts: 0,
            note,
            certificates_sent: false,
        })
    }

    pub fn from_model(
        model: TaskModel,
        participants: Vec<Participant>,
        communicator: Arc<RwLock<Communicator>>,
    ) -> Result<Self, Error> {
        let total_shares = participants.iter().map(|p| p.shares).sum();

        let protocol = create_keygen_protocol(
            model.protocol_type.into(),
            model.key_type.clone().into(),
            total_shares,
            model.threshold as u32,
            model.protocol_round as u16,
        )?;

        let name = "".into(); // TODO: Add "name" to "task" table
        let Some(certificates_sent) = model.group_certificates_sent else {
            return Err(Error::PersistenceError(
                PersistenceError::DataInconsistencyError(
                    "certificates_sent flag missing in group task".into(),
                ),
            ));
        };
        Ok(Self {
            name,
            id: model.id,
            threshold: model.threshold as u32,
            key_type: model.key_type.into(),
            participants,
            communicator,
            protocol,
            request: model.request,
            attempts: model.attempt_count as u32,
            note: model.note,
            certificates_sent, // TODO: remove the field completely
        })
    }

    fn increment_attempt_count(&mut self) {
        self.attempts += 1;
    }

    async fn start_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol
            .initialize(&mut *self.communicator.write().await, &[]);
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    async fn advance_task(&mut self) -> Result<RoundUpdate, Error> {
        self.protocol.advance(&mut *self.communicator.write().await);
        Ok(RoundUpdate::NextRound(self.protocol.round()))
    }

    async fn finalize_task(&mut self) -> Result<RoundUpdate, Error> {
        let identifier = self
            .protocol
            .finalize(&mut *self.communicator.write().await);
        let Some(identifier) = identifier else {
            let reason = "Task failed (group key not output)".to_string();
            return Ok(RoundUpdate::Failed(FailedTask { reason }));
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
            self.participants
                .iter()
                .map(|p| (utils::hextrunc(p.device.identifier()), p.shares))
                .collect::<Vec<_>>()
        );

        let group = Group::new(
            identifier.clone(),
            self.name.clone(),
            self.threshold,
            self.participants.clone(),
            self.protocol.get_type(),
            self.key_type,
            certificate,
            self.note.clone(),
        );

        self.communicator.write().await.clear_input();
        Ok(RoundUpdate::Finished(
            self.protocol.round(),
            FinishedTask::new(TaskResult::GroupEstablished(group)),
        ))
    }

    async fn next_round(&mut self) -> Result<RoundUpdate, Error> {
        if !self.certificates_sent {
            self.send_certificates().await
        } else if self.protocol.round() == 0 {
            self.start_task().await
        } else if self.protocol.round() < self.protocol.last_round() {
            self.advance_task().await
        } else {
            self.finalize_task().await
        }
    }

    async fn send_certificates(&mut self) -> Result<RoundUpdate, Error> {
        self.communicator.write().await.set_active_devices(None);

        let certs: HashMap<u32, Vec<u8>> = {
            let communicator_read = self.communicator.read().await;
            self.participants
                .iter()
                .flat_map(|p| {
                    let cert = &p.device.certificate;
                    communicator_read
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

        self.communicator.write().await.send_all(|_| certs.clone());
        self.certificates_sent = true;
        Ok(RoundUpdate::GroupCertificatesSent)
    }
}

#[async_trait]
impl ActiveTask for GroupTask {
    fn get_type(&self) -> TaskType {
        TaskType::Group
    }

    async fn get_work(&self, device_id: &[u8]) -> Vec<Vec<u8>> {
        if !self.waiting_for(device_id).await {
            return Vec::new();
        }

        self.communicator.read().await.get_messages(device_id)
    }

    fn get_round(&self) -> u16 {
        if !self.certificates_sent {
            0
        } else {
            self.protocol.round() + 1
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
    ) -> Result<RoundUpdate, Error> {
        let total_shares: u32 = self.participants.iter().map(|p| p.shares).sum();
        if self.communicator.read().await.accept_count() != total_shares {
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

        if self.communicator.read().await.round_received()
            && self.protocol.round() <= self.protocol.last_round()
        {
            return self.next_round().await;
        }

        Ok(RoundUpdate::Listen)
    }

    async fn restart(&mut self) -> Result<RestartUpdate, Error> {
        if self.is_approved().await {
            self.increment_attempt_count();
            // TODO: Should this instead be the certificate exchange round?
            let round_update = self.start_task().await?;
            Ok(RestartUpdate::Started(round_update))
        } else {
            Ok(RestartUpdate::Voting)
        }
    }

    async fn is_approved(&self) -> bool {
        let total_shares: u32 = self.participants.iter().map(|p| p.shares).sum();
        self.communicator.read().await.accept_count() == total_shares
    }

    fn get_participants(&self) -> &Vec<Participant> {
        &self.participants
    }

    async fn waiting_for(&self, device: &[u8]) -> bool {
        let communicator = self.communicator.write().await;
        if !self.certificates_sent && self.protocol.round() == 0 {
            return !communicator.device_decided(device);
        }

        communicator.waiting_for(device)
    }

    async fn decide(&mut self, device_id: &[u8], decision: bool) -> Result<DecisionUpdate, Error> {
        self.communicator.write().await.decide(device_id, decision);
        let decision_update = if self.protocol.round() == 0 {
            if self.communicator.read().await.reject_count() > 0 {
                let (accepts, rejects) = self.get_decisions().await;
                DecisionUpdate::Declined(DeclinedTask { accepts, rejects })
            } else if self.is_approved().await {
                let round_update = self.next_round().await?;
                DecisionUpdate::Accepted(round_update)
            } else {
                DecisionUpdate::Undecided
            }
        } else {
            DecisionUpdate::Undecided
        };
        Ok(decision_update)
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
