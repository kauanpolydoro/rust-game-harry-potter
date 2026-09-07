-- Jobs intentionally outlive the aggregate; they contain identifiers only until
-- independent verification and external completion have succeeded.
CREATE TABLE lifecycle_purge_jobs (
    game_id UUID PRIMARY KEY,
    room_id UUID NOT NULL UNIQUE,
    stage TEXT NOT NULL DEFAULT 'tombstone'
        CHECK (stage IN ('tombstone', 'detach', 'purge', 'verify', 'complete')),
    expires_at TIMESTAMPTZ NOT NULL,
    detected_at TIMESTAMPTZ NOT NULL,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    failures BIGINT NOT NULL DEFAULT 0,
    last_failure TEXT CHECK (last_failure IN ('database', 'ledger', 'orphans', 'timeout')),
    participant_ids UUID[] NOT NULL DEFAULT '{}',
    identity_ids UUID[] NOT NULL DEFAULT '{}',
    session_ids UUID[] NOT NULL DEFAULT '{}',
    verified_at TIMESTAMPTZ
);
CREATE INDEX lifecycle_purge_jobs_due ON lifecycle_purge_jobs (next_attempt_at);

-- Anonymous observations have no game, room, participant, session or ledger key.
CREATE TABLE lifecycle_purge_observations (
    observed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    failed_attempts BIGINT NOT NULL,
    detection_seconds DOUBLE PRECISION NOT NULL CHECK (detection_seconds >= 0),
    purge_seconds DOUBLE PRECISION NOT NULL CHECK (purge_seconds >= 0)
);
CREATE INDEX lifecycle_purge_observations_age ON lifecycle_purge_observations (observed_at);

-- UPDATE remains forbidden. DELETE requires a durable, expired purge job which
-- the worker advances only after the external ledger acknowledges the tombstone.
CREATE OR REPLACE FUNCTION reject_official_history_mutation()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' AND EXISTS (
        SELECT 1 FROM lifecycle_purge_jobs AS jobs
        JOIN games ON games.id = jobs.game_id
        WHERE jobs.game_id = OLD.game_id AND jobs.stage = 'purge'
          AND games.access_expired_at IS NOT NULL
          AND games.expires_at <= clock_timestamp()
    ) THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'official game history is append-only' USING ERRCODE = '55000';
END;
$$;

CREATE OR REPLACE FUNCTION reject_sealed_room_participant_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    target_room_ids UUID[];
BEGIN
    IF TG_OP = 'DELETE' AND EXISTS (
        SELECT 1 FROM lifecycle_purge_jobs WHERE room_id = OLD.room_id
          AND stage = 'purge' AND expires_at <= clock_timestamp()
    ) THEN
        RETURN OLD;
    END IF;
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

UPDATE application_metadata SET value = '22' WHERE key = 'schema_version';
