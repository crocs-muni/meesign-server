use dashmap::DashMap;
use log::{debug, error, warn};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::Error;
use crate::persistence::{
    Device, DeviceKind, Group, KeyType, NameValidator, Participant, PersistenceError, ProtocolType,
    Repository, TaskType,
};
use crate::proto;
use crate::task_store::TaskStore;
use crate::tasks::{
    DecisionUpdate, RoundUpdate, RunningTaskContext, Task, TaskInfo, TaskResult, VotingTask,
};
use crate::{get_timestamp, utils};
use meesign_crypto::proto::ClientMessage;
use prost::Message as _;
use rand::{prelude::IteratorRandom, thread_rng};
use tokio::sync::mpsc::Sender;
use tonic::codegen::Arc;
use tonic::Status;

pub struct State {
    devices: DashMap<Vec<u8>, Device>,
    subscribers: DashMap<Vec<u8>, Sender<Result<proto::Task, Status>>>,
    repo: Arc<Repository>,
    task_store: TaskStore,
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
        let state = State {
            devices,
            subscribers: DashMap::new(),
            repo: repo.clone(),
            task_store: TaskStore::new(repo),
            task_last_updates: DashMap::new(),
            device_last_activations: DashMap::new(),
        };
        Ok(state)
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
        &self,
        name: &str,
        device_ids: &[&[u8]],
        threshold: u32,
        protocol_type: ProtocolType,
        key_type: KeyType,
        note: Option<String>,
    ) -> Result<proto::Task, Error> {
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
            task_type: TaskType::Group,
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

        self.add_task(task).await
    }

    pub async fn add_sign_task(
        &self,
        group_id: &[u8],
        name: &str,
        data: &[u8],
    ) -> Result<proto::Task, Error> {
        let group = self.get_repo().get_group(group_id).await?;
        let Some(group) = group else {
            warn!(
                "Signing requested from an unknown group group_id={}",
                utils::hextrunc(group_id)
            );
            return Err(Error::GeneralProtocolError("Invalid group_id".into()));
        };
        let participants = self.repo.get_group_participants(group_id).await?;
        let group_id = group.id.clone();
        let key_type = group.key_type;
        let protocol_type = group.protocol;
        let accept_threshold = group.threshold as u32;
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
            task_type,
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

        self.add_task(task).await
    }

    pub async fn add_decrypt_task(
        &self,
        group_id: &[u8],
        name: &str,
        data: &[u8],
        data_type: &str,
    ) -> Result<proto::Task, Error> {
        let group: Option<Group> = self.get_repo().get_group(group_id).await?;
        let Some(group) = group else {
            warn!(
                "Decryption requested from an unknown group group_id={}",
                utils::hextrunc(group_id)
            );
            return Err(Error::GeneralProtocolError("Invalid group_id".into()));
        };
        let participants = self.repo.get_group_participants(group_id).await?;
        let group_id = group.id.clone();
        let key_type = group.key_type;
        let protocol_type = group.protocol;
        let accept_threshold = group.threshold as u32;
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
            task_type: TaskType::Decrypt,
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

        self.add_task(task).await
    }

    async fn add_task(&self, task: VotingTask) -> Result<proto::Task, Error> {
        let task_id = task.task_info.id;
        self.task_store.persist_task(task).await?;
        let task = self.task_store.get_task(&task_id).await?;
        self.send_updates(&task).await?;
        let formatted = task.format(None, None);
        Ok(formatted)
    }

    pub async fn get_formatted_active_device_tasks(
        &self,
        device_id: &[u8],
    ) -> Result<Vec<proto::Task>, Error> {
        let task_ids = self.repo.get_active_device_tasks(device_id).await?;
        let mut tasks = Vec::new();
        for task in self.task_store.get_tasks(task_ids).await? {
            tasks.push(task.await.format(Some(device_id), None));
        }
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
                let last_active = self.get_device_last_activation(entry.key());
                (entry.value().clone(), last_active)
            })
            .collect()
    }

    fn get_device_last_activation(&self, device_id: &[u8]) -> u64 {
        *self
            .device_last_activations
            .entry(device_id.to_vec())
            .or_insert(get_timestamp()) // TODO: Assume inactive device?
    }

    fn get_devices_with_ids(&self, device_ids: &[&[u8]]) -> Vec<Device> {
        device_ids
            .iter()
            .filter_map(|device_id| self.devices.get(*device_id).map(|dev| dev.clone()))
            .collect()
    }

    pub async fn get_device_groups(&self, device: &[u8]) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_device_groups(device).await?)
    }

    pub async fn get_groups(&self) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_groups().await?)
    }

    pub async fn get_formatted_tasks(&self) -> Result<Vec<proto::Task>, Error> {
        let task_ids = self.repo.get_tasks().await?;
        let mut tasks = Vec::new();
        for task in self.task_store.get_tasks(task_ids).await? {
            tasks.push(task.await.format(None, None));
        }
        Ok(tasks)
    }

    pub async fn get_formatted_voting_task(
        &self,
        task_id: &Uuid,
        device_id: Option<&[u8]>,
    ) -> Result<proto::Task, Error> {
        let task = &*self.task_store.get_task(task_id).await?;
        let request = if let Task::Voting(task) = task {
            task.request.clone()
        } else {
            return Err(Error::GeneralProtocolError(
                "Queried task is not in voting phase".into(),
            ));
        };
        let task = task.format(device_id, Some(request));
        Ok(task)
    }

    pub async fn update_task(
        &self,
        task_id: &Uuid,
        device: &[u8],
        data: Vec<ClientMessage>,
        attempt: u32,
    ) -> Result<(), Error> {
        let task_entry = &mut *self.task_store.get_task_mut(task_id).await?;
        let Task::Running(task) = task_entry else {
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

        match task.update(device, data)? {
            RoundUpdate::Listen => {}
            RoundUpdate::GroupCertificatesSent => unreachable!(),
            RoundUpdate::NextRound(round) => {
                self.repo.set_task_round(task_id, round).await?;
                self.send_updates(task_entry).await?;
            }
            RoundUpdate::Failed(task) => {
                self.repo
                    .set_task_result(task_id, &Err(task.reason.clone()))
                    .await?;
                *task_entry = Task::Failed(task);
                self.send_updates(task_entry).await?;
            }
            RoundUpdate::Finished(round, task) => {
                self.repo.set_task_round(task_id, round).await?;
                let result = &task.result;
                let result_bytes = result.as_bytes().to_vec();
                if let TaskResult::GroupEstablished(group) = result {
                    self.repo
                        .add_group(
                            &group.id,
                            task_id,
                            &group.name,
                            group.threshold as u32,
                            group.protocol,
                            group.key_type,
                            group.certificate.as_deref(),
                            group.note.as_deref(),
                        )
                        .await?;
                }
                self.repo
                    .set_task_result(task_id, &Ok(result_bytes))
                    .await?;
                *task_entry = Task::Finished(task);
                self.send_updates(task_entry).await?;
            }
        }
        Ok(())
    }

    pub async fn decide_task(
        &self,
        task_id: &Uuid,
        device_id: &[u8],
        accept: bool,
    ) -> Result<(), Error> {
        let task_entry = &mut *self.task_store.get_task_mut(task_id).await?;
        let Task::Voting(task) = task_entry else {
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
            DecisionUpdate::Undecided => {}
            DecisionUpdate::Accepted => {
                log::info!(
                    "Task approved task_id={}",
                    utils::hextrunc(task_id.as_bytes())
                );

                let active_shares = self.choose_active_shares(task).await?;

                let mut task = task
                    .running_task_context
                    .clone()
                    .create_running_task(task, active_shares)?;

                match task.initialize()? {
                    RoundUpdate::Listen => unreachable!(),
                    RoundUpdate::Finished(_, _) => unreachable!(),
                    RoundUpdate::GroupCertificatesSent => {
                        self.repo
                            .set_task_group_certificates_sent(task_id, Some(true))
                            .await?;
                        *task_entry = Task::Running(task);
                        self.send_updates(task_entry).await?;
                    }
                    RoundUpdate::NextRound(round) => {
                        self.repo.set_task_round(task_id, round).await?;
                        *task_entry = Task::Running(task);
                        self.send_updates(task_entry).await?;
                    }
                    RoundUpdate::Failed(task) => {
                        self.repo
                            .set_task_result(task_id, &Err(task.reason.clone()))
                            .await?;
                        *task_entry = Task::Failed(task);
                        self.send_updates(task_entry).await?;
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
                *task_entry = Task::Declined(task);
                self.send_updates(task_entry).await?;
            }
        };
        Ok(())
    }

    /// Picks which shares shall participate in the protocol.
    /// Considers only those devices which accepted participation.
    /// If enough devices are available, additionaly filters by response latency.
    /// Returns a mapping of protocol indices to device ids.
    /// The clients expect that out of a participant's [0..n] shares,
    /// exactly the first [0..k] will be chosen.
    async fn choose_active_shares(&self, task: &VotingTask) -> Result<HashMap<u32, Device>, Error> {
        // NOTE: Threshold tasks need to use indices from group establishment, that is,
        //       the indices assigned to all task participants. Since we don't store
        //       any such index mapping, we generate it from a sorted list of devices.
        let mut all_participants = task.task_info.participants.clone();
        all_participants.sort_by(|a, b| a.device.id.cmp(&b.device.id));
        let first_share_indices: HashMap<Vec<u8>, u32> = all_participants
            .into_iter()
            .scan(0, |idx, p| {
                let first_share = *idx;
                *idx += p.shares;
                Some((p.device.id.clone(), first_share))
            })
            .collect();

        let accepting_participants: Vec<&Participant> = task
            .task_info
            .participants
            .iter()
            .filter(|p| task.device_accepted(&p.device.id))
            .collect();
        let latest_acceptable_time = get_timestamp() - 5;
        let connected_participants: Vec<&Participant> = accepting_participants
            .iter()
            .filter(|p| {
                let last_active_time = self.get_device_last_activation(&p.device.id);
                last_active_time > latest_acceptable_time
            })
            .copied()
            .collect();

        let total_connected_shares: u32 = connected_participants.iter().map(|p| p.shares).sum();
        let candidates = if total_connected_shares >= task.accept_threshold {
            connected_participants
        } else {
            accepting_participants
        };

        let chosen_devices = candidates
            .into_iter()
            .flat_map(|p| std::iter::repeat_n(&p.device, p.shares as usize))
            .choose_multiple(&mut thread_rng(), task.accept_threshold as usize);

        let mut active_shares = HashMap::new();
        for device in &chosen_devices {
            *active_shares.entry(device.id.clone()).or_default() += 1;
        }
        self.repo
            .set_task_active_shares(&task.task_info.id, &active_shares)
            .await?;

        let active_devices = chosen_devices
            .into_iter()
            .scan(first_share_indices, |share_indices, device| {
                let share_index = share_indices[&device.id];
                *share_indices.get_mut(&device.id).unwrap() += 1;
                Some((share_index, device.clone()))
            })
            .collect();

        Ok(active_devices)
    }

    pub async fn acknowledge_task(&self, task_id: &Uuid, device: &[u8]) -> Result<(), Error> {
        let task_entry = &mut *self.task_store.get_task_mut(task_id).await?;
        let Task::Finished(task) = task_entry else {
            return Err(Error::GeneralProtocolError(
                "Cannot acknowledge unfinished task".into(),
            ));
        };
        task.acknowledge(device);
        self.repo.set_task_acknowledgement(task_id, device).await?;
        Ok(())
    }

    pub async fn restart_stale_tasks(&self) -> Result<(), Error> {
        let now = get_timestamp();
        let task_ids = self
            .repo
            .get_restart_candidates()
            .await?
            .into_iter()
            .filter(|task_id| {
                let last_update = self.get_task_last_update(task_id);
                now.saturating_sub(last_update) > 30
            })
            .collect();
        let tasks = self.task_store.get_tasks_mut(task_ids).await?;
        for task_entry in tasks {
            let task_entry = &mut *task_entry.await;
            let Task::Running(task) = task_entry else {
                return Err(PersistenceError::DataInconsistencyError(
                    "non-running task in candidates for restart".into(),
                )
                .into());
            };
            let task_id = &task.task_info().id.clone();
            debug!("Stale task detected task_id={:?}", utils::hextrunc(task_id));
            self.set_task_last_update(task_id);

            self.repo.increment_task_attempt_count(task_id).await?;
            match task.restart()? {
                RoundUpdate::Listen => unreachable!(),
                RoundUpdate::Finished(_, _) => unreachable!(),
                RoundUpdate::GroupCertificatesSent => {
                    self.repo
                        .set_task_group_certificates_sent(task_id, Some(true))
                        .await?;
                    self.send_updates(task_entry).await?;
                }
                RoundUpdate::NextRound(round) => {
                    self.repo.set_task_round(task_id, round).await?;
                    self.send_updates(task_entry).await?;
                }
                RoundUpdate::Failed(task) => {
                    self.repo
                        .set_task_result(task_id, &Err(task.reason.clone()))
                        .await?;
                    *task_entry = Task::Failed(task);
                    self.send_updates(task_entry).await?;
                }
            }
        }
        Ok(())
    }

    pub fn add_subscriber(
        &self,
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
                let result = tx.try_send(Ok(task.format(Some(device_id), None)));

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

    pub fn set_task_last_update(&self, task_id: &Uuid) {
        self.task_last_updates.insert(*task_id, get_timestamp());
    }

    pub fn get_task_last_update(&self, task_id: &Uuid) -> u64 {
        *self
            .task_last_updates
            .entry(*task_id)
            .or_insert(get_timestamp())
    }
}
