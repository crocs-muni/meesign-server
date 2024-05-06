CREATE TABLE task_result (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    task_id uuid UNIQUE NOT NULL REFERENCES "task"(id),
    is_successfull BOOLEAN NOT NULL,
    "data" bytea,
    error_message VARCHAR
);