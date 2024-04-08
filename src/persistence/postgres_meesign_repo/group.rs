use std::convert::TryInto;

use diesel::{pg::Pg, result::Error::NotFound, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::{AsyncConnection, RunQueryDsl};

use crate::persistence::{
    enums::ProtocolType,
    models::{Group, NewGroup, NewGroupParticipant},
    persistance_error::PersistenceError,
    postgres_meesign_repo::device::device_ids_to_identifiers,
};

pub async fn get_groups<Conn>(connection: &mut Conn) -> Result<Vec<Group>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    use crate::persistence::schema::signinggroup;
    Ok(signinggroup::table.load(connection).await?)
}

pub async fn add_group<'a, Conn>(
    connection: &mut Conn,
    identifier: &[u8],
    name: &str,
    devices: &[&[u8]],
    threshold: u32,
    protocol: ProtocolType,
    certificate: Option<&[u8]>,
) -> Result<Group, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    use crate::persistence::schema::groupparticipant;
    use crate::persistence::schema::signinggroup;

    let threshold: i32 = threshold.try_into()?;
    if !(1..=devices.len() as i32).contains(&threshold) {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Invalid threshold {threshold}"
        )));
    }

    let new_group = NewGroup {
        identifier,
        threshold: threshold as i32,
        protocol,
        group_name: name,
        round: 0, // TODO: check why
        group_certificate: certificate,
    };

    let devices: Vec<Vec<u8>> = devices
        .iter()
        .map(|identifier| identifier.to_vec())
        .collect();
    let ids = device_ids_to_identifiers(connection, devices).await?;
    let group = diesel::insert_into(signinggroup::table)
        .values(new_group)
        .returning(Group::as_returning())
        .get_result(connection)
        .await?;

    let group_id = group.id;
    let group_participants: Vec<NewGroupParticipant> = ids
        .into_iter()
        .map(|device_id| NewGroupParticipant {
            device_id,
            group_id,
        })
        .collect();

    diesel::insert_into(groupparticipant::table)
        .values(group_participants)
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
    use crate::persistence::schema::signinggroup::dsl::*;

    let group: Option<Group> = match signinggroup
        .filter(identifier.eq(group_identifier))
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
