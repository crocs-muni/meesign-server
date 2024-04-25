CREATE TABLE TaskParticipant (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    group_participant_id INT REFERENCES GroupParticipant(id),
    task_id uuid REFERENCES Task(id),
    decision BOOLEAN,
    acknowledgment BOOLEAN
);