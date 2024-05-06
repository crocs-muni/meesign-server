use std::convert::TryInto;

use diesel::{pg::Pg, result::Error::NotFound, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::persistence::models::NewGroupParticipant;
use crate::persistence::schema::device;
use crate::persistence::schema::group;
use crate::persistence::schema::group_participant;
use crate::persistence::schema::task;
use crate::persistence::schema::task_participant;
use crate::persistence::{
    enums::{KeyType, ProtocolType},
    error::PersistenceError,
    models::{Group, NewGroup},
};

pub async fn get_groups<Conn>(connection: &mut Conn) -> Result<Vec<Group>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    Ok(group::table.load(connection).await?)
}

pub async fn add_group<'a, Conn>(
    connection: &mut Conn,
    group_task_id: &Uuid,
    identifier: &[u8],
    name: &str,
    threshold: u32,
    protocol: ProtocolType,
    key_type: KeyType,
    certificate: Option<&[u8]>,
    note: Option<&'a str>,
) -> Result<Group, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let threshold: i32 = threshold.try_into()?;

    if identifier.is_empty() {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Empty identifier"
        )));
    }
    let new_group = NewGroup {
        identifier,
        threshold: threshold as i32,
        protocol,
        name,
        round: 0,
        certificate,
        key_type,
        note,
    };

    let group = diesel::insert_into(group::table)
        .values(new_group)
        .returning(Group::as_returning())
        .get_result(connection)
        .await?;

    let device_ids: Vec<Vec<u8>> = task_participant::table
        .inner_join(task::table)
        .filter(task::id.eq(group_task_id))
        .inner_join(device::table)
        .select(device::id)
        .load(connection)
        .await?;
    let group_participants: Vec<NewGroupParticipant> = device_ids
        .iter()
        .map(|device_id| NewGroupParticipant {
            device_id,
            group_id: group.id,
        })
        .collect();
    let expected_affected_row_count = group_participants.len();
    let affected_row_count = diesel::insert_into(group_participant::table)
        .values(group_participants)
        .execute(connection)
        .await?;
    // TODO: consider propagating the check into the prod build: assert -> Err(...)
    assert_eq!(affected_row_count, expected_affected_row_count);
    Ok(group)
}

pub async fn get_group<Conn>(
    connection: &mut Conn,
    group_identifier: &[u8],
) -> Result<Option<Group>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let group: Option<Group> = match group::table
        .filter(group::identifier.eq(group_identifier))
        .first(connection)
        // .optional() TODO
        .await
    {
        Ok(val) => Some(val),
        Err(NotFound) => None,
        Err(err) => return Err(PersistenceError::ExecutionError(err)),
    };

    Ok(group)
}

pub async fn get_device_groups<Conn>(
    connection: &mut Conn,
    device_identifier: &[u8],
) -> Result<Vec<Group>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let groups: Vec<Group> = group_participant::table
        .inner_join(group::table)
        .filter(group_participant::device_id.eq(device_identifier))
        .select(Group::as_select())
        .load(connection)
        .await?;

    Ok(groups)
}

#[cfg(test)]
mod test {
    use diesel_async::AsyncPgConnection;

    use crate::persistence::{
        repository::{device::add_device, task::create_group_task},
        tests::persistency_unit_test_context::PersistencyUnitTestContext,
        DeviceKind,
    };

    use super::*;

    const GROUP_1_IDENTIFIER: [u8; 32] = [1; 32];
    const GROUP_2_IDENTIFIER: [u8; 32] = [2; 32];
    const GROUP_1_NAME: &str = "Group 1";
    const GROUP_2_NAME: &str = "Group 2";
    const GROUP_1_CERT: [u8; 150] = [11; 150];
    const GROUP_2_CERT: [u8; 150] = [22; 150];
    const DEVICE_1_ID: [u8; 32] = [1; 32];
    const DEVICE_2_ID: [u8; 32] = [2; 32];
    const DEVICE_3_ID: [u8; 32] = [3; 32];

    #[tokio::test]
    async fn given_valid_arguments_create_group() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new();
        let mut connection: AsyncPgConnection = ctx.get_test_connection().await?;

        let devices = &vec![&DEVICE_1_ID[..], &DEVICE_2_ID[..]];
        let threshold = 2;
        add_device(
            &mut connection,
            &DEVICE_1_ID,
            "Device 1",
            &DeviceKind::User,
            &vec![42; 5],
        )
        .await?;
        add_device(
            &mut connection,
            &DEVICE_2_ID,
            "Device 2",
            &DeviceKind::Bot,
            &vec![42; 5],
        )
        .await?;

        let note = Some("time policy".into());
        let group_creation_task = create_group_task(
            &mut connection,
            None,
            devices,
            threshold,
            KeyType::SignPdf,
            ProtocolType::Gg18,
            &[],
            note.as_deref(),
        )
        .await?;

        let created_group = add_group(
            &mut connection,
            &group_creation_task.id,
            &GROUP_1_IDENTIFIER,
            GROUP_1_NAME,
            threshold,
            ProtocolType::Gg18,
            KeyType::SignPdf,
            Some(&GROUP_1_CERT),
            note.as_deref(),
        )
        .await?;
        let retrieved_group = get_group(&mut connection, &GROUP_1_IDENTIFIER).await?;
        assert!(
            retrieved_group.is_some(),
            "Created group couldn't be retrieved"
        );
        let retrieved_group = retrieved_group.unwrap();
        assert_eq!(created_group, retrieved_group);
        let target_group = Group {
            id: retrieved_group.id,
            identifier: Vec::from(GROUP_1_IDENTIFIER),
            name: GROUP_1_NAME.into(),
            threshold: threshold as i32,
            protocol: ProtocolType::Gg18,
            round: 0,
            key_type: KeyType::SignPdf,
            certificate: Some(GROUP_1_CERT.into()),
            note,
        };

        assert_eq!(retrieved_group, target_group);
        Ok(())
    }

    #[tokio::test]
    async fn given_valid_group_and_device_get_device_groups_returns_correct_group(
    ) -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new();
        let mut connection: AsyncPgConnection = ctx.get_test_connection().await?;

        let group_1_devices = &vec![&DEVICE_1_ID[..], &DEVICE_2_ID[..]];
        let group_2_devices = &vec![&DEVICE_2_ID[..], &DEVICE_3_ID[..]];
        let threshold = 2;

        add_device(
            &mut connection,
            &DEVICE_1_ID,
            "Device 1",
            &DeviceKind::User,
            &vec![42; 5],
        )
        .await?;
        add_device(
            &mut connection,
            &DEVICE_2_ID,
            "Device 2",
            &DeviceKind::User,
            &vec![42; 5],
        )
        .await?;
        add_device(
            &mut connection,
            &DEVICE_3_ID,
            "Device 3",
            &DeviceKind::User,
            &vec![42; 5],
        )
        .await?;

        let group1_creation_task = create_group_task(
            &mut connection,
            None,
            group_1_devices,
            threshold,
            KeyType::Decrypt,
            ProtocolType::ElGamal,
            &[],
            None,
        )
        .await?;
        let group_1 = add_group(
            &mut connection,
            &group1_creation_task.id,
            &GROUP_1_IDENTIFIER,
            GROUP_1_NAME,
            threshold,
            ProtocolType::ElGamal,
            KeyType::Decrypt,
            Some(&GROUP_1_CERT),
            None,
        )
        .await?;

        let group2_creation_task = create_group_task(
            &mut connection,
            None,
            group_2_devices,
            threshold,
            KeyType::SignChallenge,
            ProtocolType::Frost,
            &[],
            Some("time policy"),
        )
        .await?;
        let group_2 = add_group(
            &mut connection,
            &group2_creation_task.id,
            &GROUP_2_IDENTIFIER,
            GROUP_2_NAME,
            threshold,
            ProtocolType::Frost,
            KeyType::SignChallenge,
            Some(&GROUP_2_CERT),
            None,
        )
        .await?;

        assert_eq!(
            get_device_groups(&mut connection, &DEVICE_1_ID).await?,
            vec![group_1.clone()]
        );

        let mut expected_device2_groups = vec![group_1, group_2.clone()];
        let mut received_device2_groups = get_device_groups(&mut connection, &DEVICE_2_ID).await?;
        expected_device2_groups.sort_unstable();
        received_device2_groups.sort_unstable();
        assert_eq!(expected_device2_groups, received_device2_groups);

        assert_eq!(
            get_device_groups(&mut connection, &DEVICE_3_ID).await?,
            vec![group_2]
        );
        Ok(())
    }
}
