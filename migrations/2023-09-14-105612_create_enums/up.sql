CREATE TYPE protocol_type AS ENUM (
    'Gg18', 'ElGamal', 'Frost'
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
)