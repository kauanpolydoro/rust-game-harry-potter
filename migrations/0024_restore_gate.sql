-- This identity is copied by backups; the replacement identity lives outside them.
-- A restore cannot open traffic until reconciliation commits the new binding.
CREATE TABLE runtime_deployment (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    epoch UUID NOT NULL,
    database_login TEXT NOT NULL,
    credential_fingerprint TEXT NOT NULL,
    ledger_key_fingerprint TEXT,
    ready BOOLEAN NOT NULL DEFAULT TRUE,
    checkpoint_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    restored_from_ms BIGINT,
    reconciled_at TIMESTAMPTZ,
    CONSTRAINT deployment_fingerprint_format CHECK (credential_fingerprint ~ '^[0-9a-f]{64}$')
);

UPDATE application_metadata SET value = '24' WHERE key = 'schema_version';
