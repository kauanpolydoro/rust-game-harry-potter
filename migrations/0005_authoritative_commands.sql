ALTER TABLE games
    ADD COLUMN last_game_action_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    ADD COLUMN expires_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp() + INTERVAL '7 days',
    ADD CONSTRAINT games_expiration_after_action CHECK (expires_at > last_game_action_at);

CREATE TABLE game_events (
    game_id UUID NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    sequence BIGINT NOT NULL,
    event_version SMALLINT NOT NULL DEFAULT 1,
    event_type TEXT NOT NULL,
    command_id UUID NOT NULL,
    actor_participant_id UUID NOT NULL REFERENCES participants(id),
    state_version BIGINT NOT NULL,
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (game_id, sequence),
    UNIQUE (game_id, command_id),
    CONSTRAINT game_events_sequence_positive CHECK (sequence > 0),
    CONSTRAINT game_events_version_positive CHECK (event_version > 0),
    CONSTRAINT game_events_state_version_positive CHECK (state_version > 0),
    CONSTRAINT game_events_type_present CHECK (event_type <> '')
);

CREATE TABLE game_command_receipts (
    game_id UUID NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    command_id UUID NOT NULL,
    actor_participant_id UUID NOT NULL REFERENCES participants(id),
    command_type TEXT NOT NULL,
    expected_state_version BIGINT NOT NULL,
    payload_digest TEXT NOT NULL,
    accepted_state_version BIGINT NOT NULL,
    accepted_sequence BIGINT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (game_id, command_id),
    CONSTRAINT game_command_receipts_event
        FOREIGN KEY (game_id, accepted_sequence)
        REFERENCES game_events (game_id, sequence),
    CONSTRAINT game_command_receipts_expected_version_positive
        CHECK (expected_state_version > 0),
    CONSTRAINT game_command_receipts_accepted_version_advances
        CHECK (accepted_state_version > expected_state_version),
    CONSTRAINT game_command_receipts_sequence_positive CHECK (accepted_sequence > 0),
    CONSTRAINT game_command_receipts_type_present CHECK (command_type <> ''),
    CONSTRAINT game_command_receipts_digest_format
        CHECK (payload_digest ~ '^blake3:[0-9a-f]{64}$')
);
