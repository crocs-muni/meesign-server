CREATE TABLE Task (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    protocol_round integer NOT NULL CHECK (protocol_round > 0),
    error_message varchar,
    group_id INT REFERENCES SigningGroup(id),
    task_type TaskType NOT NULL,
    task_state TaskState NOT NULL
);