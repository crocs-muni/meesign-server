use chrono::{DateTime, Local};
use diesel::result::Error::NotFound;
use diesel::ExpressionMethods;
use diesel::NullableExpressionMethods;
use diesel::{pg::Pg, QueryDsl};
use diesel_async::{AsyncConnection, RunQueryDsl};
use std::collections::HashMap;
use uuid::Uuid;

use super::utils::NameValidator;
use crate::persistence::models::{NewTaskResult, Task};
use crate::persistence::schema::{task_participant, task_result};
use crate::persistence::{
    enums::{KeyType, ProtocolType, TaskState, TaskType},
    error::PersistenceError,
    models::{NewTask, NewTaskParticipant},
    schema::task,
};

pub async fn create_task<Conn>(
    connection: &mut Conn,
    id: Option<&Uuid>,
    group_id: &[u8],
    task_type: TaskType,
    name: &str,
    devices: &[&[u8]],
    task_data: Option<&[u8]>,
    request: &[u8],
    threshold: u32,
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

    let task = NewTask {
        id,
        protocol_round: 0,
        attempt_count: 0,
        error_message: None,
        threshold: threshold as i32,
        task_data,
        preprocessed: None,
        request: Some(request),
        task_type,
        group_id: Some(group_id),
        key_type,
        task_state: TaskState::Created,
        protocol_type,
        note: None,
        group_certificates_sent: None,
    };

    let task_id = diesel::insert_into(task::table)
        .values(task)
        .returning(task::id)
        .get_result(connection)
        .await?;

    let new_task_participants: Vec<NewTaskParticipant> = devices
        .into_iter()
        .map(|device_id| NewTaskParticipant {
            device_id,
            task_id: &task_id,
            decision: None,
            acknowledgment: None,
        })
        .collect();

    diesel::insert_into(task_participant::table)
        .values(new_task_participants)
        .execute(connection)
        .await?;

    get_task(connection, &task_id)
        .await?
        .ok_or(PersistenceError::DataInconsistencyError(
            "Already created task can't be returned".into(),
        ))
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
    note: Option<&str>,
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
        group_id: None,
        protocol_round: 0,
        attempt_count: 0,
        error_message: None,
        threshold,
        task_data: None,
        preprocessed: None,
        request: Some(request),
        task_type: TaskType::Group,
        key_type: Some(key_type),
        task_state: TaskState::Created,
        protocol_type: Some(protocol_type),
        note,
        group_certificates_sent: Some(false),
    };

    let task_id: Uuid = diesel::insert_into(task::table)
        .values(task)
        .returning(task::id)
        .get_result(connection)
        .await?;

    let new_task_participants: Vec<NewTaskParticipant> = devices
        .into_iter()
        .map(|device_id| NewTaskParticipant {
            device_id,
            task_id: &task_id,
            decision: None,
            acknowledgment: None,
        })
        .collect();

    diesel::insert_into(task_participant::table)
        .values(new_task_participants)
        .execute(connection)
        .await?;

    get_task(connection, &task_id)
        .await?
        .ok_or(PersistenceError::DataInconsistencyError(
            "Already created task can't be returned".into(),
        ))
}

macro_rules! task_model_columns {
    () => {
        (
            task::id,
            task::protocol_round,
            task::attempt_count,
            task::error_message,
            task::threshold,
            task::last_update,
            task::task_data,
            task::preprocessed,
            task::request,
            task::group_id,
            task::task_type,
            task::task_state,
            task::key_type,
            task::protocol_type,
            task::note,
            task::group_certificates_sent,
            task_result::all_columns.nullable(),
        )
    };
}

pub async fn get_task<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<Option<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let task: Option<Task> = match task::table
        .left_outer_join(task_result::table)
        .filter(task::id.eq(task_id))
        .select(task_model_columns!())
        .first(connection)
        .await
    {
        Ok(val) => Some(val),
        Err(NotFound) => None,
        Err(err) => return Err(PersistenceError::ExecutionError(err)),
    };
    Ok(task)
}

pub async fn get_tasks<Conn>(connection: &mut Conn) -> Result<Vec<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks = task::table
        .left_outer_join(task_result::table)
        .select(task_model_columns!())
        .order_by(task::id.asc())
        .load(connection)
        .await?;
    Ok(tasks)
}

pub async fn get_device_tasks<Conn>(
    connection: &mut Conn,
    identifier: &[u8],
) -> Result<Vec<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks = task::table
        .left_outer_join(task_result::table)
        .inner_join(task_participant::table)
        .filter(task_participant::device_id.eq(identifier))
        .select(task_model_columns!())
        .order_by(task::id.asc())
        .distinct_on(task::id) // NOTE: Because of multiple shares, participants can be duplicated
        .load(connection)
        .await?;
    Ok(tasks)
}

pub async fn set_task_decision<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
    device_id: &[u8],
    accept: bool,
) -> Result<(), PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    diesel::update(
        task_participant::table
            .filter(task_participant::task_id.eq(task_id))
            .filter(task_participant::device_id.eq(device_id)),
    )
    .set(task_participant::decision.eq(Some(accept)))
    .execute(connection)
    .await?;
    Ok(())
}

pub async fn get_task_decisions<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<HashMap<Vec<u8>, i8>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let mut decisions = HashMap::new();
    let pairs = task_participant::table
        .select((task_participant::device_id, task_participant::decision))
        .filter(task_participant::task_id.eq(task_id))
        .load::<(Vec<u8>, Option<bool>)>(connection)
        .await?;
    for (device_id, vote) in pairs {
        *decisions.entry(device_id).or_default() += match vote {
            Some(true) => 1,
            Some(false) => -1,
            None => 0,
        };
    }
    Ok(decisions)
}

pub async fn set_task_acknowledgement<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
    device_id: &[u8],
) -> Result<(), PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    diesel::update(
        task_participant::table
            .filter(task_participant::task_id.eq(task_id))
            .filter(task_participant::device_id.eq(device_id)),
    )
    .set(task_participant::acknowledgment.eq(Some(true)))
    .execute(connection)
    .await?;
    Ok(())
}

pub async fn get_task_acknowledgements<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<HashMap<Vec<u8>, bool>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let acknowledgements = task_participant::table
        .select((
            task_participant::device_id,
            task_participant::acknowledgment,
        ))
        .filter(task_participant::task_id.eq(task_id))
        .load::<(Vec<u8>, Option<bool>)>(connection)
        .await?
        .into_iter()
        .map(|(device_id, acknowledgement)| (device_id, acknowledgement.unwrap_or(false)))
        .collect();
    Ok(acknowledgements)
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

pub async fn set_task_group_certificates_sent<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
    group_certificates_sent: Option<bool>,
) -> Result<(), PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    diesel::update(task::table.filter(task::id.eq(task_id)))
        .set(task::group_certificates_sent.eq(group_certificates_sent))
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
