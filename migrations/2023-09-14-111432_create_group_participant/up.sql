CREATE TABLE GroupParticipant (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id bytea NOT NULL REFERENCES Device(id),
    group_id INT NOT NULL REFERENCES "group"(id)
);