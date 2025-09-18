use tokio::sync::Mutex;

use diesel::{Connection, PgConnection};
use diesel_async::{pooled_connection::AsyncDieselConnectionManager, AsyncConnection, AsyncPgConnection};
use diesel_migrations::MigrationHarness;

use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::{testcontainers::runners::AsyncRunner,};
use testcontainers_modules::testcontainers::{ReuseDirective, ContainerAsync};
use testcontainers_modules::testcontainers::ImageExt;
use testcontainers_modules::testcontainers::core::IntoContainerPort;


use diesel_async::pooled_connection::bb8::Pool;

use rand::Rng;
use rand::distributions::Alphanumeric;
use crate::persistence::{error::PersistenceError, repository::MIGRATIONS};
use diesel_async::pooled_connection::bb8::PooledConnection;



pub fn initialize_db(database_url: &str) {
    let mut connection = PgConnection::establish(database_url).expect(&format!(
        "Couldn't connect to the test DB using connection URL: {database_url}"
    ));
    connection
        .run_pending_migrations(MIGRATIONS)
        .expect("Couldn't run migrations");
}

pub struct PersistencyUnitTestContext {
    _container: ContainerAsync<Postgres>,
    pub database_url: String,
}


impl PersistencyUnitTestContext {
    pub async fn new() -> Self {
        // check if TEST_DATABASE_URL exists and use that one
        let mut rng = rand::thread_rng();
        let password: String =  (0..20).map(|_| rng.sample(Alphanumeric) as char).collect();
        let db_name = "meesign";
        let user = "meesign";
        let default_postgres_port = 5432;

        let _container = Postgres::default()
            .with_db_name(db_name)
            .with_user(user)
            .with_password(&password)
            .start().await.expect("Could not start test Postgres database Docker container. \
                Is Docker installed? Also, try setting `TEST_DATABASE_URL` to test against \
                already started container."
            );

        // Unwrapping on both host and host_port as there is not much we can do to recover.
        let host = _container.get_host().await.unwrap();
        let host_port = _container.get_host_port_ipv4(default_postgres_port).await.unwrap();

        let database_url = format!("postgres://{user}:{password}@{host}:{host_port}/{db_name}");
        initialize_db(&database_url);

        Self { _container, database_url }
    }

    pub async fn get_test_connection(&self) -> Result<AsyncPgConnection, PersistenceError> {
        let mut connection = AsyncPgConnection::establish(&self.database_url).await?;
        connection.begin_test_transaction().await?;
        Ok(connection)
    }
}
