use std::{env, sync::Once};

use diesel::{Connection, PgConnection};
use diesel_async::{AsyncConnection, AsyncPgConnection};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use dotenvy::dotenv;

use crate::persistence::persistance_error::PersistenceError;

static INIT: Once = Once::new();

pub fn initialize_db(database_url: &str) {
    INIT.call_once(|| {
        let mut conn = PgConnection::establish(database_url).unwrap();
        conn.run_pending_migrations(MIGRATIONS).unwrap();
    });
}
const MIGRATIONS: EmbeddedMigrations = embed_migrations!();

pub struct PersistencyUnitTestContext {
    database_url: String,
}

impl PersistencyUnitTestContext {
    pub fn new() -> Self {
        let _ = dotenv();
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        initialize_db(&database_url);

        Self { database_url }
    }

    pub async fn get_test_connection(&self) -> Result<AsyncPgConnection, PersistenceError> {
        let mut connection = AsyncPgConnection::establish(&self.database_url)
            .await
            .unwrap();
        connection.begin_test_transaction().await?;
        Ok(connection)
    }
}
