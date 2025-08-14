-- postgres does not support removing enum values
-- we must use a workaround

-- create a new enum type without 'Musig2'
CREATE TYPE protocol_type_new AS ENUM (
    'Gg18', 'ElGamal', 'Frost'
);

-- alter columns using 'protocol_type' to use 'protocol_type_new'
ALTER TABLE task
  ALTER COLUMN protocol_type TYPE protocol_type_new
  USING protocol_type::text::protocol_type_new;

ALTER TABLE "group"
  ALTER COLUMN protocol TYPE protocol_type_new
  USING protocol::text::protocol_type_new;

-- drop the old enum
DROP TYPE protocol_type;

-- rename the new enum to the original name
ALTER TYPE protocol_type_new RENAME TO protocol_type;
