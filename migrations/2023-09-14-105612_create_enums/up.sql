CREATE TYPE ProtocolType AS ENUM (
    'GG18', 'ElGamal', 'Frost'
);

CREATE TYPE KeyType AS ENUM (
    'SignPDF', 'SignChallenge', 'Decrypt'
);

CREATE TYPE TaskType AS ENUM (
    'Group', 'SignPdf', 'SignChallenge', 'Decrypt'
);

CREATE TYPE TaskState AS ENUM (
    'Created', 'Running', 'Finished', 'Failed'
);

CREATE TYPE TaskResultType AS ENUM (
    'GroupEstablished', 'Signed', 'SignedPdf', 'Decrypted'
);