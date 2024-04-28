use std::collections::HashMap;

use dashmap::DashMap;
use log::{debug, warn};
use uuid::Uuid;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::interfaces::grpc::format_task;
use crate::persistence::{Group, Repository};
use crate::proto::KeyType;
use crate::tasks::decrypt::DecryptTask;
use crate::tasks::group::GroupTask;
use crate::tasks::sign::SignTask;
use crate::tasks::sign_pdf::SignPDFTask;
use crate::tasks::{Task, TaskResult, TaskStatus};
use crate::utils;
use tokio::sync::mpsc::Sender;
use tonic::codegen::Arc;
use tonic::Status;

pub struct State {
    tasks: HashMap<Uuid, Box<dyn Task + Send + Sync>>,
    subscribers: DashMap<Vec<u8>, Sender<Result<crate::proto::Task, Status>>>,
    repo: Arc<Repository>,
    communicators: DashMap<Uuid, Communicator>,
}

impl State {
    pub fn new(repo: Arc<Repository>) -> Self {
        State {
            tasks: HashMap::new(),
            subscribers: DashMap::new(),
            repo,
            communicators: DashMap::default(),
        }
    }

    pub async fn add_sign_task(
        &mut self,
        group_id: &[u8],
        name: &str,
        data: &[u8],
    ) -> Result<Option<Uuid>, Error> {
        let group: Option<crate::group::Group> = self
            .get_repo()
            .get_group(group_id)
            .await?
            .map(|val| val.into());
        if group.is_none() {
            warn!(
                "Signing requested from an unknown group group_id={}",
                utils::hextrunc(group_id)
            );
            return Ok(None);
        }
        let group = group.unwrap();
        let task = match group.key_type() {
            KeyType::SignPdf => {
                SignPDFTask::try_new(group.clone(), name.to_string(), data.to_vec())
                    .ok()
                    .map(|task| Box::new(task) as Box<dyn Task + Sync + Send>)
            }
            KeyType::SignChallenge => {
                SignTask::try_new(group.clone(), name.to_string(), data.to_vec())
                    .ok()
                    .map(|task| Box::new(task) as Box<dyn Task + Sync + Send>)
            }
            KeyType::Decrypt => {
                warn!(
                    "Signing request made for decryption group group_id={}",
                    utils::hextrunc(group_id)
                );
                return Ok(None);
            }
        };

        let task_id = task.map(|task| self.add_task(task));
        if let Some(task_id) = &task_id {
            self.send_updates(task_id).await?;
        }
        Ok(task_id)
    }

    pub async fn add_decrypt_task(
        &mut self,
        group_id: &[u8],
        name: &str,
        data: &[u8],
        data_type: &str,
    ) -> Result<Option<Uuid>, Error> {
        let group: Option<crate::group::Group> = self
            .get_repo()
            .get_group(group_id)
            .await?
            .map(|val| val.into());
        if group.is_none() {
            warn!(
                "Decryption requested from an unknown group group_id={}",
                utils::hextrunc(group_id)
            );
            return Ok(None);
        }
        let group = group.unwrap();
        let task = match group.key_type() {
            KeyType::Decrypt => Some(DecryptTask::new(
                group.clone(),
                name.to_string(),
                data.to_vec(),
                data_type.to_string(),
            ))
            .map(|task| Box::new(task) as Box<dyn Task + Sync + Send>),
            KeyType::SignPdf | KeyType::SignChallenge => {
                warn!(
                    "Decryption request made for a signing group group_id={}",
                    utils::hextrunc(group_id)
                );
                return Ok(None);
            }
        };

        let task_id = task.map(|task| self.add_task(task));
        if let Some(task_id) = &task_id {
            self.send_updates(task_id).await?;
        }
        Ok(task_id)
    }

    fn add_task(&mut self, task: Box<dyn Task + Sync + Send>) -> Uuid {
        let uuid = Uuid::new_v4();
        self.tasks.insert(uuid, task);
        uuid
    }

    pub fn get_device_tasks(&self, device: &[u8]) -> Vec<(Uuid, &dyn Task)> {
        let mut tasks = Vec::new();
        for (uuid, task) in self.tasks.iter() {
            // TODO refactor
            if task.has_device(device)
                && (task.get_status() != TaskStatus::Finished
                    || (task.get_status() == TaskStatus::Finished
                        && !task.device_acknowledged(device)))
            {
                tasks.push((*uuid, task.as_ref() as &dyn Task));
            }
        }
        tasks
    }

    pub async fn get_device_groups(&self, device: &[u8]) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_device_groups(device).await?)
    }

    pub async fn get_groups(&self) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_groups().await?)
    }

    pub fn get_tasks(&self) -> &HashMap<Uuid, Box<dyn Task + Send + Sync>> {
        &self.tasks
    }

    pub fn get_task(&self, task: &Uuid) -> Option<&dyn Task> {
        self.tasks.get(task).map(|task| task.as_ref() as &dyn Task)
    }

    pub async fn update_task(
        &mut self,
        task_id: &Uuid,
        device: &[u8],
        data: &Vec<Vec<u8>>,
        attempt: u32,
    ) -> Result<bool, Error> {
        let task = self.tasks.get_mut(task_id).unwrap();
        if attempt != task.get_attempts() {
            warn!(
                "Stale update discarded task_id={} device_id={} attempt={}",
                utils::hextrunc(task_id.as_bytes()),
                utils::hextrunc(device),
                attempt
            );
            return Err(Error::GeneralProtocolError("Stale update".to_string()));
        }

        let previous_status = task.get_status();
        let update_result = task.update(device, data);
        if previous_status != TaskStatus::Finished && task.get_status() == TaskStatus::Finished {
            // TODO join if statements once #![feature(let_chains)] gets stabilized
            if let TaskResult::GroupEstablished(group) = task.get_result().unwrap() {
                let device_ids: Vec<&[u8]> = group
                    .devices()
                    .iter()
                    .map(|device| device.identifier())
                    .collect();
                self.get_repo()
                    .add_group(
                        group.identifier(),
                        task_id,
                        group.name(),
                        &device_ids[..],
                        group.threshold(),
                        group.protocol().into(),
                        group.key_type().into(),
                        group.certificate().map(|v| v.as_ref()),
                    )
                    .await?;
            }
        }
        if let Ok(true) = update_result {
            self.send_updates(task_id).await?;
        }
        update_result.map_err(|err| Error::GeneralProtocolError(err))
    }

    pub async fn decide_task(
        &mut self,
        task_id: &Uuid,
        device: &[u8],
        decision: bool,
    ) -> Result<bool, Error> {
        let task = self.tasks.get_mut(task_id).unwrap();
        let change = task.decide(device, decision);
        if change.is_some() {
            self.send_updates(task_id).await?;
            if change.unwrap() {
                log::info!(
                    "Task approved task_id={}",
                    utils::hextrunc(task_id.as_bytes())
                );
            } else {
                log::info!(
                    "Task declined task_id={}",
                    utils::hextrunc(task_id.as_bytes())
                );
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub fn acknowledge_task(&mut self, task: &Uuid, device: &[u8]) {
        let task = self.tasks.get_mut(task).unwrap();
        task.acknowledge(device);
    }

    pub async fn restart_task(&mut self, task_id: &Uuid) -> Result<bool, Error> {
        if self
            .tasks
            .get_mut(task_id)
            .and_then(|task| task.restart().ok())
            .unwrap_or(false)
        {
            self.send_updates(task_id).await?;
            Ok(true)
        } else {
            Ok(false)
        }
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
        let Some(task): Option<GroupTask> = self.get_repo().get_task(task_id).await? else {
            return Err(Error::GeneralProtocolError(format!(
                "Couldn't find task with id {}",
                task_id
            )));
        };
        let mut remove = Vec::new();

        for device_id in task.get_devices().iter().map(|device| device.identifier()) {
            if let Some(tx) = self.subscribers.get(device_id) {
                let result = tx.try_send(Ok(format_task(task_id, &task, Some(device_id), None)));

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

    pub fn get_repo(&self) -> &Arc<Repository> {
        &self.repo
    }
}
