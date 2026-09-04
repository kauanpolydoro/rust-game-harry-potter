CREATE TABLE recovery_credentials (
    id UUID PRIMARY KEY,
    participant_id UUID NOT NULL REFERENCES participants(id),
    token_hmac TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'active',
    recovery_attempt_id UUID,
    consumed_by_guest_session_id UUID UNIQUE REFERENCES guest_sessions(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    consumed_at TIMESTAMPTZ,
    CONSTRAINT recovery_credentials_token_hmac_format CHECK (
        token_hmac ~ '^hmac-sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT recovery_credentials_status_valid CHECK (
        status IN ('active', 'consumed')
    ),
    CONSTRAINT recovery_credentials_consumption_consistent CHECK (
        (
            status = 'active'
            AND recovery_attempt_id IS NULL
            AND consumed_by_guest_session_id IS NULL
            AND consumed_at IS NULL
        )
        OR (
            status = 'consumed'
            AND recovery_attempt_id IS NOT NULL
            AND consumed_by_guest_session_id IS NOT NULL
            AND consumed_at IS NOT NULL
        )
    )
);

CREATE UNIQUE INDEX recovery_credentials_one_active_per_participant
    ON recovery_credentials (participant_id)
    WHERE status = 'active';

CREATE FUNCTION require_recovery_session_for_same_participant()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.status = 'consumed' AND NOT EXISTS (
        SELECT 1
        FROM device_sessions
        WHERE guest_session_id = NEW.consumed_by_guest_session_id
          AND participant_id = NEW.participant_id
          AND status = 'active'
    ) THEN
        RAISE EXCEPTION 'recovery credential must create an active session for its participant'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE CONSTRAINT TRIGGER recovery_credentials_require_matching_session
AFTER INSERT OR UPDATE ON recovery_credentials
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION require_recovery_session_for_same_participant();

UPDATE application_metadata
SET value = '11'
WHERE key = 'schema_version';
