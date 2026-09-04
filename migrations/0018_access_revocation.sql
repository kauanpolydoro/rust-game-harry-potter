ALTER TABLE identity_security_events
    DROP CONSTRAINT identity_security_events_shape,
    ADD COLUMN session_slot SMALLINT,
    ADD COLUMN revoked_session_count BIGINT,
    ADD COLUMN recovery_epoch BIGINT,
    ADD COLUMN current_session_preserved BOOLEAN,
    ADD CONSTRAINT identity_security_events_session_slot_valid CHECK (
        session_slot IS NULL OR session_slot BETWEEN 1 AND 2
    ),
    ADD CONSTRAINT identity_security_events_revoked_session_count_valid CHECK (
        revoked_session_count IS NULL OR revoked_session_count >= 0
    ),
    ADD CONSTRAINT identity_security_events_recovery_epoch_valid CHECK (
        recovery_epoch IS NULL OR recovery_epoch > 0
    ),
    ADD CONSTRAINT identity_security_events_shape CHECK (
        (
            event_type = 'recovery_password_rotated'
            AND target_participant_id IS NULL
            AND delivery IS NULL
            AND password_generation IS NOT NULL
            AND password_generation > 0
            AND recovery_generation IS NULL
            AND session_slot IS NULL
            AND revoked_session_count IS NULL
            AND recovery_epoch IS NULL
            AND current_session_preserved IS NULL
        )
        OR (
            event_type = 'recovery_credential_regenerated'
            AND target_participant_id IS NOT NULL
            AND delivery IN ('direct', 'host_assisted')
            AND password_generation IS NULL
            AND recovery_generation IS NOT NULL
            AND recovery_generation > 0
            AND session_slot IS NULL
            AND revoked_session_count IS NULL
            AND recovery_epoch IS NULL
            AND current_session_preserved IS NULL
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
        OR (
            event_type = 'session_revoked'
            AND target_participant_id IS NOT NULL
            AND target_participant_id = actor_participant_id
            AND delivery IS NULL
            AND password_generation IS NULL
            AND recovery_generation IS NULL
            AND session_slot IS NOT NULL
            AND session_slot BETWEEN 1 AND 2
            AND revoked_session_count IS NULL
            AND recovery_epoch IS NULL
            AND current_session_preserved IS NULL
        )
        OR (
            event_type = 'participant_protected'
            AND target_participant_id IS NOT NULL
            AND target_participant_id = actor_participant_id
            AND delivery IS NULL
            AND password_generation IS NULL
            AND recovery_generation IS NOT NULL
            AND recovery_generation > 1
            AND session_slot IS NULL
            AND revoked_session_count IS NOT NULL
            AND revoked_session_count > 0
            AND recovery_epoch IS NULL
            AND current_session_preserved IS NULL
        )
        OR (
            event_type = 'room_protected'
            AND target_participant_id IS NULL
            AND delivery IS NULL
            AND password_generation IS NOT NULL
            AND password_generation > 1
            AND recovery_generation IS NULL
            AND session_slot IS NULL
            AND revoked_session_count IS NOT NULL
            AND revoked_session_count >= 0
            AND recovery_epoch IS NOT NULL
            AND recovery_epoch > 1
            AND current_session_preserved IS NOT NULL
        )
    );

CREATE TABLE device_session_revocation_requests (
    idempotency_key TEXT PRIMARY KEY,
    room_id UUID NOT NULL REFERENCES rooms(id),
    actor_participant_id UUID NOT NULL,
    target_device_session_id UUID NOT NULL REFERENCES device_sessions(id),
    request_fingerprint TEXT NOT NULL,
    revoked_session_slot SMALLINT,
    security_event_sequence BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT device_session_revocation_actor_belongs_to_room
        FOREIGN KEY (room_id, actor_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT device_session_revocation_event
        FOREIGN KEY (room_id, security_event_sequence)
        REFERENCES identity_security_events (room_id, sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT device_session_revocation_key_length CHECK (
        char_length(idempotency_key) BETWEEN 8 AND 128
    ),
    CONSTRAINT device_session_revocation_fingerprint_format CHECK (
        request_fingerprint ~ '^hmac-sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT device_session_revocation_slot_valid CHECK (
        revoked_session_slot IS NULL OR revoked_session_slot BETWEEN 1 AND 2
    ),
    CONSTRAINT device_session_revocation_completion_consistent CHECK (
        (
            revoked_session_slot IS NULL
            AND security_event_sequence IS NULL
            AND completed_at IS NULL
        )
        OR (
            revoked_session_slot IS NOT NULL
            AND security_event_sequence IS NOT NULL
            AND completed_at IS NOT NULL
        )
    )
);

CREATE TABLE participant_protection_requests (
    idempotency_key TEXT PRIMARY KEY,
    room_id UUID NOT NULL REFERENCES rooms(id),
    actor_participant_id UUID NOT NULL,
    actor_guest_session_id UUID NOT NULL REFERENCES guest_sessions(id),
    request_fingerprint TEXT NOT NULL,
    recovery_generation BIGINT,
    revoked_session_count BIGINT,
    security_event_sequence BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT participant_protection_actor_belongs_to_room
        FOREIGN KEY (room_id, actor_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT participant_protection_event
        FOREIGN KEY (room_id, security_event_sequence)
        REFERENCES identity_security_events (room_id, sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT participant_protection_key_length CHECK (
        char_length(idempotency_key) BETWEEN 8 AND 128
    ),
    CONSTRAINT participant_protection_fingerprint_format CHECK (
        request_fingerprint ~ '^hmac-sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT participant_protection_completion_consistent CHECK (
        (
            recovery_generation IS NULL
            AND revoked_session_count IS NULL
            AND security_event_sequence IS NULL
            AND completed_at IS NULL
        )
        OR (
            recovery_generation IS NOT NULL
            AND recovery_generation > 1
            AND revoked_session_count IS NOT NULL
            AND revoked_session_count > 0
            AND security_event_sequence IS NOT NULL
            AND completed_at IS NOT NULL
        )
    )
);

CREATE TABLE room_protection_requests (
    idempotency_key TEXT PRIMARY KEY,
    room_id UUID NOT NULL REFERENCES rooms(id),
    actor_participant_id UUID NOT NULL,
    actor_guest_session_id UUID NOT NULL REFERENCES guest_sessions(id),
    request_fingerprint TEXT NOT NULL,
    password_generation BIGINT,
    recovery_epoch BIGINT,
    revoked_session_count BIGINT,
    current_session_preserved BOOLEAN,
    security_event_sequence BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT room_protection_actor_belongs_to_room
        FOREIGN KEY (room_id, actor_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT room_protection_event
        FOREIGN KEY (room_id, security_event_sequence)
        REFERENCES identity_security_events (room_id, sequence)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT room_protection_key_length CHECK (
        char_length(idempotency_key) BETWEEN 8 AND 128
    ),
    CONSTRAINT room_protection_fingerprint_format CHECK (
        request_fingerprint ~ '^hmac-sha256:[0-9a-f]{64}$'
    ),
    CONSTRAINT room_protection_completion_consistent CHECK (
        (
            password_generation IS NULL
            AND recovery_epoch IS NULL
            AND revoked_session_count IS NULL
            AND current_session_preserved IS NULL
            AND security_event_sequence IS NULL
            AND completed_at IS NULL
        )
        OR (
            password_generation IS NOT NULL
            AND password_generation > 1
            AND recovery_epoch IS NOT NULL
            AND recovery_epoch > 1
            AND revoked_session_count IS NOT NULL
            AND revoked_session_count >= 0
            AND current_session_preserved IS NOT NULL
            AND security_event_sequence IS NOT NULL
            AND completed_at IS NOT NULL
        )
    )
);

UPDATE application_metadata
SET value = '18'
WHERE key = 'schema_version';
