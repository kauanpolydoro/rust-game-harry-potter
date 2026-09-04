use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::{
    ApiError, StoredCommandGame, StoredCommandReceipt, StoredGame, StoredGameEvent,
    StoredGameStart, StoredRoomActor, StoredRoomParticipant,
};
use crate::content_catalog::SelectedContent;

pub(super) struct GameStartClaim<'a> {
    pub(super) adventure_id: &'a str,
    pub(super) manifest_digest: &'a str,
    pub(super) ruleset_version: &'a str,
}

pub(super) struct NewGame<'a> {
    pub(super) id: Uuid,
    pub(super) actor: &'a StoredRoomActor,
    pub(super) content: &'a SelectedContent,
    pub(super) state: &'a game_domain::InitialGameState,
    pub(super) state_digest: &'a str,
    pub(super) snapshot_json: &'a str,
    pub(super) seed: &'a [u8; 32],
}

pub(super) struct NewGameCommand<'a> {
    pub(super) game_id: Uuid,
    pub(super) actor_participant_id: Uuid,
    pub(super) command_id: Uuid,
    pub(super) expected_state_version: u64,
    pub(super) command_type: &'a str,
    pub(super) payload_digest: &'a str,
    pub(super) state: &'a game_domain::InitialGameState,
    pub(super) state_digest: &'a str,
    pub(super) snapshot_json: &'a str,
    pub(super) event_type: &'a str,
    pub(super) event_json: &'a str,
}

struct PersistedCommandCounters {
    state_version: i64,
    sequence: i64,
    prng_counter: i64,
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
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
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
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
}

pub(super) async fn claim_game_start(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency_key: &str,
    game_id: Uuid,
    actor: &StoredRoomActor,
    request: GameStartClaim<'_>,
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
    .bind(request.adventure_id)
    .bind(request.manifest_digest)
    .bind(request.ruleset_version)
    .fetch_optional(&mut **transaction)
    .await
    .map(|claim| claim.is_some())
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
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
            $10, $11, $12, $13, $14::jsonb, $15, $16, $17, $18, $19
        )
        ",
    )
    .bind(game.id)
    .bind(game.actor.room_id)
    .bind(game.actor.participant_id)
    .bind(&game.content.adventure_id)
    .bind(&game.content.adventure_name)
    .bind(&game.content.manifest_digest)
    .bind(
        i16::try_from(game.state.manifest_version())
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
    )
    .bind(game.state.content_version())
    .bind(game.state.ruleset_version())
    .bind(
        i16::try_from(game.state.snapshot_version())
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
    )
    .bind(
        i64::try_from(game.state.state_version())
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
    )
    .bind(
        i64::try_from(game.state.sequence())
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
    )
    .bind(game.state_digest)
    .bind(game.snapshot_json)
    .bind(game.state.prng_algorithm())
    .bind(game.seed.as_slice())
    .bind(
        i64::try_from(game.state.prng_counter())
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
    )
    .bind(game.state.shuffle_algorithm())
    .bind(game.state.sampling_algorithm())
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("match persistence operation", error))?;

    insert_game_state_anchor(
        transaction,
        game.id,
        game.state.sequence(),
        game.state.snapshot_version(),
        game.state_digest,
    )
    .await
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
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?;
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
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
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
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
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
            games.prng_counter,
            games.shuffle_algorithm,
            games.sampling_algorithm,
            replace(
                to_char(games.expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
                ' ',
                'T'
            ) || 'Z' AS expires_at,
            clock_timestamp() >= games.expires_at AS expired
        FROM games
        JOIN participants ON participants.room_id = games.room_id
        WHERE participants.id = $1
        ",
    )
    .bind(participant_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
}

pub(super) async fn lock_game_for_actor(
    transaction: &mut Transaction<'_, Postgres>,
    participant_id: Uuid,
) -> Result<Option<StoredCommandGame>, ApiError> {
    sqlx::query_as::<_, StoredCommandGame>(
        r"
        SELECT
            games.id,
            games.status,
            games.adventure_id,
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
            games.prng_counter,
            games.shuffle_algorithm,
            games.sampling_algorithm,
            participants.position AS actor_position,
            clock_timestamp() >= games.expires_at AS expired
        FROM games
        JOIN participants ON participants.room_id = games.room_id
        WHERE participants.id = $1
        FOR UPDATE OF games
        ",
    )
    .bind(participant_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
}

pub(super) async fn persist_game_command(
    transaction: &mut Transaction<'_, Postgres>,
    command: NewGameCommand<'_>,
) -> Result<StoredCommandReceipt, ApiError> {
    let counters = PersistedCommandCounters {
        state_version: i64::try_from(command.state.state_version())
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
        sequence: i64::try_from(command.state.sequence())
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
        prng_counter: i64::try_from(command.state.prng_counter())
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
    };
    let (room_id, expires_at) = update_game_after_command(transaction, &command, &counters).await?;
    insert_game_state_anchor(
        transaction,
        command.game_id,
        command.state.sequence(),
        command.state.snapshot_version(),
        command.state_digest,
    )
    .await?;
    insert_game_event(transaction, &command, room_id, &counters).await?;
    insert_command_receipt(transaction, &command, room_id, &expires_at, &counters).await
}

async fn insert_game_state_anchor(
    transaction: &mut Transaction<'_, Postgres>,
    game_id: Uuid,
    sequence: u64,
    snapshot_version: u16,
    state_digest: &str,
) -> Result<(), ApiError> {
    sqlx::query(
        r"
        INSERT INTO game_state_anchors (game_id, sequence, snapshot_version, state_digest)
        VALUES ($1, $2, $3, $4)
        ",
    )
    .bind(game_id)
    .bind(
        i64::try_from(sequence)
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
    )
    .bind(
        i16::try_from(snapshot_version)
            .map_err(|error| ApiError::internal_with("match persistence operation", error))?,
    )
    .bind(state_digest)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(|error| ApiError::internal_with("persist game state replay anchor", error))
}

async fn update_game_after_command(
    transaction: &mut Transaction<'_, Postgres>,
    command: &NewGameCommand<'_>,
    counters: &PersistedCommandCounters,
) -> Result<(Uuid, String), ApiError> {
    let (room_id, expires_at) = sqlx::query_as::<_, (Uuid, String)>(
        r"
        UPDATE games
        SET
            state_version = $2,
            sequence = $3,
            state_digest = $4,
            snapshot = $5::jsonb,
            prng_counter = $6,
            last_game_action_at = clock_timestamp(),
            expires_at = clock_timestamp() + INTERVAL '7 days'
        WHERE id = $1
        RETURNING
            room_id,
            replace(
                to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
                ' ',
                'T'
            ) || 'Z'
        ",
    )
    .bind(command.game_id)
    .bind(counters.state_version)
    .bind(counters.sequence)
    .bind(command.state_digest)
    .bind(command.snapshot_json)
    .bind(counters.prng_counter)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("persist game state after command", error))?;
    Ok((room_id, expires_at))
}

async fn insert_game_event(
    transaction: &mut Transaction<'_, Postgres>,
    command: &NewGameCommand<'_>,
    room_id: Uuid,
    counters: &PersistedCommandCounters,
) -> Result<(), ApiError> {
    sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8::jsonb)
        ",
    )
    .bind(command.game_id)
    .bind(room_id)
    .bind(counters.sequence)
    .bind(command.event_type)
    .bind(command.command_id)
    .bind(command.actor_participant_id)
    .bind(counters.state_version)
    .bind(command.event_json)
    .execute(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("append game event", error))?;
    Ok(())
}

async fn insert_command_receipt(
    transaction: &mut Transaction<'_, Postgres>,
    command: &NewGameCommand<'_>,
    room_id: Uuid,
    expires_at: &str,
    counters: &PersistedCommandCounters,
) -> Result<StoredCommandReceipt, ApiError> {
    sqlx::query_as::<_, StoredCommandReceipt>(
        r"
        INSERT INTO game_command_receipts (
            game_id,
            room_id,
            command_id,
            actor_participant_id,
            command_type,
            expected_state_version,
            payload_digest,
            accepted_state_version,
            accepted_sequence,
            expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10::timestamptz)
        RETURNING
            command_id,
            actor_participant_id,
            command_type,
            expected_state_version,
            payload_digest,
            accepted_state_version,
            accepted_sequence,
            replace(
                to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
                ' ',
                'T'
            ) || 'Z' AS expires_at
        ",
    )
    .bind(command.game_id)
    .bind(room_id)
    .bind(command.command_id)
    .bind(command.actor_participant_id)
    .bind(command.command_type)
    .bind(
        i64::try_from(command.expected_state_version)
            .map_err(|_| ApiError::stale_state_version())?,
    )
    .bind(command.payload_digest)
    .bind(counters.state_version)
    .bind(counters.sequence)
    .bind(expires_at)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("persist command receipt", error))
}

pub(super) async fn command_receipt_in(
    transaction: &mut Transaction<'_, Postgres>,
    game_id: Uuid,
    command_id: Uuid,
) -> Result<Option<StoredCommandReceipt>, ApiError> {
    sqlx::query_as::<_, StoredCommandReceipt>(
        r"
        SELECT
            command_id,
            actor_participant_id,
            command_type,
            expected_state_version,
            payload_digest,
            accepted_state_version,
            accepted_sequence,
            replace(
                to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
                ' ',
                'T'
            ) || 'Z' AS expires_at
        FROM game_command_receipts
        WHERE game_id = $1
          AND command_id = $2
        ",
    )
    .bind(game_id)
    .bind(command_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
}

pub(super) async fn command_receipt_for_actor(
    database: &PgPool,
    participant_id: Uuid,
    command_id: Uuid,
) -> Result<Option<StoredCommandReceipt>, ApiError> {
    sqlx::query_as::<_, StoredCommandReceipt>(
        r"
        SELECT
            receipts.command_id,
            receipts.actor_participant_id,
            receipts.command_type,
            receipts.expected_state_version,
            receipts.payload_digest,
            receipts.accepted_state_version,
            receipts.accepted_sequence,
            replace(
                to_char(receipts.expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS.US'),
                ' ',
                'T'
            ) || 'Z' AS expires_at
        FROM game_command_receipts AS receipts
        JOIN games ON games.id = receipts.game_id
        JOIN participants ON participants.room_id = games.room_id
        WHERE participants.id = $1
          AND receipts.actor_participant_id = $1
          AND receipts.command_id = $2
        ",
    )
    .bind(participant_id)
    .bind(command_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
}

pub(super) async fn game_events_for_participant(
    database: &PgPool,
    participant_id: Uuid,
    game_id: Uuid,
    after_sequence: i64,
    through_sequence: i64,
) -> Result<Vec<StoredGameEvent>, ApiError> {
    sqlx::query_as::<_, StoredGameEvent>(
        r"
        SELECT
            events.event_version,
            events.event_type,
            events.command_id,
            events.actor_participant_id,
            actor.position AS actor_position,
            events.sequence,
            events.state_version,
            events.payload::text AS payload_json
        FROM game_events AS events
        JOIN games ON games.id = events.game_id
        JOIN participants AS viewer
          ON viewer.room_id = games.room_id
         AND viewer.id = $1
        JOIN participants AS actor
          ON actor.room_id = games.room_id
         AND actor.id = events.actor_participant_id
        WHERE events.game_id = $2
          AND events.sequence > $3
          AND events.sequence <= $4
        ORDER BY events.sequence
        ",
    )
    .bind(participant_id)
    .bind(game_id)
    .bind(after_sequence)
    .bind(through_sequence)
    .fetch_all(database)
    .await
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
}

pub(super) async fn game_cursor_for_participant(
    database: &PgPool,
    participant_id: Uuid,
    game_id: Uuid,
) -> Result<Option<(i64, i16, String)>, ApiError> {
    sqlx::query_as::<_, (i64, i16, String)>(
        r"
        SELECT games.sequence, games.snapshot_version, games.state_digest
        FROM games
        JOIN participants
          ON participants.room_id = games.room_id
         AND participants.id = $1
        WHERE games.id = $2
        ",
    )
    .bind(participant_id)
    .bind(game_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
}

pub(super) async fn game_state_anchor_for_participant(
    database: &PgPool,
    participant_id: Uuid,
    game_id: Uuid,
    sequence: i64,
) -> Result<Option<(i16, String)>, ApiError> {
    sqlx::query_as::<_, (i16, String)>(
        r"
        SELECT anchors.snapshot_version, anchors.state_digest
        FROM game_state_anchors AS anchors
        JOIN games ON games.id = anchors.game_id
        JOIN participants
          ON participants.room_id = games.room_id
         AND participants.id = $1
        WHERE anchors.game_id = $2
          AND anchors.sequence = $3
        ",
    )
    .bind(participant_id)
    .bind(game_id)
    .bind(sequence)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
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
    .map_err(|error| ApiError::internal_with("match persistence operation", error))
}

pub(super) async fn register_realtime_connection(
    database: &PgPool,
    connection_id: Uuid,
    game_id: Uuid,
    participant_id: Uuid,
    guest_session_id: Uuid,
) -> Result<bool, ApiError> {
    let registered = sqlx::query_scalar::<_, Uuid>(
        r"
        INSERT INTO game_realtime_connections (
            id,
            game_id,
            participant_id,
            guest_session_id
        )
        SELECT $1, games.id, participants.id, guest_sessions.id
        FROM games
        JOIN participants
          ON participants.room_id = games.room_id
         AND participants.id = $3
        JOIN device_sessions
          ON device_sessions.participant_id = participants.id
         AND device_sessions.guest_session_id = $4
         AND device_sessions.status = 'active'
        JOIN guest_sessions
          ON guest_sessions.id = device_sessions.guest_session_id
         AND guest_sessions.expires_at > clock_timestamp()
        WHERE games.id = $2
        RETURNING id
        ",
    )
    .bind(connection_id)
    .bind(game_id)
    .bind(participant_id)
    .bind(guest_session_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("register realtime presence", error))?
    .is_some();

    if registered {
        sqlx::query(
            r"
            DELETE FROM game_realtime_connections
            WHERE game_id = $1
              AND last_heartbeat_at < clock_timestamp() - INTERVAL '1 day'
            ",
        )
        .bind(game_id)
        .execute(database)
        .await
        .map_err(|error| ApiError::internal_with("prune realtime presence", error))?;
    }

    Ok(registered)
}

pub(super) async fn touch_realtime_connection(
    database: &PgPool,
    connection_id: Uuid,
    participant_id: Uuid,
    guest_session_id: Uuid,
) -> Result<bool, ApiError> {
    sqlx::query_scalar::<_, Uuid>(
        r"
        UPDATE game_realtime_connections AS connections
        SET last_heartbeat_at = clock_timestamp()
        FROM device_sessions, guest_sessions
        WHERE connections.id = $1
          AND connections.participant_id = $2
          AND connections.guest_session_id = $3
          AND connections.disconnected_at IS NULL
          AND device_sessions.participant_id = connections.participant_id
          AND device_sessions.guest_session_id = connections.guest_session_id
          AND device_sessions.status = 'active'
          AND guest_sessions.id = device_sessions.guest_session_id
          AND guest_sessions.expires_at > clock_timestamp()
        RETURNING connections.id
        ",
    )
    .bind(connection_id)
    .bind(participant_id)
    .bind(guest_session_id)
    .fetch_optional(database)
    .await
    .map(|connection| connection.is_some())
    .map_err(|error| ApiError::internal_with("refresh realtime presence", error))
}

pub(super) async fn disconnect_realtime_connection(
    database: &PgPool,
    connection_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query(
        r"
        UPDATE game_realtime_connections
        SET disconnected_at = clock_timestamp()
        WHERE id = $1
          AND disconnected_at IS NULL
        ",
    )
    .bind(connection_id)
    .execute(database)
    .await
    .map(|_| ())
    .map_err(|error| ApiError::internal_with("disconnect realtime presence", error))
}

pub(super) async fn game_presence(
    database: &PgPool,
    game_id: Uuid,
    online_window_seconds: i64,
    reconnecting_window_seconds: i64,
) -> Result<Option<(String, Vec<(i16, String)>)>, ApiError> {
    let snapshot_json = sqlx::query_scalar::<_, String>(
        r"
        SELECT snapshot::text
        FROM games
        WHERE id = $1
        ",
    )
    .bind(game_id)
    .fetch_optional(database)
    .await
    .map_err(|error| ApiError::internal_with("load realtime presence game", error))?;
    let Some(snapshot_json) = snapshot_json else {
        return Ok(None);
    };

    let participants = sqlx::query_as::<_, (i16, String)>(
        r"
        WITH observed AS (
            SELECT clock_timestamp() AS now
        )
        SELECT
            participants.position,
            CASE
                WHEN EXISTS (
                    SELECT 1
                    FROM game_realtime_connections AS connections
                    JOIN device_sessions
                      ON device_sessions.participant_id = connections.participant_id
                     AND device_sessions.guest_session_id = connections.guest_session_id
                     AND device_sessions.status = 'active'
                    JOIN guest_sessions
                      ON guest_sessions.id = device_sessions.guest_session_id
                    WHERE connections.game_id = games.id
                      AND connections.participant_id = participants.id
                      AND connections.disconnected_at IS NULL
                      AND connections.last_heartbeat_at >= observed.now -
                          make_interval(secs => $2::double precision)
                      AND guest_sessions.expires_at > observed.now
                ) THEN 'online'
                WHEN EXISTS (
                    SELECT 1
                    FROM game_realtime_connections AS connections
                    JOIN device_sessions
                      ON device_sessions.participant_id = connections.participant_id
                     AND device_sessions.guest_session_id = connections.guest_session_id
                     AND device_sessions.status = 'active'
                    JOIN guest_sessions
                      ON guest_sessions.id = device_sessions.guest_session_id
                    WHERE connections.game_id = games.id
                      AND connections.participant_id = participants.id
                      AND connections.last_heartbeat_at >= observed.now -
                          make_interval(secs => $3::double precision)
                      AND guest_sessions.expires_at > observed.now
                ) THEN 'reconnecting'
                ELSE 'offline'
            END AS status
        FROM games
        JOIN participants ON participants.room_id = games.room_id
        CROSS JOIN observed
        WHERE games.id = $1
        ORDER BY participants.position
        ",
    )
    .bind(game_id)
    .bind(online_window_seconds)
    .bind(reconnecting_window_seconds)
    .fetch_all(database)
    .await
    .map_err(|error| ApiError::internal_with("load realtime presence", error))?;

    Ok(Some((snapshot_json, participants)))
}
