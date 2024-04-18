use std::{env, sync::Once};

use diesel::{Connection, PgConnection};
use diesel_async::{AsyncConnection, AsyncPgConnection};
use diesel_migrations::MigrationHarness;
use dotenvy::dotenv;

use crate::persistence::{error::PersistenceError, postgres_meesign_repo::MIGRATIONS};

static INIT: Once = Once::new();
const TEST_DB_URL_ENV_NAME: &str = "TEST_DATABASE_URL";

pub fn initialize_db(database_url: &str) {
    INIT.call_once(|| {
        let mut connection = PgConnection::establish(database_url).expect(&format!(
            "Couldn't connect to the test DB using connection URL: {database_url}"
        ));
        connection
            .run_pending_migrations(MIGRATIONS)
            .expect("Couldn't run migrations");
    });
}

pub struct PersistencyUnitTestContext {
    database_url: String,
}

impl PersistencyUnitTestContext {
    pub fn new() -> Self {
        let _ = dotenv();
        let database_url =
            env::var(TEST_DB_URL_ENV_NAME).expect(&format!("{TEST_DB_URL_ENV_NAME} must be set"));
        initialize_db(&database_url);

        Self { database_url }
    }

    pub async fn get_test_connection(&self) -> Result<AsyncPgConnection, PersistenceError> {
        let mut connection = AsyncPgConnection::establish(&self.database_url).await?;
        connection.begin_test_transaction().await?;
        Ok(connection)
    }
}
