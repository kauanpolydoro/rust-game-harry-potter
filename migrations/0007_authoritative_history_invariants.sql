ALTER TABLE games
    ADD CONSTRAINT games_id_room_unique UNIQUE (id, room_id),
    ADD CONSTRAINT games_snapshot_version_supported CHECK (snapshot_version = 1),
    ADD CONSTRAINT games_state_version_tracks_sequence
        CHECK (state_version - sequence = 1);

ALTER TABLE game_events
    DROP CONSTRAINT game_events_game_id_fkey,
    ADD CONSTRAINT game_events_game_id_fkey
        FOREIGN KEY (game_id) REFERENCES games(id);

ALTER TABLE game_command_receipts
    DROP CONSTRAINT game_command_receipts_game_id_fkey,
    ADD CONSTRAINT game_command_receipts_game_id_fkey
        FOREIGN KEY (game_id) REFERENCES games(id);

ALTER TABLE game_events
    ADD COLUMN room_id UUID;

UPDATE game_events AS events
SET room_id = games.room_id
FROM games
WHERE games.id = events.game_id;

ALTER TABLE game_events
    ALTER COLUMN room_id SET NOT NULL,
    ADD CONSTRAINT game_events_version_supported CHECK (event_version = 1),
    ADD CONSTRAINT game_events_game_room
        FOREIGN KEY (game_id, room_id)
        REFERENCES games (id, room_id),
    ADD CONSTRAINT game_events_actor_belongs_to_room
        FOREIGN KEY (room_id, actor_participant_id)
        REFERENCES participants (room_id, id),
    ADD CONSTRAINT game_events_receipt_identity_unique
        UNIQUE (
            game_id,
            room_id,
            sequence,
            command_id,
            actor_participant_id,
            state_version
        );

ALTER TABLE game_command_receipts
    ADD COLUMN room_id UUID;

UPDATE game_command_receipts AS receipts
SET room_id = games.room_id
FROM games
WHERE games.id = receipts.game_id;

ALTER TABLE game_command_receipts
    ALTER COLUMN room_id SET NOT NULL,
    ADD CONSTRAINT game_command_receipts_game_room
        FOREIGN KEY (game_id, room_id)
        REFERENCES games (id, room_id),
    ADD CONSTRAINT game_command_receipts_actor_belongs_to_room
        FOREIGN KEY (room_id, actor_participant_id)
        REFERENCES participants (room_id, id),
    ADD CONSTRAINT game_command_receipts_single_transition
        CHECK (accepted_state_version - expected_state_version = 1),
    ADD CONSTRAINT game_command_receipts_match_event
        FOREIGN KEY (
            game_id,
            room_id,
            accepted_sequence,
            command_id,
            actor_participant_id,
            accepted_state_version
        )
        REFERENCES game_events (
            game_id,
            room_id,
            sequence,
            command_id,
            actor_participant_id,
            state_version
        );

CREATE OR REPLACE FUNCTION require_contiguous_game_event_sequence()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actor_position SMALLINT;
    committed_sequence BIGINT;
    committed_state_version BIGINT;
    expected_sequence BIGINT;
BEGIN
    SELECT sequence, state_version
    INTO committed_sequence, committed_state_version
    FROM games
    WHERE id = NEW.game_id
    FOR UPDATE;

    SELECT COALESCE(MAX(sequence), 0) + 1
    INTO expected_sequence
    FROM game_events
    WHERE game_id = NEW.game_id;

    IF NEW.sequence <> expected_sequence OR NEW.sequence <> committed_sequence THEN
        RAISE EXCEPTION 'game event sequence must be contiguous with the committed snapshot'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.state_version <> committed_state_version THEN
        RAISE EXCEPTION 'game event state version must match the committed snapshot'
            USING ERRCODE = '23514';
    END IF;

    SELECT position
    INTO actor_position
    FROM participants
    WHERE room_id = NEW.room_id
      AND id = NEW.actor_participant_id;

    IF actor_position IS NOT NULL AND NOT (
        NEW.payload @> jsonb_build_object(
            'event_version', NEW.event_version,
            'type', NEW.event_type,
            'sequence', NEW.sequence,
            'state_version', NEW.state_version,
            'actor_position', actor_position
        )
    ) THEN
        RAISE EXCEPTION 'game event payload metadata must match its relational envelope'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE FUNCTION reject_official_history_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'official game history is append-only'
        USING ERRCODE = '55000';
END;
$$;

CREATE TRIGGER game_events_are_append_only
BEFORE UPDATE OR DELETE ON game_events
FOR EACH ROW
EXECUTE FUNCTION reject_official_history_mutation();

CREATE TRIGGER game_command_receipts_are_append_only
BEFORE UPDATE OR DELETE ON game_command_receipts
FOR EACH ROW
EXECUTE FUNCTION reject_official_history_mutation();

UPDATE application_metadata
SET value = '7'
WHERE key = 'schema_version';
