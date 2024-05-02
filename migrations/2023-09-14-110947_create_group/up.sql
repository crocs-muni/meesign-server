CREATE TABLE "group" (
    "id" INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    "identifier" bytea UNIQUE NOT NULL,
    "name" varchar NOT NULL,
    "threshold" integer NOT NULL CHECK (threshold > 0),
    "protocol" ProtocolType NOT NULL,
    "round" integer NOT NULL CHECK(round >= 0),
    "key_type" KeyType NOT NULL,
    "certificate" bytea
);