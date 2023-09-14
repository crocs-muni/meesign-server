CREATE TABLE TaskResult (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    signed_data bytea,
    result_type TaskResultType,
    signing_group_id INT REFERENCES SigningGroup(id)
);