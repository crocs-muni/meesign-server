use diesel::{sql_types::Text, Connection, PgConnection, RunQueryDsl};
use dotenvy::dotenv;
use log::debug;
use std::env;
use uuid::Uuid;

use super::connection_error::TestContextError;

/// Creates a new DB per test context and drops it at the end of the object's scope
/// Separate tests are then independent of each other
///
/// # Examples
///
/// ```no_run
/// let _ctx = TestDbContext::new()?;
/// let ephemeral_db_url = &_ctx.get_ephemeral_db_url();
/// let repo = PostgresMeesignRepo::new(ephemeral_db_url).unwrap(); // connect to the emphemeral db
///
/// // run some tests using the repo object
/// repo.add_device(&identifier, first_device_name, &vec![1, 2, 3]).await.unwrap();
///
/// // at the end of the scope, _ctx is dropped and the ephemeral DB is removed
/// ```
#[derive(Debug)]
pub struct TestDbContext {
    /// connection string without the specified database
    base_url: String,
    /// an ephemeral DB name specific to a test
    db_name: String,
}

// TODO: consider writing a macro that would create a test context for us
impl TestDbContext {
    pub fn new() -> Result<Self, TestContextError> {
        dotenv().ok();

        let base_url = env::var("DB_BASE_URL").expect("DB_BASE_URL must be set");
        let db_name = Self::generate_db_name();
        Self::create_test_db(&base_url, &db_name)?;

        Ok(Self {
            base_url: base_url.into(),
            db_name: db_name.into(),
        })
    }

    /// Generates a unique DB name in the form meesign_test_db_{UUID} where UUID
    /// is a freshly-generated UUID without hyphens as postgres DB name can contain
    /// only alphanumeric chars + underscores
    fn generate_db_name() -> String {
        format!(
            "meesign_test_db_{}",
            Uuid::new_v4().to_string().replace("-", "")
        )
    }

    /// Returns a connection to `{base_url}/{db_name}` db
    pub fn connect_to_db(base_url: &str, db_name: &str) -> Result<PgConnection, TestContextError> {
        let db_url = format!("{}/{}", base_url, db_name);
        Ok(PgConnection::establish(&db_url)?)
    }

    /// Creates an ephemeral DB
    fn create_test_db(base_url: &str, db_name: &str) -> Result<(), TestContextError> {
        let mut conn = Self::connect_to_db(base_url, "postgres")?;
        // NOTE: we can't use the bind() function as postgres doesn't allow value binding for CREATE DATABASE queries
        // Warning: don't reuse the code-snippet as it is vulnerable to SQL injection!
        let query = diesel::sql_query(format!("CREATE DATABASE {}", db_name));
        query
            .execute(&mut conn)
            .expect(format!("Could not create database {}", db_name).as_str());
        Ok(())
    }

    /// Disconnects all users connected to the `self.db_name` database
    ///
    /// Required for dropping the DB, or else PG will refuse to drop
    /// the DB as some connections may still be active
    fn disconnect_users(&self, conn: &mut PgConnection) -> Result<usize, TestContextError> {
        Ok(diesel::sql_query(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1",
        )
        .bind::<Text, _>(&self.db_name)
        .execute(conn)?)
    }

    /// drops the `self.db_name` DB
    fn drop_db(&self, conn: &mut PgConnection) {
        // NOTE: we can't use the bind() sunction as postgres doesn't allow value binding for DROP DATABASE queries
        // Warning: don't reuse the code-snippet as it is vulnerable to SQL injection!
        let query = diesel::sql_query(format!("DROP DATABASE {}", &self.db_name));
        query
            .execute(conn)
            .expect(&format!("Couldn't drop database {}", self.db_name));
    }

    pub(crate) fn get_ephemeral_db_url(&self) -> String {
        format!("{}/{}", self.base_url, self.db_name)
    }
}

impl Drop for TestDbContext {
    fn drop(&mut self) {
        debug!("Dropping {:#?}", &self);
        let mut conn = Self::connect_to_db(&self.base_url, "postgres")
            .expect("Coudln't connect to postgres DB");
        self.disconnect_users(&mut conn)
            .expect("Coudln't disconnect users");
        self.drop_db(&mut conn);
    }
}
