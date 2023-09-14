use super::meesign_repo::MeesignRepo;
use super::models::{Device, Group};
use super::persistance_error::PersistenceError;
use crate::persistence::models::NewDevice;

use chrono::Utc;
use diesel::prelude::*;
use diesel::{Connection, PgConnection};
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use std::env;
use std::sync::Arc;

const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

pub struct PostgresMeesignRepo {
    pg_pool: Arc<PgPool>,
}

type PgPool = Pool<AsyncPgConnection>;

impl PostgresMeesignRepo {
    pub async fn from_url(database_url: &str) -> Result<Self, PersistenceError> {
        let repo = Self {
            pg_pool: Arc::new(PostgresMeesignRepo::init_pool(database_url)?),
        };
        repo.apply_migrations()?;
        Ok(repo)
    }

    pub fn apply_migrations(&self) -> Result<(), PersistenceError> {
        // TODO: can we do it in async?
        let mut conn = self.get_connection()?;
        conn.run_pending_migrations(MIGRATIONS)
            .expect("Couldn't apply migrations");
        Ok(())
    }

    fn init_pool(database_url: &str) -> Result<PgPool, PersistenceError> {
        let config = AsyncDieselConnectionManager::<diesel_async::AsyncPgConnection>::new(
            std::env::var(database_url)?,
        );
        Ok(Pool::builder(config).build()?)
    }

    async fn get_async_connection(&self) -> Result<AsyncPgConnection, PersistenceError> {
        // Ok(self.pg_pool.get().await.unwrap()) // TODO
        Ok(
            AsyncPgConnection::establish(&std::env::var("DATABASE_URL")?)
                .await
                .unwrap(),
        )
    }

    fn get_connection(&self) -> Result<PgConnection, PersistenceError> {
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        Ok(PgConnection::establish(&database_url).unwrap())
    }
}

#[tonic::async_trait]
impl MeesignRepo for PostgresMeesignRepo {
    async fn add_device(
        &self,
        identifier: &[u8],
        name: &str,
        certificate: &[u8],
    ) -> Result<(), PersistenceError> {
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

        diesel::insert_into(device::table)
            .values(new_device)
            .execute(&mut self.get_async_connection().await?)
            .await?;
        Ok(())
    }

    async fn activate_device(&self, target_identifier: &Vec<u8>) -> Result<(), PersistenceError> {
        use crate::persistence::schema::device::dsl::*;
        let rows_affected = diesel::update(device)
            .filter(identifier.eq(target_identifier))
            .set(last_active.eq(Utc::now().naive_utc()))
            .execute(&mut self.get_async_connection().await?)
            .await?;

        let expected_affected_rows_count = 1;
        if rows_affected != expected_affected_rows_count {
            return Err(PersistenceError::GeneralError(format!("Invalid number of affected rows: Expected {expected_affected_rows_count}, but got {rows_affected}.")));
        }
        Ok(())
    }

    async fn get_devices(&self) -> Result<Vec<Device>, PersistenceError> {
        use crate::persistence::schema::device;
        Ok(device::table
            .load(&mut self.get_async_connection().await?)
            .await?)
    }

    async fn get_groups(&self) -> Result<Vec<Group>, PersistenceError> {
        use crate::persistence::schema::signinggroup;
        Ok(signinggroup::table
            .load(&mut self.get_async_connection().await?)
            .await?)
    }
}
