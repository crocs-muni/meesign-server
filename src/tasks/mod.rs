pub(crate) mod decrypt;
pub(crate) mod group;
pub(crate) mod sign;
pub(crate) mod sign_pdf;

use std::sync::Arc;

use async_trait::async_trait;
use std::collections::HashSet;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::Participant;

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
    Undecided,
    Accepted(RoundUpdate),
    Declined(DeclinedTask),
}

#[must_use = "updates must be persisted"]
pub enum RestartUpdate {
    Voting,
    Started(RoundUpdate),
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
        self.acknowledgements.insert(device_id.to_vec());
    }
}
pub struct FailedTask {
    pub reason: String,
}

pub enum TaskPhase {
    Active(Box<dyn ActiveTask + Send + Sync>),
    Declined(DeclinedTask),
    Finished(FinishedTask),
    Failed(FailedTask),
}

pub struct Task {
    pub phase: TaskPhase,
}

#[async_trait]
pub trait ActiveTask: Send + Sync {
    fn get_type(&self) -> crate::proto::TaskType;
    async fn get_work(&self, device_id: &[u8]) -> Vec<Vec<u8>>;
    fn get_round(&self) -> u16;
    async fn get_decisions(&self) -> (u32, u32);
    /// Update protocol state with `data` from `device_id`
    async fn update(&mut self, device_id: &[u8], data: &Vec<Vec<u8>>)
        -> Result<RoundUpdate, Error>;

    /// Attempt to restart protocol in task
    async fn restart(&mut self) -> Result<RestartUpdate, Error>;

    /// True if the task has been approved
    async fn is_approved(&self) -> bool;

    fn get_participants(&self) -> &Vec<Participant>;
    async fn waiting_for(&self, device_id: &[u8]) -> bool;

    /// Store `decision` by `device_id`
    async fn decide(&mut self, device_id: &[u8], decision: bool) -> Result<DecisionUpdate, Error>;

    fn get_request(&self) -> &[u8];

    fn get_attempts(&self) -> u32;
    fn get_id(&self) -> &Uuid;
    fn get_communicator(&self) -> Arc<RwLock<Communicator>>;
    fn get_threshold(&self) -> u32;
    fn get_data(&self) -> Option<&[u8]>;
}
