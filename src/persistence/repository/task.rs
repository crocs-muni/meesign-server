use diesel::result::Error::NotFound;
use diesel::ExpressionMethods;
use diesel::{pg::Pg, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use super::utils::NameValidator;
use crate::persistence::schema::task_participant;
use crate::persistence::{
    enums::{KeyType, ProtocolType, TaskState, TaskType},
    error::PersistenceError,
    models::{NewTask, NewTaskParticipant, Task},
    schema::task,
};

pub async fn create_task<Conn>(
    connection: &mut Conn,
    task_type: TaskType,
    name: &str,
    data: Option<&Vec<u8>>,
    devices: &[&[u8]],
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
        threshold: 2, // TODO: decide if optional or not
        last_update: None,
        task_data: None,
        preprocessed: None,
        request: None,
        task_type,
        key_type,
        task_state: TaskState::Created,
        protocol_type,
    };

    let task: Task = diesel::insert_into(task::table)
        .values(task)
        .returning(Task::as_returning())
        .get_result(connection)
        .await?;

    let new_task_participants: Vec<NewTaskParticipant> = devices
        .into_iter()
        .map(|device_id| NewTaskParticipant {
            device_id,
            task_id: &task.id,
            decision: None,
            acknowledgment: None,
        })
        .collect();

    diesel::insert_into(task_participant::table)
        .values(new_task_participants)
        .execute(connection)
        .await?;
    Ok(task)
}

// TODO: join with create_task
pub async fn create_group_task<Conn>(
    connection: &mut Conn,
    devices: &[&[u8]],
    threshold: u32,
    key_type: KeyType,
    protocol_type: ProtocolType,
) -> Result<Task, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    if !(1..=devices.len() as u32).contains(&threshold) {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Invalid threshold {threshold}"
        )));
    }

    let threshold: i32 = threshold.try_into()?;
    let task = NewTask {
        protocol_round: 0,
        attempt_count: 0,
        error_message: None,
        threshold,
        last_update: None,
        task_data: None,
        preprocessed: None,
        request: None,
        task_type: TaskType::Group,
        key_type: Some(key_type),
        task_state: TaskState::Created,
        protocol_type: Some(protocol_type),
    };

    let task: Task = diesel::insert_into(task::table)
        .values(task)
        .returning(Task::as_returning())
        .get_result(connection)
        .await?;

    let new_task_participants: Vec<NewTaskParticipant> = devices
        .into_iter()
        .map(|device_id| NewTaskParticipant {
            device_id,
            task_id: &task.id,
            decision: None,
            acknowledgment: None,
        })
        .collect();

    diesel::insert_into(task_participant::table)
        .values(new_task_participants)
        .execute(connection)
        .await?;
    Ok(task)
}

pub async fn get_task<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<Option<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let retrieved_task: Option<Task> = match task::table
        .filter(task::id.eq(task_id))
        .first(connection)
        .await
    {
        Ok(val) => Some(val),
        Err(NotFound) => None,
        Err(err) => return Err(PersistenceError::ExecutionError(err)),
    };
    Ok(retrieved_task)
}
