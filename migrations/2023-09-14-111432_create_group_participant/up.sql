CREATE TABLE GroupParticipant (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    device_id INT REFERENCES Device(id),
    group_id INT REFERENCES SigningGroup(id)
);