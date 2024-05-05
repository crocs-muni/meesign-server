use chrono::{DateTime, Local};
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
    id: Option<&Uuid>,
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
        id,
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
    id: Option<&Uuid>,
    devices: &[&[u8]],
    threshold: u32,
    key_type: KeyType,
    protocol_type: ProtocolType,
    request: &[u8],
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
        id,
        protocol_round: 0,
        attempt_count: 0,
        error_message: None,
        threshold,
        last_update: None,
        task_data: None,
        preprocessed: None,
        request: Some(request),
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

pub async fn get_tasks<Conn>(connection: &mut Conn) -> Result<Vec<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks = task::table.load(connection).await?;
    Ok(tasks)
}

pub async fn set_task_result<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
    result: Result<Vec<u8>, String>,
) -> Result<(), PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    todo!()
}

pub async fn get_device_tasks<Conn>(
    connection: &mut Conn,
    identifier: &[u8],
) -> Result<Vec<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks = task::table
        .inner_join(task_participant::table)
        .filter(
            // not(task::task_state.eq(TaskState::Finished))
            task_participant::device_id.eq(identifier),
        )
        .select(Task::as_select())
        .load(connection)
        .await?;
    Ok(tasks)
}

pub async fn increment_round<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<u32, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let new_round: i32 = diesel::update(task::table.filter(task::id.eq(task_id)))
        .set(task::protocol_round.eq(task::protocol_round + 1))
        .returning(task::protocol_round)
        .get_result(connection)
        .await?;
    Ok(new_round as u32)
}

pub async fn set_task_last_update<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<DateTime<Local>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let now = Local::now();
    let updated_time = diesel::update(task::table.filter(task::id.eq(task_id)))
        .set(task::last_update.eq(now))
        .returning(task::last_update)
        .get_result(connection)
        .await?;
    assert_eq!(now, updated_time);
    Ok(updated_time)
}
