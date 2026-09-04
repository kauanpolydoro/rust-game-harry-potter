LOCK TABLE games IN ACCESS EXCLUSIVE MODE;
LOCK TABLE game_state_anchors IN SHARE ROW EXCLUSIVE MODE;
LOCK TABLE game_events IN ACCESS EXCLUSIVE MODE;
LOCK TABLE game_command_receipts IN SHARE ROW EXCLUSIVE MODE;
LOCK TABLE rooms IN SHARE ROW EXCLUSIVE MODE;
LOCK TABLE participants IN SHARE ROW EXCLUSIVE MODE;

CREATE OR REPLACE FUNCTION game_event_matches_command(command_type TEXT, event_type TEXT)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
RETURN CASE command_type
    WHEN 'end_hero_actions' THEN event_type = 'turn_completed'
    WHEN 'resolve_choice' THEN event_type = 'choice_resolved'
    WHEN 'play_card' THEN event_type = 'card_played'
    WHEN 'assign_attack' THEN event_type = 'attack_assigned'
    WHEN 'acquire_card' THEN event_type = 'card_acquired'
    ELSE FALSE
END;

DO $$
DECLARE
    invalid_game_id UUID;
    invalid_sequence BIGINT;
BEGIN
    SELECT games.id
    INTO invalid_game_id
    FROM games
    LEFT JOIN LATERAL (
        SELECT
            COUNT(*) AS event_count,
            MIN(events.sequence) AS first_sequence,
            MAX(events.sequence) AS last_sequence,
            BOOL_AND(
                events.state_version::NUMERIC = events.sequence::NUMERIC + 1
            ) AS versions_are_contiguous
        FROM game_events AS events
        WHERE events.game_id = games.id
    ) AS history ON TRUE
    WHERE games.state_version::NUMERIC <> games.sequence::NUMERIC + 1
       OR (
            games.sequence = 0
            AND history.event_count <> 0
       )
       OR (
            games.sequence > 0
            AND (
                history.event_count <> games.sequence
                OR history.first_sequence <> 1
                OR history.last_sequence <> games.sequence
                OR history.versions_are_contiguous IS NOT TRUE
            )
       )
    ORDER BY games.id
    LIMIT 1;

    IF invalid_game_id IS NOT NULL THEN
        RAISE EXCEPTION 'game % does not have a complete contiguous transition history',
            invalid_game_id
            USING
                ERRCODE = '55000',
                HINT = 'Repair or archive the game and its event history before applying this migration.';
    END IF;

    SELECT events.game_id, events.sequence
    INTO invalid_game_id, invalid_sequence
    FROM game_events AS events
    WHERE NOT EXISTS (
        SELECT 1
        FROM game_command_receipts AS receipts
        WHERE receipts.game_id = events.game_id
          AND receipts.room_id = events.room_id
          AND receipts.accepted_sequence = events.sequence
          AND receipts.command_id = events.command_id
          AND receipts.actor_participant_id = events.actor_participant_id
          AND receipts.accepted_state_version = events.state_version
          AND receipts.command_type = CASE events.event_type
                WHEN 'dark_arts_completed' THEN 'complete_dark_arts'
                WHEN 'choice_resolved' THEN 'resolve_choice'
                WHEN 'turn_completed' THEN 'end_hero_actions'
                WHEN 'card_played' THEN 'play_card'
                WHEN 'attack_assigned' THEN 'assign_attack'
                WHEN 'card_acquired' THEN 'acquire_card'
                ELSE NULL
              END
    )
    ORDER BY events.game_id, events.sequence
    LIMIT 1;

    IF invalid_game_id IS NOT NULL THEN
        RAISE EXCEPTION 'game event %.% has no matching official command receipt',
            invalid_game_id, invalid_sequence
            USING
                ERRCODE = '55000',
                HINT = 'Restore the missing or inconsistent command receipt before applying this migration.';
    END IF;

    SELECT games.id, games.sequence
    INTO invalid_game_id, invalid_sequence
    FROM games
    WHERE NOT EXISTS (
        SELECT 1
        FROM game_state_anchors AS anchors
        WHERE anchors.game_id = games.id
          AND anchors.sequence = games.sequence
          AND anchors.snapshot_version = games.snapshot_version
          AND anchors.state_digest = games.state_digest
    )
    ORDER BY games.id
    LIMIT 1;

    IF invalid_game_id IS NOT NULL THEN
        RAISE EXCEPTION 'game %.% has no replay anchor for its authoritative state',
            invalid_game_id, invalid_sequence
            USING
                ERRCODE = '55000',
                HINT = 'Restore the missing current replay anchor before applying this migration.';
    END IF;

    SELECT anchors.game_id, anchors.sequence
    INTO invalid_game_id, invalid_sequence
    FROM game_state_anchors AS anchors
    JOIN games ON games.id = anchors.game_id
    WHERE anchors.sequence > games.sequence
       OR (
            anchors.sequence > 0
            AND NOT EXISTS (
                SELECT 1
                FROM game_events AS events
                WHERE events.game_id = anchors.game_id
                  AND events.sequence = anchors.sequence
            )
       )
       OR (
            anchors.sequence = games.sequence
            AND (
                anchors.snapshot_version IS DISTINCT FROM games.snapshot_version
                OR anchors.state_digest IS DISTINCT FROM games.state_digest
            )
       )
    ORDER BY anchors.game_id, anchors.sequence
    LIMIT 1;

    IF invalid_game_id IS NOT NULL THEN
        RAISE EXCEPTION 'game state anchor %.% is ahead of or conflicts with the authoritative game cursor',
            invalid_game_id, invalid_sequence
            USING
                ERRCODE = '55000',
                HINT = 'Repair the invalid replay anchor under an explicit maintenance procedure before applying this migration.';
    END IF;
END;
$$;

CREATE TRIGGER game_events_cannot_be_truncated
BEFORE TRUNCATE ON game_events
FOR EACH STATEMENT
EXECUTE FUNCTION reject_official_history_mutation();

CREATE TRIGGER game_command_receipts_cannot_be_truncated
BEFORE TRUNCATE ON game_command_receipts
FOR EACH STATEMENT
EXECUTE FUNCTION reject_official_history_mutation();

CREATE TRIGGER game_state_anchors_cannot_be_truncated
BEFORE TRUNCATE ON game_state_anchors
FOR EACH STATEMENT
EXECUTE FUNCTION reject_official_history_mutation();

CREATE FUNCTION reject_closed_room_reopening()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.status <> 'open' AND NEW.status = 'open' THEN
        RAISE EXCEPTION 'closed rooms cannot be reopened'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER closed_rooms_cannot_be_reopened
BEFORE UPDATE OF status ON rooms
FOR EACH ROW
EXECUTE FUNCTION reject_closed_room_reopening();

CREATE OR REPLACE FUNCTION reject_sealed_room_participant_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    target_room_ids UUID[];
BEGIN
    target_room_ids := CASE TG_OP
        WHEN 'INSERT' THEN ARRAY[NEW.room_id]
        WHEN 'DELETE' THEN ARRAY[OLD.room_id]
        ELSE ARRAY[OLD.room_id, NEW.room_id]
    END;

    PERFORM id
    FROM rooms
    WHERE id = ANY(target_room_ids)
    ORDER BY id
    FOR UPDATE;

    IF EXISTS (
        SELECT 1
        FROM rooms
        WHERE id = ANY(target_room_ids)
          AND status <> 'open'
    ) THEN
        RAISE EXCEPTION 'sealed room participants cannot change'
            USING ERRCODE = '23514';
    END IF;
    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER sealed_room_participants_are_fixed ON participants;

CREATE TRIGGER sealed_room_participants_are_fixed
BEFORE UPDATE OF id, room_id, position, hero_id, ready ON participants
FOR EACH ROW
EXECUTE FUNCTION reject_sealed_room_participant_change();

CREATE TRIGGER sealed_room_participants_cannot_be_deleted
BEFORE DELETE ON participants
FOR EACH ROW
EXECUTE FUNCTION reject_sealed_room_participant_change();

CREATE FUNCTION compact_jsonb_octet_length(document JSONB)
RETURNS BIGINT
LANGUAGE plpgsql
IMMUTABLE
STRICT
AS $$
DECLARE
    compact_length BIGINT;
BEGIN
    CASE jsonb_typeof(document)
        WHEN 'array' THEN
            SELECT 2
                + GREATEST(COUNT(*) - 1, 0)
                + COALESCE(SUM(compact_jsonb_octet_length(element.value)), 0)
            INTO compact_length
            FROM jsonb_array_elements(document) AS element(value);
        WHEN 'object' THEN
            SELECT 2
                + GREATEST(COUNT(*) - 1, 0)
                + COALESCE(SUM(
                    octet_length(to_jsonb(entry.key)::TEXT)
                    + 1
                    + compact_jsonb_octet_length(entry.value)
                ), 0)
            INTO compact_length
            FROM jsonb_each(document) AS entry(key, value);
        ELSE
            compact_length := octet_length(document::TEXT);
    END CASE;

    RETURN compact_length;
END;
$$;

CREATE FUNCTION valid_v3_effect_choice_for_v15_replay(choice JSONB)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
RETURN valid_effect_choice_v3(choice) IS TRUE
    AND octet_length(choice ->> 'id') <= 256
    AND octet_length(choice ->> 'cause') <= 256
    AND octet_length(
        choice -> 'continuation' -> 'choice_cursor' ->> 'rule_id'
    ) <= 256
    AND NOT EXISTS (
        SELECT 1
        FROM jsonb_array_elements(choice -> 'options') AS option
        WHERE octet_length(option #>> '{}') > 256
    )
    AND NOT EXISTS (
        SELECT 1
        FROM jsonb_array_elements(choice -> 'continuation' -> 'queue') AS queued
        WHERE octet_length(queued -> 'cursor' ->> 'rule_id') > 256
    );

CREATE FUNCTION valid_legacy_game_event_for_replay(
    codec_version SMALLINT,
    codec_type TEXT,
    payload JSONB,
    relational_sequence BIGINT,
    relational_state_version BIGINT,
    relational_actor_position SMALLINT
)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    choice_id TEXT;
    choice_step TEXT;
    expected JSONB;
    terminal_count INTEGER;
BEGIN
    IF compact_jsonb_octet_length(payload) > 4194304
       OR jsonb_typeof(payload) IS DISTINCT FROM 'object'
       OR (
            codec_version IN (1, 2)
            AND codec_type IS DISTINCT FROM 'dark_arts_completed'
       )
       OR (
            codec_version = 3
            AND codec_type NOT IN (
                'dark_arts_completed', 'choice_resolved', 'card_played',
                'attack_assigned', 'card_acquired'
            )
       )
    THEN
        RETURN FALSE;
    END IF;

    IF codec_version = 1 THEN
        RETURN payload = jsonb_build_object(
                'event_version', payload -> 'event_version',
                'type', payload -> 'type',
                'sequence', payload -> 'sequence',
                'state_version', payload -> 'state_version',
                'turn', payload -> 'turn',
                'actor_position', payload -> 'actor_position'
            )
            AND jsonb_typeof(payload -> 'event_version') = 'number'
            AND payload ->> 'event_version' = '1'
            AND jsonb_typeof(payload -> 'type') = 'string'
            AND payload ->> 'type' = 'dark_arts_completed'
            AND jsonb_typeof(payload -> 'sequence') = 'number'
            AND payload ->> 'sequence' ~ '^[1-9][0-9]*$'
            AND (payload ->> 'sequence')::NUMERIC <= 9223372036854775807
            AND jsonb_typeof(payload -> 'state_version') = 'number'
            AND payload ->> 'state_version' ~ '^[1-9][0-9]*$'
            AND (payload ->> 'state_version')::NUMERIC <= 9223372036854775807
            AND jsonb_typeof(payload -> 'turn') = 'number'
            AND payload ->> 'turn' ~ '^[1-9][0-9]*$'
            AND (payload ->> 'turn')::NUMERIC <= 4294967295
            AND jsonb_typeof(payload -> 'actor_position') = 'number'
            AND payload ->> 'actor_position' ~ '^[1-4]$';
    END IF;

    IF codec_version = 3 THEN
        IF codec_type IN ('dark_arts_completed', 'choice_resolved', 'card_played')
           AND (
                jsonb_typeof(payload -> 'prng_counter') IS DISTINCT FROM 'number'
                OR payload ->> 'prng_counter' !~ '^(0|[1-9][0-9]*)$'
                OR (payload ->> 'prng_counter')::NUMERIC > 9223372036854775807
           )
        THEN
            RETURN FALSE;
        END IF;

        IF valid_game_event_v3(
                payload,
                codec_type,
                relational_sequence,
                relational_state_version,
                relational_actor_position,
                COALESCE((payload ->> 'prng_counter')::BIGINT, 0),
                CASE payload ->> 'effect_stop'
                    WHEN 'terminal' THEN COALESCE(
                        payload -> 'effects' -> -1 ->> 'outcome',
                        'in_progress'
                    )
                    ELSE 'in_progress'
                END
           ) IS NOT TRUE
           OR EXISTS (
                SELECT 1
                FROM jsonb_array_elements(payload -> 'effects') AS effect
                WHERE octet_length(effect ->> 'rule_id') > 256
                   OR (
                        effect ->> 'type' IN ('moved', 'resource_changed')
                        AND octet_length(effect ->> 'target_id') > 256
                   )
                   OR (
                        effect ->> 'type' = 'moved'
                        AND (
                            effect ->> 'from' NOT IN (
                                'active_villains', 'dark_arts_deck',
                                'dark_arts_discard', 'hero_discard_pile',
                                'hero_draw_pile', 'hero_hand', 'hero_play_area',
                                'hogwarts_deck', 'market', 'villain_deck'
                            )
                            OR effect ->> 'to' NOT IN (
                                'active_villains', 'dark_arts_deck',
                                'dark_arts_discard', 'hero_discard_pile',
                                'hero_draw_pile', 'hero_hand', 'hero_play_area',
                                'hogwarts_deck', 'market', 'villain_deck'
                            )
                            OR effect ->> 'from' IS NOT DISTINCT FROM effect ->> 'to'
                        )
                   )
           )
           OR (
                payload ->> 'effect_stop' = 'choice'
                AND valid_v3_effect_choice_for_v15_replay(payload -> 'choice')
                    IS NOT TRUE
           )
           OR (
                payload ->> 'effect_stop' = 'terminal'
                AND (
                    SELECT COUNT(*)
                    FROM jsonb_array_elements(payload -> 'effects') AS effect
                    WHERE effect ->> 'type' = 'terminal'
                ) <> 1
           )
           OR (
                codec_type = 'choice_resolved'
                AND (
                    octet_length(payload ->> 'choice_id') > 256
                    OR octet_length(payload ->> 'choice_cause') > 256
                    OR EXISTS (
                        SELECT 1
                        FROM jsonb_array_elements(payload -> 'selected_options') AS option
                        WHERE octet_length(option #>> '{}') > 256
                    )
                )
           )
        THEN
            RETURN FALSE;
        END IF;
        RETURN TRUE;
    END IF;

    IF codec_version <> 2 THEN
        RETURN FALSE;
    END IF;

    expected := jsonb_build_object(
        'event_version', payload -> 'event_version',
        'type', payload -> 'type',
        'sequence', payload -> 'sequence',
        'state_version', payload -> 'state_version',
        'turn', payload -> 'turn',
        'actor_position', payload -> 'actor_position',
        'effects', payload -> 'effects',
        'effect_stop', payload -> 'effect_stop',
        'prng_counter', payload -> 'prng_counter'
    );
    IF payload ? 'choice' THEN
        expected := expected || jsonb_build_object('choice', payload -> 'choice');
    END IF;

    IF payload IS DISTINCT FROM expected
       OR jsonb_typeof(payload -> 'event_version') IS DISTINCT FROM 'number'
       OR payload ->> 'event_version' IS DISTINCT FROM '2'
       OR jsonb_typeof(payload -> 'type') IS DISTINCT FROM 'string'
       OR payload ->> 'type' IS DISTINCT FROM 'dark_arts_completed'
       OR jsonb_typeof(payload -> 'sequence') IS DISTINCT FROM 'number'
       OR payload ->> 'sequence' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'sequence')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(payload -> 'state_version') IS DISTINCT FROM 'number'
       OR payload ->> 'state_version' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'state_version')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(payload -> 'turn') IS DISTINCT FROM 'number'
       OR payload ->> 'turn' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'turn')::NUMERIC > 4294967295
       OR jsonb_typeof(payload -> 'actor_position') IS DISTINCT FROM 'number'
       OR payload ->> 'actor_position' !~ '^[1-4]$'
       OR jsonb_typeof(payload -> 'effects') IS DISTINCT FROM 'array'
       OR jsonb_array_length(payload -> 'effects') > 4096
       OR jsonb_typeof(payload -> 'effect_stop') IS DISTINCT FROM 'string'
       OR payload ->> 'effect_stop' NOT IN ('stable', 'choice', 'terminal')
       OR jsonb_typeof(payload -> 'prng_counter') IS DISTINCT FROM 'number'
       OR payload ->> 'prng_counter' !~ '^(0|[1-9][0-9]*)$'
       OR (payload ->> 'prng_counter')::NUMERIC > 9223372036854775807
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(payload -> 'effects') AS effect
            WHERE valid_effect_outcome(effect) IS NOT TRUE
               OR (
                    effect ->> 'type' = 'moved'
                    AND (
                        effect ->> 'from' NOT IN (
                            'active_villains', 'dark_arts_deck', 'dark_arts_discard',
                            'hero_discard_pile', 'hero_draw_pile', 'hero_hand',
                            'hero_play_area', 'hogwarts_deck', 'market', 'villain_deck'
                        )
                        OR effect ->> 'to' NOT IN (
                            'active_villains', 'dark_arts_deck', 'dark_arts_discard',
                            'hero_discard_pile', 'hero_draw_pile', 'hero_hand',
                            'hero_play_area', 'hogwarts_deck', 'market', 'villain_deck'
                        )
                        OR effect ->> 'from' IS NOT DISTINCT FROM effect ->> 'to'
                    )
               )
       )
    THEN
        RETURN FALSE;
    END IF;

    terminal_count := (
        SELECT COUNT(*)
        FROM jsonb_array_elements(payload -> 'effects') AS effect
        WHERE effect ->> 'type' = 'terminal'
    );

    CASE payload ->> 'effect_stop'
        WHEN 'stable' THEN
            RETURN terminal_count = 0
                AND (NOT payload ? 'choice' OR payload -> 'choice' = 'null'::jsonb);
        WHEN 'terminal' THEN
            RETURN terminal_count = 1
                AND (NOT payload ? 'choice' OR payload -> 'choice' = 'null'::jsonb)
                AND payload -> 'effects' -> -1 ->> 'type' = 'terminal';
        WHEN 'choice' THEN
            IF terminal_count <> 0
               OR valid_effect_choice(payload -> 'choice') IS NOT TRUE
               OR (
                    SELECT COUNT(*)
                    FROM jsonb_array_elements(payload -> 'choice' -> 'options') AS option
               ) <> (
                    SELECT COUNT(DISTINCT option)
                    FROM jsonb_array_elements(payload -> 'choice' -> 'options') AS option
               )
            THEN
                RETURN FALSE;
            END IF;

            choice_id := payload -> 'choice' ->> 'id';
            IF payload -> 'choice' ->> 'kind' = 'effect' THEN
                IF choice_id !~ '^.+:effect:[0-9]+$' THEN
                    RETURN FALSE;
                END IF;
                choice_step := substring(choice_id FROM ':effect:([0-9]+)$');
            ELSE
                IF choice_id !~ '^.+:target:[0-9]+$' THEN
                    RETURN FALSE;
                END IF;
                choice_step := substring(choice_id FROM ':target:([0-9]+)$');
            END IF;
            RETURN choice_step::NUMERIC <= 18446744073709551615;
        ELSE
            RETURN FALSE;
    END CASE;
EXCEPTION
    WHEN OTHERS THEN
        RETURN FALSE;
END;
$$;

DO $$
DECLARE
    invalid_game_id UUID;
    invalid_sequence BIGINT;
BEGIN
    SELECT events.game_id, events.sequence
    INTO invalid_game_id, invalid_sequence
    FROM game_events AS events
    LEFT JOIN participants AS actor
      ON actor.room_id = events.room_id
     AND actor.id = events.actor_participant_id
    WHERE events.event_version IN (1, 2, 3)
      AND (
          actor.id IS NULL
          OR valid_legacy_game_event_for_replay(
                events.event_version,
                events.event_type,
                events.payload,
                events.sequence,
                events.state_version,
                actor.position
             ) IS NOT TRUE
          OR events.payload ->> 'event_version'
                IS DISTINCT FROM events.event_version::TEXT
          OR events.payload ->> 'type' IS DISTINCT FROM events.event_type
          OR events.payload ->> 'sequence' IS DISTINCT FROM events.sequence::TEXT
          OR events.payload ->> 'state_version'
                IS DISTINCT FROM events.state_version::TEXT
          OR events.payload ->> 'actor_position' IS DISTINCT FROM actor.position::TEXT
      )
    ORDER BY events.game_id, events.sequence
    LIMIT 1;

    IF invalid_game_id IS NOT NULL THEN
        RAISE EXCEPTION 'legacy game event %.% cannot be replayed by schema version 15',
            invalid_game_id, invalid_sequence
            USING
                ERRCODE = '55000',
                HINT = 'Repair or quarantine the invalid v1/v2/v3 event before applying this migration.';
    END IF;

    SELECT id
    INTO invalid_game_id
    FROM games
    WHERE compact_jsonb_octet_length(snapshot) > 4194304
    ORDER BY id
    LIMIT 1;

    IF invalid_game_id IS NOT NULL THEN
        RAISE EXCEPTION 'legacy game % snapshot exceeds the schema version 15 codec limit',
            invalid_game_id
            USING
                ERRCODE = '55000',
                HINT = 'Repair or quarantine the oversized snapshot before applying this migration.';
    END IF;

    IF EXISTS (
        SELECT 1
        FROM games
        WHERE status = 'in_progress'
          AND expires_at > clock_timestamp()
          AND snapshot ->> 'snapshot_version' IN ('1', '2')
          AND snapshot -> 'turn' ->> 'phase' = 'dark_arts'
          AND (
              snapshot ->> 'snapshot_version' = '1'
              OR jsonb_typeof(snapshot -> 'effects' -> 'choice')
                    IS DISTINCT FROM 'object'
          )
    ) THEN
        RAISE EXCEPTION 'live legacy games without a resumable choice must finish Dark Arts before schema version 15'
            USING
                ERRCODE = '55000',
                HINT = 'Drain or expire every legacy dark_arts game that has no v2 pending choice before applying this migration.';
    END IF;
END;
$$;

ALTER TABLE games
    DROP CONSTRAINT games_snapshot_version_supported,
    ADD CONSTRAINT games_snapshot_version_supported CHECK (snapshot_version IN (1, 2, 3));

ALTER TABLE game_events
    DROP CONSTRAINT game_events_version_supported,
    ADD CONSTRAINT game_events_version_supported CHECK (event_version IN (1, 2, 3, 4));

CREATE FUNCTION valid_effect_path_segment_v4(segment JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF valid_effect_path_segment_v3(segment) IS NOT TRUE THEN
        RETURN FALSE;
    END IF;

    RETURN CASE segment ->> 'type'
        WHEN 'choice_option' THEN
            (segment ->> 'index')::NUMERIC <= 1023
        WHEN 'sequence_effect' THEN
            (segment ->> 'index')::NUMERIC <= 1023
        WHEN 'roll_outcome' THEN (segment ->> 'index')::NUMERIC < 8
        ELSE TRUE
    END;
EXCEPTION
    WHEN OTHERS THEN
        RETURN FALSE;
END;
$$;

CREATE FUNCTION valid_effect_path_v4(path JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF jsonb_typeof(path) IS DISTINCT FROM 'array'
       OR jsonb_array_length(path) > 32
    THEN
        RETURN FALSE;
    END IF;

    RETURN NOT EXISTS (
        SELECT 1
        FROM jsonb_array_elements(path) AS segment
        WHERE valid_effect_path_segment_v4(segment) IS NOT TRUE
    );
END;
$$;

CREATE FUNCTION valid_effect_cursor_v4(cursor_value JSONB)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
RETURN cursor_value = jsonb_build_object(
        'rule_id', cursor_value -> 'rule_id',
        'path', cursor_value -> 'path'
    )
    AND jsonb_typeof(cursor_value -> 'rule_id') = 'string'
    AND cursor_value ->> 'rule_id' <> ''
    AND octet_length(cursor_value ->> 'rule_id') <= 256
    AND valid_effect_path_v4(cursor_value -> 'path') IS TRUE;

CREATE FUNCTION valid_queued_effect_v4(frame JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF jsonb_typeof(frame) IS DISTINCT FROM 'object'
       OR jsonb_typeof(frame -> 'type') IS DISTINCT FROM 'string'
       OR valid_effect_cursor_v4(frame -> 'cursor') IS NOT TRUE
    THEN
        RETURN FALSE;
    END IF;

    CASE frame ->> 'type'
        WHEN 'definition' THEN
            RETURN frame = jsonb_build_object(
                    'type', frame -> 'type',
                    'cursor', frame -> 'cursor',
                    'actor_position', frame -> 'actor_position'
                )
                AND jsonb_typeof(frame -> 'actor_position') = 'number'
                AND frame ->> 'actor_position' ~ '^[1-4]$';
        WHEN 'effect_choice' THEN
            RETURN frame = jsonb_build_object(
                    'type', frame -> 'type',
                    'cursor', frame -> 'cursor',
                    'responsible_position', frame -> 'responsible_position'
                )
                AND jsonb_typeof(frame -> 'responsible_position') = 'number'
                AND frame ->> 'responsible_position' ~ '^[1-4]$';
        ELSE
            RETURN FALSE;
    END CASE;
END;
$$;

CREATE FUNCTION valid_pending_effect_choice_v4(choice JSONB)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
RETURN valid_effect_choice_v3(choice)
    AND octet_length(choice ->> 'id') <= 256
    AND octet_length(choice ->> 'cause') <= 256
    AND valid_effect_cursor_v4(
        choice -> 'continuation' -> 'choice_cursor'
    ) IS TRUE
    AND NOT EXISTS (
        SELECT 1
        FROM jsonb_array_elements(choice -> 'options') AS option
        WHERE octet_length(option #>> '{}') > 256
    )
    AND NOT EXISTS (
        SELECT 1
        FROM jsonb_array_elements(choice -> 'continuation' -> 'queue') AS queued
        WHERE valid_queued_effect_v4(queued) IS NOT TRUE
    );

CREATE FUNCTION valid_decision_point_v4(decision JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF jsonb_typeof(decision) IS DISTINCT FROM 'object'
       OR jsonb_typeof(decision -> 'type') IS DISTINCT FROM 'string'
    THEN
        RETURN FALSE;
    END IF;

    CASE decision ->> 'type'
        WHEN 'none', 'automatic' THEN
            RETURN decision = jsonb_build_object('type', decision -> 'type');
        WHEN 'player_intent' THEN
            RETURN decision = jsonb_build_object(
                    'type', decision -> 'type',
                    'responsible_position', decision -> 'responsible_position'
                )
                AND jsonb_typeof(decision -> 'responsible_position') = 'number'
                AND decision ->> 'responsible_position' ~ '^[1-4]$';
        WHEN 'effect_choice' THEN
            RETURN decision = jsonb_build_object(
                    'type', decision -> 'type',
                    'choice', decision -> 'choice'
                )
                AND valid_pending_effect_choice_v4(decision -> 'choice');
        ELSE
            RETURN FALSE;
    END CASE;
END;
$$;

CREATE FUNCTION valid_end_turn_outcome_v4(outcome JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF jsonb_typeof(outcome) IS DISTINCT FROM 'object'
       OR jsonb_typeof(outcome -> 'type') IS DISTINCT FROM 'string'
    THEN
        RETURN FALSE;
    END IF;

    CASE outcome ->> 'type'
        WHEN 'card_moved' THEN
            RETURN outcome = jsonb_build_object(
                    'type', outcome -> 'type',
                    'card_id', outcome -> 'card_id',
                    'from', outcome -> 'from',
                    'to', outcome -> 'to'
                )
                AND jsonb_typeof(outcome -> 'card_id') = 'string'
                AND outcome ->> 'card_id' <> ''
                AND octet_length(outcome ->> 'card_id') <= 256
                AND jsonb_typeof(outcome -> 'from') = 'string'
                AND jsonb_typeof(outcome -> 'to') = 'string'
                AND (
                    (
                        outcome ->> 'from' IN ('hero_hand', 'hero_play_area')
                        AND outcome ->> 'to' = 'hero_discard_pile'
                    )
                    OR (
                        outcome ->> 'from' = 'hero_draw_pile'
                        AND outcome ->> 'to' = 'hero_hand'
                    )
                );
        WHEN 'pile_shuffled' THEN
            IF outcome IS DISTINCT FROM jsonb_build_object(
                    'type', outcome -> 'type',
                    'owner_position', outcome -> 'owner_position',
                    'zone', outcome -> 'zone',
                    'bottom_to_top', outcome -> 'bottom_to_top'
                )
               OR jsonb_typeof(outcome -> 'owner_position') IS DISTINCT FROM 'number'
               OR outcome ->> 'owner_position' !~ '^[1-4]$'
               OR jsonb_typeof(outcome -> 'zone') IS DISTINCT FROM 'string'
               OR outcome ->> 'zone' IS DISTINCT FROM 'hero_draw_pile'
               OR jsonb_typeof(outcome -> 'bottom_to_top') IS DISTINCT FROM 'array'
               OR jsonb_array_length(outcome -> 'bottom_to_top') NOT BETWEEN 1 AND 4096
               OR EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(outcome -> 'bottom_to_top') AS card
                    WHERE jsonb_typeof(card) <> 'string'
                       OR card #>> '{}' = ''
                       OR octet_length(card #>> '{}') > 256
               )
               OR (
                    SELECT COUNT(*)
                    FROM jsonb_array_elements(outcome -> 'bottom_to_top') AS card
               ) <> (
                    SELECT COUNT(DISTINCT card)
                    FROM jsonb_array_elements(outcome -> 'bottom_to_top') AS card
               )
            THEN
                RETURN FALSE;
            END IF;
            RETURN TRUE;
        WHEN 'resource_reset' THEN
            RETURN outcome = jsonb_build_object(
                    'type', outcome -> 'type',
                    'resource', outcome -> 'resource',
                    'before', outcome -> 'before'
                )
                AND jsonb_typeof(outcome -> 'resource') = 'string'
                AND outcome ->> 'resource' IN ('attack', 'influence')
                AND jsonb_typeof(outcome -> 'before') = 'number'
                AND outcome ->> 'before' ~ '^(0|[1-9][0-9]*)$'
                AND (outcome ->> 'before')::NUMERIC <= 65535;
        ELSE
            RETURN FALSE;
    END CASE;
END;
$$;

CREATE FUNCTION valid_effect_outcome_v4(effect JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF valid_effect_outcome(effect) IS NOT TRUE
       OR octet_length(effect ->> 'rule_id') > 256
       OR (
            effect ->> 'type' IN ('moved', 'resource_changed')
            AND octet_length(effect ->> 'target_id') > 256
       )
    THEN
        RETURN FALSE;
    END IF;

    IF effect ->> 'type' = 'moved' THEN
        RETURN effect ->> 'from' IN (
                'active_villains', 'dark_arts_deck', 'dark_arts_discard',
                'hero_discard_pile', 'hero_draw_pile', 'hero_hand',
                'hero_play_area', 'hogwarts_deck', 'market', 'villain_deck'
            )
            AND effect ->> 'to' IN (
                'active_villains', 'dark_arts_deck', 'dark_arts_discard',
                'hero_discard_pile', 'hero_draw_pile', 'hero_hand',
                'hero_play_area', 'hogwarts_deck', 'market', 'villain_deck'
            )
            AND effect ->> 'from' IS DISTINCT FROM effect ->> 'to';
    END IF;

    RETURN TRUE;
END;
$$;

CREATE FUNCTION valid_end_turn_sequence_v4(outcomes JSONB, actor_position INTEGER)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    current JSONB;
    outcome_index INTEGER := 0;
    outcome_count INTEGER;
    shuffled BOOLEAN := FALSE;
BEGIN
    IF jsonb_typeof(outcomes) IS DISTINCT FROM 'array'
       OR jsonb_array_length(outcomes) NOT BETWEEN 2 AND 4102
       OR actor_position IS NULL
       OR actor_position NOT BETWEEN 1 AND 4
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(outcomes) AS outcome
            WHERE valid_end_turn_outcome_v4(outcome) IS NOT TRUE
               OR (
                    outcome ->> 'type' = 'pile_shuffled'
                    AND outcome ->> 'owner_position' <> actor_position::TEXT
               )
       )
    THEN
        RETURN FALSE;
    END IF;
    outcome_count := jsonb_array_length(outcomes);

    WHILE outcome_index < outcome_count
          AND outcomes -> outcome_index ->> 'type' = 'card_moved'
          AND outcomes -> outcome_index ->> 'from' = 'hero_play_area'
          AND outcomes -> outcome_index ->> 'to' = 'hero_discard_pile'
    LOOP
        outcome_index := outcome_index + 1;
    END LOOP;
    WHILE outcome_index < outcome_count
          AND outcomes -> outcome_index ->> 'type' = 'card_moved'
          AND outcomes -> outcome_index ->> 'from' = 'hero_hand'
          AND outcomes -> outcome_index ->> 'to' = 'hero_discard_pile'
    LOOP
        outcome_index := outcome_index + 1;
    END LOOP;

    IF outcomes -> outcome_index IS DISTINCT FROM jsonb_build_object(
            'type', 'resource_reset',
            'resource', 'attack',
            'before', outcomes -> outcome_index -> 'before'
       )
    THEN
        RETURN FALSE;
    END IF;
    outcome_index := outcome_index + 1;
    IF outcomes -> outcome_index IS DISTINCT FROM jsonb_build_object(
            'type', 'resource_reset',
            'resource', 'influence',
            'before', outcomes -> outcome_index -> 'before'
       )
    THEN
        RETURN FALSE;
    END IF;
    outcome_index := outcome_index + 1;

    WHILE outcome_index < outcome_count LOOP
        current := outcomes -> outcome_index;
        IF current ->> 'type' = 'card_moved'
           AND current ->> 'from' = 'hero_draw_pile'
           AND current ->> 'to' = 'hero_hand'
        THEN
            NULL;
        ELSIF current ->> 'type' = 'pile_shuffled' AND NOT shuffled THEN
            shuffled := TRUE;
        ELSE
            RETURN FALSE;
        END IF;
        outcome_index := outcome_index + 1;
    END LOOP;
    RETURN TRUE;
END;
$$;

CREATE FUNCTION valid_turn_step_v4(step JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF step IS DISTINCT FROM jsonb_build_object(
            'phase', step -> 'phase',
            'effects', step -> 'effects'
        )
       OR step ->> 'phase' NOT IN ('dark_arts', 'villains', 'hero_actions', 'end_turn')
       OR jsonb_typeof(step -> 'phase') IS DISTINCT FROM 'string'
       OR jsonb_typeof(step -> 'effects') IS DISTINCT FROM 'array'
       OR jsonb_array_length(step -> 'effects') > 4096
    THEN
        RETURN FALSE;
    END IF;

    RETURN NOT EXISTS (
        SELECT 1
        FROM jsonb_array_elements(step -> 'effects') AS effect
        WHERE valid_effect_outcome_v4(effect) IS NOT TRUE
    );
END;
$$;

CREATE FUNCTION valid_snapshot_effect_history_v3(snapshot JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    flattened_effects JSONB;
BEGIN
    IF jsonb_typeof(snapshot -> 'effects') IS DISTINCT FROM 'object'
       OR jsonb_typeof(snapshot -> 'last_turn_steps') IS DISTINCT FROM 'array'
       OR (
            snapshot -> 'effects' ? 'outcomes'
            AND jsonb_typeof(snapshot -> 'effects' -> 'outcomes')
                IS DISTINCT FROM 'array'
       )
    THEN
        RETURN FALSE;
    END IF;

    SELECT COALESCE(
        jsonb_agg(
            outcome.value
            ORDER BY step.position, outcome.position
        ),
        '[]'::jsonb
    )
    INTO flattened_effects
    FROM jsonb_array_elements(snapshot -> 'last_turn_steps')
        WITH ORDINALITY AS step(value, position)
    CROSS JOIN LATERAL jsonb_array_elements(step.value -> 'effects')
        WITH ORDINALITY AS outcome(value, position);

    RETURN COALESCE(snapshot -> 'effects' -> 'outcomes', '[]'::jsonb)
        IS NOT DISTINCT FROM flattened_effects;
EXCEPTION
    WHEN OTHERS THEN
        RETURN FALSE;
END;
$$;

CREATE FUNCTION valid_engine_control_v4(control JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
BEGIN
    IF control IS DISTINCT FROM jsonb_build_object(
            'status', control -> 'status',
            'turn', control -> 'turn',
            'phase', control -> 'phase',
            'active_position', control -> 'active_position',
            'queued_phases', control -> 'queued_phases',
            'queued_effects', control -> 'queued_effects',
            'decision_point', control -> 'decision_point'
        )
       OR control ->> 'status' NOT IN ('in_progress', 'lost', 'won')
       OR jsonb_typeof(control -> 'status') IS DISTINCT FROM 'string'
       OR jsonb_typeof(control -> 'turn') IS DISTINCT FROM 'number'
       OR control ->> 'turn' !~ '^[1-9][0-9]*$'
       OR (control ->> 'turn')::NUMERIC > 4294967295
       OR control ->> 'phase' NOT IN ('dark_arts', 'villains', 'hero_actions', 'end_turn')
       OR jsonb_typeof(control -> 'phase') IS DISTINCT FROM 'string'
       OR jsonb_typeof(control -> 'active_position') IS DISTINCT FROM 'number'
       OR control ->> 'active_position' !~ '^[1-4]$'
       OR jsonb_typeof(control -> 'queued_phases') IS DISTINCT FROM 'array'
       OR jsonb_array_length(control -> 'queued_phases') > 3
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(control -> 'queued_phases') AS phase
            WHERE jsonb_typeof(phase) <> 'string'
               OR phase #>> '{}' NOT IN ('dark_arts', 'villains', 'hero_actions', 'end_turn')
       )
       OR (
            SELECT COUNT(*)
            FROM jsonb_array_elements(control -> 'queued_phases') AS phase
       ) <> (
            SELECT COUNT(DISTINCT phase)
            FROM jsonb_array_elements(control -> 'queued_phases') AS phase
       )
       OR jsonb_typeof(control -> 'queued_effects') IS DISTINCT FROM 'array'
       OR jsonb_array_length(control -> 'queued_effects') > 4096
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(control -> 'queued_effects') AS frame
            WHERE valid_queued_effect_v4(frame) IS NOT TRUE
       )
       OR valid_decision_point_v4(control -> 'decision_point') IS NOT TRUE
    THEN
        RETURN FALSE;
    END IF;

    IF control ->> 'status' IN ('lost', 'won') THEN
        RETURN jsonb_array_length(control -> 'queued_phases') = 0
            AND jsonb_array_length(control -> 'queued_effects') = 0
            AND control -> 'decision_point' = jsonb_build_object('type', 'none');
    END IF;

    CASE control ->> 'phase'
        WHEN 'dark_arts' THEN
            RETURN control -> 'queued_phases'
                    = '["villains", "hero_actions", "end_turn"]'::jsonb
                AND (
                    (
                        control -> 'decision_point' = jsonb_build_object('type', 'automatic')
                        AND jsonb_array_length(control -> 'queued_effects') = 0
                    )
                    OR (
                        control -> 'decision_point' ->> 'type' = 'effect_choice'
                        AND control -> 'queued_effects'
                            = control -> 'decision_point' -> 'choice'
                                -> 'continuation' -> 'queue'
                    )
                );
        WHEN 'villains' THEN
            RETURN control -> 'queued_phases' = '["hero_actions", "end_turn"]'::jsonb
                AND (
                    (
                        control -> 'decision_point' = jsonb_build_object('type', 'automatic')
                        AND jsonb_array_length(control -> 'queued_effects') = 0
                    )
                    OR (
                        control -> 'decision_point' ->> 'type' = 'effect_choice'
                        AND control -> 'queued_effects'
                            = control -> 'decision_point' -> 'choice'
                                -> 'continuation' -> 'queue'
                    )
                );
        WHEN 'hero_actions' THEN
            RETURN control -> 'queued_phases' = '["end_turn"]'::jsonb
                AND (
                    (
                        jsonb_array_length(control -> 'queued_effects') = 0
                        AND control -> 'decision_point' ->> 'type' = 'player_intent'
                        AND control -> 'decision_point' ->> 'responsible_position'
                            = control ->> 'active_position'
                    )
                    OR (
                        control -> 'decision_point' ->> 'type' = 'effect_choice'
                        AND control -> 'queued_effects'
                            = control -> 'decision_point' -> 'choice'
                                -> 'continuation' -> 'queue'
                    )
                );
        WHEN 'end_turn' THEN
            RETURN jsonb_array_length(control -> 'queued_phases') = 0
                AND jsonb_array_length(control -> 'queued_effects') = 0
                AND control -> 'decision_point' = jsonb_build_object('type', 'automatic');
        ELSE
            RETURN FALSE;
    END CASE;
END;
$$;

CREATE FUNCTION valid_snapshot_effect_entity_v3(entity JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    expected JSONB;
    zone_name TEXT;
BEGIN
    IF jsonb_typeof(entity) IS DISTINCT FROM 'object' THEN
        RETURN FALSE;
    END IF;

    expected := jsonb_build_object(
        'id', entity -> 'id',
        'zone', entity -> 'zone'
    );
    IF entity ? 'owner_position' THEN
        expected := expected || jsonb_build_object(
            'owner_position', entity -> 'owner_position'
        );
    END IF;
    IF entity ? 'kind' THEN
        expected := expected || jsonb_build_object('kind', entity -> 'kind');
    END IF;
    IF entity ? 'catalog_id' THEN
        expected := expected || jsonb_build_object('catalog_id', entity -> 'catalog_id');
    END IF;
    IF entity ? 'effect_rule_id' THEN
        expected := expected || jsonb_build_object(
            'effect_rule_id', entity -> 'effect_rule_id'
        );
    END IF;
    IF entity ? 'influence_cost' THEN
        expected := expected || jsonb_build_object(
            'influence_cost', entity -> 'influence_cost'
        );
    END IF;
    IF entity ? 'zone_index' THEN
        expected := expected || jsonb_build_object('zone_index', entity -> 'zone_index');
    END IF;
    IF entity ? 'resources' THEN
        expected := expected || jsonb_build_object('resources', entity -> 'resources');
    END IF;

    zone_name := entity ->> 'zone';
    IF entity IS DISTINCT FROM expected
       OR jsonb_typeof(entity -> 'id') IS DISTINCT FROM 'string'
       OR entity ->> 'id' = ''
       OR octet_length(entity ->> 'id') > 256
       OR jsonb_typeof(entity -> 'zone') IS DISTINCT FROM 'string'
       OR zone_name NOT IN (
            'active_location', 'active_villains', 'dark_arts_deck',
            'dark_arts_discard', 'hero_discard_pile', 'hero_draw_pile',
            'hero_hand', 'hero_play_area', 'heroes', 'hogwarts_deck',
            'market', 'villain_deck'
       )
       OR (
            entity ? 'owner_position'
            AND (
                jsonb_typeof(entity -> 'owner_position') IS DISTINCT FROM 'number'
                OR entity ->> 'owner_position' !~ '^[1-4]$'
            )
       )
       OR (zone_name = 'heroes' AND NOT entity ? 'owner_position')
       OR (
            entity ? 'kind'
            AND (
                jsonb_typeof(entity -> 'kind') IS DISTINCT FROM 'string'
                OR entity ->> 'kind' NOT IN (
                    'generic', 'hero', 'hogwarts_card', 'starter_card', 'villain'
                )
            )
       )
       OR (
            entity ? 'catalog_id'
            AND (
                jsonb_typeof(entity -> 'catalog_id') IS DISTINCT FROM 'string'
                OR entity ->> 'catalog_id' = ''
                OR octet_length(entity ->> 'catalog_id') > 256
            )
       )
       OR (
            entity ? 'effect_rule_id'
            AND (
                jsonb_typeof(entity -> 'effect_rule_id') IS DISTINCT FROM 'string'
                OR entity ->> 'effect_rule_id' = ''
                OR octet_length(entity ->> 'effect_rule_id') > 256
            )
       )
       OR (
            entity ? 'influence_cost'
            AND (
                jsonb_typeof(entity -> 'influence_cost') IS DISTINCT FROM 'number'
                OR entity ->> 'influence_cost' !~ '^(0|[1-9][0-9]*)$'
                OR (entity ->> 'influence_cost')::NUMERIC > 65535
            )
       )
    THEN
        RETURN FALSE;
    END IF;

    IF zone_name IN (
        'active_villains', 'dark_arts_deck', 'dark_arts_discard', 'hero_discard_pile',
        'hero_draw_pile', 'hero_hand', 'hero_play_area', 'hogwarts_deck',
        'market', 'villain_deck'
    ) THEN
        IF NOT entity ? 'zone_index'
           OR jsonb_typeof(entity -> 'zone_index') IS DISTINCT FROM 'number'
           OR entity ->> 'zone_index' !~ '^(0|[1-9][0-9]*)$'
           OR (entity ->> 'zone_index')::NUMERIC > 65535
        THEN
            RETURN FALSE;
        END IF;
    ELSIF entity ? 'zone_index' THEN
        RETURN FALSE;
    END IF;

    IF entity ? 'resources' THEN
        IF jsonb_typeof(entity -> 'resources') IS DISTINCT FROM 'object'
           OR NOT EXISTS (SELECT 1 FROM jsonb_each(entity -> 'resources'))
           OR EXISTS (
                SELECT 1
                FROM jsonb_each(entity -> 'resources') AS resource
                WHERE jsonb_typeof(resource.value) IS DISTINCT FROM 'number'
                   OR resource.value #>> '{}' !~ '^(0|[1-9][0-9]*)$'
                   OR (resource.value #>> '{}')::NUMERIC > 65535
                   OR NOT (
                        (zone_name = 'heroes' AND resource.key IN ('attack', 'health', 'influence'))
                        OR (zone_name = 'active_villains' AND resource.key = 'health')
                        OR (zone_name = 'active_location' AND resource.key = 'control')
                   )
           )
        THEN
            RETURN FALSE;
        END IF;
    END IF;

    RETURN TRUE;
END;
$$;

CREATE FUNCTION valid_game_snapshot_v3(snapshot JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    participant_count INTEGER;
BEGIN
    IF compact_jsonb_octet_length(snapshot) > 4194304
       OR snapshot IS DISTINCT FROM jsonb_build_object(
            'snapshot_version', snapshot -> 'snapshot_version',
            'state_version', snapshot -> 'state_version',
            'sequence', snapshot -> 'sequence',
            'status', snapshot -> 'status',
            'adventure_id', snapshot -> 'adventure_id',
            'versions', snapshot -> 'versions',
            'turn', snapshot -> 'turn',
            'queued_phases', snapshot -> 'queued_phases',
            'queued_effects', snapshot -> 'queued_effects',
            'decision_point', snapshot -> 'decision_point',
            'last_turn_steps', snapshot -> 'last_turn_steps',
            'participants', snapshot -> 'participants',
            'prng', snapshot -> 'prng',
            'effects', snapshot -> 'effects'
        )
       OR jsonb_typeof(snapshot -> 'snapshot_version') IS DISTINCT FROM 'number'
       OR snapshot ->> 'snapshot_version' <> '3'
       OR jsonb_typeof(snapshot -> 'state_version') IS DISTINCT FROM 'number'
       OR snapshot ->> 'state_version' !~ '^[1-9][0-9]*$'
       OR (snapshot ->> 'state_version')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(snapshot -> 'sequence') IS DISTINCT FROM 'number'
       OR snapshot ->> 'sequence' !~ '^(0|[1-9][0-9]*)$'
       OR (snapshot ->> 'sequence')::NUMERIC > 9223372036854775807
       OR (snapshot ->> 'state_version')::NUMERIC
            <> (snapshot ->> 'sequence')::NUMERIC + 1
       OR jsonb_typeof(snapshot -> 'status') IS DISTINCT FROM 'string'
       OR snapshot ->> 'status' NOT IN ('in_progress', 'lost', 'won')
       OR jsonb_typeof(snapshot -> 'adventure_id') IS DISTINCT FROM 'string'
       OR snapshot ->> 'adventure_id' = ''
       OR octet_length(snapshot ->> 'adventure_id') > 256
    THEN
        RETURN FALSE;
    END IF;

    IF snapshot -> 'versions' IS DISTINCT FROM jsonb_build_object(
            'content', snapshot -> 'versions' -> 'content',
            'ruleset', snapshot -> 'versions' -> 'ruleset',
            'manifest', snapshot -> 'versions' -> 'manifest',
            'manifest_digest', snapshot -> 'versions' -> 'manifest_digest',
            'prng', snapshot -> 'versions' -> 'prng',
            'shuffle', snapshot -> 'versions' -> 'shuffle',
            'sampling', snapshot -> 'versions' -> 'sampling'
        )
       OR jsonb_typeof(snapshot -> 'versions' -> 'content') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'content' = ''
       OR octet_length(snapshot -> 'versions' ->> 'content') > 256
       OR jsonb_typeof(snapshot -> 'versions' -> 'ruleset') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'ruleset' = ''
       OR octet_length(snapshot -> 'versions' ->> 'ruleset') > 256
       OR jsonb_typeof(snapshot -> 'versions' -> 'manifest') IS DISTINCT FROM 'number'
       OR snapshot -> 'versions' ->> 'manifest' !~ '^[1-9][0-9]*$'
       OR (snapshot -> 'versions' ->> 'manifest')::NUMERIC > 65535
       OR jsonb_typeof(snapshot -> 'versions' -> 'manifest_digest') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'manifest_digest' !~ '^blake3:[0-9a-f]{64}$'
       OR jsonb_typeof(snapshot -> 'versions' -> 'prng') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'prng' IS DISTINCT FROM 'chacha20-v1'
       OR jsonb_typeof(snapshot -> 'versions' -> 'shuffle') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'shuffle' IS DISTINCT FROM 'fisher-yates-v1'
       OR jsonb_typeof(snapshot -> 'versions' -> 'sampling') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'sampling'
            IS DISTINCT FROM 'rejection-sampling-v1'
    THEN
        RETURN FALSE;
    END IF;

    IF snapshot -> 'turn' IS DISTINCT FROM jsonb_build_object(
            'number', snapshot -> 'turn' -> 'number',
            'phase', snapshot -> 'turn' -> 'phase',
            'active_position', snapshot -> 'turn' -> 'active_position'
        )
       OR jsonb_typeof(snapshot -> 'turn' -> 'number') IS DISTINCT FROM 'number'
       OR snapshot -> 'turn' ->> 'number' !~ '^[1-9][0-9]*$'
       OR (snapshot -> 'turn' ->> 'number')::NUMERIC > 4294967295
       OR jsonb_typeof(snapshot -> 'turn' -> 'phase') IS DISTINCT FROM 'string'
       OR snapshot -> 'turn' ->> 'phase'
            NOT IN ('dark_arts', 'villains', 'hero_actions', 'end_turn')
       OR jsonb_typeof(snapshot -> 'turn' -> 'active_position') IS DISTINCT FROM 'number'
       OR snapshot -> 'turn' ->> 'active_position' !~ '^[1-4]$'
       OR valid_engine_control_v4(jsonb_build_object(
            'status', snapshot -> 'status',
            'turn', snapshot -> 'turn' -> 'number',
            'phase', snapshot -> 'turn' -> 'phase',
            'active_position', snapshot -> 'turn' -> 'active_position',
            'queued_phases', snapshot -> 'queued_phases',
            'queued_effects', snapshot -> 'queued_effects',
            'decision_point', snapshot -> 'decision_point'
       )) IS NOT TRUE
       OR jsonb_typeof(snapshot -> 'last_turn_steps') IS DISTINCT FROM 'array'
       OR jsonb_array_length(snapshot -> 'last_turn_steps') > 3
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'last_turn_steps') AS step
            WHERE valid_turn_step_v4(step) IS NOT TRUE
       )
       OR valid_snapshot_effect_history_v3(snapshot) IS NOT TRUE
    THEN
        RETURN FALSE;
    END IF;

    IF jsonb_typeof(snapshot -> 'participants') IS DISTINCT FROM 'array' THEN
        RETURN FALSE;
    END IF;
    participant_count := jsonb_array_length(snapshot -> 'participants');
    IF participant_count NOT BETWEEN 2 AND 4
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
            WHERE participant IS DISTINCT FROM jsonb_build_object(
                    'participant_id', participant -> 'participant_id',
                    'position', participant -> 'position',
                    'hero_id', participant -> 'hero_id'
                )
               OR jsonb_typeof(participant -> 'participant_id') IS DISTINCT FROM 'string'
               OR participant ->> 'participant_id'
                    !~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
               OR jsonb_typeof(participant -> 'position') IS DISTINCT FROM 'number'
               OR participant ->> 'position' !~ '^[1-4]$'
               OR jsonb_typeof(participant -> 'hero_id') IS DISTINCT FROM 'string'
               OR participant ->> 'hero_id' NOT IN ('harry', 'hermione', 'neville', 'ron')
       )
       OR participant_count <> (
            SELECT COUNT(DISTINCT participant ->> 'participant_id')
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
       )
       OR participant_count <> (
            SELECT COUNT(DISTINCT participant ->> 'position')
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
       )
       OR participant_count <> (
            SELECT COUNT(DISTINCT participant ->> 'hero_id')
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
       )
       OR (
            SELECT ARRAY_AGG(
                (participant ->> 'position')::INTEGER
                ORDER BY (participant ->> 'position')::INTEGER
            )
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
       ) <> ARRAY(SELECT generate_series(1, participant_count))
       OR NOT EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
            WHERE participant -> 'position'
                = snapshot -> 'turn' -> 'active_position'
       )
    THEN
        RETURN FALSE;
    END IF;

    IF snapshot -> 'prng' IS DISTINCT FROM jsonb_build_object(
            'algorithm', snapshot -> 'prng' -> 'algorithm',
            'counter', snapshot -> 'prng' -> 'counter'
        )
       OR jsonb_typeof(snapshot -> 'prng' -> 'algorithm') IS DISTINCT FROM 'string'
       OR snapshot -> 'prng' ->> 'algorithm' IS DISTINCT FROM 'chacha20-v1'
       OR jsonb_typeof(snapshot -> 'prng' -> 'counter') IS DISTINCT FROM 'number'
       OR snapshot -> 'prng' ->> 'counter' !~ '^(0|[1-9][0-9]*)$'
       OR (snapshot -> 'prng' ->> 'counter')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(snapshot -> 'effects') IS DISTINCT FROM 'object'
       OR EXISTS (
            SELECT 1
            FROM jsonb_object_keys(snapshot -> 'effects') AS effect_key
            WHERE effect_key NOT IN ('entities', 'outcomes', 'choice')
       )
       OR jsonb_typeof(snapshot -> 'effects' -> 'entities') IS DISTINCT FROM 'array'
       OR jsonb_array_length(snapshot -> 'effects' -> 'entities') NOT BETWEEN 2 AND 4096
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'effects' -> 'entities') AS entity
            WHERE valid_snapshot_effect_entity_v3(entity) IS NOT TRUE
       )
       OR jsonb_array_length(snapshot -> 'effects' -> 'entities') <> (
            SELECT COUNT(DISTINCT entity ->> 'id')
            FROM jsonb_array_elements(snapshot -> 'effects' -> 'entities') AS entity
       )
       OR (
            snapshot -> 'effects' ? 'outcomes'
            AND (
                jsonb_typeof(snapshot -> 'effects' -> 'outcomes') IS DISTINCT FROM 'array'
                OR jsonb_array_length(snapshot -> 'effects' -> 'outcomes') NOT BETWEEN 1 AND 4096
                OR EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(snapshot -> 'effects' -> 'outcomes') AS outcome
                    WHERE valid_effect_outcome_v4(outcome) IS NOT TRUE
                )
            )
       )
       OR (
            snapshot -> 'effects' ? 'choice'
            AND valid_pending_effect_choice_v4(snapshot -> 'effects' -> 'choice') IS NOT TRUE
       )
    THEN
        RETURN FALSE;
    END IF;

    IF EXISTS (
        SELECT 1
        FROM jsonb_array_elements(snapshot -> 'effects' -> 'entities') AS entity
        WHERE entity ? 'owner_position'
          AND NOT EXISTS (
                SELECT 1
                FROM jsonb_array_elements(snapshot -> 'participants') AS participant
                WHERE participant -> 'position' = entity -> 'owner_position'
          )
    )
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
            WHERE (
                SELECT COUNT(*)
                FROM jsonb_array_elements(snapshot -> 'effects' -> 'entities') AS entity
                WHERE entity ->> 'zone' = 'heroes'
                  AND entity -> 'owner_position' = participant -> 'position'
            ) <> 1
       )
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'effects' -> 'entities') AS entity
            WHERE entity ->> 'zone' IN (
                'active_villains', 'dark_arts_deck', 'dark_arts_discard', 'hero_discard_pile',
                'hero_draw_pile', 'hero_hand', 'hero_play_area', 'hogwarts_deck',
                'market', 'villain_deck'
            )
            GROUP BY entity -> 'owner_position', entity ->> 'zone'
            HAVING MIN((entity ->> 'zone_index')::INTEGER) <> 0
                OR MAX((entity ->> 'zone_index')::INTEGER) <> COUNT(*) - 1
                OR COUNT(*) <> COUNT(DISTINCT (entity ->> 'zone_index')::INTEGER)
       )
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'queued_effects') AS queued
            WHERE NOT EXISTS (
                SELECT 1
                FROM jsonb_array_elements(snapshot -> 'participants') AS participant
                WHERE participant ->> 'position' = COALESCE(
                    queued ->> 'actor_position',
                    queued ->> 'responsible_position'
                )
            )
       )
       OR (
            snapshot -> 'decision_point' ->> 'type' = 'effect_choice'
            AND NOT EXISTS (
                SELECT 1
                FROM jsonb_array_elements(snapshot -> 'participants') AS participant
                WHERE participant -> 'position'
                    = snapshot -> 'decision_point' -> 'choice'
                        -> 'responsible_position'
            )
       )
       OR (
            snapshot -> 'decision_point' ->> 'type' = 'effect_choice'
            AND snapshot -> 'effects' -> 'choice'
                IS DISTINCT FROM snapshot -> 'decision_point' -> 'choice'
       )
       OR (
            snapshot -> 'decision_point' ->> 'type' <> 'effect_choice'
            AND snapshot -> 'effects' ? 'choice'
       )
    THEN
        RETURN FALSE;
    END IF;

    RETURN TRUE;
END;
$$;

CREATE FUNCTION require_game_snapshot_v3_on_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    expected_participants JSONB;
BEGIN
    SELECT COALESCE(
        jsonb_agg(
            jsonb_build_object(
                'participant_id', participants.id::TEXT,
                'position', participants.position,
                'hero_id', participants.hero_id
            )
            ORDER BY participants.position
        ),
        '[]'::jsonb
    )
    INTO expected_participants
    FROM participants
    WHERE participants.room_id = NEW.room_id;

    IF NEW.sequence <> 0
       OR NEW.state_version <> 1
       OR valid_game_snapshot_v3(NEW.snapshot) IS NOT TRUE
       OR (NEW.snapshot ->> 'snapshot_version')::NUMERIC <> NEW.snapshot_version
       OR (NEW.snapshot ->> 'state_version')::NUMERIC <> NEW.state_version
       OR (NEW.snapshot ->> 'sequence')::NUMERIC <> NEW.sequence
       OR NEW.snapshot ->> 'status' IS DISTINCT FROM NEW.status
       OR NEW.snapshot ->> 'adventure_id' IS DISTINCT FROM NEW.adventure_id
       OR NEW.snapshot -> 'versions' ->> 'content' IS DISTINCT FROM NEW.content_version
       OR NEW.snapshot -> 'versions' ->> 'ruleset' IS DISTINCT FROM NEW.ruleset_version
       OR (NEW.snapshot -> 'versions' ->> 'manifest')::NUMERIC <> NEW.manifest_version
       OR NEW.snapshot -> 'versions' ->> 'manifest_digest' IS DISTINCT FROM NEW.manifest_digest
       OR NEW.snapshot -> 'versions' ->> 'prng' IS DISTINCT FROM NEW.prng_algorithm
       OR NEW.snapshot -> 'versions' ->> 'shuffle' IS DISTINCT FROM NEW.shuffle_algorithm
       OR NEW.snapshot -> 'versions' ->> 'sampling' IS DISTINCT FROM NEW.sampling_algorithm
       OR NEW.snapshot -> 'prng' ->> 'algorithm' IS DISTINCT FROM NEW.prng_algorithm
       OR (NEW.snapshot -> 'prng' ->> 'counter')::NUMERIC <> NEW.prng_counter
       OR NEW.snapshot -> 'participants' IS DISTINCT FROM expected_participants
    THEN
        RAISE EXCEPTION 'new game snapshot must match the current codec and relational metadata'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER games_require_current_snapshot_on_insert
BEFORE INSERT ON games
FOR EACH ROW
EXECUTE FUNCTION require_game_snapshot_v3_on_insert();

CREATE FUNCTION require_current_game_state_anchor_on_insert()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    current_sequence BIGINT;
    current_snapshot_version SMALLINT;
    current_state_digest TEXT;
BEGIN
    SELECT games.sequence, games.snapshot_version, games.state_digest
    INTO current_sequence, current_snapshot_version, current_state_digest
    FROM games
    WHERE games.id = NEW.game_id
    FOR UPDATE;

    IF NOT FOUND
       OR NEW.sequence IS DISTINCT FROM current_sequence
       OR NEW.snapshot_version IS DISTINCT FROM current_snapshot_version
       OR NEW.state_digest IS DISTINCT FROM current_state_digest
    THEN
        RAISE EXCEPTION 'replay anchor must match the current authoritative game state'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER game_state_anchors_require_current_game_state
BEFORE INSERT ON game_state_anchors
FOR EACH ROW
EXECUTE FUNCTION require_current_game_state_anchor_on_insert();

CREATE FUNCTION normalized_effect_entities_for_turn_v4(snapshot JSONB)
RETURNS JSONB
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    assigned_index INTEGER;
    entity JSONB;
    entity_position BIGINT;
    normalized JSONB;
    normalized_entities JSONB := '[]'::jsonb;
BEGIN
    IF jsonb_typeof(snapshot -> 'participants') IS DISTINCT FROM 'array' THEN
        RETURN NULL;
    END IF;

    IF jsonb_typeof(snapshot -> 'effects' -> 'entities') IS DISTINCT FROM 'array'
       OR jsonb_array_length(snapshot -> 'effects' -> 'entities') = 0
    THEN
        SELECT COALESCE(
            jsonb_agg(
                jsonb_build_object(
                    'id', format('hero:%s', participant ->> 'position'),
                    'kind', 'hero',
                    'owner_position', participant -> 'position',
                    'zone', 'heroes',
                    'resources', jsonb_build_object('health', 10)
                )
                ORDER BY format('hero:%s', participant ->> 'position')
            ),
            '[]'::jsonb
        )
        INTO normalized_entities
        FROM jsonb_array_elements(snapshot -> 'participants') AS participant;
        RETURN normalized_entities;
    END IF;

    FOR entity, entity_position IN
        SELECT candidate.value, candidate.position
        FROM jsonb_array_elements(snapshot -> 'effects' -> 'entities')
            WITH ORDINALITY AS candidate(value, position)
        ORDER BY candidate.position
    LOOP
        normalized := jsonb_build_object(
            'id', entity -> 'id',
            'kind', CASE
                WHEN jsonb_typeof(entity -> 'kind') = 'string' THEN entity ->> 'kind'
                WHEN entity ->> 'zone' = 'heroes' THEN 'hero'
                ELSE 'generic'
            END,
            'zone', entity -> 'zone'
        );
        IF jsonb_typeof(entity -> 'owner_position') = 'number' THEN
            normalized := normalized || jsonb_build_object(
                'owner_position', entity -> 'owner_position'
            );
        END IF;
        IF jsonb_typeof(entity -> 'catalog_id') = 'string' THEN
            normalized := normalized || jsonb_build_object(
                'catalog_id', entity -> 'catalog_id'
            );
        END IF;
        IF jsonb_typeof(entity -> 'effect_rule_id') = 'string' THEN
            normalized := normalized || jsonb_build_object(
                'effect_rule_id', entity -> 'effect_rule_id'
            );
        END IF;
        IF jsonb_typeof(entity -> 'influence_cost') = 'number' THEN
            normalized := normalized || jsonb_build_object(
                'influence_cost', entity -> 'influence_cost'
            );
        END IF;
        IF jsonb_typeof(entity -> 'resources') = 'object'
           AND entity -> 'resources' <> '{}'::jsonb
        THEN
            normalized := normalized || jsonb_build_object(
                'resources', entity -> 'resources'
            );
        END IF;
        IF entity ->> 'zone' IN (
            'active_villains', 'dark_arts_deck', 'dark_arts_discard', 'hero_discard_pile',
            'hero_draw_pile', 'hero_hand', 'hero_play_area', 'hogwarts_deck',
            'market', 'villain_deck'
        ) THEN
            IF jsonb_typeof(entity -> 'zone_index') = 'number' THEN
                assigned_index := (entity ->> 'zone_index')::INTEGER;
            ELSE
                SELECT
                    COALESCE(MAX((candidate.value ->> 'zone_index')::INTEGER) + 1, 0)
                    + COUNT(*) FILTER (
                        WHERE candidate.position < entity_position
                          AND jsonb_typeof(candidate.value -> 'zone_index')
                                IS DISTINCT FROM 'number'
                    )
                INTO assigned_index
                FROM jsonb_array_elements(snapshot -> 'effects' -> 'entities')
                    WITH ORDINALITY AS candidate(value, position)
                WHERE candidate.value -> 'owner_position'
                        IS NOT DISTINCT FROM entity -> 'owner_position'
                  AND candidate.value ->> 'zone' = entity ->> 'zone'
                  AND (
                        jsonb_typeof(candidate.value -> 'zone_index') = 'number'
                        OR candidate.position < entity_position
                  );
            END IF;
            normalized := normalized || jsonb_build_object(
                'zone_index', assigned_index
            );
        END IF;
        normalized_entities := normalized_entities || jsonb_build_array(normalized);
    END LOOP;

    SELECT jsonb_agg(candidate ORDER BY candidate ->> 'id')
    INTO normalized_entities
    FROM jsonb_array_elements(normalized_entities) AS candidate;
    RETURN normalized_entities;
END;
$$;

CREATE FUNCTION valid_legacy_effect_outcome_for_v15(effect JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    normalized JSONB := effect;
BEGIN
    IF jsonb_typeof(effect) IS DISTINCT FROM 'object' THEN
        RETURN FALSE;
    END IF;
    IF effect ->> 'type' IN ('moved', 'resource_changed')
       AND effect ? 'target_position'
       AND effect -> 'target_position' = 'null'::jsonb
    THEN
        normalized := effect - 'target_position';
    END IF;
    IF valid_effect_outcome(normalized) IS NOT TRUE THEN
        RETURN FALSE;
    END IF;
    IF octet_length(effect ->> 'rule_id') > 256
       OR (
            effect ->> 'type' IN ('moved', 'resource_changed')
            AND octet_length(effect ->> 'target_id') > 256
       )
    THEN
        RETURN FALSE;
    END IF;
    IF effect ->> 'type' = 'moved' THEN
        RETURN effect ->> 'from' IN (
                'active_villains', 'dark_arts_deck', 'dark_arts_discard',
                'hero_discard_pile', 'hero_draw_pile', 'hero_hand',
                'hero_play_area', 'hogwarts_deck', 'market', 'villain_deck'
            )
            AND effect ->> 'to' IN (
                'active_villains', 'dark_arts_deck', 'dark_arts_discard',
                'hero_discard_pile', 'hero_draw_pile', 'hero_hand',
                'hero_play_area', 'hogwarts_deck', 'market', 'villain_deck'
            )
            AND effect ->> 'from' IS DISTINCT FROM effect ->> 'to';
    END IF;
    RETURN TRUE;
EXCEPTION
    WHEN OTHERS THEN
        RETURN FALSE;
END;
$$;

CREATE FUNCTION valid_legacy_effect_entity_for_v15(entity JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    expected JSONB;
    zone_name TEXT;
BEGIN
    IF jsonb_typeof(entity) IS DISTINCT FROM 'object' THEN
        RETURN FALSE;
    END IF;
    expected := jsonb_build_object(
        'id', entity -> 'id',
        'zone', entity -> 'zone'
    );
    IF entity ? 'owner_position' THEN
        expected := expected || jsonb_build_object(
            'owner_position', entity -> 'owner_position'
        );
    END IF;
    IF entity ? 'kind' THEN
        expected := expected || jsonb_build_object('kind', entity -> 'kind');
    END IF;
    IF entity ? 'catalog_id' THEN
        expected := expected || jsonb_build_object('catalog_id', entity -> 'catalog_id');
    END IF;
    IF entity ? 'effect_rule_id' THEN
        expected := expected || jsonb_build_object(
            'effect_rule_id', entity -> 'effect_rule_id'
        );
    END IF;
    IF entity ? 'influence_cost' THEN
        expected := expected || jsonb_build_object(
            'influence_cost', entity -> 'influence_cost'
        );
    END IF;
    IF entity ? 'zone_index' THEN
        expected := expected || jsonb_build_object('zone_index', entity -> 'zone_index');
    END IF;
    IF entity ? 'resources' THEN
        expected := expected || jsonb_build_object('resources', entity -> 'resources');
    END IF;

    zone_name := entity ->> 'zone';
    IF entity IS DISTINCT FROM expected
       OR jsonb_typeof(entity -> 'id') IS DISTINCT FROM 'string'
       OR entity ->> 'id' = ''
       OR octet_length(entity ->> 'id') > 256
       OR jsonb_typeof(entity -> 'zone') IS DISTINCT FROM 'string'
       OR zone_name NOT IN (
            'active_location', 'active_villains', 'dark_arts_deck',
            'dark_arts_discard', 'hero_discard_pile', 'hero_draw_pile',
            'hero_hand', 'hero_play_area', 'heroes', 'hogwarts_deck',
            'market', 'villain_deck'
       )
       OR (
            entity ? 'owner_position'
            AND entity -> 'owner_position' IS DISTINCT FROM 'null'::jsonb
            AND (
                jsonb_typeof(entity -> 'owner_position') IS DISTINCT FROM 'number'
                OR entity ->> 'owner_position' !~ '^[1-4]$'
            )
       )
       OR (
            zone_name = 'heroes'
            AND (
                NOT entity ? 'owner_position'
                OR entity -> 'owner_position' = 'null'::jsonb
            )
       )
       OR (
            entity ? 'zone_index'
            AND entity -> 'zone_index' IS DISTINCT FROM 'null'::jsonb
            AND (
                jsonb_typeof(entity -> 'zone_index') IS DISTINCT FROM 'number'
                OR entity ->> 'zone_index' !~ '^(0|[1-9][0-9]*)$'
                OR (entity ->> 'zone_index')::NUMERIC > 65535
            )
       )
       OR (
            entity ? 'kind'
            AND (
                jsonb_typeof(entity -> 'kind') IS DISTINCT FROM 'string'
                OR entity ->> 'kind' NOT IN (
                    'generic', 'hero', 'hogwarts_card', 'starter_card', 'villain'
                )
            )
       )
       OR (
            entity ? 'catalog_id'
            AND entity -> 'catalog_id' IS DISTINCT FROM 'null'::jsonb
            AND (
                jsonb_typeof(entity -> 'catalog_id') IS DISTINCT FROM 'string'
                OR entity ->> 'catalog_id' = ''
                OR octet_length(entity ->> 'catalog_id') > 256
            )
       )
       OR (
            entity ? 'effect_rule_id'
            AND entity -> 'effect_rule_id' IS DISTINCT FROM 'null'::jsonb
            AND (
                jsonb_typeof(entity -> 'effect_rule_id') IS DISTINCT FROM 'string'
                OR entity ->> 'effect_rule_id' = ''
                OR octet_length(entity ->> 'effect_rule_id') > 256
            )
       )
       OR (
            entity ? 'influence_cost'
            AND entity -> 'influence_cost' IS DISTINCT FROM 'null'::jsonb
            AND (
                jsonb_typeof(entity -> 'influence_cost') IS DISTINCT FROM 'number'
                OR entity ->> 'influence_cost' !~ '^(0|[1-9][0-9]*)$'
                OR (entity ->> 'influence_cost')::NUMERIC > 65535
            )
       )
       OR (
            zone_name NOT IN (
                'active_villains', 'dark_arts_deck', 'dark_arts_discard', 'hero_discard_pile',
                'hero_draw_pile', 'hero_hand', 'hero_play_area', 'hogwarts_deck',
                'market', 'villain_deck'
            )
            AND jsonb_typeof(entity -> 'zone_index') = 'number'
       )
    THEN
        RETURN FALSE;
    END IF;

    IF entity ? 'resources'
       AND (
            jsonb_typeof(entity -> 'resources') IS DISTINCT FROM 'object'
            OR EXISTS (
                SELECT 1
                FROM jsonb_each(entity -> 'resources') AS resource
                WHERE jsonb_typeof(resource.value) IS DISTINCT FROM 'number'
                   OR resource.value #>> '{}' !~ '^(0|[1-9][0-9]*)$'
                   OR (resource.value #>> '{}')::NUMERIC > 65535
                   OR NOT (
                        (zone_name = 'heroes' AND resource.key IN ('attack', 'health', 'influence'))
                        OR (zone_name = 'active_villains' AND resource.key = 'health')
                        OR (zone_name = 'active_location' AND resource.key = 'control')
                   )
            )
       )
    THEN
        RETURN FALSE;
    END IF;
    RETURN TRUE;
EXCEPTION
    WHEN OTHERS THEN
        RETURN FALSE;
END;
$$;

CREATE FUNCTION valid_game_snapshot_for_v15_upgrade(
    snapshot JSONB,
    expected_participants JSONB
)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    control_field_count INTEGER;
    effects JSONB;
    expected JSONB;
    legacy_snapshot JSONB;
    normalized_entities JSONB;
    participant_count INTEGER;
    pending_choice JSONB;
BEGIN
    IF compact_jsonb_octet_length(snapshot) > 4194304
       OR jsonb_typeof(snapshot) IS DISTINCT FROM 'object'
       OR jsonb_typeof(expected_participants) IS DISTINCT FROM 'array'
    THEN
        RETURN FALSE;
    END IF;

    SELECT COUNT(*)
    INTO control_field_count
    FROM unnest(ARRAY[
        'queued_phases', 'queued_effects', 'decision_point', 'last_turn_steps'
    ]) AS control_field(name)
    WHERE snapshot ? control_field.name
      AND snapshot -> control_field.name IS DISTINCT FROM 'null'::jsonb;
    IF control_field_count = 4 THEN
        RETURN valid_game_snapshot_v3(snapshot) IS TRUE
            AND snapshot -> 'participants' IS NOT DISTINCT FROM expected_participants;
    END IF;
    IF control_field_count <> 0 THEN
        RETURN FALSE;
    END IF;

    legacy_snapshot := snapshot
        - 'queued_phases'
        - 'queued_effects'
        - 'decision_point'
        - 'last_turn_steps';

    expected := jsonb_build_object(
        'snapshot_version', snapshot -> 'snapshot_version',
        'state_version', snapshot -> 'state_version',
        'sequence', snapshot -> 'sequence',
        'status', snapshot -> 'status',
        'adventure_id', snapshot -> 'adventure_id',
        'versions', snapshot -> 'versions',
        'turn', snapshot -> 'turn',
        'participants', snapshot -> 'participants',
        'prng', snapshot -> 'prng'
    );
    IF snapshot ? 'effects' THEN
        expected := expected || jsonb_build_object('effects', snapshot -> 'effects');
    END IF;
    IF legacy_snapshot IS DISTINCT FROM expected
       OR jsonb_typeof(snapshot -> 'snapshot_version') IS DISTINCT FROM 'number'
       OR snapshot ->> 'snapshot_version' NOT IN ('1', '2')
       OR jsonb_typeof(snapshot -> 'state_version') IS DISTINCT FROM 'number'
       OR snapshot ->> 'state_version' !~ '^[1-9][0-9]*$'
       OR (snapshot ->> 'state_version')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(snapshot -> 'sequence') IS DISTINCT FROM 'number'
       OR snapshot ->> 'sequence' !~ '^(0|[1-9][0-9]*)$'
       OR (snapshot ->> 'sequence')::NUMERIC > 9223372036854775807
       OR (snapshot ->> 'state_version')::NUMERIC
            <> (snapshot ->> 'sequence')::NUMERIC + 1
       OR jsonb_typeof(snapshot -> 'status') IS DISTINCT FROM 'string'
       OR snapshot ->> 'status' NOT IN ('in_progress', 'lost', 'won')
       OR jsonb_typeof(snapshot -> 'adventure_id') IS DISTINCT FROM 'string'
       OR snapshot ->> 'adventure_id' = ''
    THEN
        RETURN FALSE;
    END IF;

    IF snapshot -> 'versions' IS DISTINCT FROM jsonb_build_object(
            'content', snapshot -> 'versions' -> 'content',
            'ruleset', snapshot -> 'versions' -> 'ruleset',
            'manifest', snapshot -> 'versions' -> 'manifest',
            'manifest_digest', snapshot -> 'versions' -> 'manifest_digest',
            'prng', snapshot -> 'versions' -> 'prng',
            'shuffle', snapshot -> 'versions' -> 'shuffle',
            'sampling', snapshot -> 'versions' -> 'sampling'
       )
       OR jsonb_typeof(snapshot -> 'versions' -> 'content') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'content' = ''
       OR jsonb_typeof(snapshot -> 'versions' -> 'ruleset') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'ruleset' = ''
       OR jsonb_typeof(snapshot -> 'versions' -> 'manifest') IS DISTINCT FROM 'number'
       OR snapshot -> 'versions' ->> 'manifest' !~ '^[1-9][0-9]*$'
       OR (snapshot -> 'versions' ->> 'manifest')::NUMERIC > 65535
       OR jsonb_typeof(snapshot -> 'versions' -> 'manifest_digest') IS DISTINCT FROM 'string'
       OR snapshot -> 'versions' ->> 'manifest_digest' !~ '^blake3:[0-9a-f]{64}$'
       OR snapshot -> 'versions' ->> 'prng' IS DISTINCT FROM 'chacha20-v1'
       OR snapshot -> 'versions' ->> 'shuffle' IS DISTINCT FROM 'fisher-yates-v1'
       OR snapshot -> 'versions' ->> 'sampling'
            IS DISTINCT FROM 'rejection-sampling-v1'
    THEN
        RETURN FALSE;
    END IF;

    IF snapshot -> 'turn' IS DISTINCT FROM jsonb_build_object(
            'number', snapshot -> 'turn' -> 'number',
            'phase', snapshot -> 'turn' -> 'phase',
            'active_position', snapshot -> 'turn' -> 'active_position'
       )
       OR jsonb_typeof(snapshot -> 'turn' -> 'number') IS DISTINCT FROM 'number'
       OR snapshot -> 'turn' ->> 'number' !~ '^[1-9][0-9]*$'
       OR (snapshot -> 'turn' ->> 'number')::NUMERIC > 4294967295
       OR jsonb_typeof(snapshot -> 'turn' -> 'phase') IS DISTINCT FROM 'string'
       OR snapshot -> 'turn' ->> 'phase' NOT IN (
            'dark_arts', 'villains', 'hero_action', 'hero_actions', 'end_turn'
       )
       OR jsonb_typeof(snapshot -> 'turn' -> 'active_position') IS DISTINCT FROM 'number'
       OR snapshot -> 'turn' ->> 'active_position' !~ '^[1-4]$'
       OR snapshot -> 'participants' IS DISTINCT FROM expected_participants
       OR jsonb_typeof(snapshot -> 'participants') IS DISTINCT FROM 'array'
    THEN
        RETURN FALSE;
    END IF;

    participant_count := jsonb_array_length(snapshot -> 'participants');
    IF participant_count NOT BETWEEN 2 AND 4
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
            WHERE participant IS DISTINCT FROM jsonb_build_object(
                    'participant_id', participant -> 'participant_id',
                    'position', participant -> 'position',
                    'hero_id', participant -> 'hero_id'
                )
               OR jsonb_typeof(participant -> 'participant_id') IS DISTINCT FROM 'string'
               OR participant ->> 'participant_id' = ''
               OR jsonb_typeof(participant -> 'position') IS DISTINCT FROM 'number'
               OR participant ->> 'position' !~ '^[1-4]$'
               OR jsonb_typeof(participant -> 'hero_id') IS DISTINCT FROM 'string'
               OR participant ->> 'hero_id' NOT IN ('harry', 'hermione', 'neville', 'ron')
       )
       OR participant_count <> (
            SELECT COUNT(DISTINCT participant ->> 'participant_id')
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
       )
       OR participant_count <> (
            SELECT COUNT(DISTINCT participant ->> 'position')
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
       )
       OR participant_count <> (
            SELECT COUNT(DISTINCT participant ->> 'hero_id')
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
       )
       OR (
            SELECT ARRAY_AGG(
                (participant ->> 'position')::INTEGER
                ORDER BY (participant ->> 'position')::INTEGER
            )
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
       ) <> ARRAY(SELECT generate_series(1, participant_count))
       OR NOT EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
            WHERE participant -> 'position' = snapshot -> 'turn' -> 'active_position'
       )
       OR snapshot -> 'prng' IS DISTINCT FROM jsonb_build_object(
            'algorithm', snapshot -> 'prng' -> 'algorithm',
            'counter', snapshot -> 'prng' -> 'counter'
       )
       OR snapshot -> 'prng' ->> 'algorithm' IS DISTINCT FROM 'chacha20-v1'
       OR jsonb_typeof(snapshot -> 'prng' -> 'counter') IS DISTINCT FROM 'number'
       OR snapshot -> 'prng' ->> 'counter' !~ '^(0|[1-9][0-9]*)$'
       OR (snapshot -> 'prng' ->> 'counter')::NUMERIC > 9223372036854775807
    THEN
        RETURN FALSE;
    END IF;

    effects := COALESCE(snapshot -> 'effects', '{}'::jsonb);
    IF jsonb_typeof(effects) IS DISTINCT FROM 'object'
       OR EXISTS (
            SELECT 1
            FROM jsonb_object_keys(effects) AS effect_key
            WHERE effect_key NOT IN ('entities', 'outcomes', 'choice')
       )
       OR (
            effects ? 'entities'
            AND (
                jsonb_typeof(effects -> 'entities') IS DISTINCT FROM 'array'
                OR jsonb_array_length(effects -> 'entities') > 4096
                OR EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(effects -> 'entities') AS entity
                    WHERE valid_legacy_effect_entity_for_v15(entity) IS NOT TRUE
                )
            )
       )
       OR (
            effects ? 'outcomes'
            AND (
                jsonb_typeof(effects -> 'outcomes') IS DISTINCT FROM 'array'
                OR jsonb_array_length(effects -> 'outcomes') > 4096
                OR EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(effects -> 'outcomes') AS outcome
                    WHERE valid_legacy_effect_outcome_for_v15(outcome) IS NOT TRUE
                )
            )
       )
       OR (
            effects ? 'choice'
            AND effects -> 'choice' IS DISTINCT FROM 'null'::jsonb
            AND CASE snapshot ->> 'snapshot_version'
                WHEN '1' THEN FALSE
                WHEN '2' THEN valid_pending_effect_choice_v4(effects -> 'choice')
                ELSE FALSE
            END IS NOT TRUE
       )
    THEN
        RETURN FALSE;
    END IF;

    normalized_entities := normalized_effect_entities_for_turn_v4(snapshot);
    IF jsonb_typeof(normalized_entities) IS DISTINCT FROM 'array'
       OR jsonb_array_length(normalized_entities) NOT BETWEEN 2 AND 4096
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(normalized_entities) AS entity
            WHERE valid_legacy_effect_entity_for_v15(entity) IS NOT TRUE
       )
       OR jsonb_array_length(normalized_entities) <> (
            SELECT COUNT(DISTINCT entity ->> 'id')
            FROM jsonb_array_elements(normalized_entities) AS entity
       )
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(normalized_entities) AS entity
            WHERE jsonb_typeof(entity -> 'owner_position') = 'number'
              AND NOT EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(snapshot -> 'participants') AS participant
                    WHERE participant -> 'position' = entity -> 'owner_position'
              )
       )
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(snapshot -> 'participants') AS participant
            WHERE (
                SELECT COUNT(*)
                FROM jsonb_array_elements(normalized_entities) AS entity
                WHERE entity ->> 'zone' = 'heroes'
                  AND entity -> 'owner_position' = participant -> 'position'
            ) <> 1
       )
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(normalized_entities) AS entity
            WHERE entity ->> 'zone' IN (
                'active_villains', 'dark_arts_deck', 'dark_arts_discard', 'hero_discard_pile',
                'hero_draw_pile', 'hero_hand', 'hero_play_area', 'hogwarts_deck',
                'market', 'villain_deck'
            )
            GROUP BY entity -> 'owner_position', entity ->> 'zone'
            HAVING MIN((entity ->> 'zone_index')::INTEGER) <> 0
                OR MAX((entity ->> 'zone_index')::INTEGER) <> COUNT(*) - 1
                OR COUNT(*) <> COUNT(DISTINCT (entity ->> 'zone_index')::INTEGER)
       )
    THEN
        RETURN FALSE;
    END IF;

    pending_choice := effects -> 'choice';
    IF pending_choice = 'null'::jsonb THEN
        pending_choice := NULL;
    END IF;
    IF pending_choice IS NOT NULL
       AND (
            NOT EXISTS (
                SELECT 1
                FROM jsonb_array_elements(snapshot -> 'participants') AS participant
                WHERE participant -> 'position'
                    = pending_choice -> 'responsible_position'
            )
            OR EXISTS (
                SELECT 1
                FROM jsonb_array_elements(
                    pending_choice -> 'continuation' -> 'queue'
                ) AS queued
                WHERE NOT EXISTS (
                    SELECT 1
                    FROM jsonb_array_elements(snapshot -> 'participants') AS participant
                    WHERE participant ->> 'position' = COALESCE(
                        queued ->> 'actor_position',
                        queued ->> 'responsible_position'
                    )
                )
            )
       )
    THEN
        RETURN FALSE;
    END IF;
    IF snapshot ->> 'status' = 'in_progress' THEN
        RETURN CASE snapshot -> 'turn' ->> 'phase'
            WHEN 'dark_arts' THEN TRUE
            WHEN 'hero_action' THEN pending_choice IS NULL
            WHEN 'hero_actions' THEN pending_choice IS NULL
            ELSE FALSE
        END;
    END IF;
    RETURN pending_choice IS NULL;
EXCEPTION
    WHEN OTHERS THEN
        RETURN FALSE;
END;
$$;

CREATE FUNCTION require_game_snapshot_for_v15_upgrade(
    target_game_id UUID,
    candidate_snapshot JSONB
)
RETURNS VOID
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    expected_participants JSONB;
    stored games%ROWTYPE;
BEGIN
    SELECT *
    INTO stored
    FROM games
    WHERE id = target_game_id;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'legacy game % does not exist for schema version 15 preflight',
            target_game_id
            USING ERRCODE = '55000';
    END IF;

    SELECT COALESCE(
        jsonb_agg(
            jsonb_build_object(
                'participant_id', participants.id::TEXT,
                'position', participants.position,
                'hero_id', participants.hero_id
            )
            ORDER BY participants.position
        ),
        '[]'::jsonb
    )
    INTO expected_participants
    FROM participants
    WHERE participants.room_id = stored.room_id;

    IF valid_game_snapshot_for_v15_upgrade(
            candidate_snapshot,
            expected_participants
       ) IS NOT TRUE
       OR candidate_snapshot ->> 'snapshot_version'
            IS DISTINCT FROM stored.snapshot_version::TEXT
       OR candidate_snapshot ->> 'state_version'
            IS DISTINCT FROM stored.state_version::TEXT
       OR candidate_snapshot ->> 'sequence' IS DISTINCT FROM stored.sequence::TEXT
       OR candidate_snapshot ->> 'status' IS DISTINCT FROM stored.status
       OR candidate_snapshot ->> 'adventure_id' IS DISTINCT FROM stored.adventure_id
       OR candidate_snapshot -> 'versions' ->> 'content'
            IS DISTINCT FROM stored.content_version
       OR candidate_snapshot -> 'versions' ->> 'ruleset'
            IS DISTINCT FROM stored.ruleset_version
       OR candidate_snapshot -> 'versions' ->> 'manifest'
            IS DISTINCT FROM stored.manifest_version::TEXT
       OR candidate_snapshot -> 'versions' ->> 'manifest_digest'
            IS DISTINCT FROM stored.manifest_digest
       OR candidate_snapshot -> 'versions' ->> 'prng'
            IS DISTINCT FROM stored.prng_algorithm
       OR candidate_snapshot -> 'versions' ->> 'shuffle'
            IS DISTINCT FROM stored.shuffle_algorithm
       OR candidate_snapshot -> 'versions' ->> 'sampling'
            IS DISTINCT FROM stored.sampling_algorithm
       OR candidate_snapshot -> 'prng' ->> 'algorithm'
            IS DISTINCT FROM stored.prng_algorithm
       OR candidate_snapshot -> 'prng' ->> 'counter'
            IS DISTINCT FROM stored.prng_counter::TEXT
    THEN
        RAISE EXCEPTION 'legacy game % snapshot cannot be restored by schema version 15',
            target_game_id
            USING
                ERRCODE = '55000',
                HINT = 'Repair or quarantine the invalid legacy snapshot before applying this migration.';
    END IF;
END;
$$;

DO $$
DECLARE
    legacy_game RECORD;
BEGIN
    FOR legacy_game IN
        SELECT id, snapshot
        FROM games
        ORDER BY id
    LOOP
        PERFORM require_game_snapshot_for_v15_upgrade(
            legacy_game.id,
            legacy_game.snapshot
        );
    END LOOP;
END;
$$;

CREATE FUNCTION move_effect_entity_v4(
    entities JSONB,
    target_id TEXT,
    expected_owner JSONB,
    expected_from TEXT,
    destination TEXT
)
RETURNS JSONB
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    destination_index INTEGER;
    moved JSONB;
    owner_value JSONB;
    source_index INTEGER;
    target JSONB;
    target_offset INTEGER;
BEGIN
    IF jsonb_typeof(entities) IS DISTINCT FROM 'array'
       OR target_id IS NULL
       OR expected_from IS NULL
       OR destination IS NULL
       OR expected_from = destination
    THEN
        RETURN NULL;
    END IF;

    SELECT entity.value, entity.position - 1
    INTO target, target_offset
    FROM jsonb_array_elements(entities) WITH ORDINALITY AS entity(value, position)
    WHERE entity.value ->> 'id' = target_id;

    IF NOT FOUND
       OR target ->> 'zone' IS DISTINCT FROM expected_from
       OR jsonb_typeof(target -> 'zone_index') IS DISTINCT FROM 'number'
       OR (
            target -> 'owner_position' IS DISTINCT FROM expected_owner
            AND NOT (
                target -> 'owner_position' IS NULL
                AND jsonb_typeof(expected_owner) = 'number'
                AND expected_from IN ('hogwarts_deck', 'market')
                AND destination IN (
                    'hero_discard_pile', 'hero_draw_pile',
                    'hero_hand', 'hero_play_area'
                )
            )
       )
    THEN
        RETURN NULL;
    END IF;

    owner_value := target -> 'owner_position';
    source_index := (target ->> 'zone_index')::INTEGER;
    SELECT COUNT(*)
    INTO destination_index
    FROM jsonb_array_elements(entities) AS entity
    WHERE entity -> 'owner_position' IS NOT DISTINCT FROM expected_owner
      AND entity ->> 'zone' = destination;

    SELECT jsonb_agg(
        CASE
            WHEN entity.position - 1 = target_offset THEN
                jsonb_set(
                    jsonb_set(
                        (entity.value - 'owner_position') ||
                            CASE
                                WHEN expected_owner IS NULL THEN '{}'::jsonb
                                ELSE jsonb_build_object('owner_position', expected_owner)
                            END,
                        '{zone}',
                        to_jsonb(destination),
                        FALSE
                    ),
                    '{zone_index}',
                    to_jsonb(destination_index),
                    FALSE
                )
            WHEN entity.value -> 'owner_position' IS NOT DISTINCT FROM owner_value
                 AND entity.value ->> 'zone' = expected_from
                 AND (entity.value ->> 'zone_index')::INTEGER > source_index
            THEN jsonb_set(
                entity.value,
                '{zone_index}',
                to_jsonb((entity.value ->> 'zone_index')::INTEGER - 1),
                FALSE
            )
            ELSE entity.value
        END
        ORDER BY entity.position
    )
    INTO moved
    FROM jsonb_array_elements(entities) WITH ORDINALITY AS entity(value, position);
    RETURN moved;
END;
$$;

CREATE FUNCTION change_effect_entity_resource_v4(
    entities JSONB,
    target_id TEXT,
    expected_owner JSONB,
    resource_name TEXT,
    expected_before INTEGER,
    committed_after INTEGER
)
RETURNS JSONB
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    changed JSONB;
    target JSONB;
    target_offset INTEGER;
    zone_name TEXT;
BEGIN
    SELECT entity.value, entity.position - 1
    INTO target, target_offset
    FROM jsonb_array_elements(entities) WITH ORDINALITY AS entity(value, position)
    WHERE entity.value ->> 'id' = target_id;

    IF NOT FOUND
       OR target -> 'owner_position' IS DISTINCT FROM expected_owner
       OR COALESCE((target -> 'resources' ->> resource_name)::INTEGER, 0)
            <> expected_before
    THEN
        RETURN NULL;
    END IF;

    zone_name := target ->> 'zone';
    IF NOT (
        (zone_name = 'heroes' AND resource_name IN ('attack', 'health', 'influence'))
        OR (zone_name = 'active_villains' AND resource_name = 'health')
        OR (zone_name = 'active_location' AND resource_name = 'control')
    ) THEN
        RETURN NULL;
    END IF;

    SELECT jsonb_agg(
        CASE
            WHEN entity.position - 1 = target_offset THEN jsonb_set(
                entity.value || jsonb_build_object(
                    'resources', COALESCE(entity.value -> 'resources', '{}'::jsonb)
                ),
                ARRAY['resources', resource_name],
                to_jsonb(committed_after),
                TRUE
            )
            ELSE entity.value
        END
        ORDER BY entity.position
    )
    INTO changed
    FROM jsonb_array_elements(entities) WITH ORDINALITY AS entity(value, position);
    RETURN changed;
END;
$$;

CREATE FUNCTION apply_effect_steps_v4(initial_world JSONB, steps JSONB)
RETURNS JSONB
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    effect JSONB;
    world JSONB := initial_world;
BEGIN
    IF jsonb_typeof(world) IS DISTINCT FROM 'array'
       OR jsonb_typeof(steps) IS DISTINCT FROM 'array'
    THEN
        RETURN NULL;
    END IF;

    FOR effect IN
        SELECT outcome.value
        FROM jsonb_array_elements(steps) WITH ORDINALITY AS step(value, position)
        CROSS JOIN LATERAL jsonb_array_elements(step.value -> 'effects')
            WITH ORDINALITY AS outcome(value, position)
        ORDER BY step.position, outcome.position
    LOOP
        CASE effect ->> 'type'
            WHEN 'moved' THEN
                world := move_effect_entity_v4(
                    world,
                    effect ->> 'target_id',
                    effect -> 'target_position',
                    effect ->> 'from',
                    effect ->> 'to'
                );
            WHEN 'resource_changed' THEN
                world := change_effect_entity_resource_v4(
                    world,
                    effect ->> 'target_id',
                    effect -> 'target_position',
                    effect ->> 'resource',
                    (effect ->> 'before')::INTEGER,
                    (effect ->> 'after')::INTEGER
                );
            ELSE
                NULL;
        END CASE;

        IF world IS NULL THEN
            RETURN NULL;
        END IF;
    END LOOP;

    RETURN world;
EXCEPTION
    WHEN OTHERS THEN
        RETURN NULL;
END;
$$;

CREATE FUNCTION merge_turn_steps_v4(previous_steps JSONB, incremental_steps JSONB)
RETURNS JSONB
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    incremental_index INTEGER := 0;
    merged JSONB := previous_steps;
    previous_index INTEGER;
BEGIN
    IF jsonb_typeof(previous_steps) IS DISTINCT FROM 'array'
       OR jsonb_typeof(incremental_steps) IS DISTINCT FROM 'array'
       OR jsonb_array_length(incremental_steps) = 0
    THEN
        RETURN NULL;
    END IF;

    IF jsonb_array_length(merged) > 0
       AND merged -> -1 ->> 'phase'
            IS NOT DISTINCT FROM incremental_steps -> 0 ->> 'phase'
    THEN
        previous_index := jsonb_array_length(merged) - 1;
        merged := jsonb_set(
            merged,
            ARRAY[previous_index::TEXT, 'effects'],
            (merged -> previous_index -> 'effects')
                || (incremental_steps -> 0 -> 'effects'),
            FALSE
        );
        incremental_index := 1;
    END IF;

    WHILE incremental_index < jsonb_array_length(incremental_steps) LOOP
        merged := merged || jsonb_build_array(
            incremental_steps -> incremental_index
        );
        incremental_index := incremental_index + 1;
    END LOOP;

    IF jsonb_array_length(merged) > 3 THEN
        RETURN NULL;
    END IF;
    RETURN merged;
EXCEPTION
    WHEN OTHERS THEN
        RETURN NULL;
END;
$$;

CREATE FUNCTION valid_choice_world_transition_v4(
    previous_snapshot JSONB,
    committed_entities JSONB,
    payload JSONB
)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    world JSONB;
BEGIN
    world := apply_effect_steps_v4(
        normalized_effect_entities_for_turn_v4(previous_snapshot),
        payload -> 'steps'
    );
    RETURN world IS NOT NULL
        AND world IS NOT DISTINCT FROM committed_entities;
EXCEPTION
    WHEN OTHERS THEN
        RETURN FALSE;
END;
$$;

CREATE FUNCTION valid_turn_world_transition_v4(
    previous_snapshot JSONB,
    committed_entities JSONB,
    payload JSONB
)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    actor_position INTEGER;
    before_value INTEGER;
    card_id TEXT;
    committed_cards TEXT[];
    current_cards TEXT[];
    discard_count INTEGER;
    effect JSONB;
    hand_count INTEGER;
    hero_id TEXT;
    outcome JSONB;
    outcome_index INTEGER := 0;
    outcomes JSONB;
    resource_name TEXT;
    top_card_id TEXT;
    world JSONB := normalized_effect_entities_for_turn_v4(previous_snapshot);
BEGIN
    IF jsonb_typeof(world) IS DISTINCT FROM 'array'
       OR jsonb_typeof(committed_entities) IS DISTINCT FROM 'array'
       OR jsonb_typeof(payload -> 'end_turn') IS DISTINCT FROM 'array'
       OR jsonb_typeof(payload -> 'steps') IS DISTINCT FROM 'array'
       OR payload ->> 'actor_position' !~ '^[1-4]$'
    THEN
        RETURN FALSE;
    END IF;
    actor_position := (payload ->> 'actor_position')::INTEGER;
    outcomes := payload -> 'end_turn';

    FOR resource_name IN SELECT unnest(ARRAY['hero_play_area', 'hero_hand']) LOOP
        FOR card_id IN
            SELECT entity ->> 'id'
            FROM jsonb_array_elements(world) AS entity
            WHERE entity ->> 'owner_position' = actor_position::TEXT
              AND entity ->> 'zone' = resource_name
            ORDER BY (entity ->> 'zone_index')::INTEGER
        LOOP
            outcome := outcomes -> outcome_index;
            IF outcome IS DISTINCT FROM jsonb_build_object(
                'type', 'card_moved',
                'card_id', card_id,
                'from', resource_name,
                'to', 'hero_discard_pile'
            ) THEN
                RETURN FALSE;
            END IF;
            world := move_effect_entity_v4(
                world,
                card_id,
                to_jsonb(actor_position),
                resource_name,
                'hero_discard_pile'
            );
            IF world IS NULL THEN
                RETURN FALSE;
            END IF;
            outcome_index := outcome_index + 1;
        END LOOP;
    END LOOP;

    FOR resource_name IN SELECT unnest(ARRAY['attack', 'influence']) LOOP
        SELECT
            entity ->> 'id',
            COALESCE((entity -> 'resources' ->> resource_name)::INTEGER, 0)
        INTO hero_id, before_value
        FROM jsonb_array_elements(world) AS entity
        WHERE entity ->> 'owner_position' = actor_position::TEXT
          AND entity ->> 'zone' = 'heroes';
        IF NOT FOUND THEN
            RETURN FALSE;
        END IF;

        outcome := outcomes -> outcome_index;
        IF outcome IS DISTINCT FROM jsonb_build_object(
            'type', 'resource_reset',
            'resource', resource_name,
            'before', before_value
        ) THEN
            RETURN FALSE;
        END IF;
        world := change_effect_entity_resource_v4(
            world,
            hero_id,
            to_jsonb(actor_position),
            resource_name,
            before_value,
            0
        );
        IF world IS NULL THEN
            RETURN FALSE;
        END IF;
        outcome_index := outcome_index + 1;
    END LOOP;

    LOOP
        SELECT COUNT(*)
        INTO hand_count
        FROM jsonb_array_elements(world) AS entity
        WHERE entity ->> 'owner_position' = actor_position::TEXT
          AND entity ->> 'zone' = 'hero_hand';
        EXIT WHEN hand_count >= 5;

        SELECT entity ->> 'id'
        INTO top_card_id
        FROM jsonb_array_elements(world) AS entity
        WHERE entity ->> 'owner_position' = actor_position::TEXT
          AND entity ->> 'zone' = 'hero_draw_pile'
        ORDER BY (entity ->> 'zone_index')::INTEGER DESC
        LIMIT 1;

        IF FOUND THEN
            outcome := outcomes -> outcome_index;
            IF outcome IS DISTINCT FROM jsonb_build_object(
                'type', 'card_moved',
                'card_id', top_card_id,
                'from', 'hero_draw_pile',
                'to', 'hero_hand'
            ) THEN
                RETURN FALSE;
            END IF;
            world := move_effect_entity_v4(
                world,
                top_card_id,
                to_jsonb(actor_position),
                'hero_draw_pile',
                'hero_hand'
            );
            IF world IS NULL THEN
                RETURN FALSE;
            END IF;
            outcome_index := outcome_index + 1;
            CONTINUE;
        END IF;

        SELECT COUNT(*)
        INTO discard_count
        FROM jsonb_array_elements(world) AS entity
        WHERE entity ->> 'owner_position' = actor_position::TEXT
          AND entity ->> 'zone' = 'hero_discard_pile';
        EXIT WHEN discard_count = 0;

        outcome := outcomes -> outcome_index;
        IF outcome IS NULL
           OR outcome ->> 'type' IS DISTINCT FROM 'pile_shuffled'
           OR outcome ->> 'owner_position' IS DISTINCT FROM actor_position::TEXT
           OR outcome ->> 'zone' IS DISTINCT FROM 'hero_draw_pile'
           OR jsonb_typeof(outcome -> 'bottom_to_top') IS DISTINCT FROM 'array'
        THEN
            RETURN FALSE;
        END IF;
        SELECT ARRAY_AGG(entity ->> 'id' ORDER BY entity ->> 'id')
        INTO current_cards
        FROM jsonb_array_elements(world) AS entity
        WHERE entity ->> 'owner_position' = actor_position::TEXT
          AND entity ->> 'zone' = 'hero_discard_pile';
        SELECT ARRAY_AGG(card #>> '{}' ORDER BY card #>> '{}')
        INTO committed_cards
        FROM jsonb_array_elements(outcome -> 'bottom_to_top') AS card;
        IF current_cards IS DISTINCT FROM committed_cards THEN
            RETURN FALSE;
        END IF;

        FOR card_id IN
            SELECT card #>> '{}'
            FROM jsonb_array_elements(outcome -> 'bottom_to_top') AS card
        LOOP
            world := move_effect_entity_v4(
                world,
                card_id,
                to_jsonb(actor_position),
                'hero_discard_pile',
                'hero_draw_pile'
            );
            IF world IS NULL THEN
                RETURN FALSE;
            END IF;
        END LOOP;
        outcome_index := outcome_index + 1;
    END LOOP;

    IF outcome_index <> jsonb_array_length(outcomes) THEN
        RETURN FALSE;
    END IF;

    FOR effect IN
        SELECT effect.value
        FROM jsonb_array_elements(payload -> 'steps') WITH ORDINALITY AS step(value, position)
        CROSS JOIN LATERAL jsonb_array_elements(step.value -> 'effects')
            WITH ORDINALITY AS effect(value, position)
        WHERE step.position > 1
        ORDER BY step.position, effect.position
    LOOP
        CASE effect ->> 'type'
            WHEN 'moved' THEN
                world := move_effect_entity_v4(
                    world,
                    effect ->> 'target_id',
                    effect -> 'target_position',
                    effect ->> 'from',
                    effect ->> 'to'
                );
            WHEN 'resource_changed' THEN
                world := change_effect_entity_resource_v4(
                    world,
                    effect ->> 'target_id',
                    effect -> 'target_position',
                    effect ->> 'resource',
                    (effect ->> 'before')::INTEGER,
                    (effect ->> 'after')::INTEGER
                );
            ELSE
                NULL;
        END CASE;
        IF world IS NULL THEN
            RETURN FALSE;
        END IF;
    END LOOP;

    RETURN world IS NOT DISTINCT FROM committed_entities;
END;
$$;

CREATE FUNCTION valid_turn_completed_payload_v4(payload JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    step_count INTEGER;
BEGIN
    IF compact_jsonb_octet_length(payload) > 4194304
       OR payload IS DISTINCT FROM jsonb_build_object(
            'event_version', payload -> 'event_version',
            'type', payload -> 'type',
            'sequence', payload -> 'sequence',
            'state_version', payload -> 'state_version',
            'turn', payload -> 'turn',
            'actor_position', payload -> 'actor_position',
            'end_turn', payload -> 'end_turn',
            'steps', payload -> 'steps',
            'control', payload -> 'control',
            'prng_counter', payload -> 'prng_counter'
        )
       OR jsonb_typeof(payload -> 'event_version') IS DISTINCT FROM 'number'
       OR payload ->> 'event_version' <> '4'
       OR jsonb_typeof(payload -> 'type') IS DISTINCT FROM 'string'
       OR payload ->> 'type' <> 'turn_completed'
       OR jsonb_typeof(payload -> 'sequence') IS DISTINCT FROM 'number'
       OR payload ->> 'sequence' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'sequence')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(payload -> 'state_version') IS DISTINCT FROM 'number'
       OR payload ->> 'state_version' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'state_version')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(payload -> 'turn') IS DISTINCT FROM 'number'
       OR payload ->> 'turn' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'turn')::NUMERIC >= 4294967295
       OR jsonb_typeof(payload -> 'actor_position') IS DISTINCT FROM 'number'
       OR payload ->> 'actor_position' !~ '^[1-4]$'
       OR valid_end_turn_sequence_v4(
            payload -> 'end_turn',
            (payload ->> 'actor_position')::INTEGER
       ) IS NOT TRUE
       OR jsonb_typeof(payload -> 'steps') IS DISTINCT FROM 'array'
       OR valid_engine_control_v4(payload -> 'control') IS NOT TRUE
       OR jsonb_typeof(payload -> 'prng_counter') IS DISTINCT FROM 'number'
       OR payload ->> 'prng_counter' !~ '^(0|[1-9][0-9]*)$'
       OR (payload ->> 'prng_counter')::NUMERIC > 9223372036854775807
    THEN
        RETURN FALSE;
    END IF;

    step_count := jsonb_array_length(payload -> 'steps');
    IF step_count NOT IN (2, 3)
       OR payload -> 'steps' -> 0 ->> 'phase' IS DISTINCT FROM 'end_turn'
       OR payload -> 'steps' -> 0 -> 'effects' IS DISTINCT FROM '[]'::jsonb
       OR payload -> 'steps' -> 1 ->> 'phase' IS DISTINCT FROM 'dark_arts'
       OR (
            step_count = 3
            AND payload -> 'steps' -> 2 ->> 'phase' IS DISTINCT FROM 'villains'
       )
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(payload -> 'steps') AS step
            WHERE valid_turn_step_v4(step) IS NOT TRUE
       )
       OR (
            SELECT COALESCE(SUM(jsonb_array_length(step -> 'effects')), 0)
            FROM jsonb_array_elements(payload -> 'steps') AS step
            WHERE step ->> 'phase' <> 'end_turn'
       ) > 4096
       OR (payload -> 'control' ->> 'turn')::NUMERIC
            <> (payload ->> 'turn')::NUMERIC + 1
    THEN
        RETURN FALSE;
    END IF;

    IF payload -> 'control' ->> 'status' = 'in_progress'
       AND payload -> 'control' -> 'decision_point' ->> 'type' = 'player_intent'
    THEN
        IF step_count <> 3 OR payload -> 'control' ->> 'phase' <> 'hero_actions' THEN
            RETURN FALSE;
        END IF;
    ELSIF payload -> 'control' ->> 'status' = 'in_progress'
          AND payload -> 'control' -> 'decision_point' ->> 'type' = 'effect_choice'
    THEN
        IF payload -> 'control' ->> 'phase'
            IS DISTINCT FROM payload -> 'steps' -> -1 ->> 'phase'
        THEN
            RETURN FALSE;
        END IF;
    ELSIF payload -> 'control' ->> 'status' IN ('lost', 'won') THEN
        IF payload -> 'control' ->> 'phase'
            IS DISTINCT FROM payload -> 'steps' -> -1 ->> 'phase'
        THEN
            RETURN FALSE;
        END IF;
    ELSE
        RETURN FALSE;
    END IF;

    RETURN TRUE;
END;
$$;

CREATE FUNCTION valid_choice_resolved_payload_v4(payload JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    step_count INTEGER;
BEGIN
    IF compact_jsonb_octet_length(payload) > 4194304
       OR payload IS DISTINCT FROM jsonb_build_object(
            'event_version', payload -> 'event_version',
            'type', payload -> 'type',
            'sequence', payload -> 'sequence',
            'state_version', payload -> 'state_version',
            'turn', payload -> 'turn',
            'actor_position', payload -> 'actor_position',
            'choice_id', payload -> 'choice_id',
            'choice_cause', payload -> 'choice_cause',
            'selected_options', payload -> 'selected_options',
            'steps', payload -> 'steps',
            'control', payload -> 'control',
            'prng_counter', payload -> 'prng_counter'
        )
       OR jsonb_typeof(payload -> 'event_version') IS DISTINCT FROM 'number'
       OR payload ->> 'event_version' <> '4'
       OR jsonb_typeof(payload -> 'type') IS DISTINCT FROM 'string'
       OR payload ->> 'type' <> 'choice_resolved'
       OR jsonb_typeof(payload -> 'sequence') IS DISTINCT FROM 'number'
       OR payload ->> 'sequence' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'sequence')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(payload -> 'state_version') IS DISTINCT FROM 'number'
       OR payload ->> 'state_version' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'state_version')::NUMERIC > 9223372036854775807
       OR jsonb_typeof(payload -> 'turn') IS DISTINCT FROM 'number'
       OR payload ->> 'turn' !~ '^[1-9][0-9]*$'
       OR (payload ->> 'turn')::NUMERIC > 4294967295
       OR jsonb_typeof(payload -> 'actor_position') IS DISTINCT FROM 'number'
       OR payload ->> 'actor_position' !~ '^[1-4]$'
       OR jsonb_typeof(payload -> 'choice_id') IS DISTINCT FROM 'string'
       OR payload ->> 'choice_id' = ''
       OR octet_length(payload ->> 'choice_id') > 256
       OR jsonb_typeof(payload -> 'choice_cause') IS DISTINCT FROM 'string'
       OR payload ->> 'choice_cause' = ''
       OR octet_length(payload ->> 'choice_cause') > 256
       OR jsonb_typeof(payload -> 'selected_options') IS DISTINCT FROM 'array'
       OR jsonb_array_length(payload -> 'selected_options') > 32
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(payload -> 'selected_options') AS option
            WHERE jsonb_typeof(option) <> 'string'
               OR option #>> '{}' = ''
               OR octet_length(option #>> '{}') > 256
       )
       OR (
            SELECT COUNT(*)
            FROM jsonb_array_elements(payload -> 'selected_options') AS option
       ) <> (
            SELECT COUNT(DISTINCT option)
            FROM jsonb_array_elements(payload -> 'selected_options') AS option
       )
       OR jsonb_typeof(payload -> 'steps') IS DISTINCT FROM 'array'
       OR valid_engine_control_v4(payload -> 'control') IS NOT TRUE
       OR jsonb_typeof(payload -> 'prng_counter') IS DISTINCT FROM 'number'
       OR payload ->> 'prng_counter' !~ '^(0|[1-9][0-9]*)$'
       OR (payload ->> 'prng_counter')::NUMERIC > 9223372036854775807
    THEN
        RETURN FALSE;
    END IF;

    step_count := jsonb_array_length(payload -> 'steps');
    IF step_count NOT BETWEEN 1 AND 2
       OR payload -> 'steps' -> 0 ->> 'phase'
            NOT IN ('dark_arts', 'villains', 'hero_actions')
       OR (
            step_count = 2
            AND (
                payload -> 'steps' -> 0 ->> 'phase' <> 'dark_arts'
                OR payload -> 'steps' -> 1 ->> 'phase' <> 'villains'
            )
       )
       OR EXISTS (
            SELECT 1
            FROM jsonb_array_elements(payload -> 'steps') AS step
            WHERE valid_turn_step_v4(step) IS NOT TRUE
       )
       OR (
            SELECT COALESCE(SUM(jsonb_array_length(step -> 'effects')), 0)
            FROM jsonb_array_elements(payload -> 'steps') AS step
       ) > 4096
       OR payload -> 'control' ->> 'turn' IS DISTINCT FROM payload ->> 'turn'
    THEN
        RETURN FALSE;
    END IF;

    IF payload -> 'control' ->> 'status' = 'in_progress'
       AND payload -> 'control' -> 'decision_point' ->> 'type' = 'player_intent'
    THEN
        RETURN payload -> 'control' ->> 'phase' = 'hero_actions'
            AND payload -> 'steps' -> -1 ->> 'phase' IN ('villains', 'hero_actions');
    END IF;

    IF (
        payload -> 'control' ->> 'status' = 'in_progress'
        AND payload -> 'control' -> 'decision_point' ->> 'type' = 'effect_choice'
    ) OR payload -> 'control' ->> 'status' IN ('lost', 'won')
    THEN
        RETURN payload -> 'control' ->> 'phase'
            = payload -> 'steps' -> -1 ->> 'phase';
    END IF;

    RETURN FALSE;
EXCEPTION
    WHEN OTHERS THEN
        RETURN FALSE;
END;
$$;

CREATE FUNCTION valid_hero_action_payload_v4(
    payload JSONB,
    relational_event_type TEXT,
    relational_sequence BIGINT,
    relational_state_version BIGINT,
    relational_actor_position SMALLINT,
    committed_prng_counter BIGINT,
    committed_status TEXT
)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
RETURN relational_event_type IN ('card_played', 'attack_assigned', 'card_acquired')
    AND payload ->> 'event_version' = '4'
    AND valid_game_event_v3(
        jsonb_set(payload, '{event_version}', '3'::jsonb, FALSE),
        relational_event_type,
        relational_sequence,
        relational_state_version,
        relational_actor_position,
        committed_prng_counter,
        committed_status
    );

CREATE OR REPLACE FUNCTION require_contiguous_game_event_sequence()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    actor_position SMALLINT;
    committed_prng_counter BIGINT;
    committed_sequence BIGINT;
    committed_snapshot JSONB;
    committed_status TEXT;
    committed_state_version BIGINT;
    expected_sequence BIGINT;
    next_actor_position SMALLINT;
    terminal_count BIGINT;
BEGIN
    SELECT sequence, state_version, prng_counter, snapshot, status
    INTO committed_sequence, committed_state_version, committed_prng_counter,
        committed_snapshot, committed_status
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

    SELECT COALESCE(
        MIN(position) FILTER (WHERE position > actor_position),
        MIN(position)
    )
    INTO next_actor_position
    FROM participants
    WHERE room_id = NEW.room_id;

    IF NEW.event_version = 4
       AND NEW.event_type IN ('card_played', 'attack_assigned', 'card_acquired')
    THEN
        IF actor_position IS NULL
           OR valid_hero_action_payload_v4(
                NEW.payload,
                NEW.event_type,
                NEW.sequence,
                NEW.state_version,
                actor_position,
                committed_prng_counter,
                committed_status
           ) IS NOT TRUE
        THEN
            RAISE EXCEPTION 'hero action event payload must match the current codec shape'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    IF NEW.event_version IN (1, 2, 3) OR NEW.event_type = 'dark_arts_completed' THEN
        RAISE EXCEPTION 'legacy game event codecs are read-only'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.event_version <> 4
       OR NEW.event_type NOT IN ('turn_completed', 'choice_resolved')
       OR (CASE NEW.event_type
            WHEN 'turn_completed' THEN valid_turn_completed_payload_v4(NEW.payload)
            WHEN 'choice_resolved' THEN valid_choice_resolved_payload_v4(NEW.payload)
            ELSE FALSE
          END) IS NOT TRUE
    THEN
        RAISE EXCEPTION 'game event payload must match the current codec shape'
            USING ERRCODE = '23514';
    END IF;

    IF actor_position IS NULL
       OR (NEW.payload ->> 'sequence')::NUMERIC <> NEW.sequence
       OR (NEW.payload ->> 'state_version')::NUMERIC <> NEW.state_version
       OR (NEW.payload ->> 'actor_position')::NUMERIC <> actor_position
       OR (NEW.payload ->> 'prng_counter')::NUMERIC <> committed_prng_counter
       OR NEW.payload -> 'control' ->> 'status' IS DISTINCT FROM committed_status
       OR NEW.payload -> 'control' IS DISTINCT FROM jsonb_build_object(
            'status', committed_snapshot -> 'status',
            'turn', committed_snapshot -> 'turn' -> 'number',
            'phase', committed_snapshot -> 'turn' -> 'phase',
            'active_position', committed_snapshot -> 'turn' -> 'active_position',
            'queued_phases', committed_snapshot -> 'queued_phases',
            'queued_effects', committed_snapshot -> 'queued_effects',
            'decision_point', committed_snapshot -> 'decision_point'
       )
       OR (
            NEW.event_type = 'turn_completed'
            AND (
                next_actor_position IS NULL
                OR (NEW.payload -> 'control' ->> 'active_position')::NUMERIC
                    <> next_actor_position
            )
       )
    THEN
        RAISE EXCEPTION 'game event payload metadata must match its relational envelope'
            USING ERRCODE = '23514';
    END IF;

    SELECT COUNT(*)
    INTO terminal_count
    FROM jsonb_array_elements(NEW.payload -> 'steps') AS step
    CROSS JOIN LATERAL jsonb_array_elements(step -> 'effects') AS effect
    WHERE effect ->> 'type' = 'terminal';

    IF committed_status IN ('lost', 'won') THEN
        IF terminal_count <> 1
           OR NEW.payload -> 'steps' -> -1 -> 'effects' -> -1 ->> 'type'
                IS DISTINCT FROM 'terminal'
           OR NEW.payload -> 'steps' -> -1 -> 'effects' -> -1 ->> 'outcome'
                IS DISTINCT FROM committed_status
        THEN
            RAISE EXCEPTION 'terminal effect must match the committed terminal status'
                USING ERRCODE = '23514';
        END IF;
    ELSIF committed_status <> 'in_progress' OR terminal_count <> 0 THEN
        RAISE EXCEPTION 'non-terminal event must match an in-progress game'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION require_game_transition_history()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    canonical_selections JSONB;
    expected_effects JSONB;
    expected_entities JSONB;
    expected_participants JSONB;
    expected_steps JSONB;
    old_choice JSONB;
    previous_steps JSONB;
    random_samples BIGINT;
    transition_command_type TEXT;
    transition_event_type TEXT;
    transition_payload JSONB;
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

    IF NEW.snapshot_version <> 3
       OR OLD.snapshot_version NOT IN (1, 2, 3)
    THEN
        RAISE EXCEPTION 'official transitions must commit the current snapshot codec'
            USING ERRCODE = '23514';
    END IF;

    SELECT events.payload, events.event_type, receipts.command_type
    INTO transition_payload, transition_event_type, transition_command_type
    FROM game_events AS events
    JOIN game_command_receipts AS receipts
      ON receipts.game_id = events.game_id
     AND receipts.room_id = events.room_id
     AND receipts.accepted_sequence = events.sequence
     AND receipts.command_id = events.command_id
     AND receipts.actor_participant_id = events.actor_participant_id
     AND receipts.accepted_state_version = events.state_version
     AND (
            (
                events.event_type = 'turn_completed'
                AND receipts.command_type = 'end_hero_actions'
            )
            OR (
                events.event_type = 'choice_resolved'
                AND receipts.command_type = 'resolve_choice'
            )
            OR (
                events.event_type = 'card_played'
                AND receipts.command_type = 'play_card'
            )
            OR (
                events.event_type = 'attack_assigned'
                AND receipts.command_type = 'assign_attack'
            )
            OR (
                events.event_type = 'card_acquired'
                AND receipts.command_type = 'acquire_card'
            )
         )
     AND receipts.expires_at = NEW.expires_at
    WHERE events.game_id = NEW.id
      AND events.room_id = NEW.room_id
      AND events.sequence = NEW.sequence
      AND events.state_version = NEW.state_version
      AND events.event_version = 4;

    IF transition_payload IS NULL THEN
        RAISE EXCEPTION 'game transition requires a matching official event and receipt'
            USING ERRCODE = '23514';
    END IF;

    SELECT COALESCE(
        jsonb_agg(
            jsonb_build_object(
                'participant_id', participants.id::TEXT,
                'position', participants.position,
                'hero_id', participants.hero_id
            )
            ORDER BY participants.position
        ),
        '[]'::jsonb
    )
    INTO expected_participants
    FROM participants
    WHERE participants.room_id = NEW.room_id;

    IF transition_event_type IN ('card_played', 'attack_assigned', 'card_acquired') THEN
        expected_entities := apply_effect_steps_v4(
            normalized_effect_entities_for_turn_v4(OLD.snapshot),
            jsonb_build_array(jsonb_build_object(
                'effects', transition_payload -> 'effects'
            ))
        );
        SELECT COUNT(*)
        INTO random_samples
        FROM jsonb_array_elements(transition_payload -> 'effects') AS effect
        WHERE effect ->> 'type' = 'die_rolled';

        IF OLD.status <> 'in_progress'
           OR OLD.snapshot -> 'turn' ->> 'phase'
                NOT IN ('hero_action', 'hero_actions')
           OR transition_payload ->> 'turn'
                IS DISTINCT FROM OLD.snapshot -> 'turn' ->> 'number'
           OR transition_payload ->> 'actor_position'
                IS DISTINCT FROM OLD.snapshot -> 'turn' ->> 'active_position'
           OR valid_hero_action_payload_v4(
                transition_payload,
                transition_event_type,
                NEW.sequence,
                NEW.state_version,
                (transition_payload ->> 'actor_position')::SMALLINT,
                NEW.prng_counter,
                NEW.status
           ) IS NOT TRUE
           OR expected_entities IS NULL
           OR NEW.snapshot -> 'effects' -> 'entities'
                IS DISTINCT FROM expected_entities
           OR valid_game_snapshot_v3(NEW.snapshot) IS NOT TRUE
           OR (NEW.snapshot ->> 'snapshot_version')::NUMERIC <> NEW.snapshot_version
           OR (NEW.snapshot ->> 'state_version')::NUMERIC <> NEW.state_version
           OR (NEW.snapshot ->> 'sequence')::NUMERIC <> NEW.sequence
           OR NEW.snapshot ->> 'status' IS DISTINCT FROM NEW.status
           OR NEW.snapshot ->> 'adventure_id' IS DISTINCT FROM NEW.adventure_id
           OR NEW.snapshot -> 'versions' ->> 'content' IS DISTINCT FROM NEW.content_version
           OR NEW.snapshot -> 'versions' ->> 'ruleset' IS DISTINCT FROM NEW.ruleset_version
           OR (NEW.snapshot -> 'versions' ->> 'manifest')::NUMERIC <> NEW.manifest_version
           OR NEW.snapshot -> 'versions' ->> 'manifest_digest'
                IS DISTINCT FROM NEW.manifest_digest
           OR NEW.snapshot -> 'versions' ->> 'prng' IS DISTINCT FROM NEW.prng_algorithm
           OR NEW.snapshot -> 'versions' ->> 'shuffle' IS DISTINCT FROM NEW.shuffle_algorithm
           OR NEW.snapshot -> 'versions' ->> 'sampling'
                IS DISTINCT FROM NEW.sampling_algorithm
           OR NEW.snapshot -> 'participants' IS DISTINCT FROM expected_participants
           OR NEW.snapshot -> 'turn' ->> 'number'
                IS DISTINCT FROM OLD.snapshot -> 'turn' ->> 'number'
           OR NEW.snapshot -> 'turn' ->> 'active_position'
                IS DISTINCT FROM OLD.snapshot -> 'turn' ->> 'active_position'
           OR NEW.snapshot -> 'turn' ->> 'phase' <> 'hero_actions'
           OR NEW.snapshot -> 'prng' ->> 'algorithm' IS DISTINCT FROM NEW.prng_algorithm
           OR (NEW.snapshot -> 'prng' ->> 'counter')::NUMERIC <> NEW.prng_counter
           OR NEW.prng_counter <> OLD.prng_counter + random_samples
           OR (
                transition_event_type = 'card_played'
                AND transition_payload ->> 'effect_stop' = 'choice'
                AND (
                    NEW.snapshot -> 'effects' -> 'choice'
                        IS DISTINCT FROM transition_payload -> 'choice'
                    OR NEW.snapshot -> 'decision_point' ->> 'type' <> 'effect_choice'
                    OR NEW.snapshot -> 'queued_effects'
                        IS DISTINCT FROM transition_payload -> 'choice'
                            -> 'continuation' -> 'queue'
                )
           )
           OR (
                NOT (
                    transition_event_type = 'card_played'
                    AND transition_payload ->> 'effect_stop' = 'choice'
                )
                AND NEW.snapshot -> 'effects' ? 'choice'
           )
           OR (
                OLD.snapshot_version = 3
                AND NEW.snapshot -> 'last_turn_steps' IS DISTINCT FROM
                    merge_turn_steps_v4(
                        OLD.snapshot -> 'last_turn_steps',
                        jsonb_build_array(jsonb_build_object(
                            'phase', 'hero_actions',
                            'effects', transition_payload -> 'effects'
                        ))
                    )
           )
        THEN
            RAISE EXCEPTION 'hero action transition must match its event and previous state'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    CASE transition_event_type
        WHEN 'turn_completed' THEN
            expected_steps := transition_payload -> 'steps';

            SELECT COALESCE(SUM(samples), 0)
            INTO random_samples
            FROM (
                SELECT jsonb_array_length(outcome -> 'bottom_to_top') - 1 AS samples
                FROM jsonb_array_elements(transition_payload -> 'end_turn') AS outcome
                WHERE outcome ->> 'type' = 'pile_shuffled'
                UNION ALL
                SELECT 1
                FROM jsonb_array_elements(transition_payload -> 'steps') AS step
                CROSS JOIN LATERAL jsonb_array_elements(step -> 'effects') AS effect
                WHERE effect ->> 'type' = 'die_rolled'
            ) AS consumed;

            IF transition_command_type <> 'end_hero_actions'
               OR OLD.status <> 'in_progress'
               OR OLD.snapshot -> 'turn' ->> 'phase'
                    NOT IN ('hero_action', 'hero_actions')
               OR transition_payload ->> 'turn'
                    IS DISTINCT FROM OLD.snapshot -> 'turn' ->> 'number'
               OR transition_payload ->> 'actor_position'
                    IS DISTINCT FROM OLD.snapshot -> 'turn' ->> 'active_position'
               OR (
                    OLD.snapshot ? 'decision_point'
                    AND (
                        OLD.snapshot -> 'decision_point' ->> 'type'
                            IS DISTINCT FROM 'player_intent'
                        OR OLD.snapshot -> 'decision_point' ->> 'responsible_position'
                            IS DISTINCT FROM transition_payload ->> 'actor_position'
                    )
               )
               OR (
                    OLD.snapshot -> 'effects' ? 'choice'
                    AND OLD.snapshot -> 'effects' -> 'choice'
                        IS DISTINCT FROM 'null'::jsonb
               )
               OR valid_turn_world_transition_v4(
                    OLD.snapshot,
                    NEW.snapshot -> 'effects' -> 'entities',
                    transition_payload
               ) IS NOT TRUE
            THEN
                RAISE EXCEPTION 'turn completion must match the previous player decision point'
                    USING ERRCODE = '23514';
            END IF;

        WHEN 'choice_resolved' THEN
            IF OLD.snapshot_version = 3
               AND OLD.snapshot -> 'decision_point' ->> 'type' = 'effect_choice'
            THEN
                old_choice := OLD.snapshot -> 'decision_point' -> 'choice';
                previous_steps := OLD.snapshot -> 'last_turn_steps';
            ELSIF OLD.snapshot_version = 2
                  AND jsonb_typeof(OLD.snapshot -> 'effects' -> 'choice') = 'object'
            THEN
                old_choice := OLD.snapshot -> 'effects' -> 'choice';
                previous_steps := jsonb_build_array(jsonb_build_object(
                    'phase', OLD.snapshot -> 'turn' -> 'phase',
                    'effects', COALESCE(
                        OLD.snapshot -> 'effects' -> 'outcomes',
                        '[]'::jsonb
                    )
                ));
            ELSE
                RAISE EXCEPTION 'choice resolution requires a resumable pending choice'
                    USING ERRCODE = '23514';
            END IF;

            expected_steps := merge_turn_steps_v4(
                previous_steps,
                transition_payload -> 'steps'
            );
            SELECT COALESCE(
                jsonb_agg(option.value ORDER BY option.position),
                '[]'::jsonb
            )
            INTO canonical_selections
            FROM jsonb_array_elements(old_choice -> 'options')
                WITH ORDINALITY AS option(value, position)
            WHERE transition_payload -> 'selected_options'
                ? (option.value #>> '{}');
            SELECT COUNT(*)
            INTO random_samples
            FROM jsonb_array_elements(transition_payload -> 'steps') AS step
            CROSS JOIN LATERAL jsonb_array_elements(step -> 'effects') AS effect
            WHERE effect ->> 'type' = 'die_rolled';

            IF transition_command_type <> 'resolve_choice'
               OR OLD.status <> 'in_progress'
               OR OLD.snapshot -> 'turn' ->> 'phase'
                    NOT IN ('dark_arts', 'villains', 'hero_actions')
               OR valid_pending_effect_choice_v4(old_choice) IS NOT TRUE
               OR transition_payload ->> 'turn'
                    IS DISTINCT FROM OLD.snapshot -> 'turn' ->> 'number'
               OR transition_payload ->> 'actor_position'
                    IS DISTINCT FROM old_choice ->> 'responsible_position'
               OR transition_payload ->> 'choice_id'
                    IS DISTINCT FROM old_choice ->> 'id'
               OR transition_payload ->> 'choice_cause'
                    IS DISTINCT FROM old_choice ->> 'cause'
               OR transition_payload -> 'steps' -> 0 ->> 'phase'
                    IS DISTINCT FROM OLD.snapshot -> 'turn' ->> 'phase'
               OR jsonb_array_length(transition_payload -> 'selected_options')
                    NOT BETWEEN (old_choice ->> 'min')::INTEGER
                        AND (old_choice ->> 'max')::INTEGER
               OR transition_payload -> 'selected_options'
                    IS DISTINCT FROM canonical_selections
               OR expected_steps IS NULL
               OR NEW.snapshot -> 'turn' -> 'number'
                    IS DISTINCT FROM OLD.snapshot -> 'turn' -> 'number'
               OR NEW.snapshot -> 'turn' -> 'active_position'
                    IS DISTINCT FROM OLD.snapshot -> 'turn' -> 'active_position'
               OR valid_choice_world_transition_v4(
                    OLD.snapshot,
                    NEW.snapshot -> 'effects' -> 'entities',
                    transition_payload
               ) IS NOT TRUE
            THEN
                RAISE EXCEPTION 'choice resolution must match the pending choice and continuation'
                    USING ERRCODE = '23514';
            END IF;

        ELSE
            RAISE EXCEPTION 'game transition event type is not supported'
                USING ERRCODE = '23514';
    END CASE;

    SELECT COALESCE(
        jsonb_agg(effect.value ORDER BY step.position, effect.position),
        '[]'::jsonb
    )
    INTO expected_effects
    FROM jsonb_array_elements(expected_steps)
        WITH ORDINALITY AS step(value, position)
    CROSS JOIN LATERAL jsonb_array_elements(step.value -> 'effects')
        WITH ORDINALITY AS effect(value, position);

    IF valid_game_snapshot_v3(NEW.snapshot) IS NOT TRUE
       OR (NEW.snapshot ->> 'snapshot_version')::NUMERIC <> NEW.snapshot_version
       OR (NEW.snapshot ->> 'state_version')::NUMERIC <> NEW.state_version
       OR (NEW.snapshot ->> 'sequence')::NUMERIC <> NEW.sequence
       OR NEW.snapshot ->> 'status' IS DISTINCT FROM NEW.status
       OR NEW.snapshot ->> 'adventure_id' IS DISTINCT FROM NEW.adventure_id
       OR NEW.snapshot -> 'versions' ->> 'content' IS DISTINCT FROM NEW.content_version
       OR NEW.snapshot -> 'versions' ->> 'ruleset' IS DISTINCT FROM NEW.ruleset_version
       OR (NEW.snapshot -> 'versions' ->> 'manifest')::NUMERIC <> NEW.manifest_version
       OR NEW.snapshot -> 'versions' ->> 'manifest_digest' IS DISTINCT FROM NEW.manifest_digest
       OR NEW.snapshot -> 'versions' ->> 'prng' IS DISTINCT FROM NEW.prng_algorithm
       OR NEW.snapshot -> 'versions' ->> 'shuffle' IS DISTINCT FROM NEW.shuffle_algorithm
       OR NEW.snapshot -> 'versions' ->> 'sampling' IS DISTINCT FROM NEW.sampling_algorithm
       OR NEW.snapshot -> 'turn' -> 'number' IS DISTINCT FROM transition_payload -> 'control' -> 'turn'
       OR NEW.snapshot -> 'turn' -> 'phase' IS DISTINCT FROM transition_payload -> 'control' -> 'phase'
       OR NEW.snapshot -> 'turn' -> 'active_position'
            IS DISTINCT FROM transition_payload -> 'control' -> 'active_position'
       OR NEW.snapshot -> 'queued_phases'
            IS DISTINCT FROM transition_payload -> 'control' -> 'queued_phases'
       OR NEW.snapshot -> 'queued_effects'
            IS DISTINCT FROM transition_payload -> 'control' -> 'queued_effects'
       OR NEW.snapshot -> 'decision_point'
            IS DISTINCT FROM transition_payload -> 'control' -> 'decision_point'
       OR NEW.snapshot -> 'last_turn_steps' IS DISTINCT FROM expected_steps
       OR NEW.snapshot -> 'participants' IS DISTINCT FROM expected_participants
       OR NEW.snapshot -> 'prng' ->> 'algorithm' IS DISTINCT FROM NEW.prng_algorithm
       OR (NEW.snapshot -> 'prng' ->> 'counter')::NUMERIC <> NEW.prng_counter
       OR COALESCE(NEW.snapshot -> 'effects' -> 'outcomes', '[]'::jsonb)
            IS DISTINCT FROM expected_effects
       OR NEW.prng_counter <> OLD.prng_counter + random_samples
    THEN
        RAISE EXCEPTION 'game transition snapshot must match its event and previous decision point'
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
    IF NEW.event_version <> 4
       OR NEW.event_type NOT IN (
            'turn_completed', 'choice_resolved', 'card_played',
            'attack_assigned', 'card_acquired'
       )
       OR NOT EXISTS (
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
       )
    THEN
        RAISE EXCEPTION 'official game event requires a matching command receipt'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

UPDATE application_metadata
SET value = '17'
WHERE key = 'schema_version';
