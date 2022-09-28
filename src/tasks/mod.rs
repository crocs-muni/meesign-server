pub(crate) mod group;
pub(crate) mod sign_pdf;

use crate::group::Group;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, PartialEq)]
pub enum TaskStatus {
    Created,
    Running(u16),
    // round
    Finished,
    Failed(String),
}

#[derive(Clone, PartialEq)]
pub enum TaskResult {
    GroupEstablished(Group),
    Signed(Vec<u8>),
}

impl TaskResult {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            TaskResult::GroupEstablished(group) => group.identifier(),
            TaskResult::Signed(data) => data,
        }
    }
}

pub fn get_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub enum TaskType {
    Group,
    Sign,
}

pub trait Task {
    fn get_status(&self) -> TaskStatus;
    fn get_type(&self) -> TaskType;
    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>>;
    fn get_result(&self) -> Option<TaskResult>;
    fn get_decisions(&self) -> (u32, u32);
    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<(), String>;

    /// Attempt to restart protocol in task
    ///
    /// # Returns
    /// Ok(true) if task restarted successfully; Ok(false) otherwise.
    fn restart(&mut self) -> Result<bool, String>;

    /// Get timestamp of the most recent task update
    fn last_update(&self) -> u64;

    /// True if the task has been approved
    fn is_approved(&self) -> bool;

    fn has_device(&self, device_id: &[u8]) -> bool;
    fn waiting_for(&self, device_id: &[u8]) -> bool;
    fn decide(&mut self, device_id: &[u8], decision: bool);
    fn acknowledge(&mut self, device_id: &[u8]);
    fn device_acknowledged(&self, device_id: &[u8]) -> bool;
    fn get_request(&self) -> &[u8];
}
