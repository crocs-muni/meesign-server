pub(crate) mod group;
pub(crate) mod sign_challenge;
pub(crate) mod sign_pdf;

use crate::device::Device;
use crate::group::Group;
use tonic::codegen::Arc;

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
}

impl TaskResult {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            TaskResult::GroupEstablished(group) => group.identifier(),
            TaskResult::Signed(data) => data,
        }
    }
}

pub trait Task {
    fn get_status(&self) -> TaskStatus;
    fn get_type(&self) -> crate::proto::TaskType;
    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>>;
    fn get_result(&self) -> Option<TaskResult>;
    fn get_decisions(&self) -> (u32, u32);
    /// Update protocol state with `data` from `device_id`
    ///
    /// # Returns
    /// `Ok(true)` if this update caused the next round to start; `Ok(false)` otherwise.
    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<bool, String>;

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
    fn get_devices(&self) -> Vec<Arc<Device>>;
    fn waiting_for(&self, device_id: &[u8]) -> bool;

    /// Store `decision` by `device_id`
    ///
    /// # Returns
    /// `true` if this decision caused the protocol to start; `false` otherwise.
    fn decide(&mut self, device_id: &[u8], decision: bool) -> bool;

    fn acknowledge(&mut self, device_id: &[u8]);
    fn device_acknowledged(&self, device_id: &[u8]) -> bool;
    fn get_request(&self) -> &[u8];

    fn get_attempts(&self) -> u32;
}
