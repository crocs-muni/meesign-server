CREATE TABLE TaskParticipant (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id bytea NOT NULL REFERENCES Device(id),
    task_id uuid NOT NULL REFERENCES Task(id),
    decision BOOLEAN,
    acknowledgment BOOLEAN
);