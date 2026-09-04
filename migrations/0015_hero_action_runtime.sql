CREATE FUNCTION valid_effect_target_binding(binding JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF jsonb_typeof(binding) IS DISTINCT FROM 'object'
       OR binding IS DISTINCT FROM jsonb_build_object(
            'selector_id', binding -> 'selector_id',
            'target_ids', binding -> 'target_ids'
       )
       OR jsonb_typeof(binding -> 'selector_id') IS DISTINCT FROM 'string'
       OR binding ->> 'selector_id' = ''
       OR char_length(binding ->> 'selector_id') > 256
       OR jsonb_typeof(binding -> 'target_ids') IS DISTINCT FROM 'array'
    THEN
        RETURN FALSE;
    END IF;

    RETURN jsonb_array_length(binding -> 'target_ids') <= 4096
        AND NOT EXISTS (
            SELECT 1
            FROM jsonb_array_elements(binding -> 'target_ids') AS target_id
            WHERE jsonb_typeof(target_id) <> 'string'
               OR target_id #>> '{}' = ''
               OR char_length(target_id #>> '{}') > 256
        )
        AND jsonb_array_length(binding -> 'target_ids') = (
            SELECT COUNT(DISTINCT target_id #>> '{}')
            FROM jsonb_array_elements(binding -> 'target_ids') AS target_id
        );
END;
$$;

CREATE FUNCTION game_event_matches_command(command_type TEXT, event_type TEXT)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
RETURN CASE command_type
    WHEN 'complete_dark_arts' THEN event_type = 'dark_arts_completed'
    WHEN 'resolve_choice' THEN event_type = 'choice_resolved'
    WHEN 'play_card' THEN event_type = 'card_played'
    WHEN 'assign_attack' THEN event_type = 'attack_assigned'
    WHEN 'acquire_card' THEN event_type = 'card_acquired'
    ELSE FALSE
END;

CREATE OR REPLACE FUNCTION valid_game_event_v3(
    payload JSONB,
    relational_event_type TEXT,
    relational_sequence BIGINT,
    relational_state_version BIGINT,
    relational_actor_position SMALLINT,
    committed_prng_counter BIGINT,
    committed_status TEXT
)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    expected JSONB;
BEGIN
    IF relational_actor_position IS NULL
       OR jsonb_typeof(payload) IS DISTINCT FROM 'object'
       OR jsonb_typeof(payload -> 'event_version') IS DISTINCT FROM 'number'
       OR payload ->> 'event_version' <> '3'
       OR jsonb_typeof(payload -> 'sequence') IS DISTINCT FROM 'number'
       OR payload ->> 'sequence' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(payload -> 'state_version') IS DISTINCT FROM 'number'
       OR payload ->> 'state_version' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(payload -> 'turn') IS DISTINCT FROM 'number'
       OR payload ->> 'turn' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(payload -> 'actor_position') IS DISTINCT FROM 'number'
       OR payload ->> 'actor_position' !~ '^[1-4]$'
       OR jsonb_typeof(payload -> 'effects') IS DISTINCT FROM 'array'
    THEN
        RETURN FALSE;
    END IF;

    IF (payload ->> 'turn')::NUMERIC > 4294967295
       OR jsonb_array_length(payload -> 'effects') > 4096
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(payload -> 'effects') AS effect
            WHERE valid_effect_outcome(effect) IS NOT TRUE
       )
    THEN
        RETURN FALSE;
    END IF;

    expected := jsonb_build_object(
        'event_version', 3,
        'type', relational_event_type,
        'sequence', relational_sequence,
        'state_version', relational_state_version,
        'turn', payload -> 'turn',
        'actor_position', relational_actor_position,
        'effects', payload -> 'effects'
    );

    IF relational_event_type IN ('dark_arts_completed', 'choice_resolved', 'card_played') THEN
        IF jsonb_typeof(payload -> 'effect_stop') IS DISTINCT FROM 'string'
           OR payload ->> 'effect_stop' NOT IN ('stable', 'choice', 'terminal')
           OR jsonb_typeof(payload -> 'prng_counter') IS DISTINCT FROM 'number'
           OR payload ->> 'prng_counter' !~ '^(0|[1-9][0-9]*)$'
        THEN
            RETURN FALSE;
        END IF;

        IF (payload ->> 'prng_counter')::NUMERIC > 9223372036854775807
           OR (payload ->> 'prng_counter')::BIGINT <> committed_prng_counter
        THEN
            RETURN FALSE;
        END IF;

        IF payload ->> 'effect_stop' = 'choice' THEN
            IF valid_effect_choice_v3(payload -> 'choice') IS NOT TRUE THEN
                RETURN FALSE;
            END IF;
        ELSIF payload -> 'choice' IS DISTINCT FROM 'null'::jsonb THEN
            RETURN FALSE;
        END IF;

        expected := expected || jsonb_build_object(
            'effect_stop', payload -> 'effect_stop',
            'choice', payload -> 'choice',
            'prng_counter', payload -> 'prng_counter'
        );
    END IF;

    CASE relational_event_type
        WHEN 'dark_arts_completed' THEN
            NULL;
        WHEN 'choice_resolved' THEN
            IF jsonb_typeof(payload -> 'choice_id') IS DISTINCT FROM 'string'
               OR payload ->> 'choice_id' = ''
               OR char_length(payload ->> 'choice_id') > 256
               OR jsonb_typeof(payload -> 'choice_cause') IS DISTINCT FROM 'string'
               OR payload ->> 'choice_cause' = ''
               OR char_length(payload ->> 'choice_cause') > 256
               OR jsonb_typeof(payload -> 'selected_options') IS DISTINCT FROM 'array'
            THEN
                RETURN FALSE;
            END IF;

            IF jsonb_array_length(payload -> 'selected_options') > 32
               OR EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(payload -> 'selected_options') AS option
                    WHERE jsonb_typeof(option) <> 'string'
                       OR option #>> '{}' = ''
                       OR char_length(option #>> '{}') > 256
               )
               OR EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(payload -> 'selected_options') AS option
                    GROUP BY option
                    HAVING COUNT(*) > 1
               )
            THEN
                RETURN FALSE;
            END IF;

            expected := expected || jsonb_build_object(
                'choice_id', payload -> 'choice_id',
                'choice_cause', payload -> 'choice_cause',
                'selected_options', payload -> 'selected_options'
            );
        WHEN 'card_played' THEN
            IF jsonb_typeof(payload -> 'card_id') IS DISTINCT FROM 'string'
               OR payload ->> 'card_id' = ''
               OR char_length(payload ->> 'card_id') > 256
               OR jsonb_typeof(payload -> 'targets') IS DISTINCT FROM 'array'
               OR payload ->> 'effect_stop' NOT IN ('stable', 'choice', 'terminal')
            THEN
                RETURN FALSE;
            END IF;

            IF jsonb_array_length(payload -> 'targets') > 4096
               OR EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(payload -> 'targets') AS binding
                    WHERE valid_effect_target_binding(binding) IS NOT TRUE
               )
               OR EXISTS (
                    SELECT binding ->> 'selector_id'
                    FROM jsonb_array_elements(payload -> 'targets') AS binding
                    GROUP BY binding ->> 'selector_id'
                    HAVING COUNT(*) > 1
               )
            THEN
                RETURN FALSE;
            END IF;

            expected := expected || jsonb_build_object(
                'card_id', payload -> 'card_id',
                'targets', payload -> 'targets'
            );
        WHEN 'attack_assigned' THEN
            IF jsonb_typeof(payload -> 'villain_id') IS DISTINCT FROM 'string'
               OR payload ->> 'villain_id' = ''
               OR char_length(payload ->> 'villain_id') > 256
               OR jsonb_typeof(payload -> 'amount') IS DISTINCT FROM 'number'
               OR payload ->> 'amount' !~ '^[1-9][0-9]*$'
               OR (payload ->> 'amount')::NUMERIC > 65535
            THEN
                RETURN FALSE;
            END IF;

            expected := expected || jsonb_build_object(
                'villain_id', payload -> 'villain_id',
                'amount', payload -> 'amount'
            );
        WHEN 'card_acquired' THEN
            IF jsonb_typeof(payload -> 'card_id') IS DISTINCT FROM 'string'
               OR payload ->> 'card_id' = ''
               OR char_length(payload ->> 'card_id') > 256
               OR jsonb_typeof(payload -> 'cost') IS DISTINCT FROM 'number'
               OR payload ->> 'cost' !~ '^(0|[1-9][0-9]*)$'
               OR (payload ->> 'cost')::NUMERIC > 65535
               OR (
                    jsonb_typeof(payload -> 'refill_card_id') NOT IN ('null', 'string')
                    OR (
                        jsonb_typeof(payload -> 'refill_card_id') = 'string'
                        AND (
                            payload ->> 'refill_card_id' = ''
                            OR char_length(payload ->> 'refill_card_id') > 256
                        )
                    )
               )
            THEN
                RETURN FALSE;
            END IF;

            expected := expected || jsonb_build_object(
                'card_id', payload -> 'card_id',
                'cost', payload -> 'cost',
                'refill_card_id', payload -> 'refill_card_id'
            );
        ELSE
            RETURN FALSE;
    END CASE;

    IF relational_event_type IN ('dark_arts_completed', 'choice_resolved', 'card_played')
       AND payload ->> 'effect_stop' = 'terminal'
    THEN
        IF committed_status NOT IN ('lost', 'won')
           OR jsonb_array_length(payload -> 'effects') = 0
           OR payload -> 'effects' -> -1 ->> 'type' <> 'terminal'
           OR payload -> 'effects' -> -1 ->> 'outcome' <> committed_status
        THEN
            RETURN FALSE;
        END IF;
    ELSIF committed_status <> 'in_progress'
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(payload -> 'effects') AS effect
            WHERE effect ->> 'type' = 'terminal'
       )
    THEN
        RETURN FALSE;
    END IF;

    RETURN payload = expected;
END;
$$;

CREATE OR REPLACE FUNCTION require_contiguous_game_event_sequence()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actor_position SMALLINT;
    committed_prng_counter BIGINT;
    committed_sequence BIGINT;
    committed_status TEXT;
    committed_state_version BIGINT;
    expected_sequence BIGINT;
BEGIN
    SELECT sequence, state_version, prng_counter, status
    INTO committed_sequence, committed_state_version, committed_prng_counter, committed_status
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

    IF NEW.event_version = 3 THEN
        IF NEW.event_type NOT IN (
            'dark_arts_completed',
            'choice_resolved',
            'card_played',
            'attack_assigned',
            'card_acquired'
        ) THEN
            RAISE EXCEPTION 'game event type is not supported by the current codec'
                USING ERRCODE = '23514';
        END IF;

        IF valid_game_event_v3(
            NEW.payload,
            NEW.event_type,
            NEW.sequence,
            NEW.state_version,
            actor_position,
            committed_prng_counter,
            committed_status
        ) IS NOT TRUE
        THEN
            RAISE EXCEPTION 'game event payload must match the v3 codec shape'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    IF NEW.event_type <> 'dark_arts_completed' OR NEW.event_version NOT IN (1, 2) THEN
        RAISE EXCEPTION 'game event type is not supported by the current codec'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.event_version = 1 THEN
        IF jsonb_typeof(NEW.payload) <> 'object'
           OR jsonb_typeof(NEW.payload -> 'event_version') <> 'number'
           OR NEW.payload ->> 'event_version' <> '1'
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
    END IF;

    IF jsonb_typeof(NEW.payload) <> 'object'
       OR jsonb_typeof(NEW.payload -> 'event_version') <> 'number'
       OR NEW.payload ->> 'event_version' <> '2'
       OR jsonb_typeof(NEW.payload -> 'sequence') <> 'number'
       OR NEW.payload ->> 'sequence' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(NEW.payload -> 'state_version') <> 'number'
       OR NEW.payload ->> 'state_version' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(NEW.payload -> 'turn') <> 'number'
       OR NEW.payload ->> 'turn' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(NEW.payload -> 'actor_position') <> 'number'
       OR NEW.payload ->> 'actor_position' !~ '^[1-9][0-9]*$'
       OR jsonb_typeof(NEW.payload -> 'effects') <> 'array'
       OR jsonb_array_length(NEW.payload -> 'effects') > 4096
       OR NEW.payload ->> 'effect_stop' NOT IN ('stable', 'choice', 'terminal')
       OR jsonb_typeof(NEW.payload -> 'prng_counter') <> 'number'
       OR NEW.payload ->> 'prng_counter' !~ '^(0|[1-9][0-9]*)$'
    THEN
        RAISE EXCEPTION 'game event payload must match the current codec shape'
            USING ERRCODE = '23514';
    END IF;

    IF (NEW.payload ->> 'turn')::NUMERIC > 4294967295
       OR (NEW.payload ->> 'prng_counter')::NUMERIC > 9223372036854775807
       OR (NEW.payload ->> 'prng_counter')::BIGINT <> committed_prng_counter
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(NEW.payload -> 'effects') AS effect
            WHERE valid_effect_outcome(effect) IS NOT TRUE
       )
    THEN
        RAISE EXCEPTION 'game event effect payload is invalid'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.payload ->> 'effect_stop' = 'choice' THEN
        IF valid_effect_choice(NEW.payload -> 'choice') IS NOT TRUE THEN
            RAISE EXCEPTION 'game event choice payload is invalid'
                USING ERRCODE = '23514';
        END IF;
    ELSIF NEW.payload -> 'choice' <> 'null'::jsonb THEN
        RAISE EXCEPTION 'game event choice must match its stop point'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.payload ->> 'effect_stop' = 'terminal' THEN
        IF committed_status NOT IN ('lost', 'won')
           OR jsonb_array_length(NEW.payload -> 'effects') = 0
           OR NEW.payload -> 'effects' -> -1 ->> 'type' <> 'terminal'
           OR NEW.payload -> 'effects' -> -1 ->> 'outcome' <> committed_status
        THEN
            RAISE EXCEPTION 'terminal effect stop must match the committed terminal status'
                USING ERRCODE = '23514';
        END IF;
    ELSIF committed_status <> 'in_progress'
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(NEW.payload -> 'effects') AS effect
            WHERE effect ->> 'type' = 'terminal'
       )
    THEN
        RAISE EXCEPTION 'non-terminal effect stop must match an in-progress game'
            USING ERRCODE = '23514';
    END IF;

    IF actor_position IS NOT NULL AND NEW.payload <> jsonb_build_object(
            'event_version', NEW.event_version,
            'type', NEW.event_type,
            'sequence', NEW.sequence,
            'state_version', NEW.state_version,
            'turn', NEW.payload -> 'turn',
            'actor_position', actor_position,
            'effects', NEW.payload -> 'effects',
            'effect_stop', NEW.payload -> 'effect_stop',
            'choice', NEW.payload -> 'choice',
            'prng_counter', NEW.payload -> 'prng_counter'
        )
    THEN
        RAISE EXCEPTION 'game event payload metadata must match its relational envelope'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION require_game_transition_history()
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
        IF NEW.snapshot_version IS DISTINCT FROM OLD.snapshot_version
           OR NEW.status IS DISTINCT FROM OLD.status
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

    IF NEW.snapshot_version IS DISTINCT FROM OLD.snapshot_version
       AND NOT (OLD.snapshot_version = 1 AND NEW.snapshot_version = 2)
    THEN
        RAISE EXCEPTION 'snapshot version may only be promoted from 1 to 2'
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
         AND game_event_matches_command(receipts.command_type, events.event_type)
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

CREATE OR REPLACE FUNCTION require_game_event_receipt()
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
          AND game_event_matches_command(receipts.command_type, NEW.event_type)
          AND receipts.expires_at = games.expires_at
    ) THEN
        RAISE EXCEPTION 'official game event requires a matching command receipt'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

UPDATE application_metadata
SET value = '15'
WHERE key = 'schema_version';
