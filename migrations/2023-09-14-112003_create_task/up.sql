CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE TABLE task (
    id uuid DEFAULT uuid_generate_v4() PRIMARY KEY,
    protocol_round integer NOT NULL CHECK (protocol_round >= 0),
    attempt_count integer NOT NULL CHECK (attempt_count >= 0),
    error_message varchar,
    threshold integer NOT NULL CHECK (threshold > 0),
    last_update timestamptz NOT NULL DEFAULT NOW(),
    task_data bytea,
    preprocessed bytea,
    request bytea,
    group_id INT REFERENCES "group"(id),
    task_type task_type NOT NULL,
    task_state task_state NOT NULL,
    key_type key_type,
    protocol_type protocol_type
);