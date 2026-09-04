use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
    routing::{get, post},
};
use game_domain::{
    ContentSelection, EffectDie, EffectRoller, GameCommand, GameCommandError, GameCommandInput,
    GameEngine, GameIntentError, GameIntentInput, HeroId, InitialGameState, LobbyParticipant,
    ParticipantRole, PlayerIntent, StartGameError, StartGameInput, ValidatedGameRules,
    decide_game_command,
};
use rand_chacha::{
    ChaCha20Rng,
    rand_core::{Rng, SeedableRng},
};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use sqlx::FromRow;
use std::collections::BTreeMap;
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
    persisted_snapshot, validate_persisted_json_size, verify_persisted_snapshot,
};
pub(crate) use projection::{GameProjectionResponse, projection_for_participant};

const SEED_BYTES: usize = 32;
const GAME_EVENT_VERSION: u16 = 4;

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

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ExecuteGameCommandRequest {
    EndHeroActions {
        command_id: String,
        #[serde(deserialize_with = "positive_state_version")]
        expected_state_version: u64,
    },
    ResolveChoice {
        command_id: String,
        #[serde(deserialize_with = "positive_state_version")]
        expected_state_version: u64,
        #[serde(deserialize_with = "bounded_choice_string")]
        choice_id: String,
        #[serde(deserialize_with = "bounded_choice_selection")]
        selected_options: Vec<String>,
    },
    PlayCard {
        command_id: String,
        #[serde(deserialize_with = "positive_state_version")]
        expected_state_version: u64,
        card_id: String,
        targets: Vec<RequestedTargetBinding>,
    },
    AssignAttack {
        command_id: String,
        #[serde(deserialize_with = "positive_state_version")]
        expected_state_version: u64,
        villain_id: String,
        amount: u16,
    },
    AcquireCard {
        command_id: String,
        #[serde(deserialize_with = "positive_state_version")]
        expected_state_version: u64,
        card_id: String,
    },
}

impl ExecuteGameCommandRequest {
    fn command_id(&self) -> &str {
        match self {
            Self::EndHeroActions { command_id, .. }
            | Self::ResolveChoice { command_id, .. }
            | Self::PlayCard { command_id, .. }
            | Self::AssignAttack { command_id, .. }
            | Self::AcquireCard { command_id, .. } => command_id,
        }
    }

    const fn expected_state_version(&self) -> u64 {
        match self {
            Self::EndHeroActions {
                expected_state_version,
                ..
            }
            | Self::ResolveChoice {
                expected_state_version,
                ..
            }
            | Self::PlayCard {
                expected_state_version,
                ..
            }
            | Self::AssignAttack {
                expected_state_version,
                ..
            }
            | Self::AcquireCard {
                expected_state_version,
                ..
            } => *expected_state_version,
        }
    }

    const fn command_type(&self) -> PersistedGameCommandType {
        match self {
            Self::EndHeroActions { .. } => PersistedGameCommandType::EndHeroActions,
            Self::ResolveChoice { .. } => PersistedGameCommandType::ResolveChoice,
            Self::PlayCard { .. } => PersistedGameCommandType::PlayCard,
            Self::AssignAttack { .. } => PersistedGameCommandType::AssignAttack,
            Self::AcquireCard { .. } => PersistedGameCommandType::AcquireCard,
        }
    }

    fn player_intent(&self) -> Option<PlayerIntent> {
        match self {
            Self::EndHeroActions { .. } => Some(PlayerIntent::EndHeroActions),
            Self::ResolveChoice {
                choice_id,
                selected_options,
                ..
            } => Some(PlayerIntent::ResolveChoice {
                choice_id: choice_id.clone(),
                selected_options: selected_options.clone(),
            }),
            Self::PlayCard { .. } | Self::AssignAttack { .. } | Self::AcquireCard { .. } => None,
        }
    }

    fn game_command(&self) -> Option<GameCommand> {
        match self {
            Self::PlayCard {
                card_id, targets, ..
            } => Some(GameCommand::PlayCard {
                card_id: card_id.clone(),
                targets: targets
                    .iter()
                    .map(|target| game_domain::EffectTargetBinding {
                        selector_id: target.selector_id.clone(),
                        target_ids: target.target_ids.clone(),
                    })
                    .collect(),
            }),
            Self::AssignAttack {
                villain_id, amount, ..
            } => Some(GameCommand::AssignAttack {
                villain_id: villain_id.clone(),
                amount: *amount,
            }),
            Self::AcquireCard { card_id, .. } => Some(GameCommand::AcquireCard {
                card_id: card_id.clone(),
            }),
            Self::EndHeroActions { .. } | Self::ResolveChoice { .. } => None,
        }
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        match self {
            Self::EndHeroActions {
                command_id,
                expected_state_version,
            } => serde_json::to_vec(&CanonicalEndHeroActionsCommandRequest {
                command_id,
                expected_state_version: *expected_state_version,
                command_type: "end_hero_actions",
            }),
            Self::ResolveChoice {
                command_id,
                expected_state_version,
                choice_id,
                selected_options,
            } => serde_json::to_vec(&CanonicalResolveChoiceCommandRequest {
                command_id,
                expected_state_version: *expected_state_version,
                command_type: "resolve_choice",
                choice_id,
                selected_options,
            }),
            Self::PlayCard {
                command_id,
                expected_state_version,
                card_id,
                targets,
            } => serde_json::to_vec(&CanonicalPlayCardCommandRequest {
                command_id,
                expected_state_version: *expected_state_version,
                command_type: "play_card",
                card_id,
                targets,
            }),
            Self::AssignAttack {
                command_id,
                expected_state_version,
                villain_id,
                amount,
            } => serde_json::to_vec(&CanonicalAssignAttackCommandRequest {
                command_id,
                expected_state_version: *expected_state_version,
                command_type: "assign_attack",
                villain_id,
                amount: *amount,
            }),
            Self::AcquireCard {
                command_id,
                expected_state_version,
                card_id,
            } => serde_json::to_vec(&CanonicalAcquireCardCommandRequest {
                command_id,
                expected_state_version: *expected_state_version,
                command_type: "acquire_card",
                card_id,
            }),
        }
    }
}

#[derive(Serialize)]
struct CanonicalEndHeroActionsCommandRequest<'a> {
    command_id: &'a str,
    expected_state_version: u64,
    #[serde(rename = "type")]
    command_type: &'static str,
}

#[derive(Serialize)]
struct CanonicalResolveChoiceCommandRequest<'a> {
    command_id: &'a str,
    expected_state_version: u64,
    #[serde(rename = "type")]
    command_type: &'static str,
    choice_id: &'a str,
    selected_options: &'a [String],
}

#[derive(Serialize)]
struct CanonicalPlayCardCommandRequest<'a> {
    command_id: &'a str,
    expected_state_version: u64,
    #[serde(rename = "type")]
    command_type: &'static str,
    card_id: &'a str,
    targets: &'a [RequestedTargetBinding],
}

#[derive(Serialize)]
struct CanonicalAssignAttackCommandRequest<'a> {
    command_id: &'a str,
    expected_state_version: u64,
    #[serde(rename = "type")]
    command_type: &'static str,
    villain_id: &'a str,
    amount: u16,
}

#[derive(Serialize)]
struct CanonicalAcquireCardCommandRequest<'a> {
    command_id: &'a str,
    expected_state_version: u64,
    #[serde(rename = "type")]
    command_type: &'static str,
    card_id: &'a str,
}

fn positive_state_version<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    let version = u64::deserialize(deserializer)?;
    (version > 0)
        .then_some(version)
        .ok_or_else(|| D::Error::custom("expected_state_version must be positive"))
}

fn bounded_choice_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    let length = value.chars().count();
    (1..=256)
        .contains(&length)
        .then_some(value)
        .ok_or_else(|| D::Error::custom("choice values must contain between 1 and 256 characters"))
}

fn bounded_choice_selection<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let selected = Vec::<String>::deserialize(deserializer)?;
    let unique = selected.iter().collect::<std::collections::BTreeSet<_>>();
    let valid = selected.len() <= 32
        && unique.len() == selected.len()
        && selected
            .iter()
            .all(|option| (1..=256).contains(&option.chars().count()));
    valid
        .then_some(selected)
        .ok_or_else(|| D::Error::custom("selected_options must satisfy the public contract"))
}

fn command_payload_digest(request: &ExecuteGameCommandRequest) -> Result<String, ApiError> {
    let canonical = request
        .canonical_bytes()
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    Ok(format!("blake3:{}", blake3::hash(&canonical).to_hex()))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RequestedTargetBinding {
    selector_id: String,
    target_ids: Vec<String>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PersistedGameCommandType {
    EndHeroActions,
    ResolveChoice,
    PlayCard,
    AssignAttack,
    AcquireCard,
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
    prng_seed: Vec<u8>,
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
    command_type: PersistedGameCommandType,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queued_phases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queued_effects: Option<Vec<PersistedQueuedEffect>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision_point: Option<PersistedDecisionPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_turn_steps: Option<Vec<PersistedTurnStep>>,
    participants: Vec<PersistedPlayer>,
    prng: PersistedPrng,
    #[serde(default, skip_serializing_if = "PersistedEffects::is_empty")]
    effects: PersistedEffects,
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

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEffects {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    entities: Vec<PersistedEffectEntity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    outcomes: Vec<PersistedEffectOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    choice: Option<PersistedEffectChoice>,
}

impl PersistedEffects {
    fn is_empty(&self) -> bool {
        self.entities.is_empty() && self.outcomes.is_empty() && self.choice.is_none()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEffectEntity {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalog_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_position: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effect_rule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    influence_cost: Option<u16>,
    zone: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    zone_index: Option<u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    resources: BTreeMap<String, u16>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PersistedEffectOutcome {
    DieRolled {
        rule_id: String,
        die: String,
        result: u8,
    },
    Moved {
        rule_id: String,
        target_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_position: Option<u8>,
        from: String,
        to: String,
    },
    NoOp {
        rule_id: String,
        reason: String,
    },
    ResourceChanged {
        rule_id: String,
        target_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_position: Option<u8>,
        resource: String,
        before: u16,
        after: u16,
        cause: String,
    },
    Terminal {
        rule_id: String,
        outcome: String,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEffectChoice {
    id: String,
    cause: String,
    responsible_position: u8,
    kind: String,
    options: Vec<String>,
    min: u16,
    max: u16,
    continuation: PersistedEffectContinuation,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEffectContinuation {
    choice_cursor: PersistedEffectCursor,
    queue: Vec<PersistedQueuedEffect>,
    steps_completed: usize,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEffectCursor {
    rule_id: String,
    path: Vec<PersistedEffectPathSegment>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PersistedEffectPathSegment {
    ChoiceOption { index: u16 },
    ConditionThen,
    ConditionOtherwise,
    RepeatEffect,
    RollOutcome { index: u16 },
    SequenceEffect { index: u16 },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PersistedQueuedEffect {
    Definition {
        cursor: PersistedEffectCursor,
        actor_position: u8,
    },
    EffectChoice {
        cursor: PersistedEffectCursor,
        responsible_position: u8,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedLegacyEffectChoice {
    id: String,
    responsible_position: u8,
    kind: String,
    options: Vec<String>,
    min: u16,
    max: u16,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PersistedEventChoice {
    Current(PersistedEffectChoice),
    Legacy(PersistedLegacyEffectChoice),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PersistedDecisionPoint {
    None,
    Automatic,
    PlayerIntent { responsible_position: u8 },
    EffectChoice { choice: PersistedEffectChoice },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedTurnStep {
    phase: String,
    effects: Vec<PersistedEffectOutcome>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum PersistedEndTurnOutcome {
    CardMoved {
        card_id: String,
        from: String,
        to: String,
    },
    PileShuffled {
        owner_position: u8,
        zone: String,
        bottom_to_top: Vec<String>,
    },
    ResourceReset {
        resource: String,
        before: u16,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEngineControl {
    status: String,
    turn: u32,
    phase: String,
    active_position: u8,
    queued_phases: Vec<String>,
    queued_effects: Vec<PersistedQueuedEffect>,
    decision_point: PersistedDecisionPoint,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedEffectTargetBinding {
    selector_id: String,
    target_ids: Vec<String>,
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
    #[serde(default)]
    card_id: Option<String>,
    #[serde(default)]
    targets: Vec<PersistedEffectTargetBinding>,
    #[serde(default)]
    villain_id: Option<String>,
    #[serde(default)]
    amount: Option<u16>,
    #[serde(default)]
    cost: Option<u16>,
    #[serde(default)]
    refill_card_id: Option<String>,
    #[serde(default)]
    effects: Vec<PersistedEffectOutcome>,
    #[serde(default)]
    effect_stop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    choice: Option<PersistedEventChoice>,
    #[serde(default)]
    choice_id: Option<String>,
    #[serde(default)]
    choice_cause: Option<String>,
    #[serde(default)]
    selected_options: Option<Vec<String>>,
    #[serde(default)]
    end_turn: Option<Vec<PersistedEndTurnOutcome>>,
    #[serde(default)]
    steps: Option<Vec<PersistedTurnStep>>,
    #[serde(default)]
    control: Option<PersistedEngineControl>,
    #[serde(default)]
    prng_counter: Option<u64>,
}

struct ChaChaEffectRoller {
    seed: [u8; SEED_BYTES],
    stream: u64,
}

impl ChaChaEffectRoller {
    fn new(seed: &[u8], stream: u64) -> Result<Self, ApiError> {
        let seed = seed
            .try_into()
            .map_err(|error| ApiError::internal_with("match application operation", error))?;
        Ok(Self { seed, stream })
    }
}

impl EffectRoller for ChaChaEffectRoller {
    fn roll(&mut self, die: EffectDie) -> Option<u8> {
        let sample = self.sample_below(u32::from(die.sides()))?;
        u8::try_from(sample.checked_add(1)?).ok()
    }

    fn sample_below(&mut self, upper_exclusive: u32) -> Option<u32> {
        if upper_exclusive == 0 {
            return None;
        }
        let mut generator = ChaCha20Rng::from_seed(self.seed);
        generator.set_stream(self.stream);
        self.stream = self.stream.checked_add(1)?;
        let unbiased_range = u32::MAX - (u32::MAX % upper_exclusive);
        loop {
            let value = generator.next_u32();
            if value < unbiased_range {
                return Some(value % upper_exclusive);
            }
        }
    }
}

async fn start_game(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StartGameRequest>,
) -> Result<Response, ApiError> {
    let key = idempotency_key(&headers)?;
    let participant_id = authenticated_participant(&state, &headers).await?;

    if let Some(stored) = postgres::load_game_start(&state.database, &key).await? {
        return replay_game_start(&state, stored, participant_id, &request).await;
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
        return replay_game_start(&state, stored, participant_id, &request).await;
    }
    if actor.room_status != "open" {
        return Err(ApiError::room_sealed());
    }

    let effect_rules = state
        .content
        .effect_rules(&content.manifest_digest)
        .ok_or_else(ApiError::internal)?;
    let rules = ValidatedGameRules::new(effect_rules).map_err(|_| ApiError::internal())?;
    let mut seed = [0_u8; SEED_BYTES];
    getrandom::fill(&mut seed)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    let mut random = ChaChaEffectRoller::new(&seed, 0)?;
    let (initial_state, stored_participants) =
        initialize_persisted_game(&mut transaction, &actor, &content, &rules, &mut random).await?;

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
        return replay_game_start(&state, stored, participant_id, &request).await;
    }

    let snapshot = persisted_snapshot(&initial_state, &stored_participants);
    let snapshot_json = serde_json::to_string(&snapshot)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    validate_persisted_json_size(&snapshot_json)?;
    let state_digest = format!("blake3:{}", blake3::hash(snapshot_json.as_bytes()).to_hex());

    postgres::persist_game(
        &mut transaction,
        postgres::NewGame {
            id: game_id,
            actor: &actor,
            content: &content,
            state: &initial_state,
            snapshot_version: snapshot.snapshot_version,
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

    let projection = projection_for_participant(&state, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok(no_store_json(StatusCode::CREATED, projection))
}

async fn initialize_persisted_game(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &StoredRoomActor,
    content: &SelectedContent,
    rules: &ValidatedGameRules,
    random: &mut ChaChaEffectRoller,
) -> Result<(InitialGameState, Vec<StoredRoomParticipant>), ApiError> {
    let stored_participants = postgres::room_participants(transaction, actor.room_id).await?;
    let participants = stored_participants
        .iter()
        .map(domain_participant)
        .collect::<Result<Vec<_>, _>>()?;
    let participant_positions = participants
        .iter()
        .map(|participant| participant.position)
        .collect::<Vec<_>>();
    let initial_entities = content.initial_entities(&participant_positions);
    let state = GameEngine::new(rules)
        .start(
            StartGameInput {
                actor_role: participant_role(&actor.role)?,
                participants: &participants,
                content: ContentSelection {
                    adventure_id: &content.adventure_id,
                    content_version: &content.content_version,
                    ruleset_version: &content.ruleset_version,
                    manifest_digest: &content.manifest_digest,
                    manifest_version: content.manifest_version,
                    playable: content.playable,
                    initial_entities: &initial_entities,
                },
            },
            random,
        )
        .map_err(start_error)?;
    Ok((state, stored_participants))
}

async fn replay_game_start(
    state: &AppState,
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

    let projection = projection_for_participant(state, participant_id)
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
        Uuid::parse_str(request.command_id()).map_err(|_| ApiError::invalid_command_id())?;
    let payload_digest = command_payload_digest(&request)?;
    let expected_state_version = request.expected_state_version();
    let command_type = request.command_type();
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
        let projection = projection_for_participant(&state, participant_id)
            .await?
            .ok_or_else(ApiError::internal)?;
        return Ok(no_store_json(
            StatusCode::OK,
            ExecuteGameCommandResponse {
                receipt: receipt_response(receipt)?,
                projection,
            },
        ));
    }

    let persisted = decode_persisted_snapshot(&stored.snapshot_json)?;
    verify_persisted_snapshot(&stored, &persisted)?;
    let current = command_domain_state(&persisted)?;
    let decision = decide_player_intent(&state, &stored, &current, &request)?;

    let next_snapshot = persisted_after_decision(&persisted, &decision.state);
    let snapshot_json = serde_json::to_string(&next_snapshot)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    validate_persisted_json_size(&snapshot_json)?;
    let state_digest = format!("blake3:{}", blake3::hash(snapshot_json.as_bytes()).to_hex());
    let (event_version, event_type, event_json) = persisted_event(decision.event)?;
    let receipt = postgres::persist_game_command(
        &mut transaction,
        postgres::NewGameCommand {
            game_id: stored.id,
            actor_participant_id: participant_id,
            command_id,
            expected_state_version,
            command_type: command_type_name(command_type),
            payload_digest: &payload_digest,
            state: &decision.state,
            snapshot_version: next_snapshot.snapshot_version,
            state_digest: &state_digest,
            snapshot_json: &snapshot_json,
            event_version,
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

    let projection = projection_for_participant(&state, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;
    Ok(no_store_json(
        StatusCode::OK,
        ExecuteGameCommandResponse {
            receipt: receipt_response(receipt)?,
            projection,
        },
    ))
}

fn decide_player_intent(
    state: &AppState,
    stored: &StoredCommandGame,
    current: &InitialGameState,
    request: &ExecuteGameCommandRequest,
) -> Result<game_domain::GameIntentDecision, ApiError> {
    let effect_rules = state
        .content
        .effect_rules(&stored.manifest_digest)
        .ok_or_else(ApiError::internal)?;
    let actor_position =
        u8::try_from(stored.actor_position).map_err(|_| ApiError::game_action_not_allowed())?;
    let mut random = ChaChaEffectRoller::new(&stored.prng_seed, current.prng_counter())?;
    if let Some(intent) = request.player_intent() {
        let rules = ValidatedGameRules::new(effect_rules).map_err(|_| ApiError::internal())?;
        return GameEngine::new(&rules)
            .decide(
                GameIntentInput {
                    state: current,
                    actor_position,
                    expected_state_version: request.expected_state_version(),
                    intent,
                },
                &mut random,
            )
            .map_err(command_error);
    }

    let command = request
        .game_command()
        .ok_or_else(ApiError::game_action_not_allowed)?;
    let decision = decide_game_command(GameCommandInput {
        state: current,
        actor_position,
        expected_state_version: request.expected_state_version(),
        command,
        effect_rules: &effect_rules,
        die_roller: &mut random,
    })
    .map_err(action_command_error)?;
    Ok(game_domain::GameIntentDecision {
        state: decision.state,
        event: decision.event,
    })
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
    let projection = projection_for_participant(&state, participant_id)
        .await?
        .ok_or_else(ApiError::internal)?;

    Ok(no_store_json(
        StatusCode::OK,
        ExecuteGameCommandResponse {
            receipt: receipt_response(receipt)?,
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
        StartGameError::ContentNotPlayable
        | StartGameError::InvalidContentIdentity
        | StartGameError::InvalidInitialEntities => ApiError::content_not_playable(),
        StartGameError::EffectExecutionFailed => ApiError::internal(),
    }
}

fn command_error(error: GameIntentError) -> ApiError {
    match error {
        GameIntentError::ActorNotChoiceResponsible => ApiError::choice_not_assigned(),
        GameIntentError::StaleStateVersion => ApiError::stale_state_version(),
        GameIntentError::ActorNotResponsible | GameIntentError::IntentNotLegal => {
            ApiError::game_action_not_allowed()
        }
        GameIntentError::EffectExecutionFailed
        | GameIntentError::RandomSourceFailed
        | GameIntentError::VersionOverflow => ApiError::internal(),
    }
}

fn action_command_error(error: GameCommandError) -> ApiError {
    match error {
        GameCommandError::ActorNotChoiceResponsible => ApiError::choice_not_assigned(),
        GameCommandError::StaleStateVersion => ApiError::stale_state_version(),
        GameCommandError::ActorNotActive | GameCommandError::CommandNotLegal => {
            ApiError::game_action_not_allowed()
        }
        GameCommandError::EffectExecutionFailed | GameCommandError::VersionOverflow => {
            ApiError::internal()
        }
    }
}

const fn command_type_name(command_type: PersistedGameCommandType) -> &'static str {
    match command_type {
        PersistedGameCommandType::EndHeroActions => "end_hero_actions",
        PersistedGameCommandType::ResolveChoice => "resolve_choice",
        PersistedGameCommandType::PlayCard => "play_card",
        PersistedGameCommandType::AssignAttack => "assign_attack",
        PersistedGameCommandType::AcquireCard => "acquire_card",
    }
}

fn receipt_response(receipt: StoredCommandReceipt) -> Result<GameCommandReceipt, ApiError> {
    let command_type = match receipt.command_type.as_str() {
        "end_hero_actions" => PersistedGameCommandType::EndHeroActions,
        "resolve_choice" => PersistedGameCommandType::ResolveChoice,
        "play_card" => PersistedGameCommandType::PlayCard,
        "assign_attack" => PersistedGameCommandType::AssignAttack,
        "acquire_card" => PersistedGameCommandType::AcquireCard,
        _ => return Err(ApiError::command_not_found()),
    };
    Ok(GameCommandReceipt {
        command_id: receipt.command_id.to_string(),
        command_type,
        status: "accepted",
        expected_state_version: receipt.expected_state_version,
        accepted_state_version: receipt.accepted_state_version,
        accepted_sequence: receipt.accepted_sequence,
        expires_at: receipt.expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_receipt(command_type: &str) -> StoredCommandReceipt {
        StoredCommandReceipt {
            command_id: Uuid::nil(),
            actor_participant_id: Uuid::nil(),
            command_type: command_type.to_owned(),
            expected_state_version: 1,
            payload_digest:
                "blake3:0000000000000000000000000000000000000000000000000000000000000000".to_owned(),
            accepted_state_version: 2,
            accepted_sequence: 1,
            expires_at: "2026-09-10T12:00:00Z".to_owned(),
        }
    }

    fn roller(seed: &[u8; SEED_BYTES], stream: u64) -> ChaChaEffectRoller {
        ChaChaEffectRoller::new(seed, stream).unwrap_or_else(|_| panic!("seed should be valid"))
    }

    #[test]
    fn chacha_sampling_consumes_exactly_one_stream_per_valid_request() {
        let seed = [42_u8; SEED_BYTES];
        let mut continuous = roller(&seed, 0);

        let first = continuous
            .sample_below(7)
            .expect("a positive range should produce a sample");
        let second = continuous
            .sample_below(37)
            .expect("a positive range should produce a sample");
        let mut resumed = roller(&seed, 1);

        assert!(first < 7);
        assert_eq!(
            second,
            resumed
                .sample_below(37)
                .expect("the resumed stream should produce the next sample")
        );
    }

    #[test]
    fn die_roll_and_invalid_range_preserve_stream_semantics() {
        let seed = [91_u8; SEED_BYTES];
        let mut continuous = roller(&seed, 0);

        assert!(continuous.sample_below(0).is_none());
        let roll = continuous.roll(EffectDie::D6).expect("a die should roll");
        let after_roll = continuous
            .sample_below(19)
            .expect("the next stream should produce a sample");
        let mut resumed = roller(&seed, 1);

        assert!((1..=6).contains(&roll));
        assert_eq!(
            after_roll,
            resumed
                .sample_below(19)
                .expect("one roll should consume one stream")
        );
    }

    #[test]
    fn command_receipts_never_expose_a_legacy_command_type() {
        assert!(receipt_response(stored_receipt("complete_dark_arts")).is_err());

        let Ok(receipt) = receipt_response(stored_receipt("end_hero_actions")) else {
            panic!("the current receipt type should be supported");
        };
        let serialized = serde_json::to_value(receipt).expect("the receipt should serialize");
        assert_eq!(serialized["type"], "end_hero_actions");

        let Ok(receipt) = receipt_response(stored_receipt("resolve_choice")) else {
            panic!("the published choice command type should be supported");
        };
        let serialized = serde_json::to_value(receipt).expect("the receipt should serialize");
        assert_eq!(serialized["type"], "resolve_choice");
    }

    #[test]
    fn command_digest_bytes_are_canonical_for_every_current_command_type() {
        let end_turn = ExecuteGameCommandRequest::EndHeroActions {
            command_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            expected_state_version: 7,
        };
        assert_eq!(
            end_turn.canonical_bytes().expect("end turn must serialize"),
            br#"{"command_id":"00000000-0000-0000-0000-000000000001","expected_state_version":7,"type":"end_hero_actions"}"#
        );

        let resolve = ExecuteGameCommandRequest::ResolveChoice {
            command_id: "00000000-0000-0000-0000-000000000002".to_owned(),
            expected_state_version: 8,
            choice_id: "choice:1".to_owned(),
            selected_options: vec!["option:2".to_owned()],
        };
        assert_eq!(
            resolve.canonical_bytes().expect("resolve must serialize"),
            br#"{"command_id":"00000000-0000-0000-0000-000000000002","expected_state_version":8,"type":"resolve_choice","choice_id":"choice:1","selected_options":["option:2"]}"#
        );

        let play_card = ExecuteGameCommandRequest::PlayCard {
            command_id: "00000000-0000-0000-0000-000000000003".to_owned(),
            expected_state_version: 9,
            card_id: "card:1".to_owned(),
            targets: vec![RequestedTargetBinding {
                selector_id: "target:1".to_owned(),
                target_ids: vec!["hero:2".to_owned()],
            }],
        };
        assert_eq!(
            play_card.canonical_bytes().expect("play must serialize"),
            br#"{"command_id":"00000000-0000-0000-0000-000000000003","expected_state_version":9,"type":"play_card","card_id":"card:1","targets":[{"selector_id":"target:1","target_ids":["hero:2"]}]}"#
        );

        let assign_attack = ExecuteGameCommandRequest::AssignAttack {
            command_id: "00000000-0000-0000-0000-000000000004".to_owned(),
            expected_state_version: 10,
            villain_id: "villain:1".to_owned(),
            amount: 2,
        };
        assert_eq!(
            assign_attack
                .canonical_bytes()
                .expect("attack must serialize"),
            br#"{"command_id":"00000000-0000-0000-0000-000000000004","expected_state_version":10,"type":"assign_attack","villain_id":"villain:1","amount":2}"#
        );

        let acquire_card = ExecuteGameCommandRequest::AcquireCard {
            command_id: "00000000-0000-0000-0000-000000000005".to_owned(),
            expected_state_version: 11,
            card_id: "market:1".to_owned(),
        };
        assert_eq!(
            acquire_card
                .canonical_bytes()
                .expect("acquisition must serialize"),
            br#"{"command_id":"00000000-0000-0000-0000-000000000005","expected_state_version":11,"type":"acquire_card","card_id":"market:1"}"#
        );
    }
}
