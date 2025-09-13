use diesel::QueryDsl;
use diesel::{pg::Pg, ExpressionMethods, SelectableHelper};
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::persistence::enums::DeviceKind;
use crate::persistence::schema::{device, group_participant, task, task_participant};
use crate::persistence::{
    error::PersistenceError,
    models::{Device, NewDevice, Participant},
    repository::utils::NameValidator,
};

pub async fn get_devices<Conn>(connection: &mut Conn) -> Result<Vec<Device>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    Ok(device::table.load(connection).await?)
}

pub async fn get_group_participants<Conn>(
    connection: &mut Conn,
    group_id: &[u8],
) -> Result<Vec<Participant>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let devices = group_participant::table
        .inner_join(device::table)
        .filter(group_participant::group_id.eq(group_id))
        .select((Device::as_returning(), group_participant::shares))
        .load::<(Device, i32)>(connection)
        .await?
        .into_iter()
        .map(|(device, shares)| Participant {
            device,
            shares: shares as u32,
        })
        .collect();
    Ok(devices)
}

pub async fn get_tasks_participants<Conn>(
    connection: &mut Conn,
    task_ids: &[Uuid],
) -> Result<Vec<(Uuid, Participant)>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let tasks_devices = task_participant::table
        .inner_join(task::table)
        .inner_join(device::table)
        .filter(task::id.eq_any(task_ids))
        .select((task::id, (Device::as_returning(), task_participant::shares)))
        .order_by(task::id)
        .load::<(Uuid, (Device, i32))>(connection)
        .await?
        .into_iter()
        .map(|(task_id, (device, shares))| {
            (
                task_id,
                Participant {
                    device,
                    shares: shares as u32,
                },
            )
        })
        .collect();
    Ok(tasks_devices)
}

pub async fn add_device<Conn>(
    connection: &mut Conn,
    identifier: &[u8],
    name: &str,
    kind: &DeviceKind,
    certificate: &[u8],
) -> Result<Device, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    if !name.is_name_valid() {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Invalid device name: {name}"
        )));
    }

    if identifier.is_empty() {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Empty identifier"
        )));
    }

    if certificate.is_empty() {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Empty certificate"
        )));
    }

    let new_device = NewDevice {
        id: &identifier.to_vec(),
        name,
        kind,
        certificate: &certificate.to_vec(),
    };

    let device: Device = diesel::insert_into(device::table)
        .values(new_device)
        .returning(Device::as_returning())
        .get_result(connection)
        .await?;
    Ok(device)
}

#[cfg(test)]
mod test {
    use crate::persistence::{
        enums::DeviceKind,
        error::PersistenceError,
        repository::device::{add_device, get_devices},
        tests::persistency_unit_test_context::PersistencyUnitTestContext,
    };

    #[tokio::test]
    #[cfg_attr(not(feature = "db-tests"), ignore)]
    async fn test_insert_device() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new().await;
        let mut connection = ctx
            .get_test_connection()
            .await
            .expect("Could not connect to test database");

        let identifier = vec![1];
        let name = "Test User 123";
        let certificate = vec![2];

        add_device(
            &mut connection,
            &identifier,
            name,
            &DeviceKind::User,
            &certificate,
        )
        .await?;

        let devices = get_devices(&mut connection).await?;
        assert_eq!(devices.len(), 1);
        let fetched_device = devices.first().unwrap();
        assert_eq!(fetched_device.id, identifier);
        assert_eq!(fetched_device.name, name);
        assert_eq!(fetched_device.certificate, certificate);
        Ok(())
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db-tests"), ignore)]
    async fn test_identifier_unique_constraint() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new().await;
        let mut connection = ctx
            .get_test_connection()
            .await
            .expect("Could not connect to test database");

        let identifier = vec![1];
        let first_device_name = "user1";

        add_device(
            &mut connection,
            &identifier,
            first_device_name,
            &DeviceKind::User,
            &vec![1, 2, 3],
        )
        .await?;
        let Err(_) = add_device(
            &mut connection,
            &identifier,
            "user2",
            &DeviceKind::User,
            &vec![3, 2, 1],
        )
        .await
        else {
            panic!("DB shoudln't have allowed to insert 2 devices with the same identifier");
        };

        Ok(())
    }

    #[tokio::test]
    #[cfg_attr(not(feature = "db-tests"), ignore)]
    async fn test_invalid_device() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new().await;
        let mut connection = ctx
            .get_test_connection()
            .await
            .expect("Could not connect to test database");

        let empty = vec![];
        let nonempty: Vec<u8> = vec![1];

        assert!(add_device(
            &mut connection,
            &empty,
            "Non-empty",
            &DeviceKind::User,
            &nonempty
        )
        .await
        .is_err());

        assert!(
            add_device(&mut connection, &nonempty, "", &DeviceKind::User, &nonempty)
                .await
                .is_err()
        );

        assert!(add_device(
            &mut connection,
            &nonempty,
            "Non-empty",
            &DeviceKind::User,
            &empty
        )
        .await
        .is_err());

        Ok(())
    }
}
