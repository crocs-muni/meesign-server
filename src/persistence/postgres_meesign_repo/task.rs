use std::convert::TryInto;

use diesel::pg::Pg;
use diesel_async::AsyncConnection;

use super::utils::NameValidator;
use crate::persistence::{
    enums::{KeyType, ProtocolType, TaskState, TaskType},
    error::PersistenceError,
    models::{NewTask, Task},
};

pub async fn create_task<Conn>(
    connection: &mut Conn,
    task_type: TaskType,
    name: &str,
    data: Option<&Vec<u8>>,
    devices: &[Vec<u8>],
    threshold: Option<u32>,
    key_type: Option<KeyType>,
    protocol_type: Option<ProtocolType>,
) -> Result<Task, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    if !name.is_name_valid() {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Invalid group name {name}"
        )));
    }

    // let threshold: i32 = threshold.try_into()?;
    let task = NewTask {
        protocol_round: 0,
        attempt_count: 0,
        error_message: None,
        threshold: 0, // TODO: decide if optional or not
        last_update: None,
        task_data: None,
        preprocessed: None,
        request: None,
        task_type,
        key_type,
        task_state: TaskState::Created,
        protocol_type,
    };

    todo!()
}
