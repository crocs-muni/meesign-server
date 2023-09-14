CREATE TABLE Device (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    identifier bytea UNIQUE NOT NULL,
    device_name varchar UNIQUE NOT NULL,
    device_certificate bytea NOT NULL,
    last_active timestamp NOT NULL DEFAULT NOW()
);