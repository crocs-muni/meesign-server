// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "keytype"))]
    pub struct Keytype;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "protocoltype"))]
    pub struct Protocoltype;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "taskresulttype"))]
    pub struct Taskresulttype;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "taskstate"))]
    pub struct Taskstate;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "tasktype"))]
    pub struct Tasktype;
}

diesel::table! {
    device (id) {
        id -> Int4,
        identifier -> Bytea,
        device_name -> Varchar,
        device_certificate -> Bytea,
        last_active -> Timestamp,
    }
}

diesel::table! {
    groupparticipant (id) {
        id -> Int4,
        device_id -> Nullable<Int4>,
        group_id -> Nullable<Int4>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Protocoltype;
    use super::sql_types::Keytype;

    signinggroup (id) {
        id -> Int4,
        identifier -> Bytea,
        group_name -> Varchar,
        threshold -> Int4,
        protocol -> Protocoltype,
        round -> Int4,
        key_type -> Keytype,
        group_certificate -> Nullable<Bytea>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Tasktype;
    use super::sql_types::Taskstate;

    task (id) {
        id -> Int4,
        protocol_round -> Int4,
        error_message -> Nullable<Varchar>,
        group_id -> Nullable<Int4>,
        task_type -> Tasktype,
        task_state -> Taskstate,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Taskresulttype;

    taskresult (id) {
        id -> Int4,
        signed_data -> Nullable<Bytea>,
        result_type -> Nullable<Taskresulttype>,
        signing_group_id -> Nullable<Int4>,
    }
}

diesel::joinable!(groupparticipant -> device (device_id));
diesel::joinable!(groupparticipant -> signinggroup (group_id));
diesel::joinable!(task -> signinggroup (group_id));
diesel::joinable!(taskresult -> signinggroup (signing_group_id));

diesel::allow_tables_to_appear_in_same_query!(
    device,
    groupparticipant,
    signinggroup,
    task,
    taskresult,
);
