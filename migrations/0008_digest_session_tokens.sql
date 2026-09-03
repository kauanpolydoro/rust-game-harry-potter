ALTER TABLE guest_sessions
    RENAME COLUMN token TO token_digest;

ALTER TABLE guest_sessions
    RENAME CONSTRAINT guest_sessions_token_key TO guest_sessions_token_digest_key;

ALTER TABLE guest_sessions
    DROP CONSTRAINT guest_sessions_token_format;

UPDATE guest_sessions
SET token_digest = 'sha256:' || encode(
    sha256(convert_to(token_digest, 'UTF8')),
    'hex'
);

ALTER TABLE guest_sessions
    ADD CONSTRAINT guest_sessions_token_digest_format
        CHECK (token_digest ~ '^sha256:[0-9a-f]{64}$');

UPDATE application_metadata
SET value = '8'
WHERE key = 'schema_version';
