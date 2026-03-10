// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "device_kind"))]
    pub struct DeviceKind;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "key_type"))]
    pub struct KeyType;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "protocol_type"))]
    pub struct ProtocolType;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "task_state"))]
    pub struct TaskState;

    #[derive(diesel::query_builder::QueryId, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "task_type"))]
    pub struct TaskType;
}

diesel::table! {
    active_task_participant (task_id, device_id) {
        task_id -> Uuid,
        device_id -> Bytea,
        active_shares -> Int4,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::DeviceKind;

    device (id) {
        id -> Bytea,
        name -> Varchar,
        kind -> DeviceKind,
        certificate -> Bytea,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::ProtocolType;
    use super::sql_types::KeyType;

    group (id) {
        id -> Bytea,
        name -> Varchar,
        threshold -> Int4,
        protocol -> ProtocolType,
        key_type -> KeyType,
        certificate -> Nullable<Bytea>,
        note -> Nullable<Varchar>,
    }
}

diesel::table! {
    group_participant (group_id, device_id) {
        group_id -> Bytea,
        device_id -> Bytea,
        shares -> Int4,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::TaskType;
    use super::sql_types::TaskState;
    use super::sql_types::KeyType;
    use super::sql_types::ProtocolType;

    task (id) {
        id -> Uuid,
        protocol_round -> Int4,
        attempt_count -> Int4,
        threshold -> Int4,
        name -> Varchar,
        task_data -> Nullable<Bytea>,
        preprocessed -> Nullable<Bytea>,
        request -> Bytea,
        group_id -> Nullable<Bytea>,
        task_type -> TaskType,
        task_state -> TaskState,
        key_type -> KeyType,
        protocol_type -> ProtocolType,
        note -> Nullable<Varchar>,
        group_certificates_sent -> Nullable<Bool>,
    }
}

diesel::table! {
    task_participant (task_id, device_id) {
        task_id -> Uuid,
        device_id -> Bytea,
        shares -> Int4,
        decision -> Nullable<Bool>,
        acknowledgment -> Nullable<Bool>,
    }
}

diesel::table! {
    task_result (task_id) {
        task_id -> Uuid,
        is_successful -> Bool,
        data -> Nullable<Bytea>,
        error_message -> Nullable<Varchar>,
    }
}

diesel::joinable!(active_task_participant -> device (device_id));
diesel::joinable!(active_task_participant -> task (task_id));
diesel::joinable!(group_participant -> device (device_id));
diesel::joinable!(group_participant -> group (group_id));
diesel::joinable!(task -> group (group_id));
diesel::joinable!(task_participant -> device (device_id));
diesel::joinable!(task_participant -> task (task_id));
diesel::joinable!(task_result -> task (task_id));

diesel::allow_tables_to_appear_in_same_query!(
    active_task_participant,
    device,
    group,
    group_participant,
    task,
    task_participant,
    task_result,
);
