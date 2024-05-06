use chrono::{DateTime, Local};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::{
    communicator::Communicator,
    tasks::{group::GroupTask, Task as TaskTrait},
};

use super::{
    enums::{DeviceKind, KeyType, ProtocolType, TaskType},
    error::PersistenceError,
    models::{Device, Group, Task},
};

use self::{
    device::{activate_device, add_device, get_devices, get_devices_with_ids},
    group::get_group,
    task::{
        get_device_tasks, get_tasks, increment_round, increment_task_attempt_count, set_round,
        set_task_last_update, set_task_result,
    },
};
use self::{
    device::{get_group_device_ids, get_task_devices},
    task::{create_group_task, create_task},
};
use self::{
    group::{add_group, get_device_groups, get_groups},
    task::get_task,
};

use diesel::{Connection, PgConnection};
use diesel_async::AsyncPgConnection;
use diesel_async::{
    pooled_connection::deadpool::{Object, Pool},
    AsyncConnection,
};
use diesel_async::{
    pooled_connection::AsyncDieselConnectionManager, scoped_futures::ScopedFutureExt,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use std::env;
use std::sync::Arc;

mod device;
mod group;
mod task;
pub mod utils;

pub(crate) const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

pub struct Repository {
    pg_pool: Arc<PgPool>,
}

pub type PgPool = Pool<AsyncPgConnection>;
type DeadpoolPgConnection = Object<AsyncPgConnection>;

impl Repository {
    pub async fn from_url(database_url: &str) -> Result<Self, PersistenceError> {
        let repo = Self {
            pg_pool: Arc::new(Repository::init_pool(database_url)?),
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
        let config = AsyncDieselConnectionManager::<AsyncPgConnection>::new(database_url);
        Ok(Pool::builder(config).build()?)
    }

    async fn get_async_connection(&self) -> Result<DeadpoolPgConnection, PersistenceError> {
        Ok(self.pg_pool.get().await.unwrap())
    }

    fn get_connection(&self) -> Result<PgConnection, PersistenceError> {
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        Ok(PgConnection::establish(&database_url).unwrap())
    }
}

/// Interface
impl Repository {
    /* Devices */
    pub async fn add_device(
        &self,
        identifier: &[u8],
        name: &str,
        kind: &DeviceKind,
        certificate: &[u8],
    ) -> Result<Device, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        add_device(connection, identifier, name, kind, certificate).await
    }

    pub async fn get_devices(&self) -> Result<Vec<Device>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        get_devices(connection).await
    }

    pub async fn get_devices_with_ids(
        &self,
        device_ids: &[&[u8]],
    ) -> Result<Vec<Device>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        get_devices_with_ids(connection, device_ids).await
    }

    pub async fn get_task_devices(&self, task_id: &Uuid) -> Result<Vec<Device>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        get_task_devices(connection, task_id).await
    }

    pub async fn get_group_device_ids(
        &self,
        group_id: &i32,
    ) -> Result<Vec<Vec<u8>>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        get_group_device_ids(connection, group_id).await
    }

    pub async fn activate_device(
        &self,
        target_identifier: &[u8],
    ) -> Result<DateTime<Local>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        activate_device(connection, target_identifier).await
    }

    /* Groups */
    pub async fn add_group(
        &self,
        identifier: &[u8],
        group_task_id: &Uuid,
        name: &str,
        threshold: u32,
        protocol: ProtocolType,
        key_type: KeyType,
        certificate: Option<&[u8]>,
        note: Option<&str>,
    ) -> Result<Group, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        connection
            .transaction(|connection| {
                async move {
                    add_group(
                        connection,
                        group_task_id,
                        identifier,
                        name,
                        threshold,
                        protocol,
                        key_type,
                        certificate,
                        note,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
    }

    pub async fn get_group(
        &self,
        group_identifier: &[u8],
    ) -> Result<Option<Group>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        get_group(connection, group_identifier).await
    }

    pub async fn get_groups(&self) -> Result<Vec<Group>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        get_groups(connection).await
    }

    pub async fn get_device_groups(
        &self,
        identifier: &[u8],
    ) -> Result<Vec<Group>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        get_device_groups(connection, identifier).await
    }

    pub async fn does_group_contain_device(
        &self,
        group_id: &[u8],
        device_id: &[u8],
    ) -> Result<bool, PersistenceError> {
        todo!()
    }

    /* Tasks */
    pub async fn create_group_task<'a>(
        &self,
        id: Option<&Uuid>,
        devices: &[&[u8]],
        threshold: u32,
        protocol_type: ProtocolType,
        key_type: KeyType,
        request: &[u8],
        note: Option<&str>,
    ) -> Result<Task, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        connection
            .transaction(|connection| {
                async move {
                    create_group_task(
                        connection,
                        id,
                        devices,
                        threshold,
                        key_type,
                        protocol_type,
                        request,
                        note,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
    }

    pub async fn create_sign_task<'a>(
        &self,
        id: Option<&Uuid>,
        group_identifier: &[u8],
        name: &str,
        data: &Vec<u8>,
    ) -> Result<Task, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        connection
            .transaction(|connection| {
                async move {
                    create_task(
                        connection,
                        id,
                        TaskType::SignChallenge, // TODO: based on data
                        name,
                        Some(data),
                        &vec![],
                        None,
                        None,
                        None,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
    }

    pub async fn create_decrypt_task(
        &self,
        id: Option<&Uuid>,
        group_identifier: &[u8],
        name: &str,
        data: &Vec<u8>,
    ) -> Result<Task, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        connection
            .transaction(|connection| {
                async move {
                    create_task(
                        connection,
                        id,
                        TaskType::Decrypt,
                        name,
                        Some(data),
                        &vec![],
                        None,
                        None,
                        None,
                    )
                    .await
                }
                .scope_boxed()
            })
            .await
    }

    pub async fn get_task(
        &self,
        task_id: &Uuid,
        communicator: Arc<RwLock<Communicator>>,
        repository: Arc<Repository>,
    ) -> Result<Option<Box<dyn TaskTrait>>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        let task = get_task(connection, task_id).await?;
        let device_ids = self.get_task_devices(task_id).await?;
        if task.is_none() {
            return Ok(None);
        }
        let task = task.unwrap();
        // TODO: also for other task types

        let task = Box::new(
            GroupTask::from_model(task, device_ids, communicator, repository)
                .await
                .unwrap(),
        ) as Box<dyn TaskTrait>;
        Ok(Some(task))
    }

    pub async fn get_tasks(&self) -> Result<Vec<Task>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        let tasks = get_tasks(connection).await?;
        Ok(tasks)
    }

    pub async fn get_tasks_for_restart(&self) -> Result<Vec<Task>, PersistenceError> {
        todo!()
    }

    pub async fn get_active_device_tasks(
        &self,
        identifier: &[u8],
    ) -> Result<Vec<Task>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        get_device_tasks(connection, identifier).await
    }

    pub async fn increment_round(&self, task_id: &Uuid) -> Result<u32, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        increment_round(connection, task_id).await
    }

    pub async fn set_round(&self, task_id: &Uuid, round: u16) -> Result<u16, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        set_round(connection, task_id, round).await
    }

    pub async fn set_task_last_update(
        &self,
        task_id: &Uuid,
    ) -> Result<DateTime<Local>, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        set_task_last_update(connection, task_id).await
    }

    pub async fn increment_task_attempt_count(
        &self,
        task_id: &Uuid,
    ) -> Result<u32, PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        increment_task_attempt_count(connection, task_id).await
    }

    pub async fn set_task_result(
        &self,
        task_id: &Uuid,
        result: &Result<Vec<u8>, String>,
    ) -> Result<(), PersistenceError> {
        let connection = &mut self.get_async_connection().await?;
        set_task_result(connection, task_id, result).await
    }
}
