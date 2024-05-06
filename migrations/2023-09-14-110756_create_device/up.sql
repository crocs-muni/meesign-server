CREATE TABLE device (
    "id" bytea PRIMARY KEY,
    "name" varchar UNIQUE NOT NULL,
    "kind" device_kind NOT NULL,
    "certificate" bytea NOT NULL,
    "last_active" timestamptz NOT NULL DEFAULT NOW()
);