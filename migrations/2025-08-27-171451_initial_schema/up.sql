CREATE TYPE protocol_type AS ENUM (
    'Gg18', 'ElGamal', 'Frost', 'Musig2'
);

CREATE TYPE key_type AS ENUM (
    'SignPdf', 'SignChallenge', 'Decrypt'
);

CREATE TYPE task_type AS ENUM (
    'Group', 'SignPdf', 'SignChallenge', 'Decrypt'
);

CREATE TYPE task_state AS ENUM (
    'Created', 'Running', 'Finished', 'Failed'
);

CREATE TYPE task_result_type AS ENUM (
    'GroupEstablished', 'Signed', 'SignedPdf', 'Decrypted'
);

CREATE TYPE device_kind as ENUM (
    'User', 'Bot'
);

CREATE TABLE device (
    "id" bytea PRIMARY KEY,
    "name" varchar NOT NULL,
    "kind" device_kind NOT NULL,
    "certificate" bytea NOT NULL
);

CREATE TABLE "group" (
    "id" bytea PRIMARY KEY,
    "name" varchar NOT NULL,
    "threshold" integer NOT NULL CHECK ("threshold" > 0),
    "protocol" protocol_type NOT NULL,
    "key_type" key_type NOT NULL,
    "certificate" bytea,
    "note" varchar
);

CREATE TABLE group_participant (
    "group_id" bytea NOT NULL REFERENCES "group"("id"),
    "device_id" bytea NOT NULL REFERENCES device("id"),
    "shares" integer NOT NULL CHECK ("shares" > 0),
    PRIMARY KEY ("group_id", "device_id")
);

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE TABLE task (
    "id" uuid DEFAULT uuid_generate_v4() PRIMARY KEY,
    "protocol_round" integer NOT NULL CHECK ("protocol_round" >= 0),
    "attempt_count" integer NOT NULL CHECK ("attempt_count" >= 0),
    "threshold" integer NOT NULL CHECK ("threshold" > 0),
    "task_data" bytea,
    "preprocessed" bytea,
    "request" bytea NOT NULL,
    "group_id" bytea REFERENCES "group"(id),
    "task_type" task_type NOT NULL,
    "task_state" task_state NOT NULL,
    "key_type" key_type NOT NULL,
    "protocol_type" protocol_type NOT NULL,
    "note" varchar,
    "group_certificates_sent" boolean
);

CREATE TABLE task_result (
    "task_id" uuid NOT NULL REFERENCES task("id"),
    "is_successful" boolean NOT NULL,
    "data" bytea,
    "error_message" varchar,
    PRIMARY KEY ("task_id"),
    CHECK (
        ("is_successful" = TRUE AND "data" IS NOT NULL AND "error_message" IS NULL) OR
        ("is_successful" = FALSE AND "data" IS NULL AND "error_message" IS NOT NULL)
    )
);

CREATE TABLE task_participant (
    "task_id" uuid NOT NULL REFERENCES task("id"),
    "device_id" bytea NOT NULL REFERENCES device("id"),
    "shares" integer NOT NULL CHECK ("shares" > 0),
    "decision" boolean,
    "acknowledgment" boolean,
    PRIMARY KEY ("task_id", "device_id")
);
