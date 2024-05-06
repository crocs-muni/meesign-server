use chrono::Local;
use diesel::QueryDsl;
use diesel::{pg::Pg, ExpressionMethods, SelectableHelper};
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;
use uuid::Uuid;

use crate::persistence::enums::DeviceKind;
use crate::persistence::schema::{device, task_participant};
use crate::persistence::schema::{group_participant, task};
use crate::persistence::{
    error::PersistenceError,
    models::{Device, NewDevice},
    repository::utils::NameValidator,
};

pub async fn get_devices<Conn>(connection: &mut Conn) -> Result<Vec<Device>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    Ok(device::table.load(connection).await?)
}

pub async fn get_devices_with_ids<Conn>(
    connection: &mut Conn,
    device_ids: &[&[u8]],
) -> Result<Vec<Device>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let devices = device::table
        .filter(device::id.eq_any(device_ids))
        .order_by(device::id)
        .load(connection)
        .await?;
    Ok(devices)
}

pub async fn get_task_devices<Conn>(
    connection: &mut Conn,
    task_id: &Uuid,
) -> Result<Vec<Device>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let devices: Vec<Device> = task_participant::table
        .inner_join(device::table)
        .inner_join(task::table)
        .filter(task::id.eq(task_id))
        .select(Device::as_returning())
        .load(connection)
        .await?;
    Ok(devices)
}

pub async fn get_group_device_ids<Conn>(
    connection: &mut Conn,
    group_id: &i32,
) -> Result<Vec<Vec<u8>>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let device_ids: Vec<Vec<u8>> = group_participant::table
        .filter(group_participant::group_id.eq(group_id))
        .select(group_participant::device_id)
        .load::<Vec<u8>>(connection)
        .await?
        .into_iter()
        .collect();

    Ok(device_ids)
}

pub async fn activate_device<Conn>(
    connection: &mut Conn,
    target_identifier: &[u8],
) -> Result<Option<Device>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    let activated_device = diesel::update(device::table)
        .filter(device::id.eq(target_identifier))
        .set(device::last_active.eq(Local::now()))
        .returning(Device::as_returning())
        .get_result(connection)
        .await?;

    Ok(Some(activated_device))
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

    use std::{ops::Add, time::Duration};

    use diesel_async::AsyncPgConnection;
    use tokio::time::sleep;

    use crate::persistence::{
        enums::DeviceKind,
        error::PersistenceError,
        repository::device::{activate_device, add_device, get_devices},
        tests::persistency_unit_test_context::PersistencyUnitTestContext,
    };

    #[tokio::test]
    async fn test_insert_device() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new();
        let identifier = vec![1];
        let name = "Test User 123";
        let certificate = vec![2];
        let mut connection: AsyncPgConnection = ctx.get_test_connection().await?;

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
    async fn test_identifier_unique_constraint() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new();

        let identifier = vec![1];
        let first_device_name = "user1";
        let mut connection = ctx.get_test_connection().await?;

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
    async fn test_invalid_device() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new();
        let empty = vec![];
        let nonempty: Vec<u8> = vec![1];
        let mut connection: AsyncPgConnection = ctx.get_test_connection().await?;

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

    #[tokio::test]
    async fn test_activate_device() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new();
        let mut connection = ctx.get_test_connection().await?;
        let timeout = Duration::from_millis(10);

        let device_identifier = vec![3; 32];
        add_device(
            &mut connection,
            &device_identifier,
            "user1",
            &DeviceKind::User,
            &vec![1; 400],
        )
        .await?;
        let fst_device_activation = activate_device(&mut connection, &device_identifier)
            .await?
            .unwrap();
        sleep(timeout).await;
        let snd_device_activation = activate_device(&mut connection, &device_identifier)
            .await?
            .unwrap();

        assert!(
            fst_device_activation.last_active.add(timeout) <= snd_device_activation.last_active
        );

        Ok(())
    }
}
