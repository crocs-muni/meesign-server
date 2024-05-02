CREATE TABLE task_result (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    signed_data bytea,
    result_type task_result_type,
    signing_group_id INT REFERENCES "group"(id)
);