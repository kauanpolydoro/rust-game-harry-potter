use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
};
use game_domain::{
    ContentSelection, GameCommand, GameCommandError, GameCommandInput, HeroId, InitialGameState,
    LobbyParticipant, ParticipantRole, StartGameError, StartGameInput, decide_game_command,
    initialize_game,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    AppState,
    content_catalog::SelectedContent,
    http_support::{ApiError, idempotency_key, no_store_json},
    session::authenticated_participant,
};

mod codec;
mod postgres;
mod projection;
mod realtime;

use codec::{
    command_domain_state, decode_persisted_snapshot, persisted_after_decision, persisted_event,
    persisted_snapshot, verify_persisted_snapshot,
};
pub(crate) use projection::{GameProjectionResponse, projection_for_participant};

const SEED_BYTES: usize = 32;
const GAME_EVENT_VERSION: u16 = 1;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/games", post(start_game))
        .route("/api/games/current/commands", post(execute_game_command))
        .route(
            "/api/games/current/commands/{command_id}",
            get(command_result),
        )
        .route("/api/games/current/events", get(realtime::game_events))
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
    expired: bool,
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
struct PersistedTurn {
    number: u32,
    phase: String,
    active_position: u8,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPlayer {
    participant_id: String,
    position: u8,
    hero_id: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedPrng {
    algorithm: String,
    counter: u64,
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
        .map_err(|error| ApiError::internal_with("begin start-game transaction", error))?;
    let actor = postgres::lock_room_actor(&mut transaction, participant_id)
        .await?
        .ok_or_else(ApiError::session_invalid)?;

    if let Some(stored) = postgres::load_game_start_in(&mut transaction, &key).await? {
        transaction
            .rollback()
            .await
            .map_err(|error| ApiError::internal_with("match application operation", error))?;
        return replay_game_start(&state.database, stored, participant_id, &request).await;
    }
    if actor.room_status != "open" {
        return Err(ApiError::room_sealed());
    }

    let (initial_state, stored_participants) =
        initialize_persisted_game(&mut transaction, &actor, &content).await?;

    let game_id = Uuid::new_v4();
    let claimed = postgres::claim_game_start(
        &mut transaction,
        &key,
        game_id,
        &actor,
        postgres::GameStartClaim {
            adventure_id: &request.adventure_id,
            manifest_digest: &request.manifest_digest,
            ruleset_version: &request.ruleset_version,
        },
    )
    .await?;
    if !claimed {
        transaction
            .rollback()
            .await
            .map_err(|error| ApiError::internal_with("match application operation", error))?;
        let stored = postgres::load_game_start(&state.database, &key)
            .await?
            .ok_or_else(ApiError::internal)?;
        return replay_game_start(&state.database, stored, participant_id, &request).await;
    }

    let snapshot = persisted_snapshot(&initial_state, &stored_participants);
    let snapshot_json = serde_json::to_string(&snapshot)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    let state_digest = format!("blake3:{}", blake3::hash(snapshot_json.as_bytes()).to_hex());
    let mut seed = [0_u8; SEED_BYTES];
    getrandom::fill(&mut seed)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;

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
        .map_err(|error| ApiError::internal_with("match application operation", error))?;

    let projection = projection_for_participant(&state.database, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok(no_store_json(StatusCode::CREATED, projection))
}

async fn initialize_persisted_game(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &StoredRoomActor,
    content: &SelectedContent,
) -> Result<(InitialGameState, Vec<StoredRoomParticipant>), ApiError> {
    let stored_participants = postgres::room_participants(transaction, actor.room_id).await?;
    let participants = stored_participants
        .iter()
        .map(domain_participant)
        .collect::<Result<Vec<_>, _>>()?;
    let state = initialize_game(StartGameInput {
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
    Ok((state, stored_participants))
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
    let request_json = serde_json::to_vec(&request)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    let payload_digest = format!("blake3:{}", blake3::hash(&request_json).to_hex());
    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
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
            .map_err(|error| ApiError::internal_with("match application operation", error))?;
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

    let persisted = decode_persisted_snapshot(&stored.snapshot_json)?;
    verify_persisted_snapshot(&stored, &persisted)?;
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
    let snapshot_json = serde_json::to_string(&next_snapshot)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    let state_digest = format!("blake3:{}", blake3::hash(snapshot_json.as_bytes()).to_hex());
    let (event_type, event_json) = persisted_event(decision.event)?;
    let receipt = postgres::persist_game_command(
        &mut transaction,
        postgres::NewGameCommand {
            game_id: stored.id,
            actor_participant_id: participant_id,
            command_id,
            expected_state_version: request.expected_state_version,
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
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    state.signal_game_synchronization(stored.id);

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

fn participant_role(role: &str) -> Result<ParticipantRole, ApiError> {
    match role {
        "host" => Ok(ParticipantRole::Host),
        "guest" => Ok(ParticipantRole::Guest),
        _ => Err(ApiError::internal()),
    }
}

fn hero_id(hero: &str) -> Result<HeroId, ApiError> {
    hero.parse()
        .map_err(|error| ApiError::internal_with("match application operation", error))
}

fn hero_name(hero: &str) -> Result<&'static str, ApiError> {
    hero_id(hero).map(HeroId::name)
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
