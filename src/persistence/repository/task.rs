use chrono::{DateTime, Local};
use diesel::result::Error::NotFound;
use diesel::ExpressionMethods;
use diesel::{pg::Pg, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use super::utils::NameValidator;
use crate::persistence::models::{NewTaskResult, Task, TaskResult};
use crate::persistence::schema::{task_participant, task_result};
use crate::persistence::{
    enums::{KeyType, ProtocolType, TaskState, TaskType},
    error::PersistenceError,
    models::{NewTask, NewTaskParticipant, PartialTask},
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

    let task: PartialTask = diesel::insert_into(task::table)
        .values(task)
        .returning(PartialTask::as_returning())
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
    Task::try_from(task, None)
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

    let task: PartialTask = diesel::insert_into(task::table)
        .values(task)
        .returning(PartialTask::as_returning())
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
    Task::try_from(task, None)
}

pub async fn get_task<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<Option<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let retrieved_task: Option<(PartialTask, Option<TaskResult>)> = match task::table
        .left_outer_join(task_result::table)
        .filter(task::id.eq(task_id))
        .select((PartialTask::as_select(), Option::<TaskResult>::as_select()))
        .first::<(PartialTask, Option<TaskResult>)>(connection)
        .await
    {
        Ok(val) => Some(val),
        Err(NotFound) => None,
        Err(err) => return Err(PersistenceError::ExecutionError(err)),
    };
    if let Some((task, result)) = retrieved_task {
        Ok(Some(Task::try_from(task, result)?))
    } else {
        Ok(None)
    }
}

pub async fn get_tasks<Conn>(connection: &mut Conn) -> Result<Vec<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let partial_tasks = task::table
        .left_outer_join(task_result::table)
        .select((PartialTask::as_select(), Option::<TaskResult>::as_select()))
        .load::<(PartialTask, Option<TaskResult>)>(connection)
        .await?;
    from_partial_tasks(partial_tasks)
}

pub async fn get_device_tasks<Conn>(
    connection: &mut Conn,
    identifier: &[u8],
) -> Result<Vec<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let partial_tasks = task::table
        .left_outer_join(task_result::table)
        .inner_join(task_participant::table)
        .filter(task_participant::device_id.eq(identifier))
        .select((PartialTask::as_select(), Option::<TaskResult>::as_select()))
        .load::<(PartialTask, Option<TaskResult>)>(connection)
        .await?;
    from_partial_tasks(partial_tasks)
}

pub async fn set_task_result<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
    result: &Result<Vec<u8>, String>,
) -> Result<(), PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let task_result = NewTaskResult::from(result, task_id);
    diesel::insert_into(task_result::table)
        .values(&task_result)
        .on_conflict(task_result::task_id) // All devices happen to set the result
        .do_update()
        .set(&task_result)
        .execute(connection)
        .await?;
    Ok(())
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

pub async fn set_round<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
    round: u16,
) -> Result<u16, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let new_round: i32 = diesel::update(task::table.filter(task::id.eq(task_id)))
        .set(task::protocol_round.eq(round as i32))
        .returning(task::protocol_round)
        .get_result(connection)
        .await?;
    Ok(new_round as u16)
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
    Ok(updated_time)
}

pub async fn increment_task_attempt_count<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<u32, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let new_count: i32 = diesel::update(task::table.filter(task::id.eq(task_id)))
        .set(task::attempt_count.eq(task::attempt_count + 1))
        .returning(task::attempt_count)
        .get_result(connection)
        .await?;
    Ok(new_count as u32)
}

fn from_partial_tasks(
    partial_tasks: Vec<(PartialTask, Option<TaskResult>)>,
) -> Result<Vec<Task>, PersistenceError> {
    partial_tasks
        .into_iter()
        .map(|(partial_task, result)| Task::try_from(partial_task, result))
        .collect::<Vec<Result<Task, PersistenceError>>>()
        .into_iter()
        .collect()
}
