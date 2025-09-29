use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use tokio::sync::{OwnedRwLockWriteGuard, RwLock};
use uuid::Uuid;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::{Participant, PersistenceError, Repository, Task as TaskModel, TaskType};
use crate::tasks::{
    decrypt::DecryptTask, group::GroupTask, sign::SignTask, sign_pdf::SignPDFTask, DeclinedTask,
    FailedTask, FinishedTask, RunningTask, RunningTaskContext, Task, TaskInfo, TaskResult,
    VotingTask,
};

/// A lazily populated `Task` cache.
///
/// All `get*` methods first ensure the `Task` is cached,
/// then a reference to it is returned.
pub struct TaskStore {
    // NOTE: DashMap locking applies to its internal shards, we must protect Tasks across awaits using tokio's RwLock.
    task_cache: DashMap<Uuid, Arc<RwLock<Task>>>,
    repo: Arc<Repository>,
}

impl TaskStore {
    /// Creates an empty `TaskStore`.
    pub fn new(repo: Arc<Repository>) -> Self {
        Self {
            task_cache: DashMap::new(),
            repo,
        }
    }

    /// Caches the provided `task` and persists it into the DB.
    /// If a `Task` with the same `task_id` already exists in the cache, it is returned.
    pub async fn persist_task(&self, task: VotingTask) -> Result<Option<Task>, Error> {
        let participant_ids_shares: Vec<_> = task
            .task_info
            .participants
            .iter()
            .map(|participant| (participant.device.id.as_slice(), participant.shares))
            .collect();

        match &task.running_task_context {
            RunningTaskContext::Group { threshold, note } => {
                self.persist_group_task(
                    &task,
                    &participant_ids_shares,
                    *threshold,
                    note.as_deref(),
                )
                .await?;
            }
            RunningTaskContext::SignChallenge { group, data } => {
                self.persist_threshold_task(
                    &task,
                    TaskType::SignChallenge,
                    &participant_ids_shares,
                    group,
                    data,
                )
                .await?;
            }
            RunningTaskContext::SignPdf { group, data } => {
                self.persist_threshold_task(
                    &task,
                    TaskType::SignPdf,
                    &participant_ids_shares,
                    group,
                    data,
                )
                .await?;
            }
            RunningTaskContext::Decrypt { group, data, .. } => {
                self.persist_threshold_task(
                    &task,
                    TaskType::Decrypt,
                    &participant_ids_shares,
                    group,
                    data,
                )
                .await?;
            }
        }

        let evicted_task = self
            .task_cache
            .insert(
                task.task_info.id.clone(),
                Arc::new(RwLock::new(Task::Voting(task))),
            )
            .map(|evicted_task| Arc::into_inner(evicted_task).unwrap().into_inner());
        Ok(evicted_task)
    }

    /// Returns an iterator of mutable `Task` references.
    /// Returns an error if any of the provided `task_ids`
    /// does not reference an existing `Task`.
    pub async fn get_tasks_mut(
        &self,
        task_ids: Vec<Uuid>,
    ) -> Result<
        impl Iterator<Item = impl Future<Output = impl DerefMut<Target = Task>>> + use<'_>,
        Error,
    > {
        self.get_tasks_write_guards(task_ids).await
    }

    /// Returns an iterator of shared `Task` references.
    /// Returns an error if any of the provided `task_ids`
    /// does not reference an existing `Task`.
    pub async fn get_tasks(
        &self,
        task_ids: Vec<Uuid>,
    ) -> Result<
        impl Iterator<Item = impl Future<Output = impl Deref<Target = Task>>> + use<'_>,
        Error,
    > {
        let iterator = self
            .get_tasks_write_guards(task_ids)
            .await?
            .map(|write_guard| async { write_guard.await.downgrade() });
        Ok(iterator)
    }

    /// Returns a mutable reference to a `Task`.
    /// Returns an error if the provided `task_id`
    /// does not reference an existing `Task`.
    pub async fn get_task_mut(
        &self,
        task_id: &Uuid,
    ) -> Result<impl DerefMut<Target = Task>, Error> {
        let task = self
            .get_tasks_write_guards(vec![task_id.clone()])
            .await?
            .next()
            .unwrap()
            .await;
        Ok(task)
    }

    /// Returns a shared reference to a `Task`.
    /// Returns an error if the provided `task_id`
    /// does not reference an existing `Task`.
    pub async fn get_task(&self, task_id: &Uuid) -> Result<impl Deref<Target = Task>, Error> {
        let task = self
            .get_tasks_write_guards(vec![task_id.clone()])
            .await?
            .next()
            .unwrap()
            .await
            .downgrade();
        Ok(task)
    }

    async fn persist_group_task(
        &self,
        task: &VotingTask,
        participant_ids_shares: &[(&[u8], u32)],
        threshold: u32,
        note: Option<&str>,
    ) -> Result<(), Error> {
        self.repo
            .create_group_task(
                Some(&task.task_info.id),
                &participant_ids_shares,
                threshold,
                task.task_info.protocol_type.into(),
                task.task_info.key_type.into(),
                &task.request,
                note,
            )
            .await?;
        Ok(())
    }
    async fn persist_threshold_task(
        &self,
        task: &VotingTask,
        task_type: TaskType,
        participant_ids_shares: &[(&[u8], u32)],
        group: &Group,
        data: &[u8],
    ) -> Result<(), Error> {
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
        Ok(())
    }

    async fn ensure_cached_tasks(&self, task_ids: impl Iterator<Item = Uuid>) -> Result<(), Error> {
        let uncached_task_ids: Vec<_> = task_ids
            .filter(|task_id| !self.task_cache.contains_key(task_id))
            .collect();
        let tasks = self.hydrate_tasks(&uncached_task_ids).await?;
        for task in tasks {
            let task_id = task.task_info().id.clone();
            if let Entry::Vacant(entry) = self.task_cache.entry(task_id) {
                entry.insert(Arc::new(RwLock::new(task)));
            }
        }
        Ok(())
    }

    async fn get_tasks_write_guards(
        &self,
        task_ids: Vec<Uuid>,
    ) -> Result<
        impl Iterator<Item = impl Future<Output = OwnedRwLockWriteGuard<Task>>> + use<'_>,
        Error,
    > {
        let task_ids: HashSet<Uuid> = task_ids.into_iter().collect();
        self.ensure_cached_tasks(task_ids.iter().cloned()).await?;
        let iterator = self
            .task_cache
            .iter()
            .filter(move |kv| task_ids.contains(kv.key()))
            .map(|kv| kv.clone().write_owned());
        Ok(iterator)
    }

    async fn hydrate_tasks(&self, task_ids: &[Uuid]) -> Result<Vec<Task>, Error> {
        let task_models = self.repo.get_task_models(task_ids).await?;
        if task_models.len() != task_ids.len() {
            return Err(Error::GeneralProtocolError("Invalid task id(s)".into()));
        }
        let task_id_participant_pairs = self.repo.get_tasks_participants(task_ids).await?;

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
                    let group = crate::group::Group::from_model(group_model);
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
                    .hydrate_communicator(&task_model, task_info.participants.clone())
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
            let group = crate::group::Group::from_model(group_model);

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
                    .hydrate_communicator(&task_model, task_info.participants.clone())
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

    async fn hydrate_communicator(
        &self,
        task_model: &TaskModel,
        mut all_participants: Vec<Participant>,
    ) -> Result<Communicator, Error> {
        let threshold = match task_model.task_type {
            TaskType::Group => all_participants.iter().map(|p| p.shares).sum(),
            _ => task_model.threshold as u32,
        };

        let active_shares = self.repo.get_task_active_shares(&task_model.id).await?;

        all_participants.sort_by(|a, b| a.device.id.cmp(&b.device.id));
        let first_share_indices: HashMap<Vec<u8>, u32> = all_participants
            .iter()
            .scan(0, |idx, p| {
                let first_share = *idx;
                *idx += p.shares;
                Some((p.device.id.clone(), first_share))
            })
            .collect();
        let active_shares = all_participants
            .iter()
            .filter_map(|p| active_shares.get(&p.device.id).map(|shares| (p, shares)))
            .flat_map(|(p, shares)| std::iter::repeat_n(&p.device, *shares as usize))
            .scan(first_share_indices, |share_indices, device| {
                let share_index = share_indices[&device.id];
                *share_indices.get_mut(&device.id).unwrap() += 1;
                Some((share_index, device.clone()))
            })
            .collect();

        Ok(Communicator::new(
            threshold,
            task_model.protocol_type.into(),
            active_shares,
        ))
    }
}
