use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::post,
};
use game_content::{ContentManifest, EntryKind};
use game_domain::{
    ContentSelection, GamePhase, GameStatus, HeroId, InitialGameState, LobbyParticipant,
    ParticipantRole, StartGameError, StartGameInput, initialize_game,
};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{
    AppState,
    identity_access::{ApiError, authenticated_participant, idempotency_key, no_store_json},
};

mod postgres;

const SEED_BYTES: usize = 32;

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/api/games", post(start_game))
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
    shuffle_algorithm: String,
    sampling_algorithm: String,
}

#[derive(Serialize)]
pub(crate) struct GameProjectionResponse {
    game: GameSummary,
    snapshot: SnapshotSummary,
    turn: TurnSummary,
    participant: GameParticipant,
    participants: Vec<GameParticipant>,
    legal_actions: Vec<String>,
}

#[derive(Serialize)]
struct GameSummary {
    id: String,
    status: String,
    adventure: AdventureSummary,
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
    digest: String,
    versions: GameVersions,
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

#[derive(Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
struct PersistedVersions {
    content: String,
    ruleset: String,
    manifest: u16,
    manifest_digest: String,
    prng: String,
    shuffle: String,
    sampling: String,
}

#[derive(Serialize, Deserialize)]
struct PersistedTurn {
    number: u32,
    phase: String,
    active_position: u8,
}

#[derive(Serialize, Deserialize)]
struct PersistedPlayer {
    participant_id: String,
    position: u8,
    hero_id: String,
}

#[derive(Serialize, Deserialize)]
struct PersistedPrng {
    algorithm: String,
    counter: u64,
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
        && persisted.versions.sampling == game.sampling_algorithm;
    if !snapshot_metadata_matches {
        return Err(ApiError::internal());
    }
    let participants = postgres::game_participants(database, game.id).await?;
    let current = participants
        .iter()
        .find(|participant| participant.id == participant_id)
        .ok_or_else(ApiError::internal)?;

    Ok(Some(GameProjectionResponse {
        game: GameSummary {
            id: game.id.to_string(),
            status: game.status,
            adventure: AdventureSummary {
                id: game.adventure_id,
                name: game.adventure_name,
            },
        },
        snapshot: SnapshotSummary {
            snapshot_version: game.snapshot_version,
            state_version: game.state_version,
            sequence: game.sequence,
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
        legal_actions: Vec::new(),
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
            counter: 0,
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
