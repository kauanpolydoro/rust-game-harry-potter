ALTER TABLE participants
    ADD COLUMN position SMALLINT;

UPDATE participants
SET position = 1
WHERE role = 'host';

ALTER TABLE participants
    ALTER COLUMN position SET NOT NULL,
    ADD COLUMN hero_id TEXT,
    ADD CONSTRAINT participants_position_valid CHECK (position BETWEEN 1 AND 4),
    ADD CONSTRAINT participants_guest_has_hero CHECK (role <> 'guest' OR hero_id IS NOT NULL),
    ADD CONSTRAINT participants_hero_valid CHECK (
        hero_id IS NULL OR hero_id IN ('harry', 'hermione', 'neville', 'ron')
    ),
    ADD CONSTRAINT participants_room_position_unique UNIQUE (room_id, position),
    ADD CONSTRAINT participants_room_hero_unique UNIQUE (room_id, hero_id);

CREATE TABLE room_join_requests (
    idempotency_key TEXT PRIMARY KEY,
    room_id UUID NOT NULL REFERENCES rooms(id) DEFERRABLE INITIALLY DEFERRED,
    participant_id UUID NOT NULL REFERENCES participants(id) DEFERRABLE INITIALLY DEFERRED,
    guest_session_id UUID NOT NULL REFERENCES guest_sessions(id) DEFERRABLE INITIALLY DEFERRED,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT room_join_requests_key_length CHECK (
        char_length(idempotency_key) BETWEEN 8 AND 128
    )
);
