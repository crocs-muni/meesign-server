CREATE TABLE device (
    "id" bytea PRIMARY KEY,
    "name" varchar UNIQUE NOT NULL,
    "certificate" bytea NOT NULL,
    "last_active" timestamptz NOT NULL DEFAULT NOW()
);