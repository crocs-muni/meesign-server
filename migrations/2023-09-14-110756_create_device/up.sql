CREATE TABLE Device (
    id bytea PRIMARY KEY,
    device_name varchar UNIQUE NOT NULL,
    device_certificate bytea NOT NULL,
    last_active timestamptz NOT NULL DEFAULT NOW()
);