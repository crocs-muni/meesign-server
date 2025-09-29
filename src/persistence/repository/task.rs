use diesel::result::Error::NotFound;
use diesel::{pg::Pg, QueryDsl};
use diesel::{BoolExpressionMethods, ExpressionMethods, NullableExpressionMethods};
use diesel_async::{AsyncConnection, RunQueryDsl};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::utils::NameValidator;
use crate::persistence::models::{NewTaskResult, Task};
use crate::persistence::schema::{active_task_participant, task_participant, task_result};
use crate::persistence::{
    enums::{KeyType, ProtocolType, TaskState, TaskType},
    error::PersistenceError,
    models::{ActiveTaskParticipant, NewTask, NewTaskParticipant},
    schema::task,
};

pub async fn create_task<Conn>(
    connection: &mut Conn,
    id: Option<&Uuid>,
    group_id: &[u8],
    task_type: TaskType,
    name: &str,
    participants: &[(&[u8], u32)],
    task_data: Option<&[u8]>,
    request: &[u8],
    threshold: u32,
    key_type: KeyType,
    protocol_type: ProtocolType,
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
        threshold: threshold as i32,
        task_data,
        preprocessed: None,
        request,
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

    let new_task_participants: Vec<NewTaskParticipant> = participants
        .into_iter()
        .map(|(device_id, shares)| NewTaskParticipant {
            device_id,
            task_id: &task_id,
            shares: *shares as i32,
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
    participants: &[(&[u8], u32)],
    threshold: u32,
    key_type: KeyType,
    protocol_type: ProtocolType,
    request: &[u8],
    note: Option<&str>,
) -> Result<Task, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let total_shares: u32 = participants.iter().map(|(_, shares)| shares).sum();
    if !(1..=total_shares).contains(&threshold) {
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
        threshold,
        task_data: None,
        preprocessed: None,
        request,
        task_type: TaskType::Group,
        key_type,
        task_state: TaskState::Created,
        protocol_type,
        note,
        group_certificates_sent: Some(false),
    };

    let task_id: Uuid = diesel::insert_into(task::table)
        .values(task)
        .returning(task::id)
        .get_result(connection)
        .await?;

    let new_task_participants: Vec<NewTaskParticipant> = participants
        .into_iter()
        .map(|(device_id, shares)| NewTaskParticipant {
            device_id,
            task_id: &task_id,
            shares: *shares as i32,
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
            task::threshold,
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

async fn get_task<Conn>(
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

pub async fn get_tasks<Conn>(connection: &mut Conn) -> Result<Vec<Uuid>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks = task::table
        .select(task::id)
        .order_by(task::id.asc())
        .load(connection)
        .await?;
    Ok(tasks)
}

pub async fn get_task_models<Conn>(
    connection: &mut Conn,
    task_ids: &[Uuid],
) -> Result<Vec<Task>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks = task::table
        .left_outer_join(task_result::table)
        .select(task_model_columns!())
        .filter(task::id.eq_any(task_ids))
        .order_by(task::id.asc())
        .load(connection)
        .await?;
    Ok(tasks)
}

pub async fn get_active_device_tasks<Conn>(
    connection: &mut Conn,
    device_id: &[u8],
) -> Result<Vec<Uuid>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks = task::table
        .left_outer_join(task_result::table)
        .inner_join(task_participant::table)
        .filter(task_participant::device_id.eq(device_id))
        .filter(
            task_result::is_successful
                .is_null()
                .or(task_participant::acknowledgment
                    .is_null()
                    .or(task_participant::acknowledgment.eq(false))),
        )
        .select(task::id)
        .order_by(task::id.asc())
        .load(connection)
        .await?;
    Ok(tasks)
}

pub async fn get_restart_candidates<Conn>(
    connection: &mut Conn,
) -> Result<Vec<Uuid>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks = task::table
        .left_outer_join(task_result::table)
        .filter(task_result::is_successful.is_null())
        .filter(
            task::group_certificates_sent
                .eq(Some(true))
                .or(task::protocol_round.ge(1)),
        )
        .select(task::id)
        .order_by(task::id.asc())
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
    let decisions = task_participant::table
        .select((
            task_participant::device_id,
            task_participant::decision,
            task_participant::shares,
        ))
        .filter(task_participant::task_id.eq(task_id))
        .load::<(Vec<u8>, Option<bool>, i32)>(connection)
        .await?
        .into_iter()
        .map(|(device_id, decision, shares)| {
            let vote = match decision {
                Some(true) => shares as i8,
                Some(false) => -shares as i8,
                None => 0,
            };
            (device_id, vote)
        })
        .collect();
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
) -> Result<HashSet<Vec<u8>>, PersistenceError>
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
        .filter_map(|(device_id, acknowledgement)| match acknowledgement {
            Some(true) => Some(device_id),
            _ => None,
        })
        .collect();
    Ok(acknowledgements)
}

pub async fn get_task_active_shares<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<HashMap<Vec<u8>, u32>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let active_shares = active_task_participant::table
        .select((
            active_task_participant::device_id,
            active_task_participant::active_shares,
        ))
        .filter(active_task_participant::task_id.eq(task_id))
        .load::<(Vec<u8>, i32)>(connection)
        .await?
        .into_iter()
        .map(|(device_id, active_shares)| (device_id, active_shares as u32))
        .collect();
    Ok(active_shares)
}

pub async fn set_task_active_shares<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
    active_shares: &HashMap<Vec<u8>, u32>,
) -> Result<(), PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let active_task_participants: Vec<_> = active_shares
        .iter()
        .map(|(device_id, shares)| ActiveTaskParticipant {
            task_id: task_id.clone(),
            device_id: device_id.clone(),
            active_shares: *shares as i32,
        })
        .collect();
    diesel::insert_into(active_task_participant::table)
        .values(&active_task_participants)
        .on_conflict((
            active_task_participant::task_id,
            active_task_participant::device_id,
        ))
        .do_nothing()
        .execute(connection)
        .await?;
    Ok(())
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

pub async fn set_task_round<Conn>(
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
