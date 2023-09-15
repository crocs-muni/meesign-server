CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE TABLE Task (
    id uuid DEFAULT uuid_generate_v4() PRIMARY KEY,
    protocol_round integer NOT NULL CHECK (protocol_round > 0),
    attempt_count integer NOT NULL CHECK (attempt_count > 0),
    error_message varchar,
    threshold integer NOT NULL CHECK (threshold > 0),
    last_update timestamptz NOT NULL DEFAULT NOW(),
    task_data bytea,
    preprocessed bytea,
    request bytea,
    group_id INT REFERENCES SigningGroup(id),
    task_type TaskType NOT NULL,
    task_state TaskState NOT NULL,
    key_type KeyType,
    protocol_type ProtocolType
);