use diesel_async::{AsyncPgConnection, RunQueryDsl};

use crate::persistence::{models::Group, persistance_error::PersistenceError};

pub async fn get_groups(
    connection: &mut AsyncPgConnection,
) -> Result<Vec<Group>, PersistenceError> {
    use crate::persistence::schema::signinggroup;
    Ok(signinggroup::table.load(connection).await?)
}
