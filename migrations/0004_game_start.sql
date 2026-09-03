ALTER TABLE participants
    ADD COLUMN ready BOOLEAN NOT NULL DEFAULT FALSE,
    ADD CONSTRAINT participants_ready_requires_hero CHECK (NOT ready OR hero_id IS NOT NULL);

CREATE TABLE content_manifests (
    digest TEXT PRIMARY KEY,
    manifest_version SMALLINT NOT NULL,
    content_version TEXT NOT NULL,
    ruleset_version TEXT NOT NULL,
    playable BOOLEAN NOT NULL,
    document JSONB NOT NULL,
    published_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT content_manifests_digest_format CHECK (digest ~ '^blake3:[0-9a-f]{64}$'),
    CONSTRAINT content_manifests_version_positive CHECK (manifest_version > 0),
    CONSTRAINT content_manifests_content_version_present CHECK (content_version <> ''),
    CONSTRAINT content_manifests_ruleset_version_present CHECK (ruleset_version <> '')
);

CREATE FUNCTION reject_content_manifest_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'published content manifests are immutable'
        USING ERRCODE = '23514';
END;
$$;

CREATE TRIGGER content_manifests_are_immutable
BEFORE UPDATE OR DELETE ON content_manifests
FOR EACH ROW
EXECUTE FUNCTION reject_content_manifest_mutation();

CREATE TABLE games (
    id UUID PRIMARY KEY,
    room_id UUID NOT NULL UNIQUE REFERENCES rooms(id),
    started_by_participant_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'in_progress',
    adventure_id TEXT NOT NULL,
    adventure_name TEXT NOT NULL,
    manifest_digest TEXT NOT NULL REFERENCES content_manifests(digest),
    manifest_version SMALLINT NOT NULL,
    content_version TEXT NOT NULL,
    ruleset_version TEXT NOT NULL,
    snapshot_version SMALLINT NOT NULL,
    state_version BIGINT NOT NULL,
    sequence BIGINT NOT NULL,
    state_digest TEXT NOT NULL,
    snapshot JSONB NOT NULL,
    prng_algorithm TEXT NOT NULL,
    prng_seed BYTEA NOT NULL,
    prng_counter BIGINT NOT NULL DEFAULT 0,
    shuffle_algorithm TEXT NOT NULL,
    sampling_algorithm TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT games_started_by_room_participant
        FOREIGN KEY (room_id, started_by_participant_id)
        REFERENCES participants (room_id, id),
    CONSTRAINT games_status_valid CHECK (status IN ('in_progress', 'won', 'lost', 'quarantined')),
    CONSTRAINT games_adventure_present CHECK (adventure_id <> '' AND adventure_name <> ''),
    CONSTRAINT games_versions_positive CHECK (
        manifest_version > 0
        AND snapshot_version > 0
        AND state_version > 0
        AND sequence >= 0
    ),
    CONSTRAINT games_state_digest_format CHECK (state_digest ~ '^blake3:[0-9a-f]{64}$'),
    CONSTRAINT games_prng_algorithm_fixed CHECK (prng_algorithm = 'chacha20-v1'),
    CONSTRAINT games_prng_seed_256_bits CHECK (octet_length(prng_seed) = 32),
    CONSTRAINT games_prng_counter_nonnegative CHECK (prng_counter >= 0)
);

CREATE TABLE game_start_requests (
    idempotency_key TEXT PRIMARY KEY,
    game_id UUID NOT NULL UNIQUE REFERENCES games(id) DEFERRABLE INITIALLY DEFERRED,
    room_id UUID NOT NULL REFERENCES rooms(id) DEFERRABLE INITIALLY DEFERRED,
    participant_id UUID NOT NULL REFERENCES participants(id) DEFERRABLE INITIALLY DEFERRED,
    adventure_id TEXT NOT NULL,
    manifest_digest TEXT NOT NULL,
    ruleset_version TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT game_start_requests_key_length CHECK (
        char_length(idempotency_key) BETWEEN 8 AND 128
    )
);

CREATE FUNCTION reject_sealed_room_participant_change()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    target_room_id UUID;
    target_room_status TEXT;
BEGIN
    target_room_id := CASE WHEN TG_OP = 'INSERT' THEN NEW.room_id ELSE OLD.room_id END;
    SELECT status
    INTO target_room_status
    FROM rooms
    WHERE id = target_room_id
    FOR UPDATE;
    IF target_room_status <> 'open' THEN
        RAISE EXCEPTION 'sealed room participants cannot change'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER participants_require_open_room_on_insert
BEFORE INSERT ON participants
FOR EACH ROW
EXECUTE FUNCTION reject_sealed_room_participant_change();

CREATE TRIGGER sealed_room_participants_are_fixed
BEFORE UPDATE OF room_id, position, hero_id, ready ON participants
FOR EACH ROW
EXECUTE FUNCTION reject_sealed_room_participant_change();
