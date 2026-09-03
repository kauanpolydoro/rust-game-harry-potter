use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{
        Path, Query, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::{get, post},
};
use game_content::{ContentManifest, EntryKind};
use game_domain::{
    ContentSelection, GameCommand, GameCommandError, GameCommandInput, GameEvent, GamePhase,
    GameStatus, HeroId, InitialGameState, InitialPlayer, LobbyParticipant, PRNG_ALGORITHM,
    ParticipantRole, SAMPLING_ALGORITHM, SHUFFLE_ALGORITHM, StartGameError, StartGameInput,
    decide_game_command, initialize_game,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::{
    AppState,
    identity_access::{ApiError, authenticated_participant, idempotency_key, no_store_json},
};

mod postgres;

const SEED_BYTES: usize = 32;
const REALTIME_PROTOCOL_VERSION: u16 = 1;
const REALTIME_SUBPROTOCOL: &str = "hogwarts.realtime.v1";
const REALTIME_POLL_INTERVAL: Duration = Duration::from_millis(500);
const REALTIME_REPLAY_LIMIT: u64 = 100;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/games", post(start_game))
        .route("/api/games/current/commands", post(execute_game_command))
        .route(
            "/api/games/current/commands/{command_id}",
            get(command_result),
        )
        .route("/api/games/current/events", get(game_events))
}

#[derive(Clone)]
pub(crate) struct ContentCatalog {
    manifests: Arc<[ContentManifest]>,
}

impl ContentCatalog {
    pub(crate) fn new(manifests: Vec<ContentManifest>) -> Self {
        Self {
            manifests: manifests.into(),
        }
    }

    fn selection(
        &self,
        adventure_id: &str,
        manifest_digest: &str,
        ruleset_version: &str,
    ) -> Option<SelectedContent> {
        let manifest = self.manifests.iter().find(|manifest| {
            manifest.digest == manifest_digest && manifest.ruleset_version == ruleset_version
        })?;
        let adventure = manifest.entries.iter().find(|entry| {
            entry.kind == EntryKind::Adventure && entry.catalog_id.as_str() == adventure_id
        })?;

        Some(SelectedContent {
            adventure_id: adventure.catalog_id.as_str().to_owned(),
            adventure_name: entry_name(adventure),
            content_version: manifest.content_version.clone(),
            ruleset_version: manifest.ruleset_version.clone(),
            manifest_digest: manifest.digest.clone(),
            manifest_version: manifest.manifest_version,
            playable: manifest.playable && adventure.playable,
        })
    }
}

#[derive(Serialize)]
pub(crate) struct ContentManifestOption {
    manifest_digest: String,
    manifest_version: u16,
    content_version: String,
    ruleset_version: String,
    playable: bool,
    adventures: Vec<AdventureOption>,
}

#[derive(Serialize)]
struct AdventureOption {
    id: String,
    name: String,
    playable: bool,
}

#[derive(Clone)]
struct SelectedContent {
    adventure_id: String,
    adventure_name: String,
    content_version: String,
    ruleset_version: String,
    manifest_digest: String,
    manifest_version: u16,
    playable: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartGameRequest {
    adventure_id: String,
    manifest_digest: String,
    ruleset_version: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecuteGameCommandRequest {
    command_id: String,
    expected_state_version: u64,
    #[serde(rename = "type")]
    command_type: GameCommandType,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum GameCommandType {
    CompleteDarkArts,
}

#[derive(FromRow)]
struct StoredRoomActor {
    room_id: Uuid,
    room_status: String,
    participant_id: Uuid,
    role: String,
}

#[derive(FromRow)]
struct StoredRoomParticipant {
    id: Uuid,
    display_name: String,
    role: String,
    position: i16,
    hero_id: Option<String>,
    ready: bool,
}

#[derive(FromRow)]
struct StoredGameStart {
    game_id: Uuid,
    participant_id: Uuid,
    adventure_id: String,
    manifest_digest: String,
    ruleset_version: String,
}

#[derive(FromRow)]
struct StoredGame {
    id: Uuid,
    status: String,
    adventure_id: String,
    adventure_name: String,
    manifest_digest: String,
    manifest_version: i16,
    content_version: String,
    ruleset_version: String,
    snapshot_version: i16,
    state_version: i64,
    sequence: i64,
    state_digest: String,
    snapshot_json: String,
    prng_algorithm: String,
    prng_counter: i64,
    shuffle_algorithm: String,
    sampling_algorithm: String,
    expires_at: String,
}

#[derive(FromRow)]
struct StoredCommandGame {
    id: Uuid,
    status: String,
    adventure_id: String,
    manifest_digest: String,
    manifest_version: i16,
    content_version: String,
    ruleset_version: String,
    snapshot_version: i16,
    state_version: i64,
    sequence: i64,
    state_digest: String,
    snapshot_json: String,
    prng_algorithm: String,
    prng_counter: i64,
    shuffle_algorithm: String,
    sampling_algorithm: String,
    actor_position: i16,
    expired: bool,
}

#[derive(FromRow)]
struct StoredCommandReceipt {
    command_id: Uuid,
    actor_participant_id: Uuid,
    command_type: String,
    expected_state_version: i64,
    payload_digest: String,
    accepted_state_version: i64,
    accepted_sequence: i64,
    expires_at: String,
}

#[derive(FromRow)]
struct StoredGameEvent {
    event_version: i16,
    event_type: String,
    command_id: Uuid,
    actor_participant_id: Uuid,
    actor_position: i16,
    sequence: i64,
    state_version: i64,
    payload_json: String,
}

#[derive(Serialize)]
pub(crate) struct GameProjectionResponse {
    game: GameSummary,
    snapshot: SnapshotSummary,
    turn: TurnSummary,
    participant: GameParticipant,
    participants: Vec<GameParticipant>,
    legal_actions: Vec<String>,
    choice: ChoiceSummary,
}

#[derive(Serialize)]
struct GameSummary {
    id: String,
    status: String,
    adventure: AdventureSummary,
    expires_at: String,
}

#[derive(Serialize)]
struct AdventureSummary {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct SnapshotSummary {
    snapshot_version: i16,
    state_version: i64,
    sequence: i64,
    cursor: i64,
    digest: String,
    versions: GameVersions,
}

#[derive(Serialize)]
struct ChoiceSummary {
    status: &'static str,
}

#[derive(Serialize)]
struct GameVersions {
    content: String,
    ruleset: String,
    manifest: i16,
    manifest_digest: String,
    prng: String,
    shuffle: String,
    sampling: String,
}

#[derive(Serialize)]
struct TurnSummary {
    number: u32,
    phase: String,
    active_position: u8,
}

#[derive(Serialize)]
struct GameParticipant {
    display_name: String,
    role: String,
    position: i16,
    hero: GameHero,
}

#[derive(Serialize)]
struct GameHero {
    id: String,
    name: &'static str,
}

#[derive(Serialize)]
struct ExecuteGameCommandResponse {
    receipt: GameCommandReceipt,
    projection: GameProjectionResponse,
}

#[derive(Serialize)]
struct GameCommandReceipt {
    command_id: String,
    #[serde(rename = "type")]
    command_type: String,
    status: &'static str,
    expected_state_version: i64,
    accepted_state_version: i64,
    accepted_sequence: i64,
    expires_at: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedSnapshot {
    snapshot_version: u16,
    state_version: u64,
    sequence: u64,
    status: String,
    adventure_id: String,
    versions: PersistedVersions,
    turn: PersistedTurn,
    participants: Vec<PersistedPlayer>,
    prng: PersistedPrng,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedVersions {
    content: String,
    ruleset: String,
    manifest: u16,
    manifest_digest: String,
    prng: String,
    shuffle: String,
    sampling: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedTurn {
    number: u32,
    phase: String,
    active_position: u8,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedPlayer {
    participant_id: String,
    position: u8,
    hero_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedPrng {
    algorithm: String,
    counter: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RealtimeQuery {
    cursor: Option<u64>,
    snapshot_version: Option<u16>,
}

#[derive(Serialize)]
struct RealtimeSnapshotMessage {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: &'static str,
    cursor: i64,
    projection: GameProjectionResponse,
}

#[derive(Serialize)]
struct RealtimeEventBatchMessage {
    protocol_version: u16,
    #[serde(rename = "type")]
    message_type: &'static str,
    from_cursor: i64,
    cursor: i64,
    events: Vec<RealtimeGameEvent>,
    projection: GameProjectionResponse,
}

#[derive(Serialize)]
struct RealtimeGameEvent {
    event_version: i16,
    #[serde(rename = "type")]
    event_type: &'static str,
    sequence: i64,
    state_version: i64,
    turn: u32,
    actor_position: i16,
    #[serde(skip_serializing_if = "Option::is_none")]
    command_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedGameEvent {
    event_version: u16,
    #[serde(rename = "type")]
    event_type: String,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
}

pub(crate) fn content_options(state: &AppState) -> Vec<ContentManifestOption> {
    state
        .content
        .manifests
        .iter()
        .map(|manifest| ContentManifestOption {
            manifest_digest: manifest.digest.clone(),
            manifest_version: manifest.manifest_version,
            content_version: manifest.content_version.clone(),
            ruleset_version: manifest.ruleset_version.clone(),
            playable: manifest.playable,
            adventures: manifest
                .entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::Adventure)
                .map(|entry| AdventureOption {
                    id: entry.catalog_id.as_str().to_owned(),
                    name: entry_name(entry),
                    playable: manifest.playable && entry.playable,
                })
                .collect(),
        })
        .collect()
}

pub(crate) async fn publish_content(state: &AppState) -> Result<(), sqlx::Error> {
    for manifest in state.content.manifests.iter() {
        let document = serde_json::to_string(manifest)
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
        postgres::publish_manifest(&state.database, manifest, &document).await?;
    }
    Ok(())
}

async fn start_game(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StartGameRequest>,
) -> Result<Response, ApiError> {
    let key = idempotency_key(&headers)?;
    let participant_id = authenticated_participant(&state, &headers).await?;

    if let Some(stored) = postgres::load_game_start(&state.database, &key).await? {
        return replay_game_start(&state.database, stored, participant_id, &request).await;
    }

    let content = state
        .content
        .selection(
            &request.adventure_id,
            &request.manifest_digest,
            &request.ruleset_version,
        )
        .ok_or_else(ApiError::content_not_playable)?;
    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
    let actor = postgres::lock_room_actor(&mut transaction, participant_id)
        .await?
        .ok_or_else(ApiError::session_invalid)?;

    if let Some(stored) = postgres::load_game_start_in(&mut transaction, &key).await? {
        transaction
            .rollback()
            .await
            .map_err(|_| ApiError::internal())?;
        return replay_game_start(&state.database, stored, participant_id, &request).await;
    }
    if actor.room_status != "open" {
        return Err(ApiError::room_sealed());
    }

    let stored_participants = postgres::room_participants(&mut transaction, actor.room_id).await?;
    let participants = stored_participants
        .iter()
        .map(domain_participant)
        .collect::<Result<Vec<_>, _>>()?;
    let initial_state = initialize_game(StartGameInput {
        actor_role: participant_role(&actor.role)?,
        participants: &participants,
        content: ContentSelection {
            adventure_id: &content.adventure_id,
            content_version: &content.content_version,
            ruleset_version: &content.ruleset_version,
            manifest_digest: &content.manifest_digest,
            manifest_version: content.manifest_version,
            playable: content.playable,
        },
    })
    .map_err(start_error)?;

    let game_id = Uuid::new_v4();
    let claimed =
        postgres::claim_game_start(&mut transaction, &key, game_id, &actor, &request).await?;
    if !claimed {
        transaction
            .rollback()
            .await
            .map_err(|_| ApiError::internal())?;
        let stored = postgres::load_game_start(&state.database, &key)
            .await?
            .ok_or_else(ApiError::internal)?;
        return replay_game_start(&state.database, stored, participant_id, &request).await;
    }

    let snapshot = persisted_snapshot(&initial_state, &stored_participants);
    let snapshot_json = serde_json::to_string(&snapshot).map_err(|_| ApiError::internal())?;
    let state_digest = format!("blake3:{}", blake3::hash(snapshot_json.as_bytes()).to_hex());
    let mut seed = [0_u8; SEED_BYTES];
    getrandom::fill(&mut seed).map_err(|_| ApiError::internal())?;

    postgres::persist_game(
        &mut transaction,
        postgres::NewGame {
            id: game_id,
            actor: &actor,
            content: &content,
            state: &initial_state,
            state_digest: &state_digest,
            snapshot_json: &snapshot_json,
            seed: &seed,
        },
    )
    .await?;
    postgres::seal_room(&mut transaction, actor.room_id).await?;
    transaction
        .commit()
        .await
        .map_err(|_| ApiError::internal())?;

    let projection = projection_for_participant(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok(no_store_json(StatusCode::CREATED, projection))
}

async fn replay_game_start(
    database: &sqlx::PgPool,
    stored: StoredGameStart,
    participant_id: Uuid,
    request: &StartGameRequest,
) -> Result<Response, ApiError> {
    if stored.participant_id != participant_id
        || stored.adventure_id != request.adventure_id
        || stored.manifest_digest != request.manifest_digest
        || stored.ruleset_version != request.ruleset_version
    {
        return Err(ApiError::idempotency_conflict());
    }

    let projection = projection_for_participant(database, participant_id)
        .await?
        .filter(|projection| projection.game.id == stored.game_id.to_string())
        .ok_or_else(ApiError::internal)?;
    Ok(no_store_json(StatusCode::CREATED, projection))
}

async fn execute_game_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ExecuteGameCommandRequest>,
) -> Result<Response, ApiError> {
    let participant_id = authenticated_participant(&state, &headers).await?;
    let command_id =
        Uuid::parse_str(&request.command_id).map_err(|_| ApiError::invalid_command_id())?;
    let request_json = serde_json::to_vec(&request).map_err(|_| ApiError::internal())?;
    let payload_digest = format!("blake3:{}", blake3::hash(&request_json).to_hex());
    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
    let stored = postgres::lock_game_for_actor(&mut transaction, participant_id)
        .await?
        .ok_or_else(ApiError::game_action_not_allowed)?;
    if stored.expired {
        return Err(ApiError::game_expired());
    }
    if let Some(receipt) =
        postgres::command_receipt_in(&mut transaction, stored.id, command_id).await?
    {
        transaction
            .rollback()
            .await
            .map_err(|_| ApiError::internal())?;
        if receipt.actor_participant_id != participant_id
            || receipt.payload_digest != payload_digest
        {
            return Err(ApiError::idempotency_conflict());
        }
        let projection = projection_for_participant(&state.database, participant_id)
            .await?
            .ok_or_else(ApiError::internal)?;
        return Ok(no_store_json(
            StatusCode::OK,
            ExecuteGameCommandResponse {
                receipt: receipt_response(receipt),
                projection,
            },
        ));
    }

    let persisted: PersistedSnapshot =
        serde_json::from_str(&stored.snapshot_json).map_err(|_| ApiError::internal())?;
    verify_command_snapshot(&stored, &persisted)?;
    let current = command_domain_state(&persisted)?;
    let decision = decide_game_command(GameCommandInput {
        state: &current,
        actor_position: u8::try_from(stored.actor_position)
            .map_err(|_| ApiError::game_action_not_allowed())?,
        expected_state_version: request.expected_state_version,
        command: match request.command_type {
            GameCommandType::CompleteDarkArts => GameCommand::CompleteDarkArts,
        },
    })
    .map_err(command_error)?;

    let next_snapshot = persisted_after_decision(&persisted, &decision.state);
    let snapshot_json = serde_json::to_string(&next_snapshot).map_err(|_| ApiError::internal())?;
    let state_digest = format!("blake3:{}", blake3::hash(snapshot_json.as_bytes()).to_hex());
    let (event_type, event_json) = persisted_event(&decision.events)?;
    let receipt = postgres::persist_game_command(
        &mut transaction,
        postgres::NewGameCommand {
            game_id: stored.id,
            actor_participant_id: participant_id,
            command_id,
            request: &request,
            command_type: command_type_name(request.command_type),
            payload_digest: &payload_digest,
            state: &decision.state,
            state_digest: &state_digest,
            snapshot_json: &snapshot_json,
            event_type,
            event_json: &event_json,
        },
    )
    .await?;
    transaction
        .commit()
        .await
        .map_err(|_| ApiError::internal())?;
    state.signal_game_event(stored.id);

    let projection = projection_for_participant(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok(no_store_json(
        StatusCode::OK,
        ExecuteGameCommandResponse {
            receipt: receipt_response(receipt),
            projection,
        },
    ))
}

async fn command_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(command_id): Path<String>,
) -> Result<Response, ApiError> {
    let participant_id = authenticated_participant(&state, &headers).await?;
    let command_id = Uuid::parse_str(&command_id).map_err(|_| ApiError::invalid_command_id())?;
    let receipt = postgres::command_receipt_for_actor(&state.database, participant_id, command_id)
        .await?
        .ok_or_else(ApiError::command_not_found)?;
    let projection = projection_for_participant(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;

    Ok(no_store_json(
        StatusCode::OK,
        ExecuteGameCommandResponse {
            receipt: receipt_response(receipt),
            projection,
        },
    ))
}

async fn game_events(
    State(state): State<AppState>,
    Query(query): Query<RealtimeQuery>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    require_realtime_origin(&state, &headers)?;
    if !websocket
        .requested_protocols()
        .any(|protocol| protocol.as_bytes() == REALTIME_SUBPROTOCOL.as_bytes())
    {
        return Err(ApiError::upgrade_required());
    }

    let participant_id = authenticated_participant(&state, &headers).await?;
    let projection = projection_for_participant(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::game_action_not_allowed)?;
    let game_id = Uuid::parse_str(&projection.game.id).map_err(|_| ApiError::internal())?;

    Ok(websocket
        .protocols([REALTIME_SUBPROTOCOL])
        .max_message_size(4 * 1024)
        .on_upgrade(move |socket| serve_game_events(socket, state, participant_id, game_id, query)))
}

fn require_realtime_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let origin = origins
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(ApiError::origin_not_allowed)?;
    if origins.next().is_some() || origin != state.application_origin() {
        return Err(ApiError::origin_not_allowed());
    }
    Ok(())
}

async fn serve_game_events(
    mut socket: WebSocket,
    state: AppState,
    participant_id: Uuid,
    game_id: Uuid,
    query: RealtimeQuery,
) {
    let mut cursor = query.cursor.and_then(|value| i64::try_from(value).ok());
    let mut snapshot_version = query.snapshot_version;
    let force_initial_snapshot = query.cursor.is_none() || cursor.is_none();
    if !synchronize_socket(
        &mut socket,
        &state,
        participant_id,
        game_id,
        &mut cursor,
        &mut snapshot_version,
        force_initial_snapshot,
    )
    .await
    {
        return;
    }

    let mut signal = state.subscribe_to_game_events();
    let mut poll = tokio::time::interval(REALTIME_POLL_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    poll.tick().await;

    loop {
        let should_synchronize = tokio::select! {
            message = socket.recv() => {
                match message {
                    Some(
                        Ok(Message::Close(_) | Message::Text(_) | Message::Binary(_)) | Err(_),
                    )
                    | None => return,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => false,
                }
            }
            notification = signal.recv() => {
                match notification {
                    Ok(changed_game_id) => changed_game_id == game_id,
                    Err(
                        broadcast::error::RecvError::Lagged(_)
                        | broadcast::error::RecvError::Closed,
                    ) => true,
                }
            }
            _ = poll.tick() => true,
        };

        if should_synchronize
            && !synchronize_socket(
                &mut socket,
                &state,
                participant_id,
                game_id,
                &mut cursor,
                &mut snapshot_version,
                false,
            )
            .await
        {
            return;
        }
    }
}

async fn synchronize_socket(
    socket: &mut WebSocket,
    state: &AppState,
    participant_id: Uuid,
    game_id: Uuid,
    cursor: &mut Option<i64>,
    snapshot_version: &mut Option<u16>,
    force_snapshot: bool,
) -> bool {
    let (observed_cursor, observed_snapshot_version) =
        match postgres::game_cursor_for_participant(&state.database, participant_id, game_id).await
        {
            Ok(Some((observed_cursor, observed_snapshot_version))) => {
                let Ok(observed_snapshot_version) = u16::try_from(observed_snapshot_version) else {
                    return false;
                };
                (observed_cursor, observed_snapshot_version)
            }
            Ok(None) | Err(_) => return false,
        };
    if !force_snapshot
        && *cursor == Some(observed_cursor)
        && *snapshot_version == Some(observed_snapshot_version)
    {
        return true;
    }

    let projection = match projection_for_participant(&state.database, participant_id).await {
        Ok(Some(projection)) if projection.game.id == game_id.to_string() => projection,
        Ok(Some(_) | None) | Err(_) => return false,
    };
    let current_cursor = projection.snapshot.cursor;
    let Ok(current_snapshot_version) = u16::try_from(projection.snapshot.snapshot_version) else {
        return false;
    };
    let requested_cursor = *cursor;
    let replay_distance = requested_cursor
        .and_then(|value| current_cursor.checked_sub(value))
        .and_then(|value| u64::try_from(value).ok());
    let needs_snapshot = force_snapshot
        || *snapshot_version != Some(current_snapshot_version)
        || requested_cursor.is_none_or(|value| value > current_cursor)
        || replay_distance.is_none_or(|distance| distance > REALTIME_REPLAY_LIMIT);

    if needs_snapshot {
        let message = RealtimeSnapshotMessage {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            message_type: "snapshot",
            cursor: current_cursor,
            projection,
        };
        if !send_realtime_message(socket, &message).await {
            return false;
        }
        *cursor = Some(current_cursor);
        *snapshot_version = Some(current_snapshot_version);
        return true;
    }

    let from_cursor = requested_cursor.expect("a compatible cursor was checked above");
    if from_cursor == current_cursor {
        return true;
    }
    let events = match postgres::game_events_for_participant(
        &state.database,
        participant_id,
        game_id,
        from_cursor,
        current_cursor,
    )
    .await
    {
        Ok(stored) => stored
            .iter()
            .map(|event| realtime_event(event, participant_id))
            .collect::<Result<Vec<_>, _>>(),
        Err(error) => Err(error),
    };
    let Ok(events) = events else {
        return false;
    };
    if !events_are_contiguous(&events, from_cursor, current_cursor) {
        let message = RealtimeSnapshotMessage {
            protocol_version: REALTIME_PROTOCOL_VERSION,
            message_type: "snapshot",
            cursor: current_cursor,
            projection,
        };
        if !send_realtime_message(socket, &message).await {
            return false;
        }
        *cursor = Some(current_cursor);
        *snapshot_version = Some(current_snapshot_version);
        return true;
    }

    let message = RealtimeEventBatchMessage {
        protocol_version: REALTIME_PROTOCOL_VERSION,
        message_type: "events",
        from_cursor,
        cursor: current_cursor,
        events,
        projection,
    };
    if !send_realtime_message(socket, &message).await {
        return false;
    }
    *cursor = Some(current_cursor);
    *snapshot_version = Some(current_snapshot_version);
    true
}

fn realtime_event(
    stored: &StoredGameEvent,
    participant_id: Uuid,
) -> Result<RealtimeGameEvent, ApiError> {
    let payload: PersistedGameEvent =
        serde_json::from_str(&stored.payload_json).map_err(|_| ApiError::internal())?;
    let metadata_matches = i16::try_from(payload.event_version).ok() == Some(stored.event_version)
        && payload.event_type == stored.event_type
        && i64::try_from(payload.sequence).ok() == Some(stored.sequence)
        && i64::try_from(payload.state_version).ok() == Some(stored.state_version)
        && i16::from(payload.actor_position) == stored.actor_position;
    if !metadata_matches || stored.event_type != "dark_arts_completed" {
        return Err(ApiError::internal());
    }

    Ok(RealtimeGameEvent {
        event_version: stored.event_version,
        event_type: "dark_arts_completed",
        sequence: stored.sequence,
        state_version: stored.state_version,
        turn: payload.turn,
        actor_position: stored.actor_position,
        command_id: (stored.actor_participant_id == participant_id)
            .then(|| stored.command_id.to_string()),
    })
}

fn events_are_contiguous(
    events: &[RealtimeGameEvent],
    from_cursor: i64,
    current_cursor: i64,
) -> bool {
    let Some(expected_count) = current_cursor
        .checked_sub(from_cursor)
        .and_then(|count| usize::try_from(count).ok())
    else {
        return false;
    };
    events.len() == expected_count
        && events
            .iter()
            .zip((from_cursor + 1)..=current_cursor)
            .all(|(event, expected)| event.sequence == expected)
}

async fn send_realtime_message(socket: &mut WebSocket, value: &impl Serialize) -> bool {
    let Ok(serialized) = serde_json::to_string(value) else {
        return false;
    };
    socket.send(Message::Text(serialized.into())).await.is_ok()
}

pub(crate) async fn projection_for_participant(
    database: &sqlx::PgPool,
    participant_id: Uuid,
) -> Result<Option<GameProjectionResponse>, ApiError> {
    let Some(game) = postgres::game_for_participant(database, participant_id).await? else {
        return Ok(None);
    };
    let persisted: PersistedSnapshot =
        serde_json::from_str(&game.snapshot_json).map_err(|_| ApiError::internal())?;
    let canonical_snapshot = serde_json::to_string(&persisted).map_err(|_| ApiError::internal())?;
    let verified_digest = format!(
        "blake3:{}",
        blake3::hash(canonical_snapshot.as_bytes()).to_hex()
    );
    if verified_digest != game.state_digest {
        return Err(ApiError::internal());
    }
    let snapshot_metadata_matches = i16::try_from(persisted.snapshot_version).ok()
        == Some(game.snapshot_version)
        && i64::try_from(persisted.state_version).ok() == Some(game.state_version)
        && i64::try_from(persisted.sequence).ok() == Some(game.sequence)
        && persisted.status == game.status
        && persisted.adventure_id == game.adventure_id
        && i16::try_from(persisted.versions.manifest).ok() == Some(game.manifest_version)
        && persisted.versions.content == game.content_version
        && persisted.versions.ruleset == game.ruleset_version
        && persisted.versions.manifest_digest == game.manifest_digest
        && persisted.versions.prng == game.prng_algorithm
        && persisted.versions.shuffle == game.shuffle_algorithm
        && persisted.versions.sampling == game.sampling_algorithm
        && i64::try_from(persisted.prng.counter).ok() == Some(game.prng_counter);
    if !snapshot_metadata_matches {
        return Err(ApiError::internal());
    }
    let participants = postgres::game_participants(database, game.id).await?;
    let current = participants
        .iter()
        .find(|participant| participant.id == participant_id)
        .ok_or_else(ApiError::internal)?;
    let legal_actions = if persisted.turn.phase == "dark_arts"
        && current.position == i16::from(persisted.turn.active_position)
    {
        vec!["complete_dark_arts".to_owned()]
    } else {
        Vec::new()
    };

    Ok(Some(GameProjectionResponse {
        game: GameSummary {
            id: game.id.to_string(),
            status: game.status,
            adventure: AdventureSummary {
                id: game.adventure_id,
                name: game.adventure_name,
            },
            expires_at: game.expires_at,
        },
        snapshot: SnapshotSummary {
            snapshot_version: game.snapshot_version,
            state_version: game.state_version,
            sequence: game.sequence,
            cursor: game.sequence,
            digest: game.state_digest,
            versions: GameVersions {
                content: game.content_version,
                ruleset: game.ruleset_version,
                manifest: game.manifest_version,
                manifest_digest: game.manifest_digest,
                prng: game.prng_algorithm,
                shuffle: game.shuffle_algorithm,
                sampling: game.sampling_algorithm,
            },
        },
        turn: TurnSummary {
            number: persisted.turn.number,
            phase: persisted.turn.phase,
            active_position: persisted.turn.active_position,
        },
        participant: game_participant(current)?,
        participants: participants
            .iter()
            .map(game_participant)
            .collect::<Result<Vec<_>, _>>()?,
        legal_actions,
        choice: ChoiceSummary { status: "none" },
    }))
}

fn entry_name(entry: &game_content::ManifestEntry) -> String {
    entry
        .names
        .get("pt-BR")
        .or_else(|| entry.names.get("en"))
        .or_else(|| entry.names.values().next())
        .cloned()
        .unwrap_or_else(|| entry.catalog_id.as_str().to_owned())
}

fn participant_role(role: &str) -> Result<ParticipantRole, ApiError> {
    match role {
        "host" => Ok(ParticipantRole::Host),
        "guest" => Ok(ParticipantRole::Guest),
        _ => Err(ApiError::internal()),
    }
}

fn hero_id(hero: &str) -> Result<HeroId, ApiError> {
    match hero {
        "harry" => Ok(HeroId::Harry),
        "hermione" => Ok(HeroId::Hermione),
        "neville" => Ok(HeroId::Neville),
        "ron" => Ok(HeroId::Ron),
        _ => Err(ApiError::internal()),
    }
}

fn hero_name(hero: &str) -> Result<&'static str, ApiError> {
    match hero {
        "harry" => Ok("Harry"),
        "hermione" => Ok("Hermione"),
        "neville" => Ok("Neville"),
        "ron" => Ok("Ron"),
        _ => Err(ApiError::internal()),
    }
}

fn domain_participant(stored: &StoredRoomParticipant) -> Result<LobbyParticipant, ApiError> {
    Ok(LobbyParticipant {
        role: participant_role(&stored.role)?,
        position: u8::try_from(stored.position).map_err(|_| ApiError::invalid_positions())?,
        hero: stored.hero_id.as_deref().map(hero_id).transpose()?,
        ready: stored.ready,
    })
}

fn start_error(error: StartGameError) -> ApiError {
    match error {
        StartGameError::ActorNotHost => ApiError::not_room_host(),
        StartGameError::InvalidParticipantCount => ApiError::invalid_participant_count(),
        StartGameError::InvalidHostCount | StartGameError::InvalidPositions => {
            ApiError::invalid_positions()
        }
        StartGameError::MissingHero | StartGameError::DuplicateHero => {
            ApiError::invalid_participant_heroes()
        }
        StartGameError::ParticipantNotReady => ApiError::participants_not_ready(),
        StartGameError::ContentNotPlayable | StartGameError::InvalidContentIdentity => {
            ApiError::content_not_playable()
        }
    }
}

fn command_error(error: GameCommandError) -> ApiError {
    match error {
        GameCommandError::StaleStateVersion => ApiError::stale_state_version(),
        GameCommandError::ActorNotActive | GameCommandError::CommandNotLegal => {
            ApiError::game_action_not_allowed()
        }
        GameCommandError::VersionOverflow => ApiError::internal(),
    }
}

const fn command_type_name(command_type: GameCommandType) -> &'static str {
    match command_type {
        GameCommandType::CompleteDarkArts => "complete_dark_arts",
    }
}

fn receipt_response(receipt: StoredCommandReceipt) -> GameCommandReceipt {
    GameCommandReceipt {
        command_id: receipt.command_id.to_string(),
        command_type: receipt.command_type,
        status: "accepted",
        expected_state_version: receipt.expected_state_version,
        accepted_state_version: receipt.accepted_state_version,
        accepted_sequence: receipt.accepted_sequence,
        expires_at: receipt.expires_at,
    }
}

fn verify_command_snapshot(
    game: &StoredCommandGame,
    persisted: &PersistedSnapshot,
) -> Result<(), ApiError> {
    let canonical_snapshot = serde_json::to_string(persisted).map_err(|_| ApiError::internal())?;
    let verified_digest = format!(
        "blake3:{}",
        blake3::hash(canonical_snapshot.as_bytes()).to_hex()
    );
    let metadata_matches = verified_digest == game.state_digest
        && i16::try_from(persisted.snapshot_version).ok() == Some(game.snapshot_version)
        && i64::try_from(persisted.state_version).ok() == Some(game.state_version)
        && i64::try_from(persisted.sequence).ok() == Some(game.sequence)
        && persisted.status == game.status
        && persisted.adventure_id == game.adventure_id
        && i16::try_from(persisted.versions.manifest).ok() == Some(game.manifest_version)
        && persisted.versions.content == game.content_version
        && persisted.versions.ruleset == game.ruleset_version
        && persisted.versions.manifest_digest == game.manifest_digest
        && persisted.versions.prng == game.prng_algorithm
        && persisted.versions.shuffle == game.shuffle_algorithm
        && persisted.versions.sampling == game.sampling_algorithm
        && i64::try_from(persisted.prng.counter).ok() == Some(game.prng_counter);
    if !metadata_matches {
        return Err(ApiError::internal());
    }
    Ok(())
}

fn command_domain_state(persisted: &PersistedSnapshot) -> Result<InitialGameState, ApiError> {
    let status = match persisted.status.as_str() {
        "in_progress" => GameStatus::InProgress,
        _ => return Err(ApiError::game_action_not_allowed()),
    };
    let phase = match persisted.turn.phase.as_str() {
        "dark_arts" => GamePhase::DarkArts,
        "hero_action" => GamePhase::HeroAction,
        _ => return Err(ApiError::internal()),
    };
    let players = persisted
        .participants
        .iter()
        .map(|player| {
            Ok(InitialPlayer {
                position: player.position,
                hero: hero_id(&player.hero_id)?,
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    Ok(InitialGameState {
        snapshot_version: persisted.snapshot_version,
        state_version: persisted.state_version,
        sequence: persisted.sequence,
        status,
        turn: persisted.turn.number,
        phase,
        active_position: persisted.turn.active_position,
        adventure_id: persisted.adventure_id.clone(),
        content_version: persisted.versions.content.clone(),
        ruleset_version: persisted.versions.ruleset.clone(),
        manifest_digest: persisted.versions.manifest_digest.clone(),
        manifest_version: persisted.versions.manifest,
        prng_algorithm: PRNG_ALGORITHM,
        shuffle_algorithm: SHUFFLE_ALGORITHM,
        sampling_algorithm: SAMPLING_ALGORITHM,
        prng_counter: persisted.prng.counter,
        players,
    })
}

fn persisted_after_decision(
    current: &PersistedSnapshot,
    state: &InitialGameState,
) -> PersistedSnapshot {
    let mut next = current.clone();
    next.state_version = state.state_version;
    next.sequence = state.sequence;
    next.status = match state.status {
        GameStatus::InProgress => "in_progress".to_owned(),
    };
    next.turn.number = state.turn;
    next.turn.phase = match state.phase {
        GamePhase::DarkArts => "dark_arts".to_owned(),
        GamePhase::HeroAction => "hero_action".to_owned(),
    };
    next.turn.active_position = state.active_position;
    next.prng.counter = state.prng_counter;
    next
}

fn persisted_event(events: &[GameEvent]) -> Result<(&'static str, String), ApiError> {
    match events {
        [
            GameEvent::DarkArtsCompleted {
                sequence,
                state_version,
                turn,
                actor_position,
            },
        ] => serde_json::to_string(&serde_json::json!({
            "event_version": 1,
            "type": "dark_arts_completed",
            "sequence": sequence,
            "state_version": state_version,
            "turn": turn,
            "actor_position": actor_position,
        }))
        .map(|event| ("dark_arts_completed", event))
        .map_err(|_| ApiError::internal()),
        _ => Err(ApiError::internal()),
    }
}

fn persisted_snapshot(
    state: &InitialGameState,
    participants: &[StoredRoomParticipant],
) -> PersistedSnapshot {
    PersistedSnapshot {
        snapshot_version: state.snapshot_version,
        state_version: state.state_version,
        sequence: state.sequence,
        status: match state.status {
            GameStatus::InProgress => "in_progress".to_owned(),
        },
        adventure_id: state.adventure_id.clone(),
        versions: PersistedVersions {
            content: state.content_version.clone(),
            ruleset: state.ruleset_version.clone(),
            manifest: state.manifest_version,
            manifest_digest: state.manifest_digest.clone(),
            prng: state.prng_algorithm.to_owned(),
            shuffle: state.shuffle_algorithm.to_owned(),
            sampling: state.sampling_algorithm.to_owned(),
        },
        turn: PersistedTurn {
            number: state.turn,
            phase: match state.phase {
                GamePhase::DarkArts => "dark_arts".to_owned(),
                GamePhase::HeroAction => "hero_action".to_owned(),
            },
            active_position: state.active_position,
        },
        participants: participants
            .iter()
            .filter_map(|participant| {
                participant.hero_id.as_ref().map(|hero_id| PersistedPlayer {
                    participant_id: participant.id.to_string(),
                    position: u8::try_from(participant.position)
                        .expect("validated room positions fit in u8"),
                    hero_id: hero_id.clone(),
                })
            })
            .collect(),
        prng: PersistedPrng {
            algorithm: state.prng_algorithm.to_owned(),
            counter: state.prng_counter,
        },
    }
}

fn game_participant(stored: &StoredRoomParticipant) -> Result<GameParticipant, ApiError> {
    let hero_id = stored.hero_id.as_deref().ok_or_else(ApiError::internal)?;
    Ok(GameParticipant {
        display_name: stored.display_name.clone(),
        role: stored.role.clone(),
        position: stored.position,
        hero: GameHero {
            id: hero_id.to_owned(),
            name: hero_name(hero_id)?,
        },
    })
}
