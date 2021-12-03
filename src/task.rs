pub enum TaskStatus {
    Waiting(Vec<Vec<u8>>, Vec<u8>),
    Finished(Vec<u8>)
}

pub trait Task {
    fn get_status(&self) -> TaskStatus;
    fn update(&mut self, device_id: &[u8], data: &[u8]) -> Result<TaskStatus, String>;
    fn waiting_for(&self, device_id: &[u8]) -> bool;
}
