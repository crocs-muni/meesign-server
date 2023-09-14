CREATE TABLE SigningGroup (
    id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    identifier bytea UNIQUE NOT NULL,
    group_name varchar NOT NULL,
    threshold integer NOT NULL CHECK (threshold > 0),
    protocol ProtocolType NOT NULL,
    round integer NOT NULL CHECK(round >= 0),
    key_type KeyType NOT NULL,
    group_certificate bytea
);