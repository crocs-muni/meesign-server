CREATE TABLE "group" (
    "id" INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    "identifier" bytea UNIQUE NOT NULL,
    "name" varchar NOT NULL,
    "threshold" integer NOT NULL CHECK (threshold > 0),
    "protocol" protocol_type NOT NULL,
    "round" integer NOT NULL CHECK(round >= 0),
    "key_type" key_type NOT NULL,
    "certificate" bytea,
    "note" VARCHAR
);