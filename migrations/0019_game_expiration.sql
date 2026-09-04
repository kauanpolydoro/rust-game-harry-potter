ALTER TABLE games ADD COLUMN access_expired_at TIMESTAMPTZ;

CREATE INDEX games_pending_expiration ON games (expires_at)
    WHERE access_expired_at IS NULL;

-- The optional observation is an explicit database-clock seam for boundary tests.
-- Application callers omit it so time is sampled only after acquiring the root.
CREATE FUNCTION expire_game_access(game_id UUID, observed_at TIMESTAMPTZ DEFAULT NULL)
RETURNS BOOLEAN
LANGUAGE plpgsql
AS $$
DECLARE
    deadline TIMESTAMPTZ;
    expired_at TIMESTAMPTZ;
    game_room_id UUID;
    observed TIMESTAMPTZ;
    session_id UUID;
BEGIN
    SELECT games.expires_at, games.access_expired_at, games.room_id
    INTO deadline, expired_at, game_room_id
    FROM games WHERE id = game_id
    FOR UPDATE;

    IF NOT FOUND THEN
        RETURN FALSE;
    END IF;
    IF expired_at IS NOT NULL THEN
        RETURN TRUE;
    END IF;
    observed := COALESCE(observed_at, clock_timestamp());
    IF observed < deadline THEN
        RETURN FALSE;
    END IF;

    UPDATE games SET access_expired_at = observed WHERE id = game_id;
    FOR session_id IN
    UPDATE device_sessions SET status = 'expired'
    FROM participants
    WHERE device_sessions.participant_id = participants.id
      AND participants.room_id = game_room_id
      AND device_sessions.status = 'active'
    RETURNING device_sessions.guest_session_id
    LOOP
        PERFORM pg_notify('hogwarts_session_revoked', session_id::TEXT);
    END LOOP;
    UPDATE game_realtime_connections
    SET disconnected_at = clock_timestamp()
    WHERE game_realtime_connections.game_id = expire_game_access.game_id
      AND disconnected_at IS NULL;
    RETURN TRUE;
END;
$$;

UPDATE application_metadata SET value = '19' WHERE key = 'schema_version';
