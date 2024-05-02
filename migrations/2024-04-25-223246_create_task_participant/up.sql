CREATE TABLE task_participant (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id bytea NOT NULL REFERENCES device(id),
    task_id uuid NOT NULL REFERENCES task(id),
    decision BOOLEAN,
    acknowledgment BOOLEAN
);