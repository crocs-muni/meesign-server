pub(crate) mod decrypt;
pub(crate) mod group;
pub(crate) mod sign;
pub(crate) mod sign_pdf;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::communicator::Communicator;
use crate::error::Error;
use crate::group::Group;
use crate::persistence::Device;
use crate::persistence::Repository;
use crate::persistence::Task as TaskModel;

#[derive(Clone, PartialEq, Eq)]
pub enum TaskStatus {
    Created,
    Running(u16),
    // round
    Finished,
    Failed(String),
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

#[async_trait]
pub trait Task: Send + Sync {
    fn get_status(&self) -> TaskStatus;
    fn get_type(&self) -> crate::proto::TaskType;
    async fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<Vec<u8>>>;
    fn get_result(&self) -> Option<TaskResult>;
    async fn get_decisions(&self) -> (u32, u32);
    /// Update protocol state with `data` from `device_id`
    ///
    /// # Returns
    /// `Ok(true)` if this update caused the next round to start; `Ok(false)` otherwise.
    async fn update(
        &mut self,
        device_id: &[u8],
        data: &Vec<Vec<u8>>,
        repository: Arc<Repository>,
    ) -> Result<bool, Error>;

    /// Attempt to restart protocol in task
    ///
    /// # Returns
    /// Ok(true) if task restarted successfully; Ok(false) otherwise.
    async fn restart(&mut self, repository: Arc<Repository>) -> Result<bool, String>;

    /// Get timestamp of the most recent task update
    fn last_update(&self) -> u64;

    /// True if the task has been approved
    async fn is_approved(&self) -> bool;

    fn has_device(&self, device_id: &[u8]) -> bool;
    fn get_devices(&self) -> &Vec<Device>;
    async fn waiting_for(&self, device_id: &[u8]) -> bool;

    /// Store `decision` by `device_id`
    ///
    /// # Returns
    /// `Some(true)` if this decision caused the protocol to start;
    /// `Some(false)` if this decision caused the protocol to fail;
    /// `None` otherwise.
    async fn decide(
        &mut self,
        device_id: &[u8],
        decision: bool,
        repository: Arc<Repository>,
    ) -> Option<bool>;

    async fn acknowledge(&mut self, device_id: &[u8]);
    async fn device_acknowledged(&self, device_id: &[u8]) -> bool;
    fn get_request(&self) -> &[u8];

    fn get_attempts(&self) -> u32;
    async fn from_model(
        model: TaskModel,
        devices: Vec<Device>,
        communicator: Arc<RwLock<Communicator>>,
        repository: Arc<Repository>,
    ) -> Result<Self, Error>
    where
        Self: Sized;
    fn get_id(&self) -> &Uuid;
    fn get_communicator(&self) -> Arc<RwLock<Communicator>>;
    fn get_threshold(&self) -> u32;
}
