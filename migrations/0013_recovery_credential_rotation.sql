ALTER TABLE rooms
    ADD COLUMN password_generation BIGINT NOT NULL DEFAULT 1,
    ADD COLUMN recovery_epoch BIGINT NOT NULL DEFAULT 1,
    ADD COLUMN security_event_sequence BIGINT NOT NULL DEFAULT 0,
    ADD CONSTRAINT rooms_password_generation_positive CHECK (password_generation > 0),
    ADD CONSTRAINT rooms_recovery_epoch_positive CHECK (recovery_epoch > 0),
    ADD CONSTRAINT rooms_security_event_sequence_nonnegative CHECK (security_event_sequence >= 0);

ALTER TABLE participants
    ADD COLUMN recovery_generation BIGINT NOT NULL DEFAULT 1,
    ADD CONSTRAINT participants_recovery_generation_positive CHECK (recovery_generation > 0);

ALTER TABLE recovery_credentials
    DROP CONSTRAINT recovery_credentials_status_valid,
    DROP CONSTRAINT recovery_credentials_consumption_consistent,
    ADD COLUMN recovery_password_hash TEXT,
    ADD COLUMN recovery_epoch BIGINT NOT NULL DEFAULT 1,
    ADD COLUMN password_generation BIGINT NOT NULL DEFAULT 1,
    ADD COLUMN recovery_generation BIGINT NOT NULL DEFAULT 1,
    ADD COLUMN superseded_at TIMESTAMPTZ,
    ADD CONSTRAINT recovery_credentials_generation_positive CHECK (
        recovery_epoch > 0
        AND password_generation > 0
        AND recovery_generation > 0
    ),
    ADD CONSTRAINT recovery_credentials_status_valid CHECK (
        status IN ('active', 'consumed', 'superseded')
    ),
    ADD CONSTRAINT recovery_credentials_lifecycle_consistent CHECK (
        (
            status = 'active'
            AND recovery_attempt_id IS NULL
            AND consumed_by_guest_session_id IS NULL
            AND consumed_at IS NULL
            AND superseded_at IS NULL
        )
        OR (
            status = 'consumed'
            AND recovery_attempt_id IS NOT NULL
            AND consumed_by_guest_session_id IS NOT NULL
            AND consumed_at IS NOT NULL
            AND superseded_at IS NULL
        )
        OR (
            status = 'superseded'
            AND recovery_attempt_id IS NULL
            AND consumed_by_guest_session_id IS NULL
            AND consumed_at IS NULL
            AND superseded_at IS NOT NULL
        )
    );

DROP TRIGGER recovery_credentials_require_matching_session ON recovery_credentials;

UPDATE recovery_credentials
SET recovery_password_hash = rooms.recovery_password_hash
FROM participants
JOIN rooms ON rooms.id = participants.room_id
WHERE participants.id = recovery_credentials.participant_id;

ALTER TABLE recovery_credentials
    ALTER COLUMN recovery_password_hash SET NOT NULL,
    ADD CONSTRAINT recovery_credentials_password_is_argon2id CHECK (
        recovery_password_hash LIKE '$argon2id$%'
    );

CREATE CONSTRAINT TRIGGER recovery_credentials_require_matching_session
AFTER INSERT OR UPDATE ON recovery_credentials
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION require_recovery_session_for_same_participant();

CREATE FUNCTION snapshot_recovery_credential_authority()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    SELECT
        rooms.recovery_password_hash,
        rooms.recovery_epoch,
        rooms.password_generation,
        participants.recovery_generation
    INTO
        NEW.recovery_password_hash,
        NEW.recovery_epoch,
        NEW.password_generation,
        NEW.recovery_generation
    FROM participants
    JOIN rooms ON rooms.id = participants.room_id
    WHERE participants.id = NEW.participant_id;
    RETURN NEW;
END;
$$;

CREATE TRIGGER recovery_credentials_snapshot_current_authority
BEFORE INSERT OR UPDATE OF participant_id ON recovery_credentials
FOR EACH ROW
EXECUTE FUNCTION snapshot_recovery_credential_authority();

CREATE TABLE identity_security_events (
    room_id UUID NOT NULL REFERENCES rooms(id),
    sequence BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    actor_participant_id UUID NOT NULL,
    target_participant_id UUID,
    delivery TEXT,
    password_generation BIGINT,
    recovery_generation BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (room_id, sequence),
    CONSTRAINT identity_security_events_actor_belongs_to_room
        FOREIGN KEY (room_id, actor_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT identity_security_events_target_belongs_to_room
        FOREIGN KEY (room_id, target_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT identity_security_events_sequence_positive CHECK (sequence > 0),
    CONSTRAINT identity_security_events_shape CHECK (
        (
            event_type = 'recovery_password_rotated'
            AND target_participant_id IS NULL
            AND delivery IS NULL
            AND password_generation IS NOT NULL
            AND password_generation > 0
            AND recovery_generation IS NULL
        )
        OR (
            event_type = 'recovery_credential_regenerated'
            AND target_participant_id IS NOT NULL
            AND delivery IN ('direct', 'host_assisted')
            AND password_generation IS NULL
            AND recovery_generation IS NOT NULL
            AND recovery_generation > 0
            AND (
                (
                    delivery = 'direct'
                    AND actor_participant_id = target_participant_id
                )
                OR (
                    delivery = 'host_assisted'
                    AND actor_participant_id <> target_participant_id
                )
            )
        )
    )
);

CREATE TABLE identity_security_event_recipients (
    room_id UUID NOT NULL,
    security_event_sequence BIGINT NOT NULL,
    participant_id UUID NOT NULL,
    PRIMARY KEY (room_id, security_event_sequence, participant_id),
    CONSTRAINT identity_security_event_recipients_event
        FOREIGN KEY (room_id, security_event_sequence)
        REFERENCES identity_security_events (room_id, sequence)
        ON DELETE CASCADE,
    CONSTRAINT identity_security_event_recipients_participant_belongs_to_room
        FOREIGN KEY (room_id, participant_id)
        REFERENCES participants (room_id, id)
);

CREATE TABLE recovery_password_rotation_requests (
    idempotency_key TEXT PRIMARY KEY,
    room_id UUID NOT NULL REFERENCES rooms(id),
    actor_participant_id UUID NOT NULL,
    request_fingerprint TEXT NOT NULL,
    password_generation BIGINT,
    security_event_sequence BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT recovery_password_rotation_actor_belongs_to_room
        FOREIGN KEY (room_id, actor_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT recovery_password_rotation_event
        FOREIGN KEY (room_id, security_event_sequence)
        REFERENCES identity_security_events (room_id, sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT recovery_password_rotation_key_length CHECK (
        char_length(idempotency_key) BETWEEN 8 AND 128
    ),
    CONSTRAINT recovery_password_rotation_fingerprint_format CHECK (
        request_fingerprint ~ '^hmac-sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT recovery_password_rotation_completion_consistent CHECK (
        (
            password_generation IS NULL
            AND security_event_sequence IS NULL
            AND completed_at IS NULL
        )
        OR (
            password_generation IS NOT NULL
            AND password_generation > 0
            AND security_event_sequence IS NOT NULL
            AND completed_at IS NOT NULL
        )
    )
);

CREATE TABLE recovery_credential_regeneration_requests (
    idempotency_key TEXT PRIMARY KEY,
    room_id UUID NOT NULL REFERENCES rooms(id),
    actor_participant_id UUID NOT NULL,
    target_participant_id UUID NOT NULL,
    delivery TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    recovery_generation BIGINT,
    security_event_sequence BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT recovery_credential_regeneration_actor_belongs_to_room
        FOREIGN KEY (room_id, actor_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT recovery_credential_regeneration_target_belongs_to_room
        FOREIGN KEY (room_id, target_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT recovery_credential_regeneration_event
        FOREIGN KEY (room_id, security_event_sequence)
        REFERENCES identity_security_events (room_id, sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT recovery_credential_regeneration_key_length CHECK (
        char_length(idempotency_key) BETWEEN 8 AND 128
    ),
    CONSTRAINT recovery_credential_regeneration_delivery_valid CHECK (
        delivery IN ('direct', 'host_assisted')
    ),
    CONSTRAINT recovery_credential_regeneration_delivery_consistent CHECK (
        (
            delivery = 'direct'
            AND actor_participant_id = target_participant_id
        )
        OR (
            delivery = 'host_assisted'
            AND actor_participant_id <> target_participant_id
        )
    ),
    CONSTRAINT recovery_credential_regeneration_fingerprint_format CHECK (
        request_fingerprint ~ '^hmac-sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT recovery_credential_regeneration_completion_consistent CHECK (
        (
            recovery_generation IS NULL
            AND security_event_sequence IS NULL
            AND completed_at IS NULL
        )
        OR (
            recovery_generation IS NOT NULL
            AND recovery_generation > 0
            AND security_event_sequence IS NOT NULL
            AND completed_at IS NOT NULL
        )
    )
);

UPDATE application_metadata
SET value = '13'
WHERE key = 'schema_version';
