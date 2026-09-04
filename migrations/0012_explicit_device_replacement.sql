ALTER TABLE device_sessions
    ADD COLUMN slot SMALLINT;

WITH ranked_sessions AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY participant_id
            ORDER BY created_at, id
        ) AS slot
    FROM device_sessions
)
UPDATE device_sessions
SET slot = ranked_sessions.slot
FROM ranked_sessions
WHERE ranked_sessions.id = device_sessions.id;

ALTER TABLE device_sessions
    ALTER COLUMN slot SET NOT NULL,
    ALTER COLUMN slot SET DEFAULT 1,
    ADD CONSTRAINT device_sessions_slot_valid CHECK (slot BETWEEN 1 AND 2);

UPDATE device_sessions
SET status = 'expired'
FROM guest_sessions
WHERE guest_sessions.id = device_sessions.guest_session_id
  AND device_sessions.status = 'active'
  AND guest_sessions.expires_at <= clock_timestamp();

CREATE UNIQUE INDEX device_sessions_one_active_per_slot
    ON device_sessions (participant_id, slot)
    WHERE status = 'active';

ALTER TABLE recovery_credentials
    ADD COLUMN replaced_device_session_id UUID REFERENCES device_sessions(id);

UPDATE application_metadata
SET value = '12'
WHERE key = 'schema_version';
