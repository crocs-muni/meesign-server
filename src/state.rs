use dashmap::DashMap;
use log::{debug, error, warn};
use std::collections::HashMap;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::persistence::{
    Device, DeviceKind, Group, NameValidator, Participant, PersistenceError, Repository,
    Task as TaskModel, TaskType,
};
use crate::proto::{self, KeyType, ProtocolType};
use crate::tasks::decrypt::DecryptTask;
use crate::tasks::group::GroupTask;
use crate::tasks::sign::SignTask;
use crate::tasks::sign_pdf::SignPDFTask;
use crate::tasks::{DecisionUpdate, RestartUpdate, RoundUpdate, Task, TaskResult};
use crate::{get_timestamp, utils};
use tokio::sync::mpsc::Sender;
use tonic::codegen::Arc;
use tonic::Status;

pub struct State {
    // tasks: HashMap<Uuid, Box<dyn Task + Send + Sync>>,
    subscribers: DashMap<Vec<u8>, Sender<Result<crate::proto::Task, Status>>>,
    repo: Arc<Repository>,
    communicators: DashMap<Uuid, Arc<RwLock<Communicator>>>,
    task_last_updates: DashMap<Uuid, u64>,
    device_last_activations: DashMap<Vec<u8>, u64>,
}

impl State {
    pub fn new(repo: Arc<Repository>) -> Self {
        State {
            // tasks: HashMap::new(),
            subscribers: DashMap::new(),
            repo,
            communicators: DashMap::default(),
            task_last_updates: DashMap::new(),
            device_last_activations: DashMap::new(),
        }
    }

    pub async fn add_device(
        &self,
        identifier: &[u8],
        name: &str,
        kind: &DeviceKind,
        certificate: &[u8],
    ) -> Result<Device, Error> {
        Ok(self
            .get_repo()
            .add_device(identifier, name, kind, certificate)
            .await?)
    }
    pub async fn add_group_task(
        &mut self,
        name: &str,
        device_ids: &[&[u8]],
        threshold: u32,
        protocol: ProtocolType,
        key_type: KeyType,
        note: Option<String>,
    ) -> Result<Uuid, Error> {
        if !name.is_name_valid() {
            error!("Group request with invalid group name {}", name);
            return Err(Error::GeneralProtocolError(format!(
                "Invalid group name {name}"
            )));
        }
        let mut shares: HashMap<&[u8], u32> = HashMap::new();
        for device_id in device_ids {
            *shares.entry(device_id).or_default() += 1;
        }
        let device_ids: Vec<&[u8]> = shares.keys().cloned().collect();
        let participants = self
            .get_repo()
            .get_devices_with_ids(&device_ids)
            .await?
            .into_iter()
            .map(|device| {
                let shares = shares[device.id.as_slice()];
                Participant { device, shares }
            })
            .collect();
        let task = Box::new(GroupTask::try_new(
            name,
            participants,
            threshold,
            protocol,
            key_type,
            note,
        )?) as Box<dyn Task + Sync + Send>;

        // TODO: group ID?
        let task_id = self.add_task(task, &[], key_type, protocol).await?;

        self.send_updates(&task_id).await?;
        Ok(task_id)
    }

    pub async fn add_sign_task(
        &mut self,
        group_id: &[u8],
        name: &str,
        data: &[u8],
    ) -> Result<Uuid, Error> {
        let group = self.get_repo().get_group(group_id).await?;
        let Some(group) = group else {
            warn!(
                "Signing requested from an unknown group group_id={}",
                utils::hextrunc(group_id)
            );
            return Err(Error::GeneralProtocolError("Invalid group_id".into()));
        };
        let participants = self.repo.get_group_participants(group_id).await?;
        let group = crate::group::Group::from_model(group, participants);
        let task = match group.key_type() {
            KeyType::SignPdf => {
                let task = SignPDFTask::try_new(group.clone(), name.to_string(), data.to_vec())?;
                Box::new(task) as Box<dyn Task + Sync + Send>
            }
            KeyType::SignChallenge => {
                let task = SignTask::try_new(group.clone(), name.to_string(), data.to_vec())?;
                Box::new(task) as Box<dyn Task + Sync + Send>
            }
            KeyType::Decrypt => {
                warn!(
                    "Signing request made for decryption group group_id={}",
                    utils::hextrunc(group_id)
                );
                return Err(Error::GeneralProtocolError(
                    "Decryption groups can't accept signing requests".into(),
                ));
            }
        };

        let task_id = self
            .add_task(task, group.identifier(), group.key_type(), group.protocol())
            .await?;
        self.send_updates(&task_id).await?;

        Ok(task_id)
    }

    pub async fn add_decrypt_task(
        &mut self,
        group_id: &[u8],
        name: &str,
        data: &[u8],
        data_type: &str,
    ) -> Result<Uuid, Error> {
        let group: Option<Group> = self.get_repo().get_group(group_id).await?;
        let Some(group) = group else {
            warn!(
                "Decryption requested from an unknown group group_id={}",
                utils::hextrunc(group_id)
            );
            return Err(Error::GeneralProtocolError("Invalid group_id".into()));
        };
        let participants = self.repo.get_group_participants(group_id).await?;
        let group = crate::group::Group::from_model(group, participants);
        let task = match group.key_type() {
            KeyType::Decrypt => {
                let task = DecryptTask::try_new(
                    group.clone(),
                    name.to_string(),
                    data.to_vec(),
                    data_type.to_string(),
                )?;
                Box::new(task) as Box<dyn Task + Sync + Send>
            }
            KeyType::SignPdf | KeyType::SignChallenge => {
                warn!(
                    "Decryption request made for a signing group group_id={}",
                    utils::hextrunc(group_id)
                );
                return Err(Error::GeneralProtocolError(
                    "Signing group can't accept decryption requests".into(),
                ));
            }
        };

        let task_id = self
            .add_task(task, group.identifier(), group.key_type(), group.protocol())
            .await?;
        self.send_updates(&task_id).await?;
        Ok(task_id)
    }

    async fn add_task(
        &mut self,
        task: Box<dyn Task + Sync + Send>,
        group_id: &[u8],
        key_type: KeyType,
        protocol_type: ProtocolType,
    ) -> Result<Uuid, Error> {
        let task_participants = task.get_participants();
        let participant_ids_shares: Vec<(&[u8], u32)> = task_participants
            .iter()
            .map(|participant| (participant.device.id.as_slice(), participant.shares))
            .collect();
        let created_task = match task.get_type() {
            crate::proto::TaskType::Group => {
                self.get_repo()
                    .create_group_task(
                        Some(task.get_id()),
                        &participant_ids_shares,
                        task.get_threshold(),
                        protocol_type.into(),
                        key_type.into(),
                        task.get_request(),
                        None, // TODO: missing note
                    )
                    .await?
            }
            task_type => {
                self.get_repo()
                    .create_threshold_task(
                        Some(task.get_id()),
                        group_id,
                        &participant_ids_shares,
                        task.get_threshold(),
                        "name",
                        task.get_data().unwrap(),
                        task.get_request(),
                        task_type.into(),
                        key_type.into(),
                        protocol_type.into(),
                    )
                    .await?
            }
        };
        if let Some(_communicator) = self
            .communicators
            .insert(created_task.id, task.get_communicator())
        {
            // TODO: create a new "internal error" error variant
            error!(
                "A communicator for task with id {} already exists!",
                created_task.id
            );
            return Err(Error::GeneralProtocolError("Data inconsistency".into()));
        };
        Ok(created_task.id)
    }

    pub async fn get_active_device_tasks(&self, device: &[u8]) -> Result<Vec<TaskModel>, Error> {
        Ok(self.get_repo().get_active_device_tasks(device).await?)
    }

    pub fn activate_device(&self, device_id: &[u8]) {
        self.device_last_activations
            .insert(device_id.to_vec(), get_timestamp());
    }

    pub async fn device_exists(&self, device_id: &[u8]) -> Result<bool, Error> {
        // TODO: Optimize query / cache devices in State
        let devices = self.repo.get_devices_with_ids(&[device_id]).await?;
        Ok(devices.len() == 1 && devices[0].id == device_id)
    }

    pub async fn get_devices(&self) -> Result<Vec<(Device, u64)>, Error> {
        let devices = self
            .get_repo()
            .get_devices()
            .await?
            .into_iter()
            .map(|dev| {
                let last_active = *self
                    .device_last_activations
                    .entry(dev.id.clone())
                    .or_insert(get_timestamp());
                (dev, last_active)
            })
            .collect();
        Ok(devices)
    }

    pub async fn get_device_groups(&self, device: &[u8]) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_device_groups(device).await?)
    }

    pub async fn get_groups(&self) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_groups().await?)
    }

    pub async fn get_tasks(&self) -> Result<Vec<TaskModel>, Error> {
        Ok(self.repo.get_tasks().await?)
    }

    pub async fn get_task(&self, task_id: &Uuid) -> Result<TaskModel, Error> {
        let Some(task_model) = self.repo.get_task(task_id).await? else {
            return Err(Error::GeneralProtocolError("Invalid task id".into()));
        };
        Ok(task_model)
    }

    pub async fn update_task(
        &mut self,
        task_id: &Uuid,
        device: &[u8],
        data: &Vec<Vec<u8>>,
        attempt: u32,
    ) -> Result<(), Error> {
        let task_model = self.get_task(task_id).await?;
        let mut task = self.task_from_task_model(task_model).await?;
        self.set_task_last_update(task_id);
        if attempt != task.get_attempts() {
            warn!(
                "Stale update discarded task_id={} device_id={} attempt={}",
                utils::hextrunc(task_id.as_bytes()),
                utils::hextrunc(device),
                attempt
            );
            return Err(Error::GeneralProtocolError("Stale update".to_string()));
        }

        match task.update(device, data).await? {
            RoundUpdate::Listen => {}
            RoundUpdate::GroupCertificatesSent => unreachable!(),
            RoundUpdate::NextRound(round) => {
                self.repo.set_task_round(task_id, round).await?;
                self.send_updates(task_id).await?;
            }
            RoundUpdate::Failed(reason) => {
                self.repo.set_task_result(task_id, &Err(reason)).await?;
                self.send_updates(task_id).await?;
            }
            RoundUpdate::Finished(round, result) => {
                self.repo.set_task_round(task_id, round).await?;
                let result_bytes = result.as_bytes().to_vec();
                if let TaskResult::GroupEstablished(group) = result {
                    self.repo
                        .add_group(
                            group.identifier(),
                            task_id,
                            group.name(),
                            group.threshold(),
                            group.protocol().into(),
                            group.key_type().into(),
                            group.certificate().map(|v| v.as_ref()),
                            group.note(),
                        )
                        .await?;
                }
                self.repo
                    .set_task_result(task_id, &Ok(result_bytes))
                    .await?;
                // NOTE: Updates must be sent after the group is persisted
                self.send_updates(task_id).await?;
            }
        }
        Ok(())
    }

    pub async fn decide_task(
        &mut self,
        task_id: &Uuid,
        device: &[u8],
        decision: bool,
    ) -> Result<(), Error> {
        let task_model = self.get_task(task_id).await?;
        let mut task = self.task_from_task_model(task_model).await?;
        self.set_task_last_update(task_id);
        let decision_update = task.decide(device, decision).await?;
        self.repo
            .set_task_decision(task_id, device, decision)
            .await?;
        match decision_update {
            DecisionUpdate::Undecided => {}
            DecisionUpdate::Accepted(round_update) => {
                log::info!(
                    "Task approved task_id={}",
                    utils::hextrunc(task_id.as_bytes())
                );
                match round_update {
                    RoundUpdate::Listen => unreachable!(),
                    RoundUpdate::Finished(_, _) => unreachable!(),
                    RoundUpdate::GroupCertificatesSent => {
                        self.repo
                            .set_task_group_certificates_sent(task_id, Some(true))
                            .await?;
                    }
                    RoundUpdate::NextRound(round) => {
                        self.repo.set_task_round(task_id, round).await?;
                    }
                    RoundUpdate::Failed(reason) => {
                        self.repo.set_task_result(task_id, &Err(reason)).await?;
                    }
                }
                self.send_updates(task_id).await?;
            }
            DecisionUpdate::Declined => {
                log::info!(
                    "Task declined task_id={}",
                    utils::hextrunc(task_id.as_bytes())
                );
                self.repo
                    .set_task_result(task_id, &Err("Task declined".into()))
                    .await?;
                self.send_updates(task_id).await?;
            }
        }
        Ok(())
    }

    pub async fn acknowledge_task(&mut self, task_id: &Uuid, device: &[u8]) -> Result<(), Error> {
        let task_model = self.get_task(task_id).await?;
        let mut task = self.task_from_task_model(task_model).await?;
        task.acknowledge(device).await;
        self.repo.set_task_acknowledgement(task_id, device).await?;
        Ok(())
    }

    pub async fn restart_task(&mut self, task_id: &Uuid) -> Result<bool, Error> {
        let task_model = self.get_task(task_id).await?;
        let mut task = self.task_from_task_model(task_model).await?;
        self.set_task_last_update(task_id);

        match task.restart().await? {
            RestartUpdate::AlreadyFinished => Ok(false),
            RestartUpdate::Voting => Ok(false),
            RestartUpdate::Started(round_update) => {
                self.repo.increment_task_attempt_count(task_id).await?;
                match round_update {
                    RoundUpdate::Listen => unreachable!(),
                    RoundUpdate::Finished(_, _) => unreachable!(),
                    RoundUpdate::GroupCertificatesSent => {
                        self.repo
                            .set_task_group_certificates_sent(task_id, Some(true))
                            .await?;
                    }
                    RoundUpdate::NextRound(round) => {
                        self.repo.set_task_round(task_id, round).await?;
                    }
                    RoundUpdate::Failed(reason) => {
                        self.repo.set_task_result(task_id, &Err(reason)).await?;
                    }
                }
                self.send_updates(task_id).await?;
                Ok(true)
            }
        }
    }

    pub async fn get_tasks_for_restart(&self) -> Result<Vec<Uuid>, Error> {
        let task_models = self.get_tasks().await?;
        let tasks = self.tasks_from_task_models(task_models.clone()).await?;
        let now = get_timestamp();

        let mut restarts = Vec::new();
        for (task_model, task) in task_models.into_iter().zip(tasks.into_iter()) {
            let task_id = &task_model.id;
            let result = task_model
                .result
                .map(|res| res.try_into_result())
                .transpose()?;
            let last_update = self.get_task_last_update(task_id);
            let stale = result.is_none()
                && task.is_approved().await
                && now.saturating_sub(last_update) > 30;
            if stale {
                debug!("Stale task detected task_id={:?}", utils::hextrunc(task_id));
                restarts.push(*task_id);
            }
        }
        Ok(restarts)
    }

    pub fn add_subscriber(
        &mut self,
        device_id: Vec<u8>,
        tx: Sender<Result<crate::proto::Task, Status>>,
    ) {
        self.subscribers.insert(device_id, tx);
    }

    pub fn remove_subscriber(&mut self, device_id: &Vec<u8>) {
        self.subscribers.remove(device_id);
        debug!(
            "Removing subscriber device_id={}",
            utils::hextrunc(device_id)
        );
    }

    pub fn get_subscribers(&self) -> &DashMap<Vec<u8>, Sender<Result<crate::proto::Task, Status>>> {
        &self.subscribers
    }

    pub async fn send_updates(&mut self, task_id: &Uuid) -> Result<(), Error> {
        let Some(task_model) = self.repo.get_task(task_id).await? else {
            return Err(Error::GeneralProtocolError("Invalid task id".into()));
        };
        let participants = self.repo.get_task_participants(task_id).await?;
        let mut remove = Vec::new();

        for participant in participants {
            let device_id = participant.device.identifier();
            if let Some(tx) = self.subscribers.get(device_id) {
                let result = tx.try_send(Ok(self
                    .format_task(task_model.clone(), Some(device_id), None)
                    .await?));

                if result.is_err() {
                    debug!(
                        "Closed channel detected device_id={}…",
                        utils::hextrunc(&device_id[..4])
                    );
                    remove.push(device_id.to_vec());
                }
            }
        }

        for device_id in remove {
            self.remove_subscriber(&device_id);
        }

        Ok(())
    }

    fn get_repo(&self) -> &Arc<Repository> {
        &self.repo
    }

    async fn get_communicator(&self, task_id: &Uuid) -> Result<Arc<RwLock<Communicator>>, Error> {
        use dashmap::mapref::entry::Entry;
        let communicator = match self.communicators.entry(task_id.clone()) {
            Entry::Occupied(entry) => entry.into_ref(),
            Entry::Vacant(entry) => {
                let Some(model) = self.repo.get_task(task_id).await? else {
                    return Err(Error::GeneralProtocolError("Invalid task id".into()));
                };
                let mut participants = self.repo.get_task_participants(task_id).await?;
                participants.sort_by(|a, b| a.device.identifier().cmp(b.device.identifier()));
                let decisions = self.repo.get_task_decisions(task_id).await?;
                let acknowledgements = self.repo.get_task_acknowledgements(task_id).await?;
                let threshold = match model.task_type {
                    TaskType::Group => participants.iter().map(|p| p.shares).sum(),
                    _ => model.threshold as u32,
                };

                entry.insert(Arc::new(RwLock::new(Communicator::new(
                    participants,
                    threshold,
                    model.protocol_type.into(),
                    decisions,
                    acknowledgements,
                ))))
            }
        }
        .clone();
        Ok(communicator)
    }

    async fn task_from_task_model(
        &self,
        task_model: TaskModel,
    ) -> Result<Box<dyn Task + Send + Sync>, Error> {
        let task = self
            .tasks_from_task_models(vec![task_model])
            .await?
            .pop()
            .unwrap();
        Ok(task)
    }

    async fn tasks_from_task_models(
        &self,
        task_models: Vec<TaskModel>,
    ) -> Result<Vec<Box<dyn Task + Send + Sync>>, Error> {
        // NOTE: Sorted and unique task models (strict inequality)
        assert!(task_models.windows(2).all(|w| w[0].id < w[1].id));

        let task_ids: Vec<_> = task_models.iter().map(|task| task.id.clone()).collect();

        let task_id_participant_pairs = self.get_repo().get_tasks_participants(&task_ids).await?;

        let mut task_id_participants: HashMap<_, Vec<_>> = HashMap::new();

        for (task_id, device) in task_id_participant_pairs {
            task_id_participants
                .entry(task_id)
                .or_default()
                .push(device);
        }

        // NOTE: When hydrating many tasks, `future::join_all` would cause
        //       a deadlock with `repository::get_async_connection`.
        let mut tasks = Vec::new();
        for task in task_models {
            let participants = task_id_participants.remove(&task.id).unwrap();
            let communicator = self.get_communicator(&task.id).await?;

            let task = match task.task_type {
                TaskType::Group => {
                    self.group_task_from_model(task, communicator, participants)
                        .await?
                }
                TaskType::SignChallenge | TaskType::SignPdf | TaskType::Decrypt => {
                    self.threshold_task_from_model(task, communicator, participants)
                        .await?
                }
            };

            tasks.push(task)
        }
        Ok(tasks)
    }

    async fn group_task_from_model(
        &self,
        task_model: TaskModel,
        communicator: Arc<RwLock<Communicator>>,
        participants: Vec<Participant>,
    ) -> Result<Box<dyn Task + Send + Sync>, Error> {
        let task_result = task_model.result.clone();
        let group = if let Some(task_result) = task_result {
            match task_result.try_into_result()? {
                Ok(group_id) => {
                    let Some(group_model) = self.repo.get_group(&group_id).await? else {
                        return Err(Error::PersistenceError(
                            PersistenceError::DataInconsistencyError(
                                "Group task result references nonexistent group".into(),
                            ),
                        ));
                    };
                    let group = crate::group::Group::from_model(group_model, participants.clone());
                    Some(group)
                }
                Err(_) => None,
            }
        } else {
            None
        };

        assert_eq!(task_model.task_type, TaskType::Group);

        let task = Box::new(GroupTask::from_model(
            task_model,
            participants,
            communicator,
            group,
        )?);
        Ok(task)
    }

    async fn threshold_task_from_model(
        &self,
        task_model: TaskModel,
        communicator: Arc<RwLock<Communicator>>,
        participants: Vec<Participant>,
    ) -> Result<Box<dyn Task + Send + Sync>, Error> {
        let Some(group_id) = &task_model.group_id else {
            return Err(Error::PersistenceError(
                PersistenceError::DataInconsistencyError(
                    "Threshold task is missing a group".into(),
                ),
            ));
        };
        let Some(group_model) = self.repo.get_group(group_id).await? else {
            return Err(Error::PersistenceError(
                PersistenceError::DataInconsistencyError(
                    "Threshold task references nonexistent group".into(),
                ),
            ));
        };
        let group = crate::group::Group::from_model(group_model, participants);

        let task: Box<dyn Task + Send + Sync> = match task_model.task_type {
            TaskType::Group => unreachable!(),
            TaskType::SignPdf => {
                Box::new(SignPDFTask::from_model(task_model, communicator, group)?)
            }
            TaskType::SignChallenge => {
                Box::new(SignTask::from_model(task_model, communicator, group)?)
            }
            TaskType::Decrypt => {
                Box::new(DecryptTask::from_model(task_model, communicator, group)?)
            }
        };
        Ok(task)
    }

    pub fn set_task_last_update(&self, task_id: &Uuid) {
        self.task_last_updates
            .insert(task_id.clone(), get_timestamp());
    }

    pub fn get_task_last_update(&self, task_id: &Uuid) -> u64 {
        *self
            .task_last_updates
            .entry(task_id.clone())
            .or_insert(get_timestamp())
    }

    pub async fn format_task(
        &self,
        task_model: TaskModel,
        device_id: Option<&[u8]>,
        request: Option<Vec<u8>>,
    ) -> Result<proto::Task, Error> {
        let request = request.map(Vec::from);
        let id = task_model.id.as_bytes().to_vec();
        let r#type: proto::TaskType = task_model.task_type.clone().into();
        let r#type = r#type.into();
        let attempt = task_model.attempt_count as u32;
        let task = match task_model.result.clone() {
            None => {
                let task = self.task_from_task_model(task_model).await?;
                if !task.is_approved().await {
                    let (accept, reject) = task.get_decisions().await;
                    proto::Task::created(id, r#type, accept, reject, request, attempt)
                } else {
                    let round = task.get_round() as u32;
                    let data = if let Some(device_id) = device_id {
                        task.get_work(device_id).await
                    } else {
                        Vec::new()
                    };
                    proto::Task::running(id, r#type, round, data, request, attempt)
                }
            }
            Some(result) => match result.try_into_result()? {
                Ok(result) => proto::Task::finished(id, r#type, result, request, attempt),
                Err(reason) => {
                    let round = task_model.protocol_round as u32;
                    let task = self.task_from_task_model(task_model).await?;
                    let (accept, reject) = task.get_decisions().await;
                    proto::Task::failed(id, r#type, round, accept, reject, reason, request, attempt)
                }
            },
        };
        Ok(task)
    }
}
