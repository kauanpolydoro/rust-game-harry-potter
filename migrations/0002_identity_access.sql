CREATE TABLE guest_identities (
    id UUID PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE rooms (
    id UUID PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'open',
    host_participant_id UUID NOT NULL,
    recovery_password_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT rooms_code_format CHECK (code ~ '^[23456789ABCDEFGHJKLMNPQRSTUVWXYZ]{8}$'),
    CONSTRAINT rooms_status_valid CHECK (status IN ('open', 'sealed', 'cancelled')),
    CONSTRAINT rooms_password_is_argon2id CHECK (recovery_password_hash LIKE '$argon2id$%')
);

CREATE TABLE participants (
    id UUID PRIMARY KEY,
    room_id UUID NOT NULL REFERENCES rooms(id),
    guest_identity_id UUID NOT NULL REFERENCES guest_identities(id),
    display_name TEXT NOT NULL,
    role TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT participants_display_name_length CHECK (
        char_length(display_name) BETWEEN 1 AND 40
    ),
    CONSTRAINT participants_role_valid CHECK (role IN ('host', 'guest')),
    UNIQUE (room_id, id),
    UNIQUE (room_id, guest_identity_id)
);

CREATE UNIQUE INDEX participants_one_host_per_room
    ON participants (room_id)
    WHERE role = 'host';

ALTER TABLE rooms
    ADD CONSTRAINT rooms_host_belongs_to_room
    FOREIGN KEY (id, host_participant_id)
    REFERENCES participants (room_id, id)
    DEFERRABLE INITIALLY DEFERRED;

CREATE TABLE guest_sessions (
    id UUID PRIMARY KEY,
    guest_identity_id UUID NOT NULL REFERENCES guest_identities(id),
    token TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (clock_timestamp() + INTERVAL '30 days'),
    CONSTRAINT guest_sessions_token_format CHECK (token ~ '^[0-9a-f]{64}$'),
    CONSTRAINT guest_sessions_expiry_valid CHECK (expires_at > created_at)
);

CREATE TABLE device_sessions (
    id UUID PRIMARY KEY,
    guest_session_id UUID NOT NULL UNIQUE REFERENCES guest_sessions(id),
    participant_id UUID NOT NULL REFERENCES participants(id),
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT device_sessions_status_valid CHECK (
        status IN ('active', 'revoked', 'replaced', 'expired')
    )
);

CREATE TABLE room_creation_requests (
    idempotency_key TEXT PRIMARY KEY,
    room_id UUID NOT NULL REFERENCES rooms(id) DEFERRABLE INITIALLY DEFERRED,
    participant_id UUID NOT NULL REFERENCES participants(id) DEFERRABLE INITIALLY DEFERRED,
    guest_session_id UUID NOT NULL REFERENCES guest_sessions(id) DEFERRABLE INITIALLY DEFERRED,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT room_creation_requests_key_length CHECK (
        char_length(idempotency_key) BETWEEN 8 AND 128
    )
);
