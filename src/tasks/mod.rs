pub(crate) mod decrypt;
pub(crate) mod group;
pub(crate) mod sign;
pub(crate) mod sign_pdf;

use std::sync::Arc;

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::Participant;
use crate::proto::{KeyType, ProtocolType};

#[must_use = "updates must be persisted"]
pub enum RoundUpdate {
    Listen,
    GroupCertificatesSent,
    NextRound(u16),              // round number
    Finished(u16, FinishedTask), // round number, finished task
    Failed(FailedTask),
}

#[must_use = "updates must be persisted"]
pub enum DecisionUpdate {
    Undecided(VotingTask),
    Accepted(Box<dyn RunningTask>),
    Declined(DeclinedTask),
}

#[derive(Clone)]
pub enum TaskResult {
    GroupEstablished(Group),
    Signed(Vec<u8>),
    SignedPdf(Vec<u8>),
    Decrypted(Vec<u8>),
}

impl TaskResult {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            TaskResult::GroupEstablished(group) => group.identifier(),
            TaskResult::Signed(data) => data,
            TaskResult::SignedPdf(data) => data,
            TaskResult::Decrypted(data) => data,
        }
    }
}

pub struct VotingTask {
    pub task_info: TaskInfo,
    pub decisions: HashMap<Vec<u8>, i8>,
    pub accept_threshold: u32,
    pub request: Vec<u8>,
    pub running_task_context: RunningTaskContext,
}
impl VotingTask {
    pub async fn decide(mut self, device_id: &[u8], accept: bool) -> Result<DecisionUpdate, Error> {
        let shares = self
            .task_info
            .participants
            .iter()
            .find(|p| p.device.id == device_id)
            .ok_or(Error::GeneralProtocolError(
                "Invalid task participant id".into(),
            ))?
            .shares as i8;
        let vote = if accept { shares } else { -shares };

        // TODO: Check if this device has already decided
        self.decisions.insert(device_id.to_vec(), vote);

        let (accepts, rejects) = Self::accepts_rejects(&self.decisions);

        let decision_update = if accepts >= self.accept_threshold {
            let running_task = self
                .running_task_context
                .create_running_task(self.task_info, self.decisions)
                .await?;
            DecisionUpdate::Accepted(running_task)
        } else if rejects >= self.reject_threshold() {
            DecisionUpdate::Declined(DeclinedTask { accepts, rejects })
        } else {
            DecisionUpdate::Undecided(self)
        };
        Ok(decision_update)
    }
    pub fn accepts_rejects(decisions: &HashMap<Vec<u8>, i8>) -> (u32, u32) {
        let mut accepts = 0;
        let mut rejects = 0;
        for vote in decisions.values() {
            if *vote > 0 {
                accepts += vote.abs() as u32;
            }
            if *vote < 0 {
                rejects += vote.abs() as u32;
            }
        }
        (accepts, rejects)
    }
    pub fn reject_threshold(&self) -> u32 {
        self.task_info.total_shares() - self.accept_threshold + 1
    }
}
pub struct DeclinedTask {
    pub accepts: u32,
    pub rejects: u32,
}
pub struct FinishedTask {
    pub result: TaskResult,
    pub acknowledgements: HashSet<Vec<u8>>,
}
impl FinishedTask {
    pub fn new(result: TaskResult) -> Self {
        Self {
            result,
            acknowledgements: HashSet::new(),
        }
    }
    pub fn acknowledge(&mut self, device_id: &[u8]) {
        // TODO: Check if device_id is a participant
        self.acknowledgements.insert(device_id.to_vec());
    }
}
pub struct FailedTask {
    pub reason: String,
}

pub enum Task {
    Voting(VotingTask),
    Running(Box<dyn RunningTask + Send + Sync>),
    Declined(DeclinedTask),
    Finished(FinishedTask),
    Failed(FailedTask),
}

pub struct TaskInfo {
    pub id: Uuid,
    pub name: String,
    pub protocol_type: ProtocolType,
    pub key_type: KeyType,
    pub participants: Vec<Participant>,
}
impl TaskInfo {
    pub fn total_shares(&self) -> u32 {
        self.participants.iter().map(|p| p.shares).sum()
    }
}

#[async_trait]
pub trait RunningTask: Send + Sync {
    async fn get_work(&self, device_id: &[u8]) -> Vec<Vec<u8>>;

    fn get_round(&self) -> u16;

    async fn initialize(&mut self) -> Result<RoundUpdate, Error>;

    /// Update protocol state with `data` from `device_id`
    async fn update(&mut self, device_id: &[u8], data: &Vec<Vec<u8>>)
        -> Result<RoundUpdate, Error>;

    /// Attempt to restart protocol in task
    async fn restart(&mut self) -> Result<RoundUpdate, Error>;

    async fn waiting_for(&self, device_id: &[u8]) -> bool;

    fn get_attempts(&self) -> u32;
    fn get_communicator(&self) -> Arc<RwLock<Communicator>>;
}

pub enum RunningTaskContext {
    Group {
        threshold: u32,
        note: Option<String>,
    },
    SignChallenge {
        group: Group,
        data: Vec<u8>,
    },
    SignPdf {
        group: Group,
        data: Vec<u8>,
    },
    Decrypt {
        group: Group,
        data: Vec<u8>,
    },
}
impl RunningTaskContext {
    async fn create_running_task(
        self,
        task_info: TaskInfo,
        decisions: HashMap<Vec<u8>, i8>,
    ) -> Result<Box<dyn RunningTask>, Error> {
        let task: Box<dyn RunningTask> = match self {
            Self::Group { threshold, note } => Box::new(group::GroupTask::try_new(
                task_info, threshold, note, decisions,
            )?),
            Self::SignChallenge { group, data } => {
                Box::new(sign::SignTask::try_new(task_info, group, data, decisions)?)
            }
            Self::SignPdf { group, data } => Box::new(sign_pdf::SignPDFTask::try_new(
                task_info, group, data, decisions,
            )?),
            Self::Decrypt { group, data } => Box::new(decrypt::DecryptTask::try_new(
                task_info, group, data, decisions,
            )?),
        };
        Ok(task)
    }
}
