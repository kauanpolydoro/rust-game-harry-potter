use game_content::ContentManifest;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ApiError, SelectedContent, StartGameRequest, StoredGame, StoredGameStart, StoredRoomActor,
    StoredRoomParticipant,
};

pub(super) struct NewGame<'a> {
    pub(super) id: Uuid,
    pub(super) actor: &'a StoredRoomActor,
    pub(super) content: &'a SelectedContent,
    pub(super) state: &'a game_domain::InitialGameState,
    pub(super) state_digest: &'a str,
    pub(super) snapshot_json: &'a str,
    pub(super) seed: &'a [u8; 32],
}

pub(super) async fn publish_manifest(
    database: &PgPool,
    manifest: &ContentManifest,
    document: &str,
) -> Result<(), sqlx::Error> {
    let manifest_version = i16::try_from(manifest.manifest_version).map_err(|_| {
        sqlx::Error::Protocol("manifest version does not fit PostgreSQL SMALLINT".to_owned())
    })?;
    let inserted = sqlx::query_scalar::<_, String>(
        r"
        INSERT INTO content_manifests (
            digest,
            manifest_version,
            content_version,
            ruleset_version,
            playable,
            document
        )
        VALUES ($1, $2, $3, $4, $5, $6::jsonb)
        ON CONFLICT (digest) DO NOTHING
        RETURNING digest
        ",
    )
    .bind(&manifest.digest)
    .bind(manifest_version)
    .bind(&manifest.content_version)
    .bind(&manifest.ruleset_version)
    .bind(manifest.playable)
    .bind(document)
    .fetch_optional(database)
    .await?;

    if inserted.is_none() {
        let stored = sqlx::query_as::<_, (i16, String, String, bool, String)>(
            r"
            SELECT
                manifest_version,
                content_version,
                ruleset_version,
                playable,
                document::text
            FROM content_manifests
            WHERE digest = $1
            ",
        )
        .bind(&manifest.digest)
        .fetch_one(database)
        .await?;
        let requested_document: serde_json::Value = serde_json::from_str(document)
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        let stored_document: serde_json::Value = serde_json::from_str(&stored.4)
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        if stored.0 != manifest_version
            || stored.1 != manifest.content_version
            || stored.2 != manifest.ruleset_version
            || stored.3 != manifest.playable
            || stored_document != requested_document
        {
            return Err(sqlx::Error::Protocol(
                "content manifest digest collision or immutable document mismatch".to_owned(),
            ));
        }
    }

    Ok(())
}

pub(super) async fn lock_room_actor(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
) -> Result<Option<StoredRoomActor>, ApiError> {
    sqlx::query_as::<_, StoredRoomActor>(
        r"
        SELECT
            rooms.id AS room_id,
            rooms.status AS room_status,
            participants.id AS participant_id,
            participants.role
        FROM rooms
        JOIN participants ON participants.room_id = rooms.id
        WHERE participants.id = $1
        FOR UPDATE OF rooms
        ",
    )
    .bind(participant_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| ApiError::internal())
}

pub(super) async fn room_participants(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
) -> Result<Vec<StoredRoomParticipant>, ApiError> {
    sqlx::query_as::<_, StoredRoomParticipant>(
        r"
        SELECT id, display_name, role, position, hero_id, ready
        FROM participants
        WHERE room_id = $1
        ORDER BY position
        ",
    )
    .bind(room_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| ApiError::internal())
}

pub(super) async fn claim_game_start(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    game_id: Uuid,
    actor: &StoredRoomActor,
    request: &StartGameRequest,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, String>(
        r"
        INSERT INTO game_start_requests (
            idempotency_key,
            game_id,
            room_id,
            participant_id,
            adventure_id,
            manifest_digest,
            ruleset_version
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT (idempotency_key) DO NOTHING
        RETURNING idempotency_key
        ",
    )
    .bind(idempotency_key)
    .bind(game_id)
    .bind(actor.room_id)
    .bind(actor.participant_id)
    .bind(&request.adventure_id)
    .bind(&request.manifest_digest)
    .bind(&request.ruleset_version)
    .fetch_optional(&mut **transaction)
    .await
    .map(|claim| claim.is_some())
    .map_err(|_| ApiError::internal())
}

pub(super) async fn persist_game(
    transaction: &mut Transaction<'_, Postgres>,
    game: NewGame<'_>,
) -> Result<(), ApiError> {
    sqlx::query(
        r"
        INSERT INTO games (
            id,
            room_id,
            started_by_participant_id,
            status,
            adventure_id,
            adventure_name,
            manifest_digest,
            manifest_version,
            content_version,
            ruleset_version,
            snapshot_version,
            state_version,
            sequence,
            state_digest,
            snapshot,
            prng_algorithm,
            prng_seed,
            prng_counter,
            shuffle_algorithm,
            sampling_algorithm
        )
        VALUES (
            $1, $2, $3, 'in_progress', $4, $5, $6, $7, $8, $9,
            $10, $11, $12, $13, $14::jsonb, $15, $16, 0, $17, $18
        )
        ",
    )
    .bind(game.id)
    .bind(game.actor.room_id)
    .bind(game.actor.participant_id)
    .bind(&game.content.adventure_id)
    .bind(&game.content.adventure_name)
    .bind(&game.content.manifest_digest)
    .bind(i16::try_from(game.state.manifest_version).map_err(|_| ApiError::internal())?)
    .bind(&game.state.content_version)
    .bind(&game.state.ruleset_version)
    .bind(i16::try_from(game.state.snapshot_version).map_err(|_| ApiError::internal())?)
    .bind(i64::try_from(game.state.state_version).map_err(|_| ApiError::internal())?)
    .bind(i64::try_from(game.state.sequence).map_err(|_| ApiError::internal())?)
    .bind(game.state_digest)
    .bind(game.snapshot_json)
    .bind(game.state.prng_algorithm)
    .bind(game.seed.as_slice())
    .bind(game.state.shuffle_algorithm)
    .bind(game.state.sampling_algorithm)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|_| ApiError::internal())
}

pub(super) async fn seal_room(
    transaction: &mut Transaction<'_, Postgres>,
    room_id: Uuid,
) -> Result<(), ApiError> {
    let result =
        sqlx::query("UPDATE rooms SET status = 'sealed' WHERE id = $1 AND status = 'open'")
            .bind(room_id)
            .execute(&mut **transaction)
            .await
            .map_err(|_| ApiError::internal())?;
    if result.rows_affected() != 1 {
        return Err(ApiError::room_sealed());
    }
    Ok(())
}

pub(super) async fn load_game_start(
    database: &PgPool,
    idempotency_key: &str,
) -> Result<Option<StoredGameStart>, ApiError> {
    sqlx::query_as::<_, StoredGameStart>(
        r"
        SELECT game_id, participant_id, adventure_id, manifest_digest, ruleset_version
        FROM game_start_requests
        WHERE idempotency_key = $1
        ",
    )
    .bind(idempotency_key)
    .fetch_optional(database)
    .await
    .map_err(|_| ApiError::internal())
}

pub(super) async fn load_game_start_in(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
) -> Result<Option<StoredGameStart>, ApiError> {
    sqlx::query_as::<_, StoredGameStart>(
        r"
        SELECT game_id, participant_id, adventure_id, manifest_digest, ruleset_version
        FROM game_start_requests
        WHERE idempotency_key = $1
        ",
    )
    .bind(idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| ApiError::internal())
}

pub(super) async fn game_for_participant(
    database: &PgPool,
    participant_id: Uuid,
) -> Result<Option<StoredGame>, ApiError> {
    sqlx::query_as::<_, StoredGame>(
        r"
        SELECT
            games.id,
            games.status,
            games.adventure_id,
            games.adventure_name,
            games.manifest_digest,
            games.manifest_version,
            games.content_version,
            games.ruleset_version,
            games.snapshot_version,
            games.state_version,
            games.sequence,
            games.state_digest,
            games.snapshot::text AS snapshot_json,
            games.prng_algorithm,
            games.shuffle_algorithm,
            games.sampling_algorithm
        FROM games
        JOIN participants ON participants.room_id = games.room_id
        WHERE participants.id = $1
        ",
    )
    .bind(participant_id)
    .fetch_optional(database)
    .await
    .map_err(|_| ApiError::internal())
}

pub(super) async fn game_participants(
    database: &PgPool,
    game_id: Uuid,
) -> Result<Vec<StoredRoomParticipant>, ApiError> {
    sqlx::query_as::<_, StoredRoomParticipant>(
        r"
        SELECT
            participants.id,
            participants.display_name,
            participants.role,
            participants.position,
            participants.hero_id,
            participants.ready
        FROM participants
        JOIN games ON games.room_id = participants.room_id
        WHERE games.id = $1
        ORDER BY participants.position
        ",
    )
    .bind(game_id)
    .fetch_all(database)
    .await
    .map_err(|_| ApiError::internal())
}
