use diesel::result::Error::NotFound;
use diesel::ExpressionMethods;
use diesel::{pg::Pg, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use super::utils::NameValidator;
use crate::persistence::{
    enums::{KeyType, ProtocolType, TaskState, TaskType},
    error::PersistenceError,
    models::{GroupParticipant, NewGroupParticipant, NewTask, NewTaskParticipant, Task},
    schema::{groupparticipant, task, taskparticipant},
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

    // TODO: either use this function just for group tasks, or fetch group participants from the DB by default.
    let group_participants: Vec<NewGroupParticipant> = devices
        .into_iter()
        .map(|device_id| NewGroupParticipant {
            device_id,
            group_id: None,
        })
        .collect();

    let group_participants: Vec<GroupParticipant> = diesel::insert_into(groupparticipant::table)
        .values(group_participants)
        .returning(GroupParticipant::as_returning())
        .load(connection)
        .await?;

    let task: Task = diesel::insert_into(task::table)
        .values(task)
        .returning(Task::as_returning())
        .get_result(connection)
        .await?;

    let new_task_participants: Vec<NewTaskParticipant> = group_participants
        .into_iter()
        .map(|group_participant| NewTaskParticipant {
            group_participant_id: group_participant.id,
            task_id: &task.id,
            decision: None,
            acknowledgment: None,
        })
        .collect();

    diesel::insert_into(taskparticipant::table)
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
    use crate::persistence::schema::task::dsl::*;
    let retrieved_task: Option<Task> = match task.filter(id.eq(task_id)).first(connection).await {
        Ok(val) => Some(val),
        Err(NotFound) => None,
        Err(err) => return Err(PersistenceError::ExecutionError(err)),
    };
    Ok(retrieved_task)
}
