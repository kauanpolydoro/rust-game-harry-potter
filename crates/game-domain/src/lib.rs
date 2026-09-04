//! Pure game rules.
//!
//! Game decisions enter through typed inputs and leave as typed state without
//! depending on infrastructure, clocks, global randomness, or transport DTOs.

#![forbid(unsafe_code)]

use std::{collections::BTreeSet, str::FromStr};

mod effects;

pub use effects::{
    EffectChangeCause, EffectChoiceAudience, EffectCondition, EffectContinuation, EffectCursor,
    EffectDefinition, EffectDie, EffectEligibility, EffectEntity, EffectExecutionError,
    EffectGameOutcome, EffectNoOpReason, EffectOperation, EffectOutcome, EffectPathSegment,
    EffectResource, EffectResourceCost, EffectRoller, EffectRule, EffectSelector, EffectStop,
    EffectTargetOwner, EffectTrigger, EffectWorld, EffectZone, PendingEffectChoice,
    PendingEffectChoiceKind, QueuedEffect, effect_action_is_affordable,
};

pub const SNAPSHOT_VERSION: u16 = 2;
const LEGACY_SNAPSHOT_VERSION: u16 = 1;
pub const INITIAL_STATE_VERSION: u64 = 1;
pub const INITIAL_SEQUENCE: u64 = 0;
pub const PRNG_ALGORITHM: &str = "chacha20-v1";
pub const SHUFFLE_ALGORITHM: &str = "fisher-yates-v1";
pub const SAMPLING_ALGORITHM: &str = "rejection-sampling-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticipantRole {
    Host,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HeroId {
    Harry,
    Hermione,
    Neville,
    Ron,
}

impl HeroId {
    pub const ALL: [Self; 4] = [Self::Harry, Self::Hermione, Self::Neville, Self::Ron];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Harry => "harry",
            Self::Hermione => "hermione",
            Self::Neville => "neville",
            Self::Ron => "ron",
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Harry => "Harry",
            Self::Hermione => "Hermione",
            Self::Neville => "Neville",
            Self::Ron => "Ron",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseHeroIdError;

impl std::fmt::Display for ParseHeroIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("hero ID is not supported")
    }
}

impl std::error::Error for ParseHeroIdError {}

impl FromStr for HeroId {
    type Err = ParseHeroIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "harry" => Ok(Self::Harry),
            "hermione" => Ok(Self::Hermione),
            "neville" => Ok(Self::Neville),
            "ron" => Ok(Self::Ron),
            _ => Err(ParseHeroIdError),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LobbyParticipant {
    pub role: ParticipantRole,
    pub position: u8,
    pub hero: Option<HeroId>,
    pub ready: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct ContentSelection<'a> {
    pub adventure_id: &'a str,
    pub content_version: &'a str,
    pub ruleset_version: &'a str,
    pub manifest_digest: &'a str,
    pub manifest_version: u16,
    pub playable: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct StartGameInput<'a> {
    pub actor_role: ParticipantRole,
    pub participants: &'a [LobbyParticipant],
    pub content: ContentSelection<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialGameState {
    snapshot_version: u16,
    state_version: u64,
    sequence: u64,
    status: GameStatus,
    turn: u32,
    phase: GamePhase,
    active_position: u8,
    adventure_id: String,
    content_version: String,
    ruleset_version: String,
    manifest_digest: String,
    manifest_version: u16,
    prng_algorithm: &'static str,
    shuffle_algorithm: &'static str,
    sampling_algorithm: &'static str,
    prng_counter: u64,
    players: Vec<InitialPlayer>,
    effect_world: EffectWorld,
    last_effects: Vec<EffectOutcome>,
    pending_choice: Option<PendingEffectChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStatus {
    InProgress,
    Lost,
    Won,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase {
    DarkArts,
    HeroAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameCommand {
    CompleteDarkArts,
    ResolveChoice {
        choice_id: String,
        selected_options: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCommandType {
    CompleteDarkArts,
    ResolveChoice,
}

pub struct GameCommandInput<'a> {
    pub state: &'a InitialGameState,
    pub actor_position: u8,
    pub expected_state_version: u64,
    pub command: GameCommand,
    pub effect_rules: &'a [EffectRule],
    pub die_roller: &'a mut dyn EffectRoller,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameCommandDecision {
    pub state: InitialGameState,
    pub event: GameEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameEvent {
    DarkArtsCompleted {
        sequence: u64,
        state_version: u64,
        turn: u32,
        actor_position: u8,
        effects: Vec<EffectOutcome>,
        stop: EffectStop,
        prng_counter: u64,
    },
    ChoiceResolved {
        sequence: u64,
        state_version: u64,
        turn: u32,
        actor_position: u8,
        choice_id: String,
        choice_cause: String,
        selected_options: Vec<String>,
        effects: Vec<EffectOutcome>,
        stop: EffectStop,
        prng_counter: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCommandError {
    ActorNotChoiceResponsible,
    StaleStateVersion,
    ActorNotActive,
    CommandNotLegal,
    EffectExecutionFailed,
    VersionOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameEventError {
    ActorNotActive,
    ActorNotChoiceResponsible,
    EventNotApplicable,
    EffectTransitionInvalid,
    SequenceMismatch,
    StateVersionMismatch,
    TurnMismatch,
    VersionOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitialPlayer {
    position: u8,
    hero: HeroId,
}

impl InitialPlayer {
    #[must_use]
    pub const fn new(position: u8, hero: HeroId) -> Self {
        Self { position, hero }
    }

    #[must_use]
    pub const fn position(&self) -> u8 {
        self.position
    }

    #[must_use]
    pub const fn hero(&self) -> HeroId {
        self.hero
    }
}

pub struct GameStateRestoreInput<'a> {
    pub snapshot_version: u16,
    pub state_version: u64,
    pub sequence: u64,
    pub status: GameStatus,
    pub turn: u32,
    pub phase: GamePhase,
    pub active_position: u8,
    pub adventure_id: &'a str,
    pub content_version: &'a str,
    pub ruleset_version: &'a str,
    pub manifest_digest: &'a str,
    pub manifest_version: u16,
    pub prng_algorithm: &'a str,
    pub shuffle_algorithm: &'a str,
    pub sampling_algorithm: &'a str,
    pub prng_counter: u64,
    pub players: Vec<InitialPlayer>,
    pub effect_world: EffectWorld,
    pub last_effects: Vec<EffectOutcome>,
    pub pending_choice: Option<PendingEffectChoice>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStateRestoreError {
    UnsupportedSnapshotVersion,
    InvalidVersion,
    InvalidTurn,
    InvalidContentIdentity,
    UnsupportedAlgorithm,
    InvalidPlayers,
}

impl std::fmt::Display for GameStateRestoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedSnapshotVersion => "snapshot version is not supported",
            Self::InvalidVersion => "state and sequence versions are inconsistent",
            Self::InvalidTurn => "turn is invalid",
            Self::InvalidContentIdentity => "content identity is invalid",
            Self::UnsupportedAlgorithm => "persisted algorithm is not supported",
            Self::InvalidPlayers => "persisted players violate game invariants",
        })
    }
}

impl std::error::Error for GameStateRestoreError {}

impl InitialGameState {
    #[must_use]
    pub const fn snapshot_version(&self) -> u16 {
        self.snapshot_version
    }

    #[must_use]
    pub const fn state_version(&self) -> u64 {
        self.state_version
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn status(&self) -> GameStatus {
        self.status
    }

    #[must_use]
    pub const fn turn(&self) -> u32 {
        self.turn
    }

    #[must_use]
    pub const fn phase(&self) -> GamePhase {
        self.phase
    }

    #[must_use]
    pub const fn active_position(&self) -> u8 {
        self.active_position
    }

    #[must_use]
    pub fn adventure_id(&self) -> &str {
        &self.adventure_id
    }

    #[must_use]
    pub fn content_version(&self) -> &str {
        &self.content_version
    }

    #[must_use]
    pub fn ruleset_version(&self) -> &str {
        &self.ruleset_version
    }

    #[must_use]
    pub fn manifest_digest(&self) -> &str {
        &self.manifest_digest
    }

    #[must_use]
    pub const fn manifest_version(&self) -> u16 {
        self.manifest_version
    }

    #[must_use]
    pub const fn prng_algorithm(&self) -> &'static str {
        self.prng_algorithm
    }

    #[must_use]
    pub const fn shuffle_algorithm(&self) -> &'static str {
        self.shuffle_algorithm
    }

    #[must_use]
    pub const fn sampling_algorithm(&self) -> &'static str {
        self.sampling_algorithm
    }

    #[must_use]
    pub const fn prng_counter(&self) -> u64 {
        self.prng_counter
    }

    #[must_use]
    pub fn players(&self) -> &[InitialPlayer] {
        &self.players
    }

    #[must_use]
    pub const fn effect_world(&self) -> &EffectWorld {
        &self.effect_world
    }

    #[must_use]
    pub fn last_effects(&self) -> &[EffectOutcome] {
        &self.last_effects
    }

    #[must_use]
    pub const fn pending_choice(&self) -> Option<&PendingEffectChoice> {
        self.pending_choice.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartGameError {
    ActorNotHost,
    InvalidParticipantCount,
    InvalidHostCount,
    InvalidPositions,
    MissingHero,
    DuplicateHero,
    ParticipantNotReady,
    ContentNotPlayable,
    InvalidContentIdentity,
}

/// Validates a lobby and creates the deterministic, entropy-free part of its
/// first game snapshot.
///
/// Entropy is deliberately supplied and persisted by the application layer.
///
/// # Errors
///
/// Returns the first invariant that prevents the room from being sealed.
pub fn initialize_game(input: StartGameInput<'_>) -> Result<InitialGameState, StartGameError> {
    if input.actor_role != ParticipantRole::Host {
        return Err(StartGameError::ActorNotHost);
    }
    if !(2..=4).contains(&input.participants.len()) {
        return Err(StartGameError::InvalidParticipantCount);
    }
    if input
        .participants
        .iter()
        .filter(|participant| participant.role == ParticipantRole::Host)
        .count()
        != 1
    {
        return Err(StartGameError::InvalidHostCount);
    }

    let expected_positions = (1_u8..=4)
        .take(input.participants.len())
        .collect::<BTreeSet<_>>();
    let positions = input
        .participants
        .iter()
        .map(|participant| participant.position)
        .collect::<BTreeSet<_>>();
    if positions != expected_positions {
        return Err(StartGameError::InvalidPositions);
    }

    let heroes = input
        .participants
        .iter()
        .map(|participant| participant.hero.ok_or(StartGameError::MissingHero))
        .collect::<Result<Vec<_>, _>>()?;
    if heroes.iter().copied().collect::<BTreeSet<_>>().len() != heroes.len() {
        return Err(StartGameError::DuplicateHero);
    }
    if input
        .participants
        .iter()
        .any(|participant| !participant.ready)
    {
        return Err(StartGameError::ParticipantNotReady);
    }
    if !input.content.playable {
        return Err(StartGameError::ContentNotPlayable);
    }
    if input.content.adventure_id.is_empty()
        || input.content.content_version.is_empty()
        || input.content.ruleset_version.is_empty()
        || input.content.manifest_version == 0
        || !valid_blake3_digest(input.content.manifest_digest)
    {
        return Err(StartGameError::InvalidContentIdentity);
    }

    let mut players = input
        .participants
        .iter()
        .zip(heroes)
        .map(|(participant, hero)| InitialPlayer {
            position: participant.position,
            hero,
        })
        .collect::<Vec<_>>();
    players.sort_by_key(|player| player.position);

    Ok(InitialGameState {
        snapshot_version: SNAPSHOT_VERSION,
        state_version: INITIAL_STATE_VERSION,
        sequence: INITIAL_SEQUENCE,
        status: GameStatus::InProgress,
        turn: 1,
        phase: GamePhase::DarkArts,
        active_position: 1,
        adventure_id: input.content.adventure_id.to_owned(),
        content_version: input.content.content_version.to_owned(),
        ruleset_version: input.content.ruleset_version.to_owned(),
        manifest_digest: input.content.manifest_digest.to_owned(),
        manifest_version: input.content.manifest_version,
        prng_algorithm: PRNG_ALGORITHM,
        shuffle_algorithm: SHUFFLE_ALGORITHM,
        sampling_algorithm: SAMPLING_ALGORITHM,
        prng_counter: 0,
        effect_world: EffectWorld::new(
            players
                .iter()
                .map(|player| EffectEntity::hero(player.position))
                .collect(),
        ),
        last_effects: Vec::new(),
        pending_choice: None,
        players,
    })
}

/// Restores a persisted game only when every currently supported invariant is
/// represented by the supplied snapshot.
///
/// # Errors
///
/// Returns an error for an unsupported codec version, inconsistent counters,
/// invalid players, unknown algorithms, or incomplete content identity.
pub fn restore_game_state(
    input: GameStateRestoreInput<'_>,
) -> Result<InitialGameState, GameStateRestoreError> {
    let supported_snapshot = input.snapshot_version == SNAPSHOT_VERSION
        || (input.snapshot_version == LEGACY_SNAPSHOT_VERSION && input.pending_choice.is_none());
    if !supported_snapshot {
        return Err(GameStateRestoreError::UnsupportedSnapshotVersion);
    }
    if input.state_version == 0
        || input.sequence.checked_add(1) != Some(input.state_version)
        || input.manifest_version == 0
    {
        return Err(GameStateRestoreError::InvalidVersion);
    }
    if input.turn == 0 {
        return Err(GameStateRestoreError::InvalidTurn);
    }
    if input.adventure_id.is_empty()
        || input.content_version.is_empty()
        || input.ruleset_version.is_empty()
        || !valid_blake3_digest(input.manifest_digest)
    {
        return Err(GameStateRestoreError::InvalidContentIdentity);
    }
    if input.prng_algorithm != PRNG_ALGORITHM
        || input.shuffle_algorithm != SHUFFLE_ALGORITHM
        || input.sampling_algorithm != SAMPLING_ALGORITHM
    {
        return Err(GameStateRestoreError::UnsupportedAlgorithm);
    }
    let expected_positions = (1_u8..=4)
        .take(input.players.len())
        .collect::<BTreeSet<_>>();
    let positions = input
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<BTreeSet<_>>();
    let heroes = input
        .players
        .iter()
        .map(InitialPlayer::hero)
        .collect::<BTreeSet<_>>();
    let participant_positions = input
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    if !(2..=4).contains(&input.players.len())
        || positions != expected_positions
        || heroes.len() != input.players.len()
        || !positions.contains(&input.active_position)
        || !input
            .effect_world
            .is_valid_for_positions(&participant_positions)
        || input
            .pending_choice
            .as_ref()
            .is_some_and(|choice| !choice.is_valid_for_positions(&participant_positions))
        || (input.pending_choice.is_some()
            && (input.status != GameStatus::InProgress || input.phase != GamePhase::DarkArts))
    {
        return Err(GameStateRestoreError::InvalidPlayers);
    }

    Ok(InitialGameState {
        snapshot_version: SNAPSHOT_VERSION,
        state_version: input.state_version,
        sequence: input.sequence,
        status: input.status,
        turn: input.turn,
        phase: input.phase,
        active_position: input.active_position,
        adventure_id: input.adventure_id.to_owned(),
        content_version: input.content_version.to_owned(),
        ruleset_version: input.ruleset_version.to_owned(),
        manifest_digest: input.manifest_digest.to_owned(),
        manifest_version: input.manifest_version,
        prng_algorithm: PRNG_ALGORITHM,
        shuffle_algorithm: SHUFFLE_ALGORITHM,
        sampling_algorithm: SAMPLING_ALGORITHM,
        prng_counter: input.prng_counter,
        players: input.players,
        effect_world: input.effect_world,
        last_effects: input.last_effects,
        pending_choice: input.pending_choice,
    })
}

fn valid_blake3_digest(value: &str) -> bool {
    value.strip_prefix("blake3:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// Applies a game command without consulting infrastructure, clocks, or entropy.
///
/// The returned state and events form one stable decision that the application
/// layer can persist atomically.
///
/// # Errors
///
/// Returns an error when the observed version is stale, the actor is not
/// authorized for the current decision, or the command is not legal now.
pub fn decide_game_command(
    input: GameCommandInput<'_>,
) -> Result<GameCommandDecision, GameCommandError> {
    let GameCommandInput {
        state,
        actor_position,
        expected_state_version,
        command,
        effect_rules,
        die_roller,
    } = input;
    if expected_state_version != state.state_version {
        return Err(GameCommandError::StaleStateVersion);
    }
    match command {
        GameCommand::CompleteDarkArts => {
            decide_complete_dark_arts(state, actor_position, effect_rules, die_roller)
        }
        GameCommand::ResolveChoice {
            choice_id,
            selected_options,
        } => decide_choice_response(
            state,
            actor_position,
            choice_id,
            &selected_options,
            effect_rules,
            die_roller,
        ),
    }
}

fn decide_complete_dark_arts(
    state: &InitialGameState,
    actor_position: u8,
    effect_rules: &[EffectRule],
    die_roller: &mut dyn EffectRoller,
) -> Result<GameCommandDecision, GameCommandError> {
    if actor_position != state.active_position {
        return Err(GameCommandError::ActorNotActive);
    }
    if !legal_game_commands(state, actor_position, effect_rules)
        .contains(&GameCommandType::CompleteDarkArts)
    {
        return Err(GameCommandError::CommandNotLegal);
    }
    let (state_version, sequence) = next_event_versions(state)?;
    let mut effect_world = state.effect_world.clone();
    let resolution = effects::execute_effects(
        &mut effect_world,
        actor_position,
        effect_rules,
        EffectTrigger::DarkArtsCompleted,
        die_roller,
    )
    .map_err(game_command_effect_error)?;
    let prng_counter = next_prng_counter(state, resolution.rolls_consumed)?;
    finish_game_command(
        state,
        GameEvent::DarkArtsCompleted {
            sequence,
            state_version,
            turn: state.turn,
            actor_position,
            effects: resolution.outcomes,
            stop: resolution.stop,
            prng_counter,
        },
    )
}

fn decide_choice_response(
    state: &InitialGameState,
    actor_position: u8,
    choice_id: String,
    selected_options: &[String],
    effect_rules: &[EffectRule],
    die_roller: &mut dyn EffectRoller,
) -> Result<GameCommandDecision, GameCommandError> {
    let pending = state
        .pending_choice
        .as_ref()
        .ok_or(GameCommandError::CommandNotLegal)?;
    if actor_position != pending.responsible_position {
        return Err(GameCommandError::ActorNotChoiceResponsible);
    }
    if !legal_game_commands(state, actor_position, effect_rules)
        .contains(&GameCommandType::ResolveChoice)
        || choice_id != pending.id
    {
        return Err(GameCommandError::CommandNotLegal);
    }
    let selected_options = effects::normalize_effect_choice_selection(pending, selected_options)
        .ok_or(GameCommandError::CommandNotLegal)?;
    let (state_version, sequence) = next_event_versions(state)?;
    let mut effect_world = state.effect_world.clone();
    let resolution = effects::resume_effects(
        &mut effect_world,
        pending,
        &selected_options,
        effect_rules,
        die_roller,
    )
    .map_err(game_command_effect_error)?;
    let prng_counter = next_prng_counter(state, resolution.rolls_consumed)?;
    finish_game_command(
        state,
        GameEvent::ChoiceResolved {
            sequence,
            state_version,
            turn: state.turn,
            actor_position,
            choice_id,
            choice_cause: pending.cause.clone(),
            selected_options,
            effects: resolution.outcomes,
            stop: resolution.stop,
            prng_counter,
        },
    )
}

fn next_event_versions(state: &InitialGameState) -> Result<(u64, u64), GameCommandError> {
    let state_version = state
        .state_version
        .checked_add(1)
        .ok_or(GameCommandError::VersionOverflow)?;
    let sequence = state
        .sequence
        .checked_add(1)
        .ok_or(GameCommandError::VersionOverflow)?;
    Ok((state_version, sequence))
}

fn next_prng_counter(
    state: &InitialGameState,
    rolls_consumed: u64,
) -> Result<u64, GameCommandError> {
    state
        .prng_counter
        .checked_add(rolls_consumed)
        .ok_or(GameCommandError::VersionOverflow)
}

const fn game_command_effect_error(error: EffectExecutionError) -> GameCommandError {
    match error {
        EffectExecutionError::InvalidChoice | EffectExecutionError::UnaffordableCost => {
            GameCommandError::CommandNotLegal
        }
        EffectExecutionError::InvalidDefinition
        | EffectExecutionError::InvalidRoll
        | EffectExecutionError::StepLimitExceeded => GameCommandError::EffectExecutionFailed,
    }
}

fn finish_game_command(
    previous: &InitialGameState,
    event: GameEvent,
) -> Result<GameCommandDecision, GameCommandError> {
    let state = apply_game_event(previous, &event).map_err(|error| match error {
        GameEventError::VersionOverflow => GameCommandError::VersionOverflow,
        GameEventError::ActorNotChoiceResponsible => GameCommandError::ActorNotChoiceResponsible,
        GameEventError::ActorNotActive
        | GameEventError::EventNotApplicable
        | GameEventError::EffectTransitionInvalid
        | GameEventError::SequenceMismatch
        | GameEventError::StateVersionMismatch
        | GameEventError::TurnMismatch => GameCommandError::CommandNotLegal,
    })?;
    Ok(GameCommandDecision { state, event })
}

/// Returns the commands that the current game rules permit for one actor.
///
/// External gates such as database-clock expiration are applied by the
/// application before exposing this result.
#[must_use]
pub fn legal_game_commands(
    state: &InitialGameState,
    actor_position: u8,
    effect_rules: &[EffectRule],
) -> Vec<GameCommandType> {
    if state.status != GameStatus::InProgress {
        return Vec::new();
    }
    if let Some(choice) = &state.pending_choice {
        return if actor_position == choice.responsible_position {
            vec![GameCommandType::ResolveChoice]
        } else {
            Vec::new()
        };
    }
    if state.phase == GamePhase::DarkArts
        && state.pending_choice.is_none()
        && actor_position == state.active_position
        && effect_action_is_affordable(
            &state.effect_world,
            actor_position,
            effect_rules,
            EffectTrigger::DarkArtsCompleted,
        )
    {
        vec![GameCommandType::CompleteDarkArts]
    } else {
        Vec::new()
    }
}

/// Returns the participant who must make the current human decision.
///
/// Automatic resolution points expose no legal command and therefore do not
/// require a connected participant. Availability remains an application-layer
/// concern and must never change the rule decision itself.
#[must_use]
pub fn required_participant_for_decision(
    state: &InitialGameState,
    effect_rules: &[EffectRule],
) -> Option<u8> {
    if let Some(choice) = &state.pending_choice {
        return Some(choice.responsible_position);
    }
    state
        .players
        .iter()
        .map(InitialPlayer::position)
        .find(|position| !legal_game_commands(state, *position, effect_rules).is_empty())
}

/// Applies one official event to a game state using the same transition as a
/// live command decision.
///
/// # Errors
///
/// Returns an error when the event is not the exact next legal fact for the
/// supplied state.
pub fn apply_game_event(
    state: &InitialGameState,
    event: &GameEvent,
) -> Result<InitialGameState, GameEventError> {
    match event {
        GameEvent::DarkArtsCompleted {
            sequence,
            state_version,
            turn,
            actor_position,
            effects,
            stop,
            prng_counter,
        } => apply_dark_arts_completed(
            state,
            EffectEventTransition {
                sequence: *sequence,
                state_version: *state_version,
                turn: *turn,
                actor_position: *actor_position,
                effects,
                stop,
                prng_counter: *prng_counter,
            },
        ),
        GameEvent::ChoiceResolved {
            sequence,
            state_version,
            turn,
            actor_position,
            choice_id,
            choice_cause,
            selected_options,
            effects,
            stop,
            prng_counter,
        } => apply_choice_resolved(
            state,
            choice_id,
            choice_cause,
            selected_options,
            EffectEventTransition {
                sequence: *sequence,
                state_version: *state_version,
                turn: *turn,
                actor_position: *actor_position,
                effects,
                stop,
                prng_counter: *prng_counter,
            },
        ),
    }
}

#[derive(Clone, Copy)]
struct EffectEventTransition<'a> {
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
    effects: &'a [EffectOutcome],
    stop: &'a EffectStop,
    prng_counter: u64,
}

fn apply_dark_arts_completed(
    state: &InitialGameState,
    transition: EffectEventTransition<'_>,
) -> Result<InitialGameState, GameEventError> {
    if state.status != GameStatus::InProgress
        || state.phase != GamePhase::DarkArts
        || state.pending_choice.is_some()
    {
        return Err(GameEventError::EventNotApplicable);
    }
    if transition.actor_position != state.active_position {
        return Err(GameEventError::ActorNotActive);
    }
    apply_effect_event_transition(state, transition)
}

fn apply_choice_resolved(
    state: &InitialGameState,
    choice_id: &str,
    choice_cause: &str,
    selected_options: &[String],
    transition: EffectEventTransition<'_>,
) -> Result<InitialGameState, GameEventError> {
    if state.status != GameStatus::InProgress {
        return Err(GameEventError::EventNotApplicable);
    }
    let pending = state
        .pending_choice
        .as_ref()
        .ok_or(GameEventError::EventNotApplicable)?;
    if transition.actor_position != pending.responsible_position {
        return Err(GameEventError::ActorNotChoiceResponsible);
    }
    let normalized = effects::normalize_effect_choice_selection(pending, selected_options)
        .ok_or(GameEventError::EffectTransitionInvalid)?;
    if choice_id != pending.id || choice_cause != pending.cause || normalized != selected_options {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    apply_effect_event_transition(state, transition)
}

fn apply_effect_event_transition(
    state: &InitialGameState,
    transition: EffectEventTransition<'_>,
) -> Result<InitialGameState, GameEventError> {
    if transition.turn != state.turn {
        return Err(GameEventError::TurnMismatch);
    }
    let expected_sequence = state
        .sequence
        .checked_add(1)
        .ok_or(GameEventError::VersionOverflow)?;
    if transition.sequence != expected_sequence {
        return Err(GameEventError::SequenceMismatch);
    }
    let expected_state_version = state
        .state_version
        .checked_add(1)
        .ok_or(GameEventError::VersionOverflow)?;
    if transition.state_version != expected_state_version {
        return Err(GameEventError::StateVersionMismatch);
    }
    let participant_positions = state
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    if !effects::effect_transition_is_valid(
        transition.effects,
        transition.stop,
        &participant_positions,
    ) {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    let rolled = transition
        .effects
        .iter()
        .filter(|outcome| matches!(outcome, EffectOutcome::DieRolled { .. }))
        .count();
    let expected_prng_counter = state
        .prng_counter
        .checked_add(u64::try_from(rolled).map_err(|_| GameEventError::VersionOverflow)?)
        .ok_or(GameEventError::VersionOverflow)?;
    if transition.prng_counter != expected_prng_counter {
        return Err(GameEventError::EffectTransitionInvalid);
    }

    let mut next = state.clone();
    effects::apply_effect_outcomes(&mut next.effect_world, transition.effects)
        .map_err(|_| GameEventError::EffectTransitionInvalid)?;
    next.sequence = transition.sequence;
    next.state_version = transition.state_version;
    next.prng_counter = transition.prng_counter;
    next.last_effects = transition.effects.to_vec();
    apply_effect_stop(&mut next, transition.stop);
    Ok(next)
}

fn apply_effect_stop(state: &mut InitialGameState, stop: &EffectStop) {
    match stop {
        EffectStop::Choice(choice) => {
            state.pending_choice = Some(choice.clone());
        }
        EffectStop::Stable => {
            state.phase = GamePhase::HeroAction;
            state.pending_choice = None;
        }
        EffectStop::Terminal(outcome) => {
            state.status = match outcome {
                EffectGameOutcome::Lost => GameStatus::Lost,
                EffectGameOutcome::Won => GameStatus::Won,
            };
            state.pending_choice = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct ScriptedRoller {
        rolls: VecDeque<u8>,
    }

    impl ScriptedRoller {
        fn new(rolls: &[u8]) -> Self {
            Self {
                rolls: rolls.iter().copied().collect(),
            }
        }
    }

    impl EffectRoller for ScriptedRoller {
        fn roll(&mut self, _die: EffectDie) -> Option<u8> {
            self.rolls.pop_front()
        }
    }

    const CONTENT: ContentSelection<'static> = ContentSelection {
        adventure_id: "adventure:001",
        content_version: "fixture-v1",
        ruleset_version: "fixture-rules-v1",
        manifest_digest: "blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        manifest_version: 1,
        playable: true,
    };

    fn valid_participants() -> Vec<LobbyParticipant> {
        vec![
            LobbyParticipant {
                role: ParticipantRole::Host,
                position: 1,
                hero: Some(HeroId::Harry),
                ready: true,
            },
            LobbyParticipant {
                role: ParticipantRole::Guest,
                position: 2,
                hero: Some(HeroId::Hermione),
                ready: true,
            },
        ]
    }

    fn decide_without_rolls(
        state: &InitialGameState,
        actor_position: u8,
        command: GameCommand,
        effect_rules: &[EffectRule],
    ) -> Result<GameCommandDecision, GameCommandError> {
        let mut roller = ScriptedRoller::new(&[]);
        decide_game_command(GameCommandInput {
            state,
            actor_position,
            expected_state_version: state.state_version(),
            command,
            effect_rules,
            die_roller: &mut roller,
        })
    }

    fn restore_input(state: &InitialGameState, snapshot_version: u16) -> GameStateRestoreInput<'_> {
        GameStateRestoreInput {
            snapshot_version,
            state_version: state.state_version(),
            sequence: state.sequence(),
            status: state.status(),
            turn: state.turn(),
            phase: state.phase(),
            active_position: state.active_position(),
            adventure_id: state.adventure_id(),
            content_version: state.content_version(),
            ruleset_version: state.ruleset_version(),
            manifest_digest: state.manifest_digest(),
            manifest_version: state.manifest_version(),
            prng_algorithm: state.prng_algorithm(),
            shuffle_algorithm: state.shuffle_algorithm(),
            sampling_algorithm: state.sampling_algorithm(),
            prng_counter: state.prng_counter(),
            players: state.players().to_vec(),
            effect_world: state.effect_world().clone(),
            last_effects: state.last_effects().to_vec(),
            pending_choice: state.pending_choice().cloned(),
        }
    }

    #[test]
    fn valid_lobby_creates_the_versioned_initial_state() {
        let participants = valid_participants();
        let state = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");

        assert_eq!(state.snapshot_version, SNAPSHOT_VERSION);
        assert_eq!(state.state_version, 1);
        assert_eq!(state.sequence, 0);
        assert_eq!(state.turn, 1);
        assert_eq!(state.phase, GamePhase::DarkArts);
        assert_eq!(state.active_position, 1);
        assert_eq!(state.prng_algorithm, "chacha20-v1");
        assert_eq!(state.players.len(), 2);
    }

    #[test]
    fn only_the_host_can_start() {
        let participants = valid_participants();

        assert_eq!(
            initialize_game(StartGameInput {
                actor_role: ParticipantRole::Guest,
                participants: &participants,
                content: CONTENT,
            }),
            Err(StartGameError::ActorNotHost)
        );
    }

    #[test]
    fn participant_count_is_bounded() {
        let participants = &valid_participants()[..1];

        assert_eq!(
            initialize_game(StartGameInput {
                actor_role: ParticipantRole::Host,
                participants,
                content: CONTENT,
            }),
            Err(StartGameError::InvalidParticipantCount)
        );
    }

    #[test]
    fn every_participant_needs_a_unique_hero_and_readiness() {
        let baseline = valid_participants();
        let cases = [
            (
                vec![
                    baseline[0],
                    LobbyParticipant {
                        hero: None,
                        ..baseline[1]
                    },
                ],
                StartGameError::MissingHero,
            ),
            (
                vec![
                    baseline[0],
                    LobbyParticipant {
                        hero: Some(HeroId::Harry),
                        ..baseline[1]
                    },
                ],
                StartGameError::DuplicateHero,
            ),
            (
                vec![
                    baseline[0],
                    LobbyParticipant {
                        ready: false,
                        ..baseline[1]
                    },
                ],
                StartGameError::ParticipantNotReady,
            ),
        ];

        for (participants, expected) in cases {
            assert_eq!(
                initialize_game(StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                }),
                Err(expected)
            );
        }
    }

    #[test]
    fn unplayable_content_fails_closed() {
        let participants = valid_participants();

        assert_eq!(
            initialize_game(StartGameInput {
                actor_role: ParticipantRole::Host,
                participants: &participants,
                content: ContentSelection {
                    playable: false,
                    ..CONTENT
                },
            }),
            Err(StartGameError::ContentNotPlayable)
        );
    }

    #[test]
    fn malformed_manifest_digest_fails_closed() {
        let participants = valid_participants();

        assert_eq!(
            initialize_game(StartGameInput {
                actor_role: ParticipantRole::Host,
                participants: &participants,
                content: ContentSelection {
                    manifest_digest: "blake3:not-a-digest",
                    ..CONTENT
                },
            }),
            Err(StartGameError::InvalidContentIdentity)
        );
    }

    #[test]
    fn active_participant_completes_dark_arts_at_a_stable_decision_point() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");

        assert_eq!(
            legal_game_commands(&initial, 1, &[]),
            vec![GameCommandType::CompleteDarkArts]
        );
        assert!(legal_game_commands(&initial, 2, &[]).is_empty());

        let mut roller = ScriptedRoller::new(&[]);
        let decision = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
            effect_rules: &[],
            die_roller: &mut roller,
        })
        .expect("the active participant should complete the phase");

        assert_eq!(initial.phase, GamePhase::DarkArts);
        assert_eq!(initial.state_version, 1);
        assert_eq!(decision.state.phase, GamePhase::HeroAction);
        assert_eq!(decision.state.state_version, 2);
        assert_eq!(decision.state.sequence, 1);
        assert_eq!(decision.state.prng_counter, 0);
        assert_eq!(
            decision.event,
            GameEvent::DarkArtsCompleted {
                sequence: 1,
                state_version: 2,
                turn: 1,
                actor_position: 1,
                effects: vec![],
                stop: EffectStop::Stable,
                prng_counter: 0,
            }
        );
        assert_eq!(
            apply_game_event(&initial, &decision.event)
                .expect("the official event should reconstruct the decided state"),
            decision.state
        );
        assert!(legal_game_commands(&decision.state, 1, &[]).is_empty());
        assert_eq!(required_participant_for_decision(&initial, &[]), Some(1));
        assert_eq!(
            required_participant_for_decision(&decision.state, &[]),
            None
        );
    }

    #[test]
    fn command_decision_rejects_stale_or_unauthorized_intentions_without_mutation() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");

        let mut stale_roller = ScriptedRoller::new(&[]);
        assert_eq!(
            decide_game_command(GameCommandInput {
                state: &initial,
                actor_position: 1,
                expected_state_version: 0,
                command: GameCommand::CompleteDarkArts,
                effect_rules: &[],
                die_roller: &mut stale_roller,
            }),
            Err(GameCommandError::StaleStateVersion)
        );
        let mut unauthorized_roller = ScriptedRoller::new(&[]);
        assert_eq!(
            decide_game_command(GameCommandInput {
                state: &initial,
                actor_position: 2,
                expected_state_version: 1,
                command: GameCommand::CompleteDarkArts,
                effect_rules: &[],
                die_roller: &mut unauthorized_roller,
            }),
            Err(GameCommandError::ActorNotActive)
        );
        assert_eq!(initial.phase, GamePhase::DarkArts);
        assert_eq!(initial.state_version, 1);
        assert_eq!(initial.sequence, 0);
    }

    #[test]
    fn persisted_state_restores_only_through_domain_invariants() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");

        let restored = restore_game_state(restore_input(&initial, SNAPSHOT_VERSION))
            .expect("the canonical initial snapshot should restore");

        assert_eq!(restored, initial);
    }

    #[test]
    fn legacy_snapshot_restores_only_without_a_pending_choice() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");

        let restored = restore_game_state(restore_input(&initial, 1))
            .expect("a legacy stable snapshot should be promoted");
        assert_eq!(restored.snapshot_version(), SNAPSHOT_VERSION);
        assert_eq!(restored, initial);

        let effects = [each_hero_choice_rule()];
        let pending = decide_without_rolls(&initial, 1, GameCommand::CompleteDarkArts, &effects)
            .expect("the effect should produce a pending choice");
        assert_eq!(
            restore_game_state(restore_input(&pending.state, 1)),
            Err(GameStateRestoreError::UnsupportedSnapshotVersion)
        );
    }

    #[test]
    fn persisted_choice_values_respect_the_public_transport_bound() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let effects = [each_hero_choice_rule()];
        let pending = decide_without_rolls(&initial, 1, GameCommand::CompleteDarkArts, &effects)
            .expect("the effect should produce a pending choice");

        for mutate in [
            |choice: &mut PendingEffectChoice| choice.id = "x".repeat(257),
            |choice: &mut PendingEffectChoice| choice.options[0] = "x".repeat(257),
            |choice: &mut PendingEffectChoice| {
                choice.options = (0..=4_096).map(|index| format!("option:{index}")).collect();
            },
            |choice: &mut PendingEffectChoice| {
                choice.options = (0..33).map(|index| format!("option:{index}")).collect();
                choice.max = 33;
            },
            |choice: &mut PendingEffectChoice| choice.min = 0,
            |choice: &mut PendingEffectChoice| choice.max = 2,
            |choice: &mut PendingEffectChoice| {
                choice.kind = PendingEffectChoiceKind::Target;
                choice.min = 0;
                choice.max = 0;
            },
            |choice: &mut PendingEffectChoice| {
                choice.kind = PendingEffectChoiceKind::Target;
                choice.max = u16::try_from(choice.options.len()).expect("fixture length fits");
            },
        ] {
            let mut invalid = pending.state.clone();
            mutate(
                invalid
                    .pending_choice
                    .as_mut()
                    .expect("the fixture choice should remain pending"),
            );

            assert_eq!(
                restore_game_state(restore_input(&invalid, SNAPSHOT_VERSION)),
                Err(GameStateRestoreError::InvalidPlayers)
            );
        }
    }

    #[test]
    fn persisted_choice_requires_an_in_progress_dark_arts_stop() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let effects = [each_hero_choice_rule()];
        let pending = decide_without_rolls(&initial, 1, GameCommand::CompleteDarkArts, &effects)
            .expect("the effect should produce a pending choice");

        let mut terminal = pending.state.clone();
        terminal.status = GameStatus::Won;
        assert_eq!(
            restore_game_state(restore_input(&terminal, SNAPSHOT_VERSION)),
            Err(GameStateRestoreError::InvalidPlayers)
        );

        let mut wrong_phase = pending.state;
        wrong_phase.phase = GamePhase::HeroAction;
        assert_eq!(
            restore_game_state(restore_input(&wrong_phase, SNAPSHOT_VERSION)),
            Err(GameStateRestoreError::InvalidPlayers)
        );
    }

    #[test]
    fn persisted_entity_ids_respect_the_public_transport_bound() {
        let participants = valid_participants();
        let mut initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let mut entities = initial.effect_world.entities().to_vec();
        entities.push(EffectEntity::new(
            "x".repeat(257),
            Some(1),
            EffectZone::HeroHand,
        ));
        initial.effect_world = EffectWorld::new(entities);

        assert_eq!(
            restore_game_state(restore_input(&initial, SNAPSHOT_VERSION)),
            Err(GameStateRestoreError::InvalidPlayers)
        );
    }

    #[test]
    fn persisted_state_rejects_inconsistent_versions_and_unknown_algorithms() {
        let players = vec![
            InitialPlayer::new(1, HeroId::Harry),
            InitialPlayer::new(2, HeroId::Hermione),
        ];
        let restore = |state_version, sequence, prng_algorithm| {
            restore_game_state(GameStateRestoreInput {
                snapshot_version: SNAPSHOT_VERSION,
                state_version,
                sequence,
                status: GameStatus::InProgress,
                turn: 1,
                phase: GamePhase::DarkArts,
                active_position: 1,
                adventure_id: CONTENT.adventure_id,
                content_version: CONTENT.content_version,
                ruleset_version: CONTENT.ruleset_version,
                manifest_digest: CONTENT.manifest_digest,
                manifest_version: CONTENT.manifest_version,
                prng_algorithm,
                shuffle_algorithm: SHUFFLE_ALGORITHM,
                sampling_algorithm: SAMPLING_ALGORITHM,
                prng_counter: 0,
                players: players.clone(),
                effect_world: EffectWorld::new(
                    players
                        .iter()
                        .map(|player| EffectEntity::hero(player.position()))
                        .collect(),
                ),
                last_effects: Vec::new(),
                pending_choice: None,
            })
        };

        assert_eq!(
            restore(3, 1, PRNG_ALGORITHM),
            Err(GameStateRestoreError::InvalidVersion)
        );
        assert_eq!(
            restore(1, 0, "unknown-prng"),
            Err(GameStateRestoreError::UnsupportedAlgorithm)
        );
    }

    fn actor_selector(zone: EffectZone) -> EffectSelector {
        EffectSelector {
            zone,
            owner: EffectTargetOwner::Actor,
            min: 1,
            max: 1,
            eligibility: Vec::new(),
        }
    }

    fn rule(cost: Vec<EffectResourceCost>, effect: EffectDefinition) -> EffectRule {
        EffectRule {
            id: "rule:synthetic".to_owned(),
            trigger: EffectTrigger::DarkArtsCompleted,
            cost,
            effect,
        }
    }

    fn each_hero_choice_rule() -> EffectRule {
        rule(
            vec![],
            EffectDefinition::Choice {
                audience: EffectChoiceAudience::EachHero,
                options: vec![
                    EffectDefinition::Apply {
                        target: actor_selector(EffectZone::Heroes),
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Attack,
                            amount: 1,
                        },
                    },
                    EffectDefinition::Apply {
                        target: actor_selector(EffectZone::Heroes),
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Influence,
                            amount: 1,
                        },
                    },
                ],
            },
        )
    }

    fn comprehensive_effect_rule() -> EffectRule {
        rule(
            vec![EffectResourceCost {
                resource: EffectResource::Health,
                amount: 1,
            }],
            EffectDefinition::Sequence {
                effects: vec![
                    EffectDefinition::Apply {
                        target: actor_selector(EffectZone::Heroes),
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Attack,
                            amount: 1,
                        },
                    },
                    EffectDefinition::Condition {
                        condition: EffectCondition::ResourceAtLeast {
                            target: actor_selector(EffectZone::Heroes),
                            resource: EffectResource::Attack,
                            amount: 1,
                        },
                        then: Box::new(EffectDefinition::Repeat {
                            times: 2,
                            effect: Box::new(EffectDefinition::Apply {
                                target: actor_selector(EffectZone::Heroes),
                                operation: EffectOperation::ModifyResource {
                                    resource: EffectResource::Influence,
                                    amount: 1,
                                },
                            }),
                        }),
                        otherwise: None,
                    },
                    EffectDefinition::Roll {
                        die: EffectDie::D4,
                        outcomes: vec![
                            EffectDefinition::NoOp,
                            EffectDefinition::NoOp,
                            EffectDefinition::NoOp,
                            EffectDefinition::Apply {
                                target: actor_selector(EffectZone::HeroHand),
                                operation: EffectOperation::Move {
                                    to: EffectZone::HeroDiscardPile,
                                },
                            },
                        ],
                    },
                ],
            },
        )
    }

    #[test]
    fn closed_effects_resolve_sequences_conditions_repetition_zones_resources_and_dice() {
        let participants = valid_participants();
        let mut initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        initial.effect_world = EffectWorld::new(vec![
            EffectEntity::hero(1),
            EffectEntity::hero(2),
            EffectEntity::new("card:synthetic", Some(1), EffectZone::HeroHand),
        ]);
        let effects = [comprehensive_effect_rule()];

        let mut roller = ScriptedRoller::new(&[4]);
        let decision = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
            effect_rules: &effects,
            die_roller: &mut roller,
        })
        .expect("the validated effect should resolve to a stable point");

        assert_eq!(decision.state.phase(), GamePhase::HeroAction);
        assert_eq!(decision.state.prng_counter(), 1);
        assert_eq!(
            decision
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Health),
            Some(9)
        );
        assert_eq!(
            decision
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Attack),
            Some(1)
        );
        assert_eq!(
            decision
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Influence),
            Some(2)
        );
        assert!(
            decision
                .state
                .effect_world()
                .entities()
                .iter()
                .any(|entity| {
                    entity.id() == "card:synthetic" && entity.zone() == EffectZone::HeroDiscardPile
                })
        );
        assert!(decision.state.last_effects().iter().any(|outcome| matches!(
            outcome,
            EffectOutcome::DieRolled {
                die: EffectDie::D4,
                result: 4,
                ..
            }
        )));
        assert_eq!(
            apply_game_event(&initial, &decision.event)
                .expect("the effect event should replay deterministically"),
            decision.state
        );
    }

    #[test]
    fn selectors_stop_at_an_owned_choice_and_impossible_targets_become_no_ops() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let target_choice = [rule(
            vec![],
            EffectDefinition::Apply {
                target: EffectSelector {
                    zone: EffectZone::Heroes,
                    owner: EffectTargetOwner::Any,
                    min: 1,
                    max: 1,
                    eligibility: vec![EffectEligibility::ResourceAtLeast {
                        resource: EffectResource::Health,
                        amount: 1,
                    }],
                },
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Attack,
                    amount: 1,
                },
            },
        )];

        let mut choice_roller = ScriptedRoller::new(&[]);
        let choice = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
            effect_rules: &target_choice,
            die_roller: &mut choice_roller,
        })
        .expect("multiple eligible targets should stop at a human choice");

        assert_eq!(choice.state.phase(), GamePhase::DarkArts);
        assert_eq!(
            choice
                .state
                .pending_choice()
                .map(|pending| pending.options.clone()),
            Some(vec!["hero:1".to_owned(), "hero:2".to_owned()])
        );
        assert_eq!(
            required_participant_for_decision(&choice.state, &target_choice),
            Some(1)
        );
        assert_eq!(
            legal_game_commands(&choice.state, 1, &target_choice),
            vec![GameCommandType::ResolveChoice]
        );

        let impossible = [rule(
            vec![],
            EffectDefinition::Apply {
                target: actor_selector(EffectZone::HeroHand),
                operation: EffectOperation::Discard,
            },
        )];
        let mut no_op_roller = ScriptedRoller::new(&[]);
        let no_op = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
            effect_rules: &impossible,
            die_roller: &mut no_op_roller,
        })
        .expect("an impossible mandatory target should not block resolution");

        assert_eq!(no_op.state.phase(), GamePhase::HeroAction);
        assert!(matches!(
            no_op.state.last_effects(),
            [EffectOutcome::NoOp {
                reason: EffectNoOpReason::NoEligibleTarget,
                ..
            }]
        ));
    }

    #[test]
    fn legal_command_types_expose_choice_only_to_its_responsible_participant() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let effects = [rule(
            vec![],
            EffectDefinition::Choice {
                audience: EffectChoiceAudience::Actor,
                options: vec![EffectDefinition::NoOp, EffectDefinition::NoOp],
            },
        )];
        let mut roller = ScriptedRoller::new(&[]);
        let pending = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
            effect_rules: &effects,
            die_roller: &mut roller,
        })
        .expect("the actor choice should become pending");

        assert_eq!(
            legal_game_commands(&pending.state, 1, &effects),
            vec![GameCommandType::ResolveChoice]
        );
        assert!(legal_game_commands(&pending.state, 2, &effects).is_empty());
    }

    #[test]
    fn duplicate_effect_rule_ids_are_rejected_before_execution_or_resume() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let duplicate_rules = [
            rule(vec![], EffectDefinition::NoOp),
            rule(
                vec![],
                EffectDefinition::Terminal {
                    outcome: EffectGameOutcome::Won,
                },
            ),
        ];

        assert_eq!(
            decide_without_rolls(&initial, 1, GameCommand::CompleteDarkArts, &duplicate_rules,),
            Err(GameCommandError::CommandNotLegal)
        );

        let choice_rule = each_hero_choice_rule();
        let pending = decide_without_rolls(
            &initial,
            1,
            GameCommand::CompleteDarkArts,
            std::slice::from_ref(&choice_rule),
        )
        .expect("the unique rule should produce a pending choice");
        let choice = pending
            .state
            .pending_choice()
            .expect("the choice should remain pending");
        let duplicate_resume_rules = [choice_rule, rule(vec![], EffectDefinition::NoOp)];

        assert_eq!(
            decide_without_rolls(
                &pending.state,
                choice.responsible_position,
                GameCommand::ResolveChoice {
                    choice_id: choice.id.clone(),
                    selected_options: vec![choice.options[0].clone()],
                },
                &duplicate_resume_rules,
            ),
            Err(GameCommandError::EffectExecutionFailed)
        );
    }

    #[test]
    fn executable_rule_ids_leave_room_for_public_choice_ids() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let mut maximum = each_hero_choice_rule();
        maximum.id = "r".repeat(244);
        let mut too_long = maximum.clone();
        too_long.id.push('r');

        assert_eq!(
            legal_game_commands(&initial, 1, &[maximum]),
            vec![GameCommandType::CompleteDarkArts]
        );
        assert!(legal_game_commands(&initial, 1, &[too_long]).is_empty());
    }

    #[test]
    fn target_choice_applies_one_target_and_resumes_the_remaining_effect_queue() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let effects = [rule(
            vec![],
            EffectDefinition::Sequence {
                effects: vec![
                    EffectDefinition::Apply {
                        target: EffectSelector {
                            zone: EffectZone::Heroes,
                            owner: EffectTargetOwner::Any,
                            min: 1,
                            max: 1,
                            eligibility: Vec::new(),
                        },
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Attack,
                            amount: 1,
                        },
                    },
                    EffectDefinition::Apply {
                        target: actor_selector(EffectZone::Heroes),
                        operation: EffectOperation::ModifyResource {
                            resource: EffectResource::Influence,
                            amount: 1,
                        },
                    },
                ],
            },
        )];
        let mut roller = ScriptedRoller::new(&[]);
        let pending = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
            effect_rules: &effects,
            die_roller: &mut roller,
        })
        .expect("two eligible heroes should require one target choice");
        let choice = pending
            .state
            .pending_choice()
            .expect("the target choice should be globally pending");
        assert_eq!(choice.kind, PendingEffectChoiceKind::Target);
        assert_eq!(choice.options, ["hero:1", "hero:2"]);

        let mut roller = ScriptedRoller::new(&[]);
        let stable = decide_game_command(GameCommandInput {
            state: &pending.state,
            actor_position: 1,
            expected_state_version: 2,
            command: GameCommand::ResolveChoice {
                choice_id: choice.id.clone(),
                selected_options: vec!["hero:2".to_owned()],
            },
            effect_rules: &effects,
            die_roller: &mut roller,
        })
        .expect("the selected target should resolve before the remaining effect");

        assert_eq!(
            stable
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Attack),
            Some(0)
        );
        assert_eq!(
            stable
                .state
                .effect_world()
                .hero_resource(2, EffectResource::Attack),
            Some(1)
        );
        assert_eq!(
            stable
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Influence),
            Some(1)
        );
        assert_eq!(stable.state.phase(), GamePhase::HeroAction);
        assert!(stable.state.pending_choice().is_none());
        assert_eq!(
            apply_game_event(&pending.state, &stable.event)
                .expect("the target choice event should replay"),
            stable.state
        );
    }

    #[test]
    fn choice_events_record_canonical_options_and_validate_their_cause() {
        let mut participants = valid_participants();
        participants.push(LobbyParticipant {
            role: ParticipantRole::Guest,
            position: 3,
            hero: Some(HeroId::Neville),
            ready: true,
        });
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the three participant lobby should start");
        let effects = [rule(
            vec![],
            EffectDefinition::Apply {
                target: EffectSelector {
                    zone: EffectZone::Heroes,
                    owner: EffectTargetOwner::Any,
                    min: 2,
                    max: 2,
                    eligibility: Vec::new(),
                },
                operation: EffectOperation::ModifyResource {
                    resource: EffectResource::Attack,
                    amount: 1,
                },
            },
        )];
        let mut roller = ScriptedRoller::new(&[]);
        let pending = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
            effect_rules: &effects,
            die_roller: &mut roller,
        })
        .expect("three eligible heroes should require two target selections");
        let choice_id = pending
            .state
            .pending_choice()
            .expect("the target choice should be pending")
            .id
            .clone();

        let mut roller = ScriptedRoller::new(&[]);
        let resolved = decide_game_command(GameCommandInput {
            state: &pending.state,
            actor_position: 1,
            expected_state_version: 2,
            command: GameCommand::ResolveChoice {
                choice_id,
                selected_options: vec!["hero:3".to_owned(), "hero:1".to_owned()],
            },
            effect_rules: &effects,
            die_roller: &mut roller,
        })
        .expect("the valid target selection should resolve");

        let mut invalid_cause = resolved.event.clone();
        let GameEvent::ChoiceResolved { choice_cause, .. } = &mut invalid_cause else {
            panic!("the response should emit a choice event");
        };
        *choice_cause = "rule:other".to_owned();
        assert_eq!(
            apply_game_event(&pending.state, &invalid_cause),
            Err(GameEventError::EffectTransitionInvalid)
        );

        let GameEvent::ChoiceResolved {
            choice_cause,
            selected_options,
            ..
        } = resolved.event
        else {
            panic!("the response should emit a choice event");
        };
        assert_eq!(choice_cause, "rule:synthetic");
        assert_eq!(selected_options, ["hero:1", "hero:3"]);
    }

    #[test]
    fn participant_choices_resume_in_position_order_during_another_heroes_turn() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let effects = [each_hero_choice_rule()];

        let first = decide_without_rolls(&initial, 1, GameCommand::CompleteDarkArts, &effects)
            .expect("the effect should stop at the first participant choice");
        let first_choice = first
            .state
            .pending_choice()
            .expect("one global choice should be pending");
        assert_eq!(first_choice.cause, "rule:synthetic");
        assert_eq!(first_choice.responsible_position, 1);
        assert_eq!(first_choice.options, ["option:1", "option:2"]);

        assert_eq!(
            decide_without_rolls(
                &first.state,
                2,
                GameCommand::ResolveChoice {
                    choice_id: first_choice.id.clone(),
                    selected_options: vec!["option:1".to_owned()],
                },
                &effects,
            ),
            Err(GameCommandError::ActorNotChoiceResponsible)
        );

        let second = decide_without_rolls(
            &first.state,
            1,
            GameCommand::ResolveChoice {
                choice_id: first_choice.id.clone(),
                selected_options: vec!["option:1".to_owned()],
            },
            &effects,
        )
        .expect("the responsible participant should resolve the first choice");
        let second_choice = second
            .state
            .pending_choice()
            .expect("the second participant choice should become globally pending");
        assert_eq!(second_choice.cause, "rule:synthetic");
        assert_eq!(second_choice.responsible_position, 2);
        assert_eq!(second.state.active_position(), 1);

        let stable = decide_without_rolls(
            &second.state,
            2,
            GameCommand::ResolveChoice {
                choice_id: second_choice.id.clone(),
                selected_options: vec!["option:2".to_owned()],
            },
            &effects,
        )
        .expect("the delegated participant should resolve the second choice");

        assert_eq!(stable.state.phase(), GamePhase::HeroAction);
        assert!(stable.state.pending_choice().is_none());
        assert_eq!(
            stable
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Attack),
            Some(1)
        );
        assert_eq!(
            stable
                .state
                .effect_world()
                .hero_resource(2, EffectResource::Influence),
            Some(1)
        );
        assert_eq!(
            apply_game_event(&second.state, &stable.event)
                .expect("the delegated choice event should replay deterministically"),
            stable.state
        );
    }

    #[test]
    fn unaffordable_cost_removes_the_action_and_terminal_effects_end_resolution() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let unaffordable = [rule(
            vec![EffectResourceCost {
                resource: EffectResource::Influence,
                amount: 1,
            }],
            EffectDefinition::NoOp,
        )];

        assert!(legal_game_commands(&initial, 1, &unaffordable).is_empty());
        let mut unaffordable_roller = ScriptedRoller::new(&[]);
        assert_eq!(
            decide_game_command(GameCommandInput {
                state: &initial,
                actor_position: 1,
                expected_state_version: 1,
                command: GameCommand::CompleteDarkArts,
                effect_rules: &unaffordable,
                die_roller: &mut unaffordable_roller,
            }),
            Err(GameCommandError::CommandNotLegal)
        );

        let terminal = [rule(
            vec![],
            EffectDefinition::Terminal {
                outcome: EffectGameOutcome::Won,
            },
        )];
        let mut terminal_roller = ScriptedRoller::new(&[]);
        let decision = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
            effect_rules: &terminal,
            die_roller: &mut terminal_roller,
        })
        .expect("a terminal effect should produce an official terminal state");

        assert_eq!(decision.state.status(), GameStatus::Won);
        assert!(legal_game_commands(&decision.state, 1, &terminal).is_empty());
    }

    #[test]
    fn replay_rejects_malformed_effect_outcomes_and_stop_points() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");
        let invalid_events = [
            GameEvent::DarkArtsCompleted {
                sequence: 1,
                state_version: 2,
                turn: 1,
                actor_position: 1,
                effects: vec![EffectOutcome::DieRolled {
                    rule_id: "rule:synthetic".to_owned(),
                    die: EffectDie::D4,
                    result: 5,
                }],
                stop: EffectStop::Stable,
                prng_counter: 1,
            },
            GameEvent::DarkArtsCompleted {
                sequence: 1,
                state_version: 2,
                turn: 1,
                actor_position: 1,
                effects: vec![EffectOutcome::Terminal {
                    rule_id: "rule:synthetic".to_owned(),
                    outcome: EffectGameOutcome::Won,
                }],
                stop: EffectStop::Terminal(EffectGameOutcome::Lost),
                prng_counter: 0,
            },
            GameEvent::DarkArtsCompleted {
                sequence: 1,
                state_version: 2,
                turn: 1,
                actor_position: 1,
                effects: vec![EffectOutcome::ResourceChanged {
                    rule_id: "rule:synthetic".to_owned(),
                    target_id: "hero:1".to_owned(),
                    target_position: Some(2),
                    resource: EffectResource::Health,
                    before: 10,
                    after: 9,
                    cause: EffectChangeCause::Effect,
                }],
                stop: EffectStop::Stable,
                prng_counter: 0,
            },
        ];

        for event in invalid_events {
            assert_eq!(
                apply_game_event(&initial, &event),
                Err(GameEventError::EffectTransitionInvalid)
            );
        }
    }
}
