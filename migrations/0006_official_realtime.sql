CREATE FUNCTION require_contiguous_game_event_sequence()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    committed_sequence BIGINT;
    expected_sequence BIGINT;
BEGIN
    SELECT sequence
    INTO committed_sequence
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

    RETURN NEW;
END;
$$;

CREATE TRIGGER game_events_require_contiguous_sequence
BEFORE INSERT ON game_events
FOR EACH ROW
EXECUTE FUNCTION require_contiguous_game_event_sequence();
