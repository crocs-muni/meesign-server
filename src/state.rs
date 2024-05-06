use dashmap::DashMap;
use futures::future;
use log::{debug, error, warn};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::interfaces::grpc::format_task;
use crate::persistence::{Group, NameValidator, Repository, Task as TaskModel};
use crate::proto::{KeyType, ProtocolType};
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
    // tasks: HashMap<Uuid, Box<dyn Task + Send + Sync>>,
    subscribers: DashMap<Vec<u8>, Sender<Result<crate::proto::Task, Status>>>,
    repo: Arc<Repository>,
    communicators: DashMap<Uuid, Arc<RwLock<Communicator>>>,
}

impl State {
    pub fn new(repo: Arc<Repository>) -> Self {
        State {
            // tasks: HashMap::new(),
            subscribers: DashMap::new(),
            repo,
            communicators: DashMap::default(),
        }
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
        let devices = self.get_repo().get_devices_with_ids(device_ids).await?;
        let task = Box::new(GroupTask::try_new(
            name,
            devices,
            threshold,
            protocol,
            key_type,
            note,
            self.repo.clone(),
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
            return Err(Error::GeneralProtocolError("Invalid group_id".into()));
        }
        let group = group.unwrap();
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
            return Err(Error::GeneralProtocolError("Invalid group_id".into()));
        }
        let group = group.unwrap();
        let task = match group.key_type() {
            KeyType::Decrypt => {
                let task = DecryptTask::new(
                    group.clone(),
                    name.to_string(),
                    data.to_vec(),
                    data_type.to_string(),
                );
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
        let created_task = match task.get_type() {
            crate::proto::TaskType::Group => {
                let task_devices = task.get_devices();
                let device_ids: Vec<&[u8]> = task_devices
                    .iter()
                    .map(|device| device.id.as_slice())
                    .collect();
                self.get_repo()
                    .create_group_task(
                        Some(task.get_id()),
                        &device_ids,
                        2, // TODO
                        protocol_type.into(),
                        key_type.into(),
                        task.get_request(),
                        None, // TODO: missing note
                    )
                    .await?
            }
            _ => {
                todo!()
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

    pub async fn get_active_device_tasks(
        &self,
        device: &[u8],
    ) -> Result<Vec<Box<dyn Task>>, Error> {
        let task_models = self.get_repo().get_active_device_tasks(device).await?;
        let tasks = self.tasks_from_task_models(task_models).await?;
        let mut filtered_tasks = Vec::new();
        // TODO: can we simplify the condition so that we can filter it in DB? Maybe store task acknowledgements in task_participant
        for task in tasks.into_iter() {
            if task.has_device(device)
                && (task.get_status() != TaskStatus::Finished
                    || (task.get_status() == TaskStatus::Finished
                        && !task.device_acknowledged(device).await))
            {
                filtered_tasks.push(task as Box<dyn Task>);
            }
        }
        Ok(filtered_tasks)
    }

    pub async fn get_device_groups(&self, device: &[u8]) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_device_groups(device).await?)
    }

    pub async fn get_groups(&self) -> Result<Vec<Group>, Error> {
        Ok(self.get_repo().get_groups().await?)
    }

    pub async fn get_tasks(&self) -> Result<Vec<Box<dyn Task + Send + Sync>>, Error> {
        let task_models = self.get_repo().get_tasks().await?;
        self.tasks_from_task_models(task_models).await
    }

    pub async fn get_task(&self, task_id: &Uuid) -> Result<Box<dyn Task>, Error> {
        let Some(communicator) = self.communicators.get(task_id) else {
            return Err(Error::GeneralProtocolError("Invalid task id".into()));
        };
        let Some(task) = self
            .get_repo()
            .get_task(task_id, communicator.clone(), self.repo.clone())
            .await?
        else {
            return Err(Error::GeneralProtocolError("Invalid task id".into()));
        };

        Ok(task)
    }

    pub async fn update_task(
        &mut self,
        task_id: &Uuid,
        device: &[u8],
        data: &Vec<Vec<u8>>,
        attempt: u32,
    ) -> Result<bool, Error> {
        let mut task = self.get_task(task_id).await?;
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
        let update_result = task.update(device, data, self.repo.clone()).await;
        if previous_status != TaskStatus::Finished && task.get_status() == TaskStatus::Finished {
            // TODO join if statements once #![feature(let_chains)] gets stabilized
            if let TaskResult::GroupEstablished(group) = task.get_result().unwrap() {
                self.get_repo()
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
        let repo = self.repo.clone();
        let Some(communicator) = self.communicators.get(task_id) else {
            return Err(Error::GeneralProtocolError("Invalid task id".into()));
        };
        let Some(mut task) = self
            .get_repo()
            .get_task(task_id, communicator.clone(), self.repo.clone())
            .await?
        else {
            return Err(Error::GeneralProtocolError("Invalid task id".into()));
        };
        drop(communicator);
        let change = task.decide(device, decision, repo).await;
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

    pub async fn acknowledge_task(&mut self, task_id: &Uuid, device: &[u8]) {
        let mut task = self.get_task(task_id).await.unwrap();
        task.acknowledge(device).await;
    }

    pub async fn restart_task(&mut self, task_id: &Uuid) -> Result<bool, Error> {
        let mut task = self.get_task(task_id).await?;

        if task.restart(self.repo.clone()).await? {
            self.send_updates(task_id).await?;
            Ok(true)
        } else {
            Ok(false) // TODO: can we change this to Err? How will clients handle the change?
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
        let communicator = self.communicators.get(task_id).unwrap(); // TODO
        let Some(task) = self
            .get_repo()
            .get_task(task_id, communicator.clone(), self.repo.clone())
            .await?
        else {
            return Err(Error::GeneralProtocolError(format!(
                "Couldn't find task with id {}",
                task_id
            )));
        };
        let mut remove = Vec::new();

        for device_id in task.get_devices().iter().map(|device| device.identifier()) {
            if let Some(tx) = self.subscribers.get(device_id) {
                let result =
                    tx.try_send(Ok(format_task(task_id, &*task, Some(device_id), None).await));

                if result.is_err() {
                    debug!(
                        "Closed channel detected device_id={}…",
                        utils::hextrunc(&device_id[..4])
                    );
                    remove.push(device_id.to_vec());
                }
            }
        }

        drop(communicator); // so we can have a mutable borrow of self
        for device_id in remove {
            self.remove_subscriber(&device_id);
        }

        Ok(())
    }

    pub fn get_repo(&self) -> &Arc<Repository> {
        &self.repo
    }

    async fn tasks_from_task_models(
        &self,
        task_models: Vec<TaskModel>,
    ) -> Result<Vec<Box<dyn Task + Send + Sync>>, Error> {
        // TODO: do the grouping in DB
        let tasks = future::join_all(task_models.into_iter().map(|task| async {
            let devices = self.get_repo().get_task_devices(&task.id).await?;
            // TODO: for other task types as well
            let communicator = self.communicators.entry(task.id).or_insert_with(|| {
                // TODO: decide what to do when the server has restarted and the task communicator is not present
                Arc::new(RwLock::new(Communicator::new(
                    devices.clone(),
                    task.threshold as u32,
                    task.protocol_type.unwrap().into(),
                )))
            });

            let task =
                GroupTask::from_model(task, devices, communicator.clone(), self.repo.clone())
                    .await?;
            Ok(Box::new(task) as Box<dyn Task + Send + Sync>)
        }))
        .await
        .into_iter()
        .collect();
        tasks
    }
}
