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
use crate::tasks::{
    DecisionUpdate, DeclinedTask, FailedTask, FinishedTask, RoundUpdate, RunningTask,
    RunningTaskContext, Task, TaskInfo, TaskResult, VotingTask,
};
use crate::{get_timestamp, utils};
use prost::Message as _;
use tokio::sync::mpsc::Sender;
use tonic::codegen::Arc;
use tonic::Status;

pub struct State {
    // tasks: HashMap<Uuid, Box<dyn Task + Send + Sync>>,
    devices: DashMap<Vec<u8>, Device>,
    subscribers: DashMap<Vec<u8>, Sender<Result<crate::proto::Task, Status>>>,
    repo: Arc<Repository>,
    communicators: DashMap<Uuid, Arc<RwLock<Communicator>>>,
    task_last_updates: DashMap<Uuid, u64>,
    device_last_activations: DashMap<Vec<u8>, u64>,
}

impl State {
    pub async fn restore(repo: Arc<Repository>) -> Result<Self, Error> {
        let devices = repo
            .get_devices()
            .await?
            .into_iter()
            .map(|dev| (dev.id.clone(), dev))
            .collect();
        Ok(State {
            // tasks: HashMap::new(),
            devices,
            subscribers: DashMap::new(),
            repo,
            communicators: DashMap::default(),
            task_last_updates: DashMap::new(),
            device_last_activations: DashMap::new(),
        })
    }

    pub async fn add_device(
        &self,
        identifier: &[u8],
        name: &str,
        kind: &DeviceKind,
        certificate: &[u8],
    ) -> Result<Device, Error> {
        let device = self
            .get_repo()
            .add_device(identifier, name, kind, certificate)
            .await?;
        self.devices.insert(device.id.clone(), device.clone());
        Ok(device)
    }
    pub async fn add_group_task(
        &mut self,
        name: &str,
        device_ids: &[&[u8]],
        threshold: u32,
        protocol_type: ProtocolType,
        key_type: KeyType,
        note: Option<String>,
    ) -> Result<Task, Error> {
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
            .get_devices_with_ids(&device_ids)
            .into_iter()
            .map(|device| {
                let shares = shares[device.id.as_slice()];
                Participant { device, shares }
            })
            .collect();
        let task_info = TaskInfo {
            id: Uuid::new_v4(),
            name: name.to_string(),
            task_type: TaskType::Group.into(),
            protocol_type,
            key_type,
            participants,
            attempts: 0,
        };
        let accept_threshold = task_info.total_shares();
        let request = (proto::GroupRequest {
            device_ids: device_ids.into_iter().map(Vec::from).collect(),
            name: task_info.name.clone(),
            threshold,
            protocol: protocol_type as i32,
            key_type: key_type as i32,
            note: note.clone(),
        })
        .encode_to_vec();
        let running_task_context = RunningTaskContext::Group {
            threshold,
            note: note.clone(),
        };
        let task = VotingTask {
            task_info,
            decisions: HashMap::new(),
            accept_threshold,
            request,
            running_task_context,
        };

        let participant_ids_shares: Vec<_> = task
            .task_info
            .participants
            .iter()
            .map(|participant| (participant.device.id.as_slice(), participant.shares))
            .collect();
        self.repo
            .create_group_task(
                Some(&task.task_info.id),
                &participant_ids_shares,
                threshold,
                task.task_info.protocol_type.into(),
                task.task_info.key_type.into(),
                &task.request,
                note.as_deref(),
            )
            .await?;

        let task = Task::Voting(task);
        self.send_updates(&task).await?;
        Ok(task)
    }

    pub async fn add_sign_task(
        &mut self,
        group_id: &[u8],
        name: &str,
        data: &[u8],
    ) -> Result<Task, Error> {
        let group = self.get_repo().get_group(group_id).await?;
        let Some(group) = group else {
            warn!(
                "Signing requested from an unknown group group_id={}",
                utils::hextrunc(group_id)
            );
            return Err(Error::GeneralProtocolError("Invalid group_id".into()));
        };
        let participants = self.repo.get_group_participants(group_id).await?;
        let group = crate::group::Group::from_model(group, participants.clone());
        let group_id = group.identifier().to_vec();
        let key_type = group.key_type();
        let protocol_type = group.protocol();
        let accept_threshold = group.threshold();
        let data = data.to_vec();
        let request = proto::SignRequest {
            group_id: group_id.clone(),
            name: name.to_string(),
            data: data.clone(),
        }
        .encode_to_vec();
        let (task_type, running_task_context) = match key_type {
            KeyType::SignPdf => (
                TaskType::SignPdf,
                RunningTaskContext::SignPdf { group, data },
            ),
            KeyType::SignChallenge => (
                TaskType::SignChallenge,
                RunningTaskContext::SignChallenge { group, data },
            ),
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
        let task_info = TaskInfo {
            id: Uuid::new_v4(),
            name: name.to_string(),
            task_type: task_type.into(),
            protocol_type,
            key_type,
            participants,
            attempts: 0,
        };
        let task = VotingTask {
            task_info,
            decisions: HashMap::new(),
            accept_threshold,
            request,
            running_task_context,
        };

        self.add_threshold_task(task).await
    }

    pub async fn add_decrypt_task(
        &mut self,
        group_id: &[u8],
        name: &str,
        data: &[u8],
        data_type: &str,
    ) -> Result<Task, Error> {
        let group: Option<Group> = self.get_repo().get_group(group_id).await?;
        let Some(group) = group else {
            warn!(
                "Decryption requested from an unknown group group_id={}",
                utils::hextrunc(group_id)
            );
            return Err(Error::GeneralProtocolError("Invalid group_id".into()));
        };
        let participants = self.repo.get_group_participants(group_id).await?;
        let group = crate::group::Group::from_model(group, participants.clone());
        let group_id = group.identifier().to_vec();
        let key_type = group.key_type();
        let protocol_type = group.protocol();
        let accept_threshold = group.threshold();
        let data = data.to_vec();
        let request = proto::DecryptRequest {
            group_id: group_id.clone(),
            name: name.to_string(),
            data: data.clone(),
            data_type: data_type.to_string(),
        }
        .encode_to_vec();
        let running_task_context = match key_type {
            KeyType::Decrypt => RunningTaskContext::Decrypt { group, data },
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
        let task_info = TaskInfo {
            id: Uuid::new_v4(),
            name: name.to_string(),
            task_type: TaskType::Decrypt.into(),
            protocol_type,
            key_type,
            participants,
            attempts: 0,
        };
        let task = VotingTask {
            task_info,
            decisions: HashMap::new(),
            accept_threshold,
            request,
            running_task_context,
        };

        self.add_threshold_task(task).await
    }

    async fn add_threshold_task(&mut self, task: VotingTask) -> Result<Task, Error> {
        let participant_ids_shares: Vec<(&[u8], u32)> = task
            .task_info
            .participants
            .iter()
            .map(|participant| (participant.device.id.as_slice(), participant.shares))
            .collect();
        let (task_type, group, data) = match &task.running_task_context {
            RunningTaskContext::Group { .. } => unreachable!(),
            RunningTaskContext::SignChallenge { group, data } => {
                (TaskType::SignChallenge, group, data)
            }
            RunningTaskContext::SignPdf { group, data } => (TaskType::SignPdf, group, data),
            RunningTaskContext::Decrypt { group, data, .. } => (TaskType::Decrypt, group, data),
        };
        self.repo
            .create_threshold_task(
                Some(&task.task_info.id),
                group.identifier(),
                &participant_ids_shares,
                group.threshold(),
                "name", // TODO: Fix name checks
                &data,
                &task.request,
                task_type.into(),
                task.task_info.key_type.into(),
                task.task_info.protocol_type.into(),
            )
            .await?;

        let task = Task::Voting(task);
        self.send_updates(&task).await?;
        Ok(task)
    }

    pub async fn get_active_device_tasks(&self, device: &[u8]) -> Result<Vec<Task>, Error> {
        let task_models = self.get_repo().get_active_device_tasks(device).await?;
        let tasks = self.tasks_from_task_models(task_models).await?;
        Ok(tasks)
    }

    pub fn activate_device(&self, device_id: &[u8]) {
        self.device_last_activations
            .insert(device_id.to_vec(), get_timestamp());
    }

    pub fn device_exists(&self, device_id: &[u8]) -> bool {
        self.devices.contains_key(device_id)
    }

    pub fn get_devices(&self) -> Vec<(Device, u64)> {
        self.devices
            .iter()
            .map(|entry| {
                let last_active = *self
                    .device_last_activations
                    .entry(entry.key().clone())
                    .or_insert(get_timestamp()); // TODO: Assume inactive device?
                (entry.value().clone(), last_active)
            })
            .collect()
    }

    fn get_devices_with_ids(&self, device_ids: &[&[u8]]) -> Vec<Device> {
        device_ids
            .into_iter()
            .filter_map(|device_id| self.devices.get(*device_id).map(|dev| dev.clone()))
            .collect()
    }

    pub async fn get_device_groups(&self, device: &[u8]) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_device_groups(device).await?)
    }

    pub async fn get_groups(&self) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_groups().await?)
    }

    pub async fn get_tasks(&self) -> Result<Vec<Task>, Error> {
        let task_models = self.repo.get_tasks().await?;
        let tasks = self.tasks_from_task_models(task_models).await?;
        Ok(tasks)
    }

    pub async fn get_task(&self, task_id: &Uuid) -> Result<Task, Error> {
        let Some(task_model) = self.repo.get_task(task_id).await? else {
            return Err(Error::GeneralProtocolError("Invalid task id".into()));
        };
        let task = self
            .tasks_from_task_models(vec![task_model])
            .await?
            .pop()
            .unwrap();
        Ok(task)
    }

    pub async fn update_task(
        &mut self,
        task_id: &Uuid,
        device: &[u8],
        data: &Vec<Vec<u8>>,
        attempt: u32,
    ) -> Result<(), Error> {
        let Task::Running(mut task) = self.get_task(task_id).await? else {
            return Err(Error::GeneralProtocolError(
                "Cannot update non-running task".into(),
            ));
        };
        self.set_task_last_update(task_id);
        if attempt != task.task_info().attempts {
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
                self.send_updates(&Task::Running(task)).await?;
            }
            RoundUpdate::Failed(task) => {
                self.repo
                    .set_task_result(task_id, &Err(task.reason.clone()))
                    .await?;
                self.send_updates(&Task::Failed(task)).await?;
            }
            RoundUpdate::Finished(round, task) => {
                self.repo.set_task_round(task_id, round).await?;
                let result = &task.result;
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
                self.send_updates(&Task::Finished(task)).await?;
            }
        }
        Ok(())
    }

    pub async fn decide_task(
        &mut self,
        task_id: &Uuid,
        device_id: &[u8],
        accept: bool,
    ) -> Result<(), Error> {
        let Task::Voting(task) = self.get_task(task_id).await? else {
            return Err(Error::GeneralProtocolError(
                "Cannot decide non-voting task".into(),
            ));
        };
        self.set_task_last_update(task_id);
        let decision_update = task.decide(device_id, accept).await?;
        self.repo
            .set_task_decision(task_id, device_id, accept)
            .await?;
        match decision_update {
            DecisionUpdate::Undecided(_) => {}
            DecisionUpdate::Accepted(mut task) => {
                log::info!(
                    "Task approved task_id={}",
                    utils::hextrunc(task_id.as_bytes())
                );

                if let Some(_communicator) = self
                    .communicators
                    .insert(task_id.clone(), task.get_communicator())
                {
                    error!(
                        "A communicator for task with id {} already exists!",
                        task_id
                    );
                    return Err(Error::GeneralProtocolError("Data inconsistency".into()));
                };

                match task.initialize().await? {
                    RoundUpdate::Listen => unreachable!(),
                    RoundUpdate::Finished(_, _) => unreachable!(),
                    RoundUpdate::GroupCertificatesSent => {
                        self.repo
                            .set_task_group_certificates_sent(task_id, Some(true))
                            .await?;
                        self.send_updates(&Task::Running(task)).await?;
                    }
                    RoundUpdate::NextRound(round) => {
                        self.repo.set_task_round(task_id, round).await?;
                        self.send_updates(&Task::Running(task)).await?;
                    }
                    RoundUpdate::Failed(task) => {
                        self.repo
                            .set_task_result(task_id, &Err(task.reason.clone()))
                            .await?;
                        self.send_updates(&Task::Failed(task)).await?;
                    }
                }
            }
            DecisionUpdate::Declined(task) => {
                log::info!(
                    "Task declined task_id={}",
                    utils::hextrunc(task_id.as_bytes())
                );
                self.repo
                    .set_task_result(task_id, &Err("Task declined".into()))
                    .await?;
                self.send_updates(&Task::Declined(task)).await?;
            }
        }
        Ok(())
    }

    pub async fn acknowledge_task(&mut self, task_id: &Uuid, device: &[u8]) -> Result<(), Error> {
        let Task::Finished(mut task) = self.get_task(task_id).await? else {
            return Err(Error::GeneralProtocolError(
                "Cannot acknowledge unfinished task".into(),
            ));
        };
        task.acknowledge(device);
        self.repo.set_task_acknowledgement(task_id, device).await?;
        Ok(())
    }

    async fn restart_task(&self, mut task: Box<dyn RunningTask>) -> Result<(), Error> {
        let task_id = &task.task_info().id.clone();
        self.set_task_last_update(task_id);

        self.repo.increment_task_attempt_count(task_id).await?;
        match task.restart().await? {
            RoundUpdate::Listen => unreachable!(),
            RoundUpdate::Finished(_, _) => unreachable!(),
            RoundUpdate::GroupCertificatesSent => {
                self.repo
                    .set_task_group_certificates_sent(task_id, Some(true))
                    .await?;
                self.send_updates(&Task::Running(task)).await?;
            }
            RoundUpdate::NextRound(round) => {
                self.repo.set_task_round(task_id, round).await?;
                self.send_updates(&Task::Running(task)).await?;
            }
            RoundUpdate::Failed(task) => {
                self.repo
                    .set_task_result(task_id, &Err(task.reason.clone()))
                    .await?;
                self.send_updates(&Task::Failed(task)).await?;
            }
        }
        Ok(())
    }

    pub async fn restart_stale_tasks(&self) -> Result<(), Error> {
        // TODO: Filter tasks in DB
        let tasks = self.get_tasks().await?;
        let now = get_timestamp();

        for task in tasks {
            let Task::Running(task) = task else {
                // NOTE: Only running tasks can be restarted
                continue;
            };
            let task_id = &task.task_info().id;
            let last_update = self.get_task_last_update(task_id);
            if now.saturating_sub(last_update) > 30 {
                debug!("Stale task detected task_id={:?}", utils::hextrunc(task_id));
                self.restart_task(task).await?;
            }
        }
        Ok(())
    }

    pub fn add_subscriber(
        &mut self,
        device_id: Vec<u8>,
        tx: Sender<Result<crate::proto::Task, Status>>,
    ) {
        self.subscribers.insert(device_id, tx);
    }

    pub fn remove_subscriber(&self, device_id: &Vec<u8>) {
        self.subscribers.remove(device_id);
        debug!(
            "Removing subscriber device_id={}",
            utils::hextrunc(device_id)
        );
    }

    pub fn get_subscribers(&self) -> &DashMap<Vec<u8>, Sender<Result<crate::proto::Task, Status>>> {
        &self.subscribers
    }

    pub async fn send_updates(&self, task: &Task) -> Result<(), Error> {
        let mut remove = Vec::new();

        for participant in &task.task_info().participants {
            let device_id = participant.device.identifier();
            if let Some(tx) = self.subscribers.get(device_id) {
                let result =
                    tx.try_send(Ok(self.format_task(&task, Some(device_id), None).await?));

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

    async fn get_communicator(
        &self,
        task_model: &TaskModel,
        mut participants: Vec<Participant>,
    ) -> Result<Arc<RwLock<Communicator>>, Error> {
        use dashmap::mapref::entry::Entry;
        let communicator = match self.communicators.entry(task_model.id.clone()) {
            Entry::Occupied(entry) => entry.into_ref(),
            Entry::Vacant(entry) => {
                participants.sort_by(|a, b| a.device.identifier().cmp(b.device.identifier()));
                let decisions = self.repo.get_task_decisions(&task_model.id).await?;
                let threshold = match task_model.task_type {
                    TaskType::Group => participants.iter().map(|p| p.shares).sum(),
                    _ => task_model.threshold as u32,
                };

                entry.insert(Arc::new(RwLock::new(Communicator::new(
                    participants,
                    threshold,
                    task_model.protocol_type.into(),
                    decisions,
                ))))
            }
        }
        .clone();
        Ok(communicator)
    }

    async fn tasks_from_task_models(
        &self,
        task_models: Vec<TaskModel>,
    ) -> Result<Vec<Task>, Error> {
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
        for task_model in task_models {
            let participants = task_id_participants.remove(&task_model.id).unwrap();
            let task_info = TaskInfo {
                id: task_model.id.clone(),
                name: "".into(), // TODO: Persist "name" in TaskModel
                task_type: task_model.task_type.clone().into(),
                protocol_type: task_model.protocol_type.into(),
                key_type: task_model.key_type.clone().into(),
                participants,
                attempts: task_model.attempt_count as u32,
            };
            let task = match task_model.task_type {
                TaskType::Group => self.group_task_from_model(task_info, task_model).await?,
                TaskType::SignChallenge | TaskType::SignPdf | TaskType::Decrypt => {
                    self.threshold_task_from_model(task_info, task_model)
                        .await?
                }
            };

            tasks.push(task)
        }
        Ok(tasks)
    }

    async fn group_task_from_model(
        &self,
        task_info: TaskInfo,
        task_model: TaskModel,
    ) -> Result<Task, Error> {
        assert_eq!(task_model.task_type, TaskType::Group);

        let task_result = task_model.result.clone();
        let task = if let Some(task_result) = task_result {
            match task_result.try_into_result()? {
                Ok(group_id) => {
                    let Some(group_model) = self.repo.get_group(&group_id).await? else {
                        return Err(Error::PersistenceError(
                            PersistenceError::DataInconsistencyError(
                                "Group task result references nonexistent group".into(),
                            ),
                        ));
                    };
                    let group = crate::group::Group::from_model(
                        group_model,
                        task_info.participants.clone(),
                    );
                    let result = TaskResult::GroupEstablished(group);
                    let acknowledgements =
                        self.repo.get_task_acknowledgements(&task_model.id).await?;
                    Task::Finished(FinishedTask {
                        task_info,
                        result,
                        acknowledgements,
                    })
                }
                Err(reason) => {
                    let decisions = self.repo.get_task_decisions(&task_model.id).await?;
                    let (accepts, rejects) = VotingTask::accepts_rejects(&decisions);
                    if rejects > 0 {
                        Task::Declined(DeclinedTask {
                            task_info,
                            accepts,
                            rejects,
                        })
                    } else {
                        Task::Failed(FailedTask { task_info, reason })
                    }
                }
            }
        } else {
            let decisions = self.repo.get_task_decisions(&task_model.id).await?;
            let (accepts, _) = VotingTask::accepts_rejects(&decisions);
            let accept_threshold = task_info.total_shares();
            if accepts < accept_threshold {
                let running_task_context = RunningTaskContext::Group {
                    threshold: task_model.threshold as u32,
                    note: task_model.note,
                };
                let task = VotingTask {
                    task_info,
                    decisions,
                    accept_threshold,
                    request: task_model.request,
                    running_task_context,
                };
                Task::Voting(task)
            } else {
                let communicator = self
                    .get_communicator(&task_model, task_info.participants.clone())
                    .await?;
                let task = Box::new(GroupTask::from_model(task_info, task_model, communicator)?);
                Task::Running(task)
            }
        };
        Ok(task)
    }

    async fn threshold_task_from_model(
        &self,
        task_info: TaskInfo,
        task_model: TaskModel,
    ) -> Result<Task, Error> {
        let task_result = task_model.result.clone();
        let task = if let Some(task_result) = task_result {
            match task_result.try_into_result()? {
                Ok(data) => {
                    let result = match task_model.task_type {
                        TaskType::Group => unreachable!(),
                        TaskType::SignPdf => TaskResult::SignedPdf(data),
                        TaskType::SignChallenge => TaskResult::Signed(data),
                        TaskType::Decrypt => TaskResult::Decrypted(data),
                    };
                    let acknowledgements =
                        self.repo.get_task_acknowledgements(&task_model.id).await?;
                    Task::Finished(FinishedTask {
                        task_info,
                        result,
                        acknowledgements,
                    })
                }
                Err(reason) => {
                    let decisions = self.repo.get_task_decisions(&task_model.id).await?;
                    let (accepts, rejects) = VotingTask::accepts_rejects(&decisions);
                    let total_shares = task_info.total_shares();
                    let reject_threshold = total_shares - task_model.threshold as u32 + 1;
                    if rejects >= reject_threshold {
                        Task::Declined(DeclinedTask {
                            task_info,
                            accepts,
                            rejects,
                        })
                    } else {
                        Task::Failed(FailedTask { task_info, reason })
                    }
                }
            }
        } else {
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
            let group =
                crate::group::Group::from_model(group_model, task_info.participants.clone());

            let decisions = self.repo.get_task_decisions(&task_model.id).await?;
            let (accepts, _) = VotingTask::accepts_rejects(&decisions);
            let accept_threshold = group.threshold();
            if accepts < accept_threshold {
                let data = task_model
                    .task_data
                    .ok_or(PersistenceError::DataInconsistencyError(
                        "Threshold task has no task data".into(),
                    ))?;
                let running_task_context = match task_model.task_type {
                    TaskType::Group => unreachable!(),
                    TaskType::SignPdf => RunningTaskContext::SignPdf { group, data },
                    TaskType::SignChallenge => RunningTaskContext::SignChallenge { group, data },
                    TaskType::Decrypt => RunningTaskContext::Decrypt { group, data },
                };
                let task = VotingTask {
                    task_info,
                    decisions,
                    accept_threshold,
                    request: task_model.request,
                    running_task_context,
                };
                Task::Voting(task)
            } else {
                let communicator = self
                    .get_communicator(&task_model, task_info.participants.clone())
                    .await?;
                let task: Box<dyn RunningTask + Send + Sync> = match task_model.task_type {
                    TaskType::Group => unreachable!(),
                    TaskType::SignPdf => Box::new(SignPDFTask::from_model(
                        task_info,
                        task_model,
                        communicator,
                        group,
                    )?),
                    TaskType::SignChallenge => Box::new(SignTask::from_model(
                        task_info,
                        task_model,
                        communicator,
                        group,
                    )?),
                    TaskType::Decrypt => Box::new(DecryptTask::from_model(
                        task_info,
                        task_model,
                        communicator,
                        group,
                    )?),
                };
                Task::Running(task)
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
        task: &Task,
        device_id: Option<&[u8]>,
        request: Option<Vec<u8>>,
    ) -> Result<proto::Task, Error> {
        let request = request.map(Vec::from);
        let task_info = task.task_info();
        let id = task_info.id.as_bytes().to_vec();
        let r#type = task_info.task_type.clone().into();
        let attempt = task_info.attempts;
        let task = match task {
            Task::Voting(task) => {
                let (accept, reject) = VotingTask::accepts_rejects(&task.decisions);
                proto::Task::created(id, r#type, accept, reject, request, attempt)
            }
            Task::Running(task) => {
                let round = task.get_round() as u32;
                let data = if let Some(device_id) = device_id {
                    task.get_work(device_id).await
                } else {
                    Vec::new()
                };
                proto::Task::running(id, r#type, round, data, request, attempt)
            }
            Task::Finished(task) => proto::Task::finished(
                id,
                r#type,
                task.result.as_bytes().to_vec(),
                request,
                attempt,
            ),
            Task::Failed(task) => {
                proto::Task::failed(id, r#type, task.reason.clone(), request, attempt)
            }
            Task::Declined(task) => {
                proto::Task::declined(id, r#type, task.accepts, task.rejects, request, attempt)
            }
        };
        Ok(task)
    }
}
