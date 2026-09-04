CREATE TABLE game_state_anchors (
    game_id UUID NOT NULL REFERENCES games(id),
    sequence BIGINT NOT NULL,
    snapshot_version SMALLINT NOT NULL,
    state_digest TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (game_id, sequence),
    CONSTRAINT game_state_anchors_sequence_nonnegative CHECK (sequence >= 0),
    CONSTRAINT game_state_anchors_snapshot_version_positive CHECK (snapshot_version > 0),
    CONSTRAINT game_state_anchors_digest_format
        CHECK (state_digest ~ '^blake3:[0-9a-f]{64}$')
);

INSERT INTO game_state_anchors (game_id, sequence, snapshot_version, state_digest)
SELECT id, sequence, snapshot_version, state_digest
FROM games;

CREATE FUNCTION require_current_game_state_anchor()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM game_state_anchors AS anchors
        WHERE anchors.game_id = NEW.id
          AND anchors.sequence = NEW.sequence
          AND anchors.snapshot_version = NEW.snapshot_version
          AND anchors.state_digest = NEW.state_digest
    ) THEN
        RAISE EXCEPTION 'authoritative game state requires a matching replay anchor'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE CONSTRAINT TRIGGER game_states_have_replay_anchors
AFTER INSERT OR UPDATE ON games
DEFERRABLE INITIALLY DEFERRED
FOR EACH ROW
EXECUTE FUNCTION require_current_game_state_anchor();

CREATE TRIGGER game_state_anchors_are_append_only
BEFORE UPDATE OR DELETE ON game_state_anchors
FOR EACH ROW
EXECUTE FUNCTION reject_official_history_mutation();

UPDATE application_metadata
SET value = '9'
WHERE key = 'schema_version';
