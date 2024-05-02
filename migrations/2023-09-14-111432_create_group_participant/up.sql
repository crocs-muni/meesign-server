CREATE TABLE group_participant (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id bytea NOT NULL REFERENCES device(id),
    group_id INT NOT NULL REFERENCES "group"(id)
);