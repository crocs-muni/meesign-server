use chrono::Utc;
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::AsyncPgConnection;

use crate::persistence::{
    models::{Device, NewDevice},
    persistance_error::PersistenceError,
};

use diesel_async::RunQueryDsl;

pub async fn get_devices(
    connection: &mut AsyncPgConnection,
) -> Result<Vec<Device>, PersistenceError> {
    use crate::persistence::schema::device;
    Ok(device::table.load(connection).await?)
}

pub async fn activate_device(
    connection: &mut AsyncPgConnection,
    target_identifier: &Vec<u8>,
) -> Result<Option<Device>, PersistenceError> {
    use crate::persistence::schema::device::dsl::*;
    let activated_device = diesel::update(device)
        .filter(identifier.eq(target_identifier))
        .set(last_active.eq(Utc::now().naive_local()))
        .returning(Device::as_returning())
        .get_result(connection)
        .await?;

    Ok(Some(activated_device))
}

pub async fn add_device(
    connection: &mut AsyncPgConnection,
    identifier: &[u8],
    name: &str,
    certificate: &[u8],
) -> Result<Device, PersistenceError> {
    const MAX_NAME_LEN: usize = 64;

    if name.chars().count() > MAX_NAME_LEN
        || name
            .chars()
            .any(|x| x.is_ascii_punctuation() || x.is_control())
    {
        return Err(PersistenceError::InvalidArgumentError(format!(
            "Invalid device name: {name}"
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

pub async fn device_ids_to_identifiers(
    connection: &mut AsyncPgConnection,
    identifiers: Vec<Vec<u8>>,
) -> Result<Vec<i32>, PersistenceError> {
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
        let Err(_) = add_device(& mut connection, &identifier, "user2", &vec![3, 2, 1]).await else {
            panic!("DB shoudln't have allowed to insert 2 devices with the same identifier");
        };

        Ok(())
    }
}
