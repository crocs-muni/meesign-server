use std::convert::TryInto;

use diesel::alias;
use diesel::{pg::Pg, ExpressionMethods, JoinOnDsl, OptionalExtension, QueryDsl};
use diesel_async::{AsyncConnection, RunQueryDsl};
use std::collections::HashMap;
use uuid::Uuid;

use crate::persistence::models::NewGroupParticipant;
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
    let group_participant_rows = group_participant::table
        .select((
            group_participant::group_id,
            group_participant::device_id,
            group_participant::shares,
        ))
        .load::<(Vec<u8>, Vec<u8>, i32)>(connection)
        .await?;
    let mut group_participants: HashMap<Vec<u8>, Vec<(Vec<u8>, u32)>> = HashMap::new();
    for (group_id, device_id, shares) in group_participant_rows {
        group_participants
            .entry(group_id)
            .or_default()
            .push((device_id, shares as u32));
    }
    let groups = group::table
        .select((
            group::id,
            group::name,
            group::threshold,
            group::protocol,
            group::key_type,
            group::certificate,
            group::note,
        ))
        .load(connection)
        .await?
        .into_iter()
        .map(
            |(id, name, threshold, protocol, key_type, certificate, note)| {
                let participant_ids_shares = group_participants.remove(&id).unwrap_or_default();
                Group {
                    id,
                    name,
                    threshold,
                    protocol,
                    key_type,
                    certificate,
                    note,
                    participant_ids_shares,
                }
            },
        )
        .collect();
    Ok(groups)
}

pub async fn add_group<'a, Conn>(
    connection: &mut Conn,
    group_task_id: &Uuid,
    id: &[u8],
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

    if id.is_empty() {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Empty identifier"
        )));
    }
    let new_group = NewGroup {
        id,
        threshold: threshold as i32,
        protocol,
        name,
        certificate,
        key_type,
        note,
    };

    let _ = diesel::insert_into(group::table)
        .values(new_group)
        .execute(connection)
        .await?;

    let devices: Vec<(Vec<u8>, i32)> = task_participant::table
        .inner_join(task::table)
        .filter(task::id.eq(group_task_id))
        .select((task_participant::device_id, task_participant::shares))
        .load(connection)
        .await?;
    let group_participants: Vec<NewGroupParticipant> = devices
        .iter()
        .map(|(device_id, shares)| NewGroupParticipant {
            device_id,
            group_id: id,
            shares: *shares,
        })
        .collect();
    let expected_affected_row_count = group_participants.len();
    let affected_row_count = diesel::insert_into(group_participant::table)
        .values(group_participants)
        .execute(connection)
        .await?;
    // TODO: consider propagating the check into the prod build: assert -> Err(...)
    assert_eq!(affected_row_count, expected_affected_row_count);
    get_group(connection, id)
        .await?
        .ok_or(PersistenceError::DataInconsistencyError(
            "Already created group was not found!".into(),
        ))
}

pub async fn get_group<Conn>(
    connection: &mut Conn,
    group_identifier: &[u8],
) -> Result<Option<Group>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let participant_ids_shares = group_participant::table
        .select((group_participant::device_id, group_participant::shares))
        .filter(group_participant::group_id.eq(group_identifier))
        .load::<(Vec<u8>, i32)>(connection)
        .await?
        .into_iter()
        .map(|(device_id, shares)| (device_id, shares as u32))
        .collect();
    let group = group::table
        .filter(group::id.eq(group_identifier))
        .select((
            group::id,
            group::name,
            group::threshold,
            group::protocol,
            group::key_type,
            group::certificate,
            group::note,
        ))
        .first(connection)
        .await
        .optional()?
        .map(
            |(id, name, threshold, protocol, key_type, certificate, note)| Group {
                id,
                name,
                threshold,
                protocol,
                key_type,
                certificate,
                note,
                participant_ids_shares,
            },
        );
    Ok(group)
}

pub async fn get_device_groups<Conn>(
    connection: &mut Conn,
    device_id: &[u8],
) -> Result<Vec<Group>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let group_participant2 = alias!(group_participant as group_participant2);

    let group_participant_rows = group_participant::table
        .inner_join(
            group_participant2.on(group_participant2
                .field(group_participant::group_id)
                .eq(group_participant::group_id)),
        )
        .filter(
            group_participant2
                .field(group_participant::device_id)
                .eq(device_id),
        )
        .select((
            group_participant::group_id,
            group_participant::device_id,
            group_participant::shares,
        ))
        .load::<(Vec<u8>, Vec<u8>, i32)>(connection)
        .await?;
    let mut group_participants: HashMap<Vec<u8>, Vec<(Vec<u8>, u32)>> = HashMap::new();
    for (group_id, device_id, shares) in group_participant_rows {
        group_participants
            .entry(group_id)
            .or_default()
            .push((device_id, shares as u32));
    }

    let groups = group::table
        .inner_join(group_participant::table)
        .select((
            group::id,
            group::name,
            group::threshold,
            group::protocol,
            group::key_type,
            group::certificate,
            group::note,
        ))
        .filter(group_participant::device_id.eq(device_id))
        .load(connection)
        .await?
        .into_iter()
        .map(
            |(id, name, threshold, protocol, key_type, certificate, note)| {
                let participant_ids_shares = group_participants.remove(&id).unwrap_or_default();
                Group {
                    id,
                    name,
                    threshold,
                    protocol,
                    key_type,
                    certificate,
                    note,
                    participant_ids_shares,
                }
            },
        )
        .collect();
    Ok(groups)
}

#[cfg(test)]
mod test {
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
    #[cfg_attr(not(feature = "db-tests"), ignore)]
    async fn given_valid_arguments_create_group() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new().await;
        let mut connection = ctx.get_test_connection().await.expect("Could not connect to test database");

        let participants = &vec![(&DEVICE_1_ID[..], 1), (&DEVICE_2_ID[..], 1)];
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
            participants,
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
        let mut participant_ids_shares: Vec<(Vec<u8>, u32)> =
            vec![(DEVICE_1_ID.into(), 1), (DEVICE_2_ID.into(), 1)];
        participant_ids_shares.sort_by(|(a, _), (b, _)| a.cmp(b));
        let target_group = Group {
            id: Vec::from(GROUP_1_IDENTIFIER),
            name: GROUP_1_NAME.into(),
            threshold: threshold as i32,
            protocol: ProtocolType::Gg18,
            key_type: KeyType::SignPdf,
            certificate: Some(GROUP_1_CERT.into()),
            note,
            participant_ids_shares,
        };

        assert_eq!(retrieved_group, target_group);
        Ok(())
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db-tests"), ignore)]
    async fn given_valid_group_and_device_get_device_groups_returns_correct_group(
    ) -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new().await;
        let mut connection = ctx.get_test_connection().await.expect("Could not connect to test database");

        let group_1_participants = &vec![(&DEVICE_1_ID[..], 1), (&DEVICE_2_ID[..], 1)];
        let group_2_participants = &vec![(&DEVICE_2_ID[..], 1), (&DEVICE_3_ID[..], 1)];
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
            group_1_participants,
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
            group_2_participants,
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
