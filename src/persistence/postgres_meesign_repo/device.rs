use chrono::Local;
use diesel::{pg::Pg, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::AsyncConnection;
use diesel_async::RunQueryDsl;

use crate::persistence::{
    models::{Device, NewDevice},
    persistance_error::PersistenceError,
    postgres_meesign_repo::utils::NameValidator,
};

pub async fn get_devices<Conn>(connection: &mut Conn) -> Result<Vec<Device>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    use crate::persistence::schema::device;
    Ok(device::table.load(connection).await?)
}

pub async fn activate_device<Conn>(
    connection: &mut Conn,
    target_identifier: &[u8],
) -> Result<Option<Device>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    use crate::persistence::schema::device::dsl::*;
    let activated_device = diesel::update(device)
        .filter(identifier.eq(target_identifier))
        .set(last_active.eq(Local::now()))
        .returning(Device::as_returning())
        .get_result(connection)
        .await?;

    Ok(Some(activated_device))
}

pub async fn add_device<Conn>(
    connection: &mut Conn,
    identifier: &[u8],
    name: &str,
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
        identifier: &identifier.to_vec(),
        device_name: name,
        device_certificate: &certificate.to_vec(),
    };
    use crate::persistence::schema::device;

    let device: Device = diesel::insert_into(device::table)
        .values(new_device)
        .returning(Device::as_returning())
        .get_result(connection)
        .await?;
    Ok(device)
}

pub async fn device_ids_to_identifiers<Conn>(
    connection: &mut Conn,
    identifiers: Vec<Vec<u8>>,
) -> Result<Vec<i32>, PersistenceError>
where
    Conn: AsyncConnection<Backend = Pg>,
{
    use crate::persistence::schema::device::dsl::*;
    Ok(device
        .filter(identifier.eq_any(identifiers))
        .select(id)
        .load(connection)
        .await?)
}

#[cfg(test)]
mod test {

    use diesel_async::AsyncPgConnection;

    use crate::persistence::{
        persistance_error::PersistenceError,
        postgres_meesign_repo::device::{add_device, get_devices},
        tests::persistency_unit_test_context::PersistencyUnitTestContext,
    };

    #[tokio::test]
    async fn test_insert_device() -> Result<(), PersistenceError> {
        let ctx = PersistencyUnitTestContext::new();
        let identifier = vec![1];
        let name = "Test User 123";
        let certificate = vec![2];
        let mut connection: AsyncPgConnection = ctx.get_test_connection().await?;

        add_device(&mut connection, &identifier, name, &certificate).await?;

        let devices = get_devices(&mut connection).await?;
        assert_eq!(devices.len(), 1);
        let fetched_device = devices.first().unwrap();
        assert_eq!(fetched_device.identifier, identifier);
        assert_eq!(fetched_device.device_name, name);
        assert_eq!(fetched_device.device_certificate, certificate);
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
            &vec![1, 2, 3],
        )
        .await?;
        let Err(_) = add_device(&mut connection, &identifier, "user2", &vec![3, 2, 1]).await else {
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

        assert!(add_device(&mut connection, &empty, "Non-empty", &nonempty)
            .await
            .is_err());

        assert!(add_device(&mut connection, &nonempty, "", &nonempty)
            .await
            .is_err());

        assert!(add_device(&mut connection, &nonempty, "Non-empty", &empty)
            .await
            .is_err());

        Ok(())
    }
}
