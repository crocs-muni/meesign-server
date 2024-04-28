use std::convert::TryInto;

use diesel::{pg::Pg, result::Error::NotFound, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

use crate::persistence::schema::groupparticipant;
use crate::persistence::schema::signinggroup;
use crate::persistence::{
    enums::{KeyType, ProtocolType},
    error::PersistenceError,
    models::{Group, NewGroup},
    schema::taskparticipant,
};

pub async fn get_groups<Conn>(connection: &mut Conn) -> Result<Vec<Group>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    Ok(signinggroup::table.load(connection).await?)
}

pub async fn add_group<'a, Conn>(
    connection: &mut Conn,
    group_task_id: &Uuid,
    identifier: &[u8],
    name: &str,
    devices: &[&[u8]],
    threshold: u32,
    protocol: ProtocolType,
    key_type: KeyType,
    certificate: Option<&[u8]>,
) -> Result<Group, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let threshold: i32 = threshold.try_into()?;
    if !(1..=devices.len() as i32).contains(&threshold) {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Invalid threshold {threshold}"
        )));
    }

    if identifier.is_empty() {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Empty identifier"
        )));
    }
    let new_group = NewGroup {
        identifier,
        threshold: threshold as i32,
        protocol,
        group_name: name,
        round: 0,
        group_certificate: certificate,
        key_type,
    };

    let group = diesel::insert_into(signinggroup::table)
        .values(new_group)
        .returning(Group::as_returning())
        .get_result(connection)
        .await?;

    // group participants already exist as they were participating in the task
    // overwrite null values with the correct group_id
    let group_participant_alias = diesel::alias!(groupparticipant as group_participant_alias);
    let group_participant_ids = group_participant_alias
        .inner_join(taskparticipant::table)
        .filter(taskparticipant::task_id.eq(Some(&group_task_id)))
        .select(group_participant_alias.field(groupparticipant::id));
    diesel::update(groupparticipant::table)
        .filter(groupparticipant::id.eq_any(group_participant_ids))
        .set(groupparticipant::group_id.eq(Some(group.id)))
        .execute(connection)
        .await?;
    Ok(group)
}

pub async fn get_group<Conn>(
    connection: &mut Conn,
    group_identifier: &[u8],
) -> Result<Option<Group>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let group: Option<Group> = match signinggroup::table
        .filter(signinggroup::identifier.eq(group_identifier))
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
    let groups: Vec<Group> = groupparticipant::table
        .inner_join(signinggroup::table)
        .filter(groupparticipant::device_id.eq(device_identifier))
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
        add_device(&mut connection, &DEVICE_1_ID, "Device 1", &vec![42; 5]).await?;
        add_device(&mut connection, &DEVICE_2_ID, "Device 2", &vec![42; 5]).await?;

        let group_creation_task = create_group_task(
            &mut connection,
            GROUP_1_NAME,
            devices,
            threshold,
            KeyType::Decrypt,
            ProtocolType::ElGamal,
        )
        .await?;

        let created_group = add_group(
            &mut connection,
            &group_creation_task.id,
            &GROUP_1_IDENTIFIER,
            GROUP_1_NAME,
            devices,
            threshold,
            ProtocolType::ElGamal,
            KeyType::Decrypt,
            Some(&GROUP_1_CERT),
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
            group_name: GROUP_1_NAME.into(),
            threshold: threshold as i32,
            protocol: ProtocolType::ElGamal,
            round: 0,
            key_type: KeyType::Decrypt,
            group_certificate: Some(GROUP_1_CERT.into()),
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

        add_device(&mut connection, &DEVICE_1_ID, "Device 1", &vec![42; 5]).await?;
        add_device(&mut connection, &DEVICE_2_ID, "Device 2", &vec![42; 5]).await?;
        add_device(&mut connection, &DEVICE_3_ID, "Device 3", &vec![42; 5]).await?;

        let group1_creation_task = create_group_task(
            &mut connection,
            GROUP_1_NAME,
            group_1_devices,
            threshold,
            KeyType::Decrypt,
            ProtocolType::ElGamal,
        )
        .await?;
        let group_1 = add_group(
            &mut connection,
            &group1_creation_task.id,
            &GROUP_1_IDENTIFIER,
            GROUP_1_NAME,
            group_1_devices,
            threshold,
            ProtocolType::ElGamal,
            KeyType::Decrypt,
            Some(&GROUP_1_CERT),
        )
        .await?;

        let group2_creation_task = create_group_task(
            &mut connection,
            GROUP_2_NAME,
            group_2_devices,
            threshold,
            KeyType::SignChallenge,
            ProtocolType::Frost,
        )
        .await?;
        let group_2 = add_group(
            &mut connection,
            &group2_creation_task.id,
            &GROUP_2_IDENTIFIER,
            GROUP_2_NAME,
            group_2_devices,
            threshold,
            ProtocolType::Frost,
            KeyType::SignChallenge,
            Some(&GROUP_2_CERT),
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
