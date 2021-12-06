use crate::group::Group;

pub enum TaskStatus {
    Waiting(Vec<Vec<u8>>),
    GroupEstablished(Group),
    Signed(Vec<u8>),
    Failed(Vec<u8>),
}

pub trait Task {
    fn get_status(&self) -> TaskStatus;
    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<TaskStatus, String>;
    fn get_work(&self, device_id: &[u8]) -> Option<Vec<u8>>;

    fn waiting_for(&self, device_id: &[u8]) -> bool {
        match self.get_status() {
            TaskStatus::Waiting(devices) => devices.contains(&device_id.to_vec()),
            _ => false
        }
    }
}
