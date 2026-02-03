use std::{env, sync::Once};

use diesel::{Connection, PgConnection};
use diesel_async::{AsyncConnection, AsyncPgConnection};
use diesel_migrations::MigrationHarness;
use dotenvy::dotenv;

use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ContainerAsync;

use crate::persistence::{error::PersistenceError, postgres_repository::MIGRATIONS};
use rand::distributions::Alphanumeric;
use rand::Rng;

static INIT: Once = Once::new();
const TEST_DB_URL_ENV_NAME: &str = "TEST_DATABASE_URL";

pub fn initialize_db_inner(database_url: &str) {
    let mut connection = PgConnection::establish(database_url).expect(&format!(
        "Couldn't connect to the test DB using connection URL: {database_url}"
    ));
    connection
        .run_pending_migrations(MIGRATIONS)
        .expect("Couldn't run migrations");
}

pub fn initialize_db(database_url: &str, only_once: bool) {
    match only_once {
        // TODO Is there a cleaner way to call function with parameters as a closure without
        // any parameters?
        true => INIT.call_once(|| initialize_db_inner(database_url)),
        false => initialize_db_inner(database_url),
    }
}

pub struct PersistencyUnitTestContext {
    _container: Option<ContainerAsync<Postgres>>,
    pub database_url: String,
}

impl PersistencyUnitTestContext {
    pub async fn new() -> Self {
        let _ = dotenv();
        match env::var(TEST_DB_URL_ENV_NAME) {
            Ok(database_url) => {
                initialize_db(&database_url, true);
                Self {
                    _container: None,
                    database_url,
                }
            }
            Err(_) => {
                let mut rng = rand::thread_rng();
                let password: String = (0..20).map(|_| rng.sample(Alphanumeric) as char).collect();
                let db_name = "meesign";
                let user = "meesign";
                let default_postgres_port = 5432;

                let _container = Postgres::default()
                    .with_db_name(db_name)
                    .with_user(user)
                    .with_password(&password)
                    // TODO: use with_reuse(...) to speed up local testing?
                    .start()
                    .await
                    .expect(
                        "Could not start test Postgres database Docker container. \
                    Is Docker installed? Also, try setting `TEST_DATABASE_URL` to test against \
                    already started container.",
                    );

                // Unwrapping on both host and host_port as there is not much we can do to recover.
                let host = _container.get_host().await.unwrap();
                let host_port = _container
                    .get_host_port_ipv4(default_postgres_port)
                    .await
                    .unwrap();

                let database_url =
                    format!("postgres://{user}:{password}@{host}:{host_port}/{db_name}");
                initialize_db(&database_url, false);

                Self {
                    _container: Some(_container),
                    database_url,
                }
            }
        }
    }

    pub async fn get_test_connection(&self) -> Result<AsyncPgConnection, PersistenceError> {
        let mut connection = AsyncPgConnection::establish(&self.database_url).await?;
        connection.begin_test_transaction().await?;
        Ok(connection)
    }
}
