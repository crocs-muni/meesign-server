use crate::group::Group;

#[derive(Clone, PartialEq)]
pub enum TaskStatus {
    Created,
    Running(u16), // round
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

pub enum TaskType {
    Group,
    Sign,
}


pub trait Task {
    fn get_status(&self) -> TaskStatus;
    fn get_type(&self) -> TaskType;
    fn get_work(&self, device_id: Option<&[u8]>) -> Option<Vec<u8>>;
    fn get_result(&self) -> Option<TaskResult>;
    fn get_confirmations(&self) -> (usize, usize);
    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<(), String>;
    fn has_device(&self, device_id: &[u8]) -> bool;
    fn waiting_for(&self, device_id: &[u8]) -> bool;
    fn confirmation(&mut self, device_id: &[u8], agreement: bool);
}
