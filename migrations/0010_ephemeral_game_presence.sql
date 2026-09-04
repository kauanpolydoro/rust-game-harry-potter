CREATE UNLOGGED TABLE game_realtime_connections (
    id UUID PRIMARY KEY,
    game_id UUID NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    participant_id UUID NOT NULL REFERENCES participants(id) ON DELETE CASCADE,
    guest_session_id UUID NOT NULL REFERENCES guest_sessions(id) ON DELETE CASCADE,
    connected_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    disconnected_at TIMESTAMPTZ,
    CONSTRAINT game_realtime_connections_heartbeat_order
        CHECK (last_heartbeat_at >= connected_at),
    CONSTRAINT game_realtime_connections_disconnect_order
        CHECK (disconnected_at IS NULL OR disconnected_at >= connected_at)
);

CREATE INDEX game_realtime_connections_game_participant_activity
    ON game_realtime_connections (game_id, participant_id, last_heartbeat_at DESC);

CREATE INDEX game_realtime_connections_session
    ON game_realtime_connections (guest_session_id);

UPDATE application_metadata
SET value = '10'
WHERE key = 'schema_version';
