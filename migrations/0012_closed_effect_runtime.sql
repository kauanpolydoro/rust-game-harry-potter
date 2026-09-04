ALTER TABLE game_events
    DROP CONSTRAINT game_events_version_supported,
    ADD CONSTRAINT game_events_version_supported CHECK (event_version IN (1, 2));

CREATE FUNCTION valid_effect_outcome(effect JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    expected JSONB;
BEGIN
    IF jsonb_typeof(effect) <> 'object'
       OR jsonb_typeof(effect -> 'rule_id') <> 'string'
       OR effect ->> 'rule_id' = ''
    THEN
        RETURN FALSE;
    END IF;

    CASE effect ->> 'type'
        WHEN 'die_rolled' THEN
            expected := jsonb_build_object(
                'type', 'die_rolled',
                'rule_id', effect -> 'rule_id',
                'die', effect -> 'die',
                'result', effect -> 'result'
            );
            RETURN effect = expected
                AND effect ->> 'die' IN ('d4', 'd6', 'd8')
                AND jsonb_typeof(effect -> 'result') = 'number'
                AND effect ->> 'result' ~ '^[1-9][0-9]*$'
                AND (effect ->> 'result')::NUMERIC <= CASE effect ->> 'die'
                    WHEN 'd4' THEN 4
                    WHEN 'd6' THEN 6
                    WHEN 'd8' THEN 8
                END;
        WHEN 'moved' THEN
            expected := jsonb_build_object(
                'type', 'moved',
                'rule_id', effect -> 'rule_id',
                'target_id', effect -> 'target_id',
                'from', effect -> 'from',
                'to', effect -> 'to'
            );
            IF effect ? 'target_position' THEN
                expected := expected || jsonb_build_object(
                    'target_position', effect -> 'target_position'
                );
            END IF;
            RETURN effect = expected
                AND jsonb_typeof(effect -> 'target_id') = 'string'
                AND effect ->> 'target_id' <> ''
                AND effect ->> 'from' IN (
                    'active_location', 'active_villains', 'dark_arts_deck',
                    'dark_arts_discard', 'hero_discard_pile', 'hero_draw_pile',
                    'hero_hand', 'hero_play_area', 'heroes', 'hogwarts_deck',
                    'market', 'villain_deck'
                )
                AND effect ->> 'to' IN (
                    'active_location', 'active_villains', 'dark_arts_deck',
                    'dark_arts_discard', 'hero_discard_pile', 'hero_draw_pile',
                    'hero_hand', 'hero_play_area', 'heroes', 'hogwarts_deck',
                    'market', 'villain_deck'
                )
                AND (
                    NOT effect ? 'target_position'
                    OR (
                        jsonb_typeof(effect -> 'target_position') = 'number'
                        AND effect ->> 'target_position' ~ '^[1-4]$'
                    )
                );
        WHEN 'no_op' THEN
            expected := jsonb_build_object(
                'type', 'no_op',
                'rule_id', effect -> 'rule_id',
                'reason', effect -> 'reason'
            );
            RETURN effect = expected
                AND effect ->> 'reason' IN (
                    'explicit', 'no_eligible_target', 'zero_cardinality'
                );
        WHEN 'resource_changed' THEN
            expected := jsonb_build_object(
                'type', 'resource_changed',
                'rule_id', effect -> 'rule_id',
                'target_id', effect -> 'target_id',
                'resource', effect -> 'resource',
                'before', effect -> 'before',
                'after', effect -> 'after',
                'cause', effect -> 'cause'
            );
            IF effect ? 'target_position' THEN
                expected := expected || jsonb_build_object(
                    'target_position', effect -> 'target_position'
                );
            END IF;
            RETURN effect = expected
                AND jsonb_typeof(effect -> 'target_id') = 'string'
                AND effect ->> 'target_id' <> ''
                AND effect ->> 'resource' IN ('attack', 'control', 'health', 'influence')
                AND effect ->> 'cause' IN ('cost', 'effect')
                AND jsonb_typeof(effect -> 'before') = 'number'
                AND effect ->> 'before' ~ '^(0|[1-9][0-9]*)$'
                AND (effect ->> 'before')::NUMERIC <= 65535
                AND jsonb_typeof(effect -> 'after') = 'number'
                AND effect ->> 'after' ~ '^(0|[1-9][0-9]*)$'
                AND (effect ->> 'after')::NUMERIC <= 65535
                AND (
                    NOT effect ? 'target_position'
                    OR (
                        jsonb_typeof(effect -> 'target_position') = 'number'
                        AND effect ->> 'target_position' ~ '^[1-4]$'
                    )
                );
        WHEN 'terminal' THEN
            expected := jsonb_build_object(
                'type', 'terminal',
                'rule_id', effect -> 'rule_id',
                'outcome', effect -> 'outcome'
            );
            RETURN effect = expected AND effect ->> 'outcome' IN ('lost', 'won');
        ELSE
            RETURN FALSE;
    END CASE;
END;
$$;

CREATE FUNCTION valid_effect_choice(choice JSONB)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
RETURN choice = jsonb_build_object(
        'id', choice -> 'id',
        'responsible_position', choice -> 'responsible_position',
        'kind', choice -> 'kind',
        'options', choice -> 'options',
        'min', choice -> 'min',
        'max', choice -> 'max'
    )
    AND jsonb_typeof(choice -> 'id') = 'string'
    AND choice ->> 'id' <> ''
    AND jsonb_typeof(choice -> 'responsible_position') = 'number'
    AND choice ->> 'responsible_position' ~ '^[1-4]$'
    AND choice ->> 'kind' IN ('effect', 'target')
    AND jsonb_typeof(choice -> 'options') = 'array'
    AND jsonb_array_length(choice -> 'options') >= 2
    AND jsonb_array_length(choice -> 'options') <= 4096
    AND NOT EXISTS (
        SELECT 1
        FROM jsonb_array_elements(choice -> 'options') AS option
        WHERE jsonb_typeof(option) <> 'string' OR option #>> '{}' = ''
    )
    AND jsonb_typeof(choice -> 'min') = 'number'
    AND choice ->> 'min' ~ '^(0|[1-9][0-9]*)$'
    AND jsonb_typeof(choice -> 'max') = 'number'
    AND choice ->> 'max' ~ '^(0|[1-9][0-9]*)$'
    AND (choice ->> 'min')::NUMERIC <= (choice ->> 'max')::NUMERIC
    AND (choice ->> 'max')::NUMERIC <= jsonb_array_length(choice -> 'options');

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

UPDATE application_metadata
SET value = '12'
WHERE key = 'schema_version';
