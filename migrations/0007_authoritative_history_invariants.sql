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

    IF NEW.event_version <> 1 OR NEW.event_type <> 'dark_arts_completed' THEN
        RAISE EXCEPTION 'game event type is not supported by the current codec'
            USING ERRCODE = '23514';
    END IF;

    IF jsonb_typeof(NEW.payload) <> 'object'
       OR jsonb_typeof(NEW.payload -> 'event_version') <> 'number'
       OR NEW.payload ->> 'event_version' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(NEW.payload -> 'sequence') <> 'number'
       OR NEW.payload ->> 'sequence' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(NEW.payload -> 'state_version') <> 'number'
       OR NEW.payload ->> 'state_version' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(NEW.payload -> 'turn') <> 'number'
       OR NEW.payload ->> 'turn' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(NEW.payload -> 'actor_position') <> 'number'
       OR NEW.payload ->> 'actor_position' !~ '^[1-9][0-9]*$'
    THEN
        RAISE EXCEPTION 'game event payload must match the current codec shape'
            USING ERRCODE = '23514';
    END IF;

    IF (NEW.payload ->> 'turn')::NUMERIC > 4294967295 THEN
        RAISE EXCEPTION 'game event turn exceeds the current codec range'
            USING ERRCODE = '23514';
    END IF;

    IF actor_position IS NOT NULL AND NEW.payload <> jsonb_build_object(
            'event_version', NEW.event_version,
            'type', NEW.event_type,
            'sequence', NEW.sequence,
            'state_version', NEW.state_version,
            'turn', NEW.payload -> 'turn',
            'actor_position', actor_position
        )
    THEN
        RAISE EXCEPTION 'game event payload metadata must match its relational envelope'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE FUNCTION require_game_transition_history()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id
       OR NEW.room_id IS DISTINCT FROM OLD.room_id
       OR NEW.started_by_participant_id IS DISTINCT FROM OLD.started_by_participant_id
       OR NEW.adventure_id IS DISTINCT FROM OLD.adventure_id
       OR NEW.adventure_name IS DISTINCT FROM OLD.adventure_name
       OR NEW.manifest_digest IS DISTINCT FROM OLD.manifest_digest
       OR NEW.manifest_version IS DISTINCT FROM OLD.manifest_version
       OR NEW.content_version IS DISTINCT FROM OLD.content_version
       OR NEW.ruleset_version IS DISTINCT FROM OLD.ruleset_version
       OR NEW.snapshot_version IS DISTINCT FROM OLD.snapshot_version
       OR NEW.prng_algorithm IS DISTINCT FROM OLD.prng_algorithm
       OR NEW.prng_seed IS DISTINCT FROM OLD.prng_seed
       OR NEW.shuffle_algorithm IS DISTINCT FROM OLD.shuffle_algorithm
       OR NEW.sampling_algorithm IS DISTINCT FROM OLD.sampling_algorithm
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'started game identity and algorithms are immutable'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.sequence = OLD.sequence AND NEW.state_version = OLD.state_version THEN
        IF NEW.status IS DISTINCT FROM OLD.status
           OR NEW.state_digest IS DISTINCT FROM OLD.state_digest
           OR NEW.snapshot::text IS DISTINCT FROM OLD.snapshot::text
           OR NEW.prng_counter IS DISTINCT FROM OLD.prng_counter
        THEN
            RAISE EXCEPTION 'authoritative game state cannot change without advancing its cursor'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    IF NEW.sequence <> OLD.sequence + 1 OR NEW.state_version <> OLD.state_version + 1 THEN
        RAISE EXCEPTION 'game state must advance by exactly one official transition'
            USING ERRCODE = '23514';
    END IF;

    IF NOT EXISTS (
        SELECT 1
        FROM game_events AS events
        JOIN game_command_receipts AS receipts
          ON receipts.game_id = events.game_id
         AND receipts.room_id = events.room_id
         AND receipts.accepted_sequence = events.sequence
         AND receipts.command_id = events.command_id
         AND receipts.actor_participant_id = events.actor_participant_id
         AND receipts.accepted_state_version = events.state_version
         AND receipts.command_type = 'complete_dark_arts'
         AND receipts.expires_at = NEW.expires_at
        WHERE events.game_id = NEW.id
          AND events.room_id = NEW.room_id
          AND events.sequence = NEW.sequence
          AND events.state_version = NEW.state_version
    ) THEN
        RAISE EXCEPTION 'game transition requires a matching official event and receipt'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE FUNCTION require_game_event_receipt()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM game_command_receipts AS receipts
        JOIN games
          ON games.id = receipts.game_id
         AND games.room_id = receipts.room_id
        WHERE receipts.game_id = NEW.game_id
          AND receipts.room_id = NEW.room_id
          AND receipts.accepted_sequence = NEW.sequence
          AND receipts.command_id = NEW.command_id
          AND receipts.actor_participant_id = NEW.actor_participant_id
          AND receipts.accepted_state_version = NEW.state_version
          AND receipts.command_type = 'complete_dark_arts'
          AND receipts.expires_at = games.expires_at
    ) THEN
        RAISE EXCEPTION 'official game event requires a matching command receipt'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE CONSTRAINT TRIGGER game_state_transitions_have_history
AFTER UPDATE ON games
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION require_game_transition_history();

CREATE CONSTRAINT TRIGGER game_events_have_receipts
AFTER INSERT ON game_events
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION require_game_event_receipt();

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
