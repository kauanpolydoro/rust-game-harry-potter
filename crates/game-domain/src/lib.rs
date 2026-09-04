//! Pure game rules.
//!
//! Game decisions enter through typed inputs and leave as typed state without
//! depending on infrastructure, clocks, global randomness, or transport DTOs.

#![forbid(unsafe_code)]

use std::{collections::BTreeSet, str::FromStr};

mod effects;

pub use effects::{
    EffectChangeCause, EffectChoiceAudience, EffectCondition, EffectContinuation, EffectCursor,
    EffectDefinition, EffectDie, EffectEligibility, EffectEntity, EffectEntityKind,
    EffectEntityPlacement, EffectExecutionError, EffectGameOutcome, EffectNoOpReason,
    EffectOperation, EffectOutcome, EffectPathSegment, EffectResource, EffectResourceCost,
    EffectRoller, EffectRule, EffectSelector, EffectStop, EffectTargetBinding, EffectTargetOwner,
    EffectTrigger, EffectWorld, EffectZone, HERO_MAX_HEALTH, MAX_EFFECT_BRANCH_INDEX,
    MAX_EFFECT_PATH_DEPTH, MAX_EFFECT_ROLL_INDEX, PendingEffectChoice, PendingEffectChoiceKind,
    QueuedEffect, effect_action_is_affordable,
};

pub const SNAPSHOT_VERSION: u16 = 4;
const HERO_ACTION_SNAPSHOT_VERSION: u16 = 3;
const PARTICIPANT_CHOICE_SNAPSHOT_VERSION: u16 = 2;
const LEGACY_SNAPSHOT_VERSION: u16 = 1;
const STRUCTURAL_OUTCOME_RULE_ID: &str = "system:game-outcome";
pub const INITIAL_STATE_VERSION: u64 = 1;
pub const INITIAL_SEQUENCE: u64 = 0;
pub const MAX_TURN_STEPS: usize = 3;
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
    pub initial_entities: &'a [EffectEntityPlacement],
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
    active_villain_limit: u8,
    players: Vec<InitialPlayer>,
    effect_world: EffectWorld,
    last_effects: Vec<EffectOutcome>,
    pending_choice: Option<PendingEffectChoice>,
    queued_phases: Vec<GamePhase>,
    queued_effects: Vec<QueuedEffect>,
    decision_point: Option<DecisionPoint>,
    last_turn_steps: Vec<TurnStep>,
}

pub type GameState = InitialGameState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStatus {
    InProgress,
    Lost,
    Won,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase {
    DarkArts,
    Villains,
    HeroActions,
    EndTurn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionPoint {
    Automatic,
    PlayerIntent { responsible_position: u8 },
    EffectChoice(PendingEffectChoice),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnStep {
    phase: GamePhase,
    effects: Vec<EffectOutcome>,
}

impl TurnStep {
    #[must_use]
    pub fn new(phase: GamePhase, effects: Vec<EffectOutcome>) -> Self {
        Self { phase, effects }
    }

    #[must_use]
    pub const fn phase(&self) -> GamePhase {
        self.phase
    }

    #[must_use]
    pub fn effects(&self) -> &[EffectOutcome] {
        &self.effects
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerIntent {
    EndHeroActions,
    ResolveChoice {
        choice_id: String,
        selected_options: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerIntentType {
    EndHeroActions,
    ResolveChoice,
}

#[derive(Clone)]
pub struct GameIntentInput<'a> {
    pub state: &'a InitialGameState,
    pub actor_position: u8,
    pub expected_state_version: u64,
    pub intent: PlayerIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameIntentDecision {
    pub state: InitialGameState,
    pub event: GameEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndTurnOutcome {
    LocationAdvanced {
        location_id: String,
        next_location_id: Option<String>,
    },
    VillainRevealed {
        villain_id: String,
    },
    CardMoved {
        card_id: String,
        from: EffectZone,
        to: EffectZone,
    },
    PileShuffled {
        owner_position: u8,
        zone: EffectZone,
        bottom_to_top: Vec<String>,
    },
    ResourceReset {
        resource: EffectResource,
        before: u16,
    },
    HeroRecovered {
        position: u8,
        before: u16,
        after: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineControl {
    pub status: GameStatus,
    pub turn: u32,
    pub phase: GamePhase,
    pub active_position: u8,
    pub queued_phases: Vec<GamePhase>,
    pub queued_effects: Vec<QueuedEffect>,
    pub decision_point: Option<DecisionPoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameIntentError {
    StaleStateVersion,
    ActorNotResponsible,
    ActorNotChoiceResponsible,
    IntentNotLegal,
    EffectExecutionFailed,
    RandomSourceFailed,
    VersionOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedGameRules {
    effect_rules: Vec<EffectRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidatedGameRulesError {
    AutomaticRuleHasCost,
    DuplicatePhaseOrder,
    DuplicateRuleId,
}

impl ValidatedGameRules {
    /// Orders validated effect roots independently from their declaration order.
    ///
    /// # Errors
    ///
    /// Returns an error when two roots occupy the same position in one automatic phase.
    pub fn new(mut effect_rules: Vec<EffectRule>) -> Result<Self, ValidatedGameRulesError> {
        let mut rule_ids = BTreeSet::new();
        if effect_rules
            .iter()
            .any(|rule| !rule_ids.insert(rule.id.as_str()))
        {
            return Err(ValidatedGameRulesError::DuplicateRuleId);
        }
        if effect_rules.iter().any(|rule| {
            matches!(
                rule.trigger,
                EffectTrigger::DarkArts | EffectTrigger::Villains
            ) && !rule.cost.is_empty()
        }) {
            return Err(ValidatedGameRulesError::AutomaticRuleHasCost);
        }
        effect_rules.sort_by(|left, right| {
            effect_trigger_order(left.trigger)
                .cmp(&effect_trigger_order(right.trigger))
                .then(left.order.cmp(&right.order))
                .then(left.id.cmp(&right.id))
        });
        if effect_rules.windows(2).any(|pair| {
            pair[0].trigger == pair[1].trigger
                && pair[0].order == pair[1].order
                && matches!(
                    pair[0].trigger,
                    EffectTrigger::DarkArts | EffectTrigger::Villains
                )
        }) {
            return Err(ValidatedGameRulesError::DuplicatePhaseOrder);
        }
        Ok(Self { effect_rules })
    }

    #[must_use]
    pub fn effect_rules(&self) -> &[EffectRule] {
        &self.effect_rules
    }
}

const fn effect_trigger_order(trigger: EffectTrigger) -> u8 {
    match trigger {
        EffectTrigger::DarkArts | EffectTrigger::DarkArtsCompleted => 0,
        EffectTrigger::Villains => 1,
        EffectTrigger::Manual => 2,
        EffectTrigger::VillainReward => 3,
    }
}

pub struct GameEngine<'rules> {
    rules: &'rules ValidatedGameRules,
}

impl<'rules> GameEngine<'rules> {
    #[must_use]
    pub const fn new(rules: &'rules ValidatedGameRules) -> Self {
        Self { rules }
    }

    /// Starts a game and resolves mandatory phases until a human decision or terminal state.
    ///
    /// # Errors
    ///
    /// Returns an error when the lobby is invalid or validated effects cannot be executed.
    pub fn start(
        &self,
        input: StartGameInput<'_>,
        roller: &mut dyn EffectRoller,
    ) -> Result<InitialGameState, StartGameError> {
        let state = initialize_game(input)?;
        settle_initial_turn(state, self.rules.effect_rules(), roller)
    }

    #[must_use]
    pub fn legal_intent_types(
        &self,
        state: &InitialGameState,
        actor_position: u8,
    ) -> Vec<PlayerIntentType> {
        if state.status != GameStatus::InProgress {
            return Vec::new();
        }

        match state.decision_point.as_ref() {
            Some(DecisionPoint::PlayerIntent {
                responsible_position,
            }) if state.phase == GamePhase::HeroActions
                && *responsible_position == actor_position =>
            {
                vec![PlayerIntentType::EndHeroActions]
            }
            Some(DecisionPoint::EffectChoice(choice))
                if choice.responsible_position == actor_position
                    && state.pending_choice.as_ref() == Some(choice) =>
            {
                vec![PlayerIntentType::ResolveChoice]
            }
            Some(
                DecisionPoint::Automatic
                | DecisionPoint::PlayerIntent { .. }
                | DecisionPoint::EffectChoice(_),
            )
            | None => Vec::new(),
        }
    }

    /// Accepts one free player intent and advances all mandatory work that follows it.
    ///
    /// # Errors
    ///
    /// Returns an error without mutating the supplied state when the version, actor, intent,
    /// persisted state, or random stream is invalid.
    pub fn decide(
        &self,
        input: GameIntentInput<'_>,
        random: &mut dyn EffectRoller,
    ) -> Result<GameIntentDecision, GameIntentError> {
        if input.expected_state_version != input.state.state_version {
            return Err(GameIntentError::StaleStateVersion);
        }

        match input.intent {
            PlayerIntent::EndHeroActions => {
                if input.state.decision_point
                    != Some(DecisionPoint::PlayerIntent {
                        responsible_position: input.actor_position,
                    })
                {
                    return Err(GameIntentError::ActorNotResponsible);
                }
                self.end_hero_actions(input.state, input.actor_position, random)
            }
            PlayerIntent::ResolveChoice {
                choice_id,
                selected_options,
            } => self.resolve_choice(
                input.state,
                input.actor_position,
                choice_id,
                &selected_options,
                random,
            ),
        }
    }

    fn resolve_choice(
        &self,
        current: &InitialGameState,
        actor_position: u8,
        choice_id: String,
        selected_options: &[String],
        random: &mut dyn EffectRoller,
    ) -> Result<GameIntentDecision, GameIntentError> {
        if current.status != GameStatus::InProgress {
            return Err(GameIntentError::IntentNotLegal);
        }
        let pending = current
            .pending_choice
            .as_ref()
            .filter(|pending| {
                matches!(
                    current.decision_point.as_ref(),
                    Some(DecisionPoint::EffectChoice(decision)) if decision == *pending
                ) && current.queued_effects.as_slice() == pending.continuation.queue.as_slice()
            })
            .ok_or(GameIntentError::IntentNotLegal)?;
        if actor_position != pending.responsible_position {
            return Err(GameIntentError::ActorNotChoiceResponsible);
        }
        if choice_id != pending.id {
            return Err(GameIntentError::IntentNotLegal);
        }
        let selected_options =
            effects::normalize_effect_choice_selection(pending, selected_options)
                .ok_or(GameIntentError::IntentNotLegal)?;
        let sequence = current
            .sequence
            .checked_add(1)
            .ok_or(GameIntentError::VersionOverflow)?;
        let state_version = current
            .state_version
            .checked_add(1)
            .ok_or(GameIntentError::VersionOverflow)?;
        let choice_cause = pending.cause.clone();
        let phase = current.phase;
        let mut next = current.clone();
        let mut resolution = effects::resume_effects(
            &mut next.effect_world,
            pending,
            &selected_options,
            self.rules.effect_rules(),
            random,
        )
        .map_err(game_intent_effect_error)?;
        settle_structural_outcome(
            &next.effect_world,
            &mut resolution.outcomes,
            &mut resolution.stop,
        );
        next.prng_counter = next
            .prng_counter
            .checked_add(resolution.rolls_consumed)
            .ok_or(GameIntentError::VersionOverflow)?;
        next.last_effects
            .extend(resolution.outcomes.iter().cloned());
        if next.last_effects.len() > 4_096 {
            return Err(GameIntentError::EffectExecutionFailed);
        }
        append_phase_effects(&mut next.last_turn_steps, phase, &resolution.outcomes);
        let mut steps = vec![TurnStep::new(phase, resolution.outcomes)];

        match resolution.stop {
            EffectStop::Choice(choice) => {
                if resolution.queue != choice.continuation.queue {
                    return Err(GameIntentError::EffectExecutionFailed);
                }
                next.queued_effects.clone_from(&choice.continuation.queue);
                next.pending_choice = Some(choice.clone());
                next.decision_point = Some(DecisionPoint::EffectChoice(choice));
            }
            EffectStop::Stable => {
                advance_after_automatic_phase(&mut next);
                steps.extend(
                    settle_automatic_phases(&mut next, self.rules.effect_rules(), random)
                        .map_err(game_intent_effect_error)?,
                );
            }
            EffectStop::Terminal(outcome) => finish_terminal_state(&mut next, outcome),
        }
        next.sequence = sequence;
        next.state_version = state_version;

        let event = GameEvent::ChoiceResolved {
            sequence,
            state_version,
            turn: current.turn,
            actor_position,
            choice_id,
            choice_cause,
            selected_options,
            steps,
            control: EngineControl::from_state(&next),
            prng_counter: next.prng_counter,
        };
        Ok(GameIntentDecision { state: next, event })
    }

    fn end_hero_actions(
        &self,
        current: &InitialGameState,
        actor_position: u8,
        random: &mut dyn EffectRoller,
    ) -> Result<GameIntentDecision, GameIntentError> {
        let sequence = current
            .sequence
            .checked_add(1)
            .ok_or(GameIntentError::VersionOverflow)?;
        let state_version = current
            .state_version
            .checked_add(1)
            .ok_or(GameIntentError::VersionOverflow)?;
        let mut next = current.clone();
        next.phase = GamePhase::EndTurn;
        next.queued_phases.clear();
        next.queued_effects.clear();
        next.pending_choice = None;
        next.decision_point = Some(DecisionPoint::Automatic);
        next.last_effects.clear();
        next.last_turn_steps = vec![TurnStep {
            phase: GamePhase::EndTurn,
            effects: Vec::new(),
        }];

        let (end_turn, samples_consumed) = perform_end_turn(
            &mut next.effect_world,
            actor_position,
            next.active_villain_limit,
            random,
        )?;
        next.prng_counter = next
            .prng_counter
            .checked_add(samples_consumed)
            .ok_or(GameIntentError::VersionOverflow)?;
        let final_location_controlled = end_turn.iter().any(|outcome| {
            matches!(
                outcome,
                EndTurnOutcome::LocationAdvanced {
                    next_location_id: None,
                    ..
                }
            )
        });
        if final_location_controlled {
            next.last_effects.push(EffectOutcome::Terminal {
                rule_id: STRUCTURAL_OUTCOME_RULE_ID.to_owned(),
                outcome: EffectGameOutcome::Lost,
            });
            next.last_turn_steps[0]
                .effects
                .clone_from(&next.last_effects);
            finish_terminal_state(&mut next, EffectGameOutcome::Lost);
        } else {
            let player_index = next
                .players
                .iter()
                .position(|player| player.position == actor_position)
                .ok_or(GameIntentError::ActorNotResponsible)?;
            let next_player_index = (player_index + 1) % next.players.len();
            next.active_position = next.players[next_player_index].position;
            next.turn = next
                .turn
                .checked_add(1)
                .ok_or(GameIntentError::VersionOverflow)?;
            next.phase = GamePhase::DarkArts;
            next.queued_phases = vec![
                GamePhase::Villains,
                GamePhase::HeroActions,
                GamePhase::EndTurn,
            ];
            settle_automatic_phases(&mut next, self.rules.effect_rules(), random)
                .map_err(|_| GameIntentError::EffectExecutionFailed)?;
        }
        next.sequence = sequence;
        next.state_version = state_version;

        let event = GameEvent::TurnCompleted {
            sequence,
            state_version,
            turn: current.turn,
            actor_position,
            end_turn,
            steps: next.last_turn_steps.clone(),
            control: EngineControl::from_state(&next),
            prng_counter: next.prng_counter,
        };
        Ok(GameIntentDecision { state: next, event })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameCommand {
    CompleteDarkArts,
    ResolveChoice {
        choice_id: String,
        selected_options: Vec<String>,
    },
    PlayCard {
        card_id: String,
        targets: Vec<EffectTargetBinding>,
    },
    AssignAttack {
        villain_id: String,
        amount: u16,
    },
    AcquireCard {
        card_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCommandType {
    CompleteDarkArts,
    ResolveChoice,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegalGameIntentions {
    pub complete_dark_arts: bool,
    pub playable_cards: Vec<LegalPlayableCard>,
    pub attack_targets: Vec<LegalAttackTarget>,
    pub acquisitions: Vec<LegalAcquisition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegalPlayableCard {
    pub card_id: String,
    pub target_slots: Vec<LegalTargetSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegalTargetSlot {
    pub selector_id: String,
    pub min: u16,
    pub max: u16,
    pub target_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegalAttackTarget {
    pub villain_id: String,
    pub max_amount: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegalAcquisition {
    pub card_id: String,
    pub cost: u16,
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
        steps: Vec<TurnStep>,
        control: EngineControl,
        prng_counter: u64,
    },
    TurnCompleted {
        sequence: u64,
        state_version: u64,
        turn: u32,
        actor_position: u8,
        end_turn: Vec<EndTurnOutcome>,
        steps: Vec<TurnStep>,
        control: EngineControl,
        prng_counter: u64,
    },
    CardPlayed {
        sequence: u64,
        state_version: u64,
        turn: u32,
        actor_position: u8,
        card_id: String,
        targets: Vec<EffectTargetBinding>,
        effects: Vec<EffectOutcome>,
        stop: EffectStop,
        prng_counter: u64,
    },
    AttackAssigned {
        sequence: u64,
        state_version: u64,
        turn: u32,
        actor_position: u8,
        villain_id: String,
        amount: u16,
        effects: Vec<EffectOutcome>,
        stop: EffectStop,
        prng_counter: u64,
    },
    CardAcquired {
        sequence: u64,
        state_version: u64,
        turn: u32,
        actor_position: u8,
        card_id: String,
        cost: u16,
        refill_card_id: Option<String>,
        effects: Vec<EffectOutcome>,
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
    pub active_villain_limit: u8,
    pub players: Vec<InitialPlayer>,
    pub effect_world: EffectWorld,
    pub last_effects: Vec<EffectOutcome>,
    pub pending_choice: Option<PendingEffectChoice>,
    pub queued_phases: Vec<GamePhase>,
    pub queued_effects: Vec<QueuedEffect>,
    pub decision_point: Option<DecisionPoint>,
    pub last_turn_steps: Vec<TurnStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStateRestoreError {
    UnsupportedSnapshotVersion,
    InvalidVersion,
    InvalidTurn,
    InvalidContentIdentity,
    UnsupportedAlgorithm,
    InvalidPlayers,
    InvalidControlState,
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
            Self::InvalidControlState => "persisted engine control violates game invariants",
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
    pub const fn active_villain_limit(&self) -> u8 {
        self.active_villain_limit
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

    #[must_use]
    pub fn queued_phases(&self) -> &[GamePhase] {
        &self.queued_phases
    }

    #[must_use]
    pub fn queued_effects(&self) -> &[QueuedEffect] {
        &self.queued_effects
    }

    #[must_use]
    pub const fn decision_point(&self) -> Option<&DecisionPoint> {
        self.decision_point.as_ref()
    }

    #[must_use]
    pub fn last_turn_steps(&self) -> &[TurnStep] {
        &self.last_turn_steps
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
    EffectExecutionFailed,
    InvalidInitialEntities,
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

    let players = initial_players(input.participants)?;
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

    let effect_world = EffectWorld::new(
        players
            .iter()
            .map(|player| {
                EffectEntityPlacement::new(EffectEntity::hero(player.position), EffectZone::Heroes)
            })
            .chain(input.content.initial_entities.iter().cloned())
            .collect(),
    );
    let active_villain_limit =
        u8::try_from(effect_world.entities_in(EffectZone::ActiveVillains).len())
            .map_err(|_| StartGameError::InvalidInitialEntities)?;
    let player_positions = players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    if !effect_world.is_valid_for_positions(&player_positions) {
        return Err(StartGameError::InvalidInitialEntities);
    }

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
        active_villain_limit,
        effect_world,
        last_effects: Vec::new(),
        pending_choice: None,
        queued_phases: vec![
            GamePhase::Villains,
            GamePhase::HeroActions,
            GamePhase::EndTurn,
        ],
        queued_effects: Vec::new(),
        decision_point: Some(DecisionPoint::Automatic),
        last_turn_steps: Vec::new(),
        players,
    })
}

fn initial_players(
    participants: &[LobbyParticipant],
) -> Result<Vec<InitialPlayer>, StartGameError> {
    let heroes = participants
        .iter()
        .map(|participant| participant.hero.ok_or(StartGameError::MissingHero))
        .collect::<Result<Vec<_>, _>>()?;
    if heroes.iter().copied().collect::<BTreeSet<_>>().len() != heroes.len() {
        return Err(StartGameError::DuplicateHero);
    }

    let mut players = participants
        .iter()
        .zip(heroes)
        .map(|(participant, hero)| InitialPlayer {
            position: participant.position,
            hero,
        })
        .collect::<Vec<_>>();
    players.sort_by_key(|player| player.position);
    Ok(players)
}

fn settle_initial_turn(
    mut state: InitialGameState,
    rules: &[EffectRule],
    roller: &mut dyn EffectRoller,
) -> Result<InitialGameState, StartGameError> {
    settle_automatic_phases(&mut state, rules, roller)
        .map_err(|_| StartGameError::EffectExecutionFailed)?;
    Ok(state)
}

fn settle_automatic_phases(
    state: &mut InitialGameState,
    rules: &[EffectRule],
    roller: &mut dyn EffectRoller,
) -> Result<Vec<TurnStep>, EffectExecutionError> {
    let mut all_effects = std::mem::take(&mut state.last_effects);
    let mut history = std::mem::take(&mut state.last_turn_steps);
    let mut resolved_steps = Vec::new();

    loop {
        let (phase, trigger) = match state.phase {
            GamePhase::DarkArts => (GamePhase::DarkArts, EffectTrigger::DarkArts),
            GamePhase::Villains => (GamePhase::Villains, EffectTrigger::Villains),
            GamePhase::HeroActions | GamePhase::EndTurn => {
                state.last_effects = all_effects;
                state.last_turn_steps = history;
                return Ok(resolved_steps);
            }
        };
        let mut resolution = effects::execute_effects(
            &mut state.effect_world,
            state.active_position,
            rules,
            trigger,
            roller,
        )?;
        settle_structural_outcome(
            &state.effect_world,
            &mut resolution.outcomes,
            &mut resolution.stop,
        );
        let effects::EffectResolution {
            outcomes,
            stop,
            rolls_consumed,
            queue,
        } = resolution;
        state.prng_counter = state
            .prng_counter
            .checked_add(rolls_consumed)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        all_effects.extend(outcomes.iter().cloned());
        if all_effects.len() > 4_096 {
            return Err(EffectExecutionError::StepLimitExceeded);
        }
        let step = TurnStep::new(phase, outcomes);
        history.push(step.clone());
        resolved_steps.push(step);
        match stop {
            EffectStop::Choice(choice) => {
                if queue != choice.continuation.queue {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                state.queued_effects.clone_from(&choice.continuation.queue);
                state.pending_choice = Some(choice.clone());
                state.decision_point = Some(DecisionPoint::EffectChoice(choice));
                state.last_effects = all_effects;
                state.last_turn_steps = history;
                return Ok(resolved_steps);
            }
            EffectStop::Terminal(outcome) => {
                finish_terminal_state(state, outcome);
                state.last_effects = all_effects;
                state.last_turn_steps = history;
                return Ok(resolved_steps);
            }
            EffectStop::Stable => advance_after_automatic_phase(state),
        }
    }
}

fn advance_after_automatic_phase(state: &mut InitialGameState) {
    state.pending_choice = None;
    state.queued_effects.clear();
    match state.phase {
        GamePhase::DarkArts => {
            state.phase = GamePhase::Villains;
            state.queued_phases = vec![GamePhase::HeroActions, GamePhase::EndTurn];
            state.decision_point = Some(DecisionPoint::Automatic);
        }
        GamePhase::Villains => {
            state.phase = GamePhase::HeroActions;
            state.queued_phases = vec![GamePhase::EndTurn];
            state.decision_point = Some(DecisionPoint::PlayerIntent {
                responsible_position: state.active_position,
            });
        }
        GamePhase::HeroActions => {
            state.queued_phases = vec![GamePhase::EndTurn];
            state.decision_point = Some(DecisionPoint::PlayerIntent {
                responsible_position: state.active_position,
            });
        }
        GamePhase::EndTurn => {}
    }
}

fn finish_terminal_state(state: &mut InitialGameState, outcome: EffectGameOutcome) {
    state.status = match outcome {
        EffectGameOutcome::Lost => GameStatus::Lost,
        EffectGameOutcome::Won => GameStatus::Won,
    };
    state.pending_choice = None;
    state.decision_point = None;
    state.queued_phases.clear();
    state.queued_effects.clear();
}

fn settle_structural_outcome(
    world: &EffectWorld,
    outcomes: &mut Vec<EffectOutcome>,
    stop: &mut EffectStop,
) {
    if !matches!(
        stop,
        EffectStop::Stable | EffectStop::Terminal(EffectGameOutcome::Won)
    ) {
        return;
    }
    let Some(outcome) = world.structural_game_outcome() else {
        return;
    };
    if matches!(stop, EffectStop::Terminal(EffectGameOutcome::Won)) {
        if outcome != EffectGameOutcome::Lost {
            return;
        }
        outcomes.pop();
    }
    outcomes.push(EffectOutcome::Terminal {
        rule_id: STRUCTURAL_OUTCOME_RULE_ID.to_owned(),
        outcome,
    });
    *stop = EffectStop::Terminal(outcome);
}

fn append_phase_effects(history: &mut Vec<TurnStep>, phase: GamePhase, effects: &[EffectOutcome]) {
    if let Some(step) = history.last_mut().filter(|step| step.phase == phase) {
        step.effects.extend_from_slice(effects);
    } else {
        history.push(TurnStep::new(phase, effects.to_vec()));
    }
}

const fn game_intent_effect_error(error: EffectExecutionError) -> GameIntentError {
    match error {
        EffectExecutionError::InvalidChoice
        | EffectExecutionError::InvalidTargetSelection
        | EffectExecutionError::UnaffordableCost => GameIntentError::IntentNotLegal,
        EffectExecutionError::InvalidDefinition
        | EffectExecutionError::InvalidRoll
        | EffectExecutionError::StepLimitExceeded => GameIntentError::EffectExecutionFailed,
    }
}

impl EngineControl {
    fn from_state(state: &InitialGameState) -> Self {
        Self {
            status: state.status,
            turn: state.turn,
            phase: state.phase,
            active_position: state.active_position,
            queued_phases: state.queued_phases.clone(),
            queued_effects: state.queued_effects.clone(),
            decision_point: state.decision_point.clone(),
        }
    }
}

fn advance_shared_table(
    world: &mut EffectWorld,
    active_villain_limit: u8,
) -> Result<Vec<EndTurnOutcome>, EffectExecutionError> {
    let mut outcomes = Vec::new();
    if let Some((location_id, next_location_id)) = world.advance_controlled_location()? {
        outcomes.push(EndTurnOutcome::LocationAdvanced {
            location_id,
            next_location_id,
        });
    }
    outcomes.extend(
        world
            .refill_villains(active_villain_limit)?
            .into_iter()
            .map(|villain_id| EndTurnOutcome::VillainRevealed { villain_id }),
    );
    Ok(outcomes)
}

fn perform_end_turn(
    world: &mut EffectWorld,
    actor_position: u8,
    active_villain_limit: u8,
    random: &mut dyn EffectRoller,
) -> Result<(Vec<EndTurnOutcome>, u64), GameIntentError> {
    let mut outcomes = Vec::new();
    outcomes.extend(
        advance_shared_table(world, active_villain_limit)
            .map_err(|_| GameIntentError::EffectExecutionFailed)?,
    );
    for from in [EffectZone::HeroPlayArea, EffectZone::HeroHand] {
        for card_id in world.card_ids_in_zone(actor_position, from) {
            world
                .move_card(&card_id, from, EffectZone::HeroDiscardPile)
                .map_err(|_| GameIntentError::EffectExecutionFailed)?;
            outcomes.push(EndTurnOutcome::CardMoved {
                card_id,
                from,
                to: EffectZone::HeroDiscardPile,
            });
        }
    }

    for resource in [EffectResource::Attack, EffectResource::Influence] {
        let before = world
            .hero_resource(actor_position, resource)
            .ok_or(GameIntentError::EffectExecutionFailed)?;
        world
            .reset_hero_resource(actor_position, resource, before)
            .map_err(|_| GameIntentError::EffectExecutionFailed)?;
        outcomes.push(EndTurnOutcome::ResourceReset { resource, before });
    }

    for (position, after) in world
        .recover_stunned_heroes()
        .map_err(|_| GameIntentError::EffectExecutionFailed)?
    {
        outcomes.push(EndTurnOutcome::HeroRecovered {
            position,
            before: 0,
            after,
        });
    }

    let mut samples_consumed = 0_u64;
    while world
        .card_ids_in_zone(actor_position, EffectZone::HeroHand)
        .len()
        < 5
    {
        if let Some(card_id) = world.top_card_id(actor_position, EffectZone::HeroDrawPile) {
            world
                .move_card(&card_id, EffectZone::HeroDrawPile, EffectZone::HeroHand)
                .map_err(|_| GameIntentError::EffectExecutionFailed)?;
            outcomes.push(EndTurnOutcome::CardMoved {
                card_id,
                from: EffectZone::HeroDrawPile,
                to: EffectZone::HeroHand,
            });
            continue;
        }

        let mut shuffled = world.card_ids_in_zone(actor_position, EffectZone::HeroDiscardPile);
        if shuffled.is_empty() {
            break;
        }
        for index in (1..shuffled.len()).rev() {
            let upper_exclusive =
                u32::try_from(index + 1).map_err(|_| GameIntentError::EffectExecutionFailed)?;
            let selected = random
                .sample_below(upper_exclusive)
                .ok_or(GameIntentError::RandomSourceFailed)?;
            let selected =
                usize::try_from(selected).map_err(|_| GameIntentError::RandomSourceFailed)?;
            if selected > index {
                return Err(GameIntentError::RandomSourceFailed);
            }
            shuffled.swap(index, selected);
            samples_consumed = samples_consumed
                .checked_add(1)
                .ok_or(GameIntentError::VersionOverflow)?;
        }
        for card_id in &shuffled {
            world
                .move_card(
                    card_id,
                    EffectZone::HeroDiscardPile,
                    EffectZone::HeroDrawPile,
                )
                .map_err(|_| GameIntentError::EffectExecutionFailed)?;
        }
        world
            .set_card_order(actor_position, EffectZone::HeroDrawPile, &shuffled)
            .map_err(|_| GameIntentError::EffectExecutionFailed)?;
        outcomes.push(EndTurnOutcome::PileShuffled {
            owner_position: actor_position,
            zone: EffectZone::HeroDrawPile,
            bottom_to_top: shuffled,
        });
    }

    Ok((outcomes, samples_consumed))
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
        || input.snapshot_version == HERO_ACTION_SNAPSHOT_VERSION
        || input.snapshot_version == PARTICIPANT_CHOICE_SNAPSHOT_VERSION
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
        || !valid_villain_capacity(&input.effect_world, input.active_villain_limit)
        || input
            .pending_choice
            .as_ref()
            .is_some_and(|choice| !choice.is_valid_for_positions(&participant_positions))
        || (input.pending_choice.is_some()
            && (input.status != GameStatus::InProgress
                || !matches!(
                    input.phase,
                    GamePhase::DarkArts | GamePhase::Villains | GamePhase::HeroActions
                )))
    {
        return Err(GameStateRestoreError::InvalidPlayers);
    }
    if !restored_control_is_valid(&input) {
        return Err(GameStateRestoreError::InvalidControlState);
    }

    let mut players = input.players;
    players.sort_by_key(InitialPlayer::position);

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
        active_villain_limit: input.active_villain_limit,
        players,
        effect_world: input.effect_world,
        last_effects: input.last_effects,
        pending_choice: input.pending_choice,
        queued_phases: input.queued_phases,
        queued_effects: input.queued_effects,
        decision_point: input.decision_point,
        last_turn_steps: input.last_turn_steps,
    })
}

fn valid_villain_capacity(world: &EffectWorld, capacity: u8) -> bool {
    world.entities_in(EffectZone::ActiveVillains).len() <= usize::from(capacity)
        && (capacity > 0
            || !world
                .entities()
                .any(|(_, entity)| entity.kind() == EffectEntityKind::Villain))
}

fn restored_control_is_valid(input: &GameStateRestoreInput<'_>) -> bool {
    if input.queued_phases.len() > 3
        || input.queued_effects.len() > 4_096
        || input.last_effects.len() > 4_096
        || input.last_turn_steps.len() > MAX_TURN_STEPS
        || input
            .last_turn_steps
            .iter()
            .any(|step| step.effects.len() > 4_096)
        || (!input.last_turn_steps.is_empty()
            && input.last_effects
                != input
                    .last_turn_steps
                    .iter()
                    .flat_map(|step| step.effects.iter().cloned())
                    .collect::<Vec<_>>())
        || input
            .queued_effects
            .iter()
            .any(|queued| !queued.is_valid_for_positions(&participant_positions(input)))
        || input
            .pending_choice
            .as_ref()
            .is_some_and(|choice| !choice.is_valid_for_positions(&participant_positions(input)))
    {
        return false;
    }
    if input.status != GameStatus::InProgress {
        return input.pending_choice.is_none()
            && input.queued_phases.is_empty()
            && input.queued_effects.is_empty()
            && input.decision_point.is_none();
    }
    match input.phase {
        GamePhase::DarkArts => {
            input.queued_phases
                == [
                    GamePhase::Villains,
                    GamePhase::HeroActions,
                    GamePhase::EndTurn,
                ]
                && automatic_decision_is_valid(input)
        }
        GamePhase::Villains => {
            input.queued_phases == [GamePhase::HeroActions, GamePhase::EndTurn]
                && automatic_decision_is_valid(input)
        }
        GamePhase::HeroActions => {
            input.queued_phases == [GamePhase::EndTurn]
                && if input.pending_choice.is_some() {
                    automatic_decision_is_valid(input)
                } else {
                    input.queued_effects.is_empty()
                        && input.decision_point
                            == Some(DecisionPoint::PlayerIntent {
                                responsible_position: input.active_position,
                            })
                }
        }
        GamePhase::EndTurn => {
            input.queued_phases.is_empty()
                && input.queued_effects.is_empty()
                && input.pending_choice.is_none()
                && input.decision_point == Some(DecisionPoint::Automatic)
        }
    }
}

fn participant_positions(input: &GameStateRestoreInput<'_>) -> Vec<u8> {
    input.players.iter().map(InitialPlayer::position).collect()
}

fn automatic_decision_is_valid(input: &GameStateRestoreInput<'_>) -> bool {
    match (&input.pending_choice, &input.decision_point) {
        (None, Some(DecisionPoint::Automatic)) => input.queued_effects.is_empty(),
        (Some(choice), Some(DecisionPoint::EffectChoice(decision))) => {
            choice == decision
                && input.queued_effects.as_slice() == choice.continuation.queue.as_slice()
        }
        _ => false,
    }
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
    if !matches!(&command, GameCommand::ResolveChoice { .. })
        && actor_position != state.active_position
    {
        return Err(GameCommandError::ActorNotActive);
    }
    let legal_intentions = legal_game_intentions(state, actor_position, effect_rules);
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
        GameCommand::PlayCard { card_id, targets } => decide_play_card(
            state,
            actor_position,
            &legal_intentions,
            effect_rules,
            die_roller,
            card_id,
            targets,
        ),
        GameCommand::AssignAttack { villain_id, amount } => decide_assign_attack(
            state,
            actor_position,
            &legal_intentions,
            effect_rules,
            die_roller,
            villain_id,
            amount,
        ),
        GameCommand::AcquireCard { card_id } => {
            decide_acquire_card(state, actor_position, &legal_intentions, card_id)
        }
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
    let effects::EffectResolution {
        mut outcomes,
        mut stop,
        rolls_consumed,
        queue,
    } = resolution;
    settle_structural_outcome(&effect_world, &mut outcomes, &mut stop);
    let prng_counter = next_prng_counter(state, rolls_consumed)?;
    let mut control = EngineControl::from_state(state);
    let mut steps = vec![TurnStep::new(state.phase, outcomes)];
    match &stop {
        EffectStop::Choice(choice) => {
            if queue != choice.continuation.queue {
                return Err(GameCommandError::EffectExecutionFailed);
            }
            control
                .queued_effects
                .clone_from(&choice.continuation.queue);
            control.decision_point = Some(DecisionPoint::EffectChoice(choice.clone()));
        }
        EffectStop::Stable => {
            if state.phase == GamePhase::DarkArts {
                steps.push(TurnStep::new(GamePhase::Villains, Vec::new()));
            }
            control.phase = GamePhase::HeroActions;
            control.queued_phases = vec![GamePhase::EndTurn];
            control.queued_effects.clear();
            control.decision_point = Some(DecisionPoint::PlayerIntent {
                responsible_position: state.active_position,
            });
        }
        EffectStop::Terminal(outcome) => {
            control.status = match outcome {
                EffectGameOutcome::Lost => GameStatus::Lost,
                EffectGameOutcome::Won => GameStatus::Won,
            };
            control.queued_phases.clear();
            control.queued_effects.clear();
            control.decision_point = None;
        }
    }
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
            steps,
            control,
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
        EffectExecutionError::InvalidChoice
        | EffectExecutionError::InvalidTargetSelection
        | EffectExecutionError::UnaffordableCost => GameCommandError::CommandNotLegal,
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

fn decide_play_card(
    state: &InitialGameState,
    actor_position: u8,
    legal_intentions: &LegalGameIntentions,
    effect_rules: &[EffectRule],
    die_roller: &mut dyn EffectRoller,
    card_id: String,
    targets: Vec<EffectTargetBinding>,
) -> Result<GameCommandDecision, GameCommandError> {
    legal_intentions
        .playable_cards
        .iter()
        .find(|playable| playable.card_id == card_id)
        .filter(|playable| target_bindings_match_slots(&targets, &playable.target_slots))
        .ok_or(GameCommandError::CommandNotLegal)?;
    let (_, card) = state
        .effect_world
        .entity(&card_id)
        .filter(|(zone, card)| {
            *zone == EffectZone::HeroHand
                && card.owner_position() == Some(actor_position)
                && matches!(
                    card.kind(),
                    EffectEntityKind::HogwartsCard | EffectEntityKind::StarterCard
                )
        })
        .ok_or(GameCommandError::CommandNotLegal)?;
    let rule_id = card
        .effect_rule_id()
        .ok_or(GameCommandError::CommandNotLegal)?;
    let matching_rules = effect_rules
        .iter()
        .filter(|rule| rule.id == rule_id && rule.trigger == EffectTrigger::Manual)
        .collect::<Vec<_>>();
    let [rule] = matching_rules.as_slice() else {
        return Err(GameCommandError::CommandNotLegal);
    };

    let state_version = state
        .state_version
        .checked_add(1)
        .ok_or(GameCommandError::VersionOverflow)?;
    let sequence = state
        .sequence
        .checked_add(1)
        .ok_or(GameCommandError::VersionOverflow)?;
    let mut effect_world = state.effect_world.clone();
    effect_world
        .move_to_back(
            &card_id,
            EffectZone::HeroHand,
            EffectZone::HeroPlayArea,
            Some(actor_position),
        )
        .map_err(|_| GameCommandError::CommandNotLegal)?;
    let resolution = effects::execute_effect_rule(
        &mut effect_world,
        actor_position,
        rule,
        effect_rules,
        &targets,
        die_roller,
    )
    .map_err(map_effect_execution_error)?;
    let effects::EffectResolution {
        mut outcomes,
        mut stop,
        rolls_consumed,
        ..
    } = resolution;
    settle_structural_outcome(&effect_world, &mut outcomes, &mut stop);
    let prng_counter = state
        .prng_counter
        .checked_add(rolls_consumed)
        .ok_or(GameCommandError::VersionOverflow)?;
    let mut event_effects = Vec::with_capacity(outcomes.len() + 2);
    event_effects.push(EffectOutcome::Moved {
        rule_id: "system:play-card".to_owned(),
        target_id: card_id.clone(),
        target_position: Some(actor_position),
        from: EffectZone::HeroHand,
        to: EffectZone::HeroPlayArea,
    });
    event_effects.extend(outcomes);
    let event = GameEvent::CardPlayed {
        sequence,
        state_version,
        turn: state.turn,
        actor_position,
        card_id,
        targets,
        effects: event_effects,
        stop,
        prng_counter,
    };
    let state = apply_game_event(state, &event).map_err(map_game_event_error)?;
    Ok(GameCommandDecision { state, event })
}

fn decide_assign_attack(
    state: &InitialGameState,
    actor_position: u8,
    legal_intentions: &LegalGameIntentions,
    effect_rules: &[EffectRule],
    die_roller: &mut dyn EffectRoller,
    villain_id: String,
    amount: u16,
) -> Result<GameCommandDecision, GameCommandError> {
    legal_intentions
        .attack_targets
        .iter()
        .find(|target| target.villain_id == villain_id)
        .filter(|target| amount > 0 && amount <= target.max_amount)
        .ok_or(GameCommandError::CommandNotLegal)?;
    let hero = state
        .effect_world
        .entities_in(EffectZone::Heroes)
        .iter()
        .find(|entity| entity.owner_position() == Some(actor_position))
        .ok_or(GameCommandError::CommandNotLegal)?;
    let available_attack = hero.resource(EffectResource::Attack);
    let (_, villain) = state
        .effect_world
        .entity(&villain_id)
        .filter(|(zone, entity)| {
            *zone == EffectZone::ActiveVillains && entity.kind() == EffectEntityKind::Villain
        })
        .ok_or(GameCommandError::CommandNotLegal)?;
    let villain_health = villain.resource(EffectResource::Health);
    if amount > available_attack || amount > villain_health {
        return Err(GameCommandError::CommandNotLegal);
    }
    let reward_rule_id = villain.reward_rule_id().map(str::to_owned);
    let sequence = state
        .sequence
        .checked_add(1)
        .ok_or(GameCommandError::VersionOverflow)?;
    let state_version = state
        .state_version
        .checked_add(1)
        .ok_or(GameCommandError::VersionOverflow)?;
    let mut effects = attack_assignment_effects(hero, actor_position, villain, amount);
    let defeated = amount == villain_health;
    if defeated {
        effects.push(EffectOutcome::Moved {
            rule_id: "system:defeat-villain".to_owned(),
            target_id: villain_id.clone(),
            target_position: None,
            from: EffectZone::ActiveVillains,
            to: EffectZone::VillainDiscard,
        });
    }
    let mut effect_world = state.effect_world.clone();
    effects::apply_effect_outcomes(&mut effect_world, &effects)
        .map_err(map_effect_execution_error)?;
    let (mut stop, rolls_consumed) =
        if let Some(reward_rule_id) = reward_rule_id.filter(|_| defeated) {
            let matching_rules = effect_rules
                .iter()
                .filter(|rule| {
                    rule.id == reward_rule_id && rule.trigger == EffectTrigger::VillainReward
                })
                .collect::<Vec<_>>();
            let [reward_rule] = matching_rules.as_slice() else {
                return Err(GameCommandError::EffectExecutionFailed);
            };
            let resolution = effects::execute_forced_effect_rule(
                &mut effect_world,
                actor_position,
                reward_rule,
                effect_rules,
                die_roller,
            )
            .map_err(map_effect_execution_error)?;
            effects.extend(resolution.outcomes);
            (resolution.stop, resolution.rolls_consumed)
        } else {
            (EffectStop::Stable, 0)
        };
    settle_structural_outcome(&effect_world, &mut effects, &mut stop);
    let prng_counter = next_prng_counter(state, rolls_consumed)?;
    let event = GameEvent::AttackAssigned {
        sequence,
        state_version,
        turn: state.turn,
        actor_position,
        villain_id,
        amount,
        effects,
        stop,
        prng_counter,
    };
    let state = apply_game_event(state, &event).map_err(map_game_event_error)?;
    Ok(GameCommandDecision { state, event })
}

fn decide_acquire_card(
    state: &InitialGameState,
    actor_position: u8,
    legal_intentions: &LegalGameIntentions,
    card_id: String,
) -> Result<GameCommandDecision, GameCommandError> {
    legal_intentions
        .acquisitions
        .iter()
        .find(|acquisition| acquisition.card_id == card_id)
        .ok_or(GameCommandError::CommandNotLegal)?;
    let hero = state
        .effect_world
        .entities_in(EffectZone::Heroes)
        .iter()
        .find(|entity| entity.owner_position() == Some(actor_position))
        .ok_or(GameCommandError::CommandNotLegal)?;
    let available_influence = hero.resource(EffectResource::Influence);
    let (_, card) = state
        .effect_world
        .entity(&card_id)
        .filter(|(zone, entity)| {
            *zone == EffectZone::Market
                && entity.kind() == EffectEntityKind::HogwartsCard
                && entity.owner_position().is_none()
        })
        .ok_or(GameCommandError::CommandNotLegal)?;
    let cost = card
        .influence_cost()
        .ok_or(GameCommandError::CommandNotLegal)?;
    if cost > available_influence {
        return Err(GameCommandError::CommandNotLegal);
    }
    let refill_card_id = state
        .effect_world
        .entities_in(EffectZone::HogwartsDeck)
        .first()
        .map(|entity| entity.id().to_owned());
    let effects =
        card_acquisition_effects(hero, actor_position, card, cost, refill_card_id.as_deref());
    let sequence = state
        .sequence
        .checked_add(1)
        .ok_or(GameCommandError::VersionOverflow)?;
    let state_version = state
        .state_version
        .checked_add(1)
        .ok_or(GameCommandError::VersionOverflow)?;
    let event = GameEvent::CardAcquired {
        sequence,
        state_version,
        turn: state.turn,
        actor_position,
        card_id,
        cost,
        refill_card_id,
        effects,
    };
    let state = apply_game_event(state, &event).map_err(map_game_event_error)?;
    Ok(GameCommandDecision { state, event })
}

fn attack_assignment_effects(
    hero: &EffectEntity,
    actor_position: u8,
    villain: &EffectEntity,
    amount: u16,
) -> Vec<EffectOutcome> {
    let available_attack = hero.resource(EffectResource::Attack);
    let villain_health = villain.resource(EffectResource::Health);
    vec![
        EffectOutcome::ResourceChanged {
            rule_id: "system:assign-attack".to_owned(),
            target_id: hero.id().to_owned(),
            target_position: Some(actor_position),
            resource: EffectResource::Attack,
            before: available_attack,
            after: available_attack - amount,
            cause: EffectChangeCause::Cost,
        },
        EffectOutcome::ResourceChanged {
            rule_id: "system:assign-attack".to_owned(),
            target_id: villain.id().to_owned(),
            target_position: None,
            resource: EffectResource::Health,
            before: villain_health,
            after: villain_health - amount,
            cause: EffectChangeCause::Effect,
        },
    ]
}

fn card_acquisition_effects(
    hero: &EffectEntity,
    actor_position: u8,
    card: &EffectEntity,
    cost: u16,
    refill_card_id: Option<&str>,
) -> Vec<EffectOutcome> {
    let available_influence = hero.resource(EffectResource::Influence);
    let mut effects = vec![
        EffectOutcome::ResourceChanged {
            rule_id: "system:acquire-card".to_owned(),
            target_id: hero.id().to_owned(),
            target_position: Some(actor_position),
            resource: EffectResource::Influence,
            before: available_influence,
            after: available_influence - cost,
            cause: EffectChangeCause::Cost,
        },
        EffectOutcome::Moved {
            rule_id: "system:acquire-card".to_owned(),
            target_id: card.id().to_owned(),
            target_position: Some(actor_position),
            from: EffectZone::Market,
            to: EffectZone::HeroDiscardPile,
        },
    ];
    if let Some(refill_card_id) = refill_card_id {
        effects.push(EffectOutcome::Moved {
            rule_id: "system:refill-market".to_owned(),
            target_id: refill_card_id.to_owned(),
            target_position: None,
            from: EffectZone::HogwartsDeck,
            to: EffectZone::Market,
        });
    }
    effects
}

fn map_effect_execution_error(error: EffectExecutionError) -> GameCommandError {
    match error {
        EffectExecutionError::InvalidChoice
        | EffectExecutionError::InvalidTargetSelection
        | EffectExecutionError::UnaffordableCost => GameCommandError::CommandNotLegal,
        EffectExecutionError::InvalidDefinition
        | EffectExecutionError::InvalidRoll
        | EffectExecutionError::StepLimitExceeded => GameCommandError::EffectExecutionFailed,
    }
}

fn map_game_event_error(error: GameEventError) -> GameCommandError {
    match error {
        GameEventError::VersionOverflow => GameCommandError::VersionOverflow,
        GameEventError::ActorNotChoiceResponsible => GameCommandError::ActorNotChoiceResponsible,
        GameEventError::ActorNotActive
        | GameEventError::EventNotApplicable
        | GameEventError::EffectTransitionInvalid
        | GameEventError::SequenceMismatch
        | GameEventError::StateVersionMismatch
        | GameEventError::TurnMismatch => GameCommandError::CommandNotLegal,
    }
}

fn target_bindings_match_slots(
    bindings: &[EffectTargetBinding],
    slots: &[LegalTargetSlot],
) -> bool {
    bindings.len() == slots.len()
        && slots.iter().all(|slot| {
            bindings
                .iter()
                .find(|binding| binding.selector_id == slot.selector_id)
                .is_some_and(|binding| {
                    let selected = binding.target_ids.iter().collect::<BTreeSet<_>>();
                    selected.len() == binding.target_ids.len()
                        && binding.target_ids.len() >= usize::from(slot.min)
                        && binding.target_ids.len() <= usize::from(slot.max)
                        && binding
                            .target_ids
                            .iter()
                            .all(|target_id| slot.target_ids.contains(target_id))
                })
        })
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

/// Returns the concrete hero intentions that are legal for one participant.
///
/// Target and stack order follows the canonical order stored by the world.
#[must_use]
pub fn legal_game_intentions(
    state: &InitialGameState,
    actor_position: u8,
    effect_rules: &[EffectRule],
) -> LegalGameIntentions {
    if state.status != GameStatus::InProgress
        || state.pending_choice.is_some()
        || actor_position != state.active_position
    {
        return LegalGameIntentions::default();
    }
    if state.phase == GamePhase::DarkArts {
        return LegalGameIntentions {
            complete_dark_arts: effect_action_is_affordable(
                &state.effect_world,
                actor_position,
                effect_rules,
                EffectTrigger::DarkArtsCompleted,
            ),
            ..LegalGameIntentions::default()
        };
    }

    if state.phase != GamePhase::HeroActions
        || state.decision_point
            != Some(DecisionPoint::PlayerIntent {
                responsible_position: actor_position,
            })
    {
        return LegalGameIntentions::default();
    }

    legal_hero_action_intentions(state, actor_position, effect_rules)
}

fn legal_hero_action_intentions(
    state: &InitialGameState,
    actor_position: u8,
    effect_rules: &[EffectRule],
) -> LegalGameIntentions {
    LegalGameIntentions {
        complete_dark_arts: false,
        playable_cards: legal_playable_cards(state, actor_position, effect_rules),
        attack_targets: legal_attack_targets(state, actor_position),
        acquisitions: legal_acquisitions(state, actor_position),
    }
}

fn legal_playable_cards(
    state: &InitialGameState,
    actor_position: u8,
    effect_rules: &[EffectRule],
) -> Vec<LegalPlayableCard> {
    state
        .effect_world
        .entities_in(EffectZone::HeroHand)
        .iter()
        .filter(|card| {
            card.owner_position() == Some(actor_position)
                && matches!(
                    card.kind(),
                    EffectEntityKind::HogwartsCard | EffectEntityKind::StarterCard
                )
        })
        .filter_map(|card| {
            let rule_id = card.effect_rule_id()?;
            let mut matching = effect_rules
                .iter()
                .filter(|rule| rule.id == rule_id && rule.trigger == EffectTrigger::Manual);
            let rule = matching.next()?;
            if matching.next().is_some()
                || !effect_action_is_affordable(
                    &state.effect_world,
                    actor_position,
                    std::slice::from_ref(rule),
                    EffectTrigger::Manual,
                )
            {
                return None;
            }
            let mut effect_world = state.effect_world.clone();
            effect_world
                .move_to_back(
                    card.id(),
                    EffectZone::HeroHand,
                    EffectZone::HeroPlayArea,
                    Some(actor_position),
                )
                .ok()?;
            let target_slots =
                effects::atomic_manual_target_slots(&effect_world, actor_position, rule)?
                    .into_iter()
                    .map(|slot| LegalTargetSlot {
                        selector_id: slot.selector_id,
                        min: slot.min,
                        max: slot.max,
                        target_ids: slot.target_ids,
                    })
                    .collect();
            Some(LegalPlayableCard {
                card_id: card.id().to_owned(),
                target_slots,
            })
        })
        .collect()
}

fn legal_attack_targets(state: &InitialGameState, actor_position: u8) -> Vec<LegalAttackTarget> {
    let available_attack = state
        .effect_world
        .hero_resource(actor_position, EffectResource::Attack)
        .unwrap_or(0);
    if available_attack == 0 {
        return Vec::new();
    }

    state
        .effect_world
        .entities_in(EffectZone::ActiveVillains)
        .iter()
        .filter(|entity| entity.kind() == EffectEntityKind::Villain)
        .filter_map(|villain| {
            let health = villain.resource(EffectResource::Health);
            let max_amount = available_attack.min(health);
            (max_amount > 0).then(|| LegalAttackTarget {
                villain_id: villain.id().to_owned(),
                max_amount,
            })
        })
        .collect()
}

fn legal_acquisitions(state: &InitialGameState, actor_position: u8) -> Vec<LegalAcquisition> {
    let available_influence = state
        .effect_world
        .hero_resource(actor_position, EffectResource::Influence)
        .unwrap_or(0);
    state
        .effect_world
        .entities_in(EffectZone::Market)
        .iter()
        .filter(|entity| {
            entity.kind() == EffectEntityKind::HogwartsCard && entity.owner_position().is_none()
        })
        .filter_map(|card| {
            let cost = card.influence_cost()?;
            (cost <= available_influence).then(|| LegalAcquisition {
                card_id: card.id().to_owned(),
                cost,
            })
        })
        .collect()
}

/// Returns the participant who must make the current human decision.
///
/// Automatic resolution points expose no legal command and therefore do not
/// require a connected participant. Availability remains an application-layer
/// concern and must never change the rule decision itself.
#[must_use]
#[cfg(test)]
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
        .find(|position| {
            let intentions = legal_game_intentions(state, *position, effect_rules);
            intentions.complete_dark_arts
                || !intentions.playable_cards.is_empty()
                || !intentions.attack_targets.is_empty()
                || !intentions.acquisitions.is_empty()
        })
}

#[derive(Clone, Copy)]
struct GameEventMetadata {
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
}

impl GameEvent {
    const fn metadata(&self) -> GameEventMetadata {
        match self {
            Self::DarkArtsCompleted {
                sequence,
                state_version,
                turn,
                actor_position,
                ..
            }
            | Self::ChoiceResolved {
                sequence,
                state_version,
                turn,
                actor_position,
                ..
            }
            | Self::TurnCompleted {
                sequence,
                state_version,
                turn,
                actor_position,
                ..
            }
            | Self::CardPlayed {
                sequence,
                state_version,
                turn,
                actor_position,
                ..
            }
            | Self::AttackAssigned {
                sequence,
                state_version,
                turn,
                actor_position,
                ..
            }
            | Self::CardAcquired {
                sequence,
                state_version,
                turn,
                actor_position,
                ..
            } => GameEventMetadata {
                sequence: *sequence,
                state_version: *state_version,
                turn: *turn,
                actor_position: *actor_position,
            },
        }
    }
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
    let metadata = event.metadata();
    match event {
        GameEvent::DarkArtsCompleted { .. } => apply_dark_arts_completed_event(state, event),
        GameEvent::ChoiceResolved { .. } => apply_choice_resolved_event(state, event),
        GameEvent::TurnCompleted { .. } => apply_turn_completed_event(state, event),
        GameEvent::CardPlayed {
            card_id,
            targets,
            effects,
            stop,
            prng_counter,
            ..
        } => apply_card_played_event(
            state,
            metadata,
            card_id,
            targets,
            effects,
            stop,
            *prng_counter,
        ),
        GameEvent::AttackAssigned {
            villain_id,
            amount,
            effects,
            stop,
            prng_counter,
            ..
        } => apply_attack_assigned_event(
            state,
            metadata,
            villain_id,
            *amount,
            effects,
            stop,
            *prng_counter,
        ),
        GameEvent::CardAcquired {
            card_id,
            cost,
            refill_card_id,
            effects,
            ..
        } => apply_card_acquired_event(
            state,
            metadata,
            card_id,
            *cost,
            refill_card_id.as_deref(),
            effects,
        ),
    }
}

fn apply_dark_arts_completed_event(
    state: &InitialGameState,
    event: &GameEvent,
) -> Result<InitialGameState, GameEventError> {
    let GameEvent::DarkArtsCompleted {
        sequence,
        state_version,
        turn,
        actor_position,
        effects,
        stop,
        prng_counter,
    } = event
    else {
        return Err(GameEventError::EventNotApplicable);
    };
    validate_legacy_effect_event_metadata(
        state,
        *sequence,
        *state_version,
        *turn,
        *actor_position,
    )?;
    let positions = state
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    if !effects::effect_transition_is_valid(effects, stop, &positions) {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    validate_effect_prng_counter(state.prng_counter, effects, *prng_counter)?;

    let mut next = state.clone();
    effects::apply_effect_outcomes(&mut next.effect_world, effects)
        .map_err(|_| GameEventError::EffectTransitionInvalid)?;
    next.sequence = *sequence;
    next.state_version = *state_version;
    next.prng_counter = *prng_counter;
    next.last_effects.clone_from(effects);
    next.last_turn_steps = vec![TurnStep::new(GamePhase::DarkArts, effects.clone())];
    apply_legacy_effect_stop(&mut next, stop);
    Ok(next)
}

fn validate_legacy_effect_event_metadata(
    state: &InitialGameState,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
) -> Result<(), GameEventError> {
    if state.status != GameStatus::InProgress
        || state.phase != GamePhase::DarkArts
        || state.pending_choice.is_some()
    {
        return Err(GameEventError::EventNotApplicable);
    }
    if actor_position != state.active_position {
        return Err(GameEventError::ActorNotActive);
    }
    validate_next_event_metadata(state, sequence, state_version, turn)
}

fn apply_legacy_effect_stop(state: &mut InitialGameState, stop: &EffectStop) {
    match stop {
        EffectStop::Choice(choice) => {
            state.queued_phases = vec![
                GamePhase::Villains,
                GamePhase::HeroActions,
                GamePhase::EndTurn,
            ];
            state.queued_effects.clone_from(&choice.continuation.queue);
            state.pending_choice = Some(choice.clone());
            state.decision_point = Some(DecisionPoint::EffectChoice(choice.clone()));
        }
        EffectStop::Stable => {
            state.phase = GamePhase::HeroActions;
            state.queued_phases = vec![GamePhase::EndTurn];
            state.queued_effects.clear();
            state.pending_choice = None;
            state.decision_point = Some(DecisionPoint::PlayerIntent {
                responsible_position: state.active_position,
            });
        }
        EffectStop::Terminal(outcome) => finish_terminal_state(state, *outcome),
    }
}

fn apply_choice_resolved_event(
    state: &InitialGameState,
    event: &GameEvent,
) -> Result<InitialGameState, GameEventError> {
    let GameEvent::ChoiceResolved {
        sequence,
        state_version,
        turn,
        actor_position,
        choice_id,
        choice_cause,
        selected_options,
        steps,
        control,
        prng_counter,
    } = event
    else {
        return Err(GameEventError::EventNotApplicable);
    };
    if state.status != GameStatus::InProgress {
        return Err(GameEventError::EventNotApplicable);
    }
    let pending = state
        .pending_choice
        .as_ref()
        .filter(|pending| {
            matches!(
                state.decision_point.as_ref(),
                Some(DecisionPoint::EffectChoice(decision)) if decision == *pending
            ) && state.queued_effects.as_slice() == pending.continuation.queue.as_slice()
        })
        .ok_or(GameEventError::EventNotApplicable)?;
    if *actor_position != pending.responsible_position {
        return Err(GameEventError::ActorNotChoiceResponsible);
    }
    let normalized = effects::normalize_effect_choice_selection(pending, selected_options)
        .ok_or(GameEventError::EffectTransitionInvalid)?;
    if choice_id != &pending.id
        || choice_cause != &pending.cause
        || normalized.as_slice() != selected_options
    {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    validate_next_event_metadata(state, *sequence, *state_version, *turn)?;
    validate_choice_steps(state, steps, control)?;
    let incremental_effects = steps
        .iter()
        .flat_map(|step| step.effects.iter())
        .cloned()
        .collect::<Vec<_>>();
    validate_effect_prng_counter(state.prng_counter, &incremental_effects, *prng_counter)?;

    let mut next = state.clone();
    for step in steps {
        effects::apply_effect_outcomes(&mut next.effect_world, &step.effects)
            .map_err(|_| GameEventError::EffectTransitionInvalid)?;
        append_phase_effects(&mut next.last_turn_steps, step.phase, &step.effects);
        next.last_effects.extend(step.effects.iter().cloned());
    }
    if next.last_effects.len() > 4_096 {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    next.sequence = *sequence;
    next.state_version = *state_version;
    next.prng_counter = *prng_counter;
    apply_engine_control(&mut next, control);
    Ok(next)
}

fn validate_choice_steps(
    state: &InitialGameState,
    steps: &[TurnStep],
    control: &EngineControl,
) -> Result<(), GameEventError> {
    let phases = steps.iter().map(TurnStep::phase).collect::<Vec<_>>();
    let valid_phases = match state.phase {
        GamePhase::DarkArts => matches!(
            phases.as_slice(),
            [GamePhase::DarkArts] | [GamePhase::DarkArts, GamePhase::Villains]
        ),
        GamePhase::Villains => phases.as_slice() == [GamePhase::Villains],
        GamePhase::HeroActions => phases.as_slice() == [GamePhase::HeroActions],
        GamePhase::EndTurn => false,
    };
    if !valid_phases
        || control.turn != state.turn
        || control.active_position != state.active_position
    {
        return Err(GameEventError::EffectTransitionInvalid);
    }

    let positions = state
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    for (index, step) in steps.iter().enumerate() {
        let is_last = index + 1 == steps.len();
        let stop = if is_last {
            effect_stop_from_control(control)
        } else {
            EffectStop::Stable
        };
        if !effects::effect_transition_is_valid(&step.effects, &stop, &positions) {
            return Err(GameEventError::EffectTransitionInvalid);
        }
    }
    if !choice_control_matches_steps(control, &phases, &positions) {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    Ok(())
}

fn effect_stop_from_control(control: &EngineControl) -> EffectStop {
    match (&control.decision_point, control.status) {
        (Some(DecisionPoint::EffectChoice(choice)), GameStatus::InProgress) => {
            EffectStop::Choice(choice.clone())
        }
        (_, GameStatus::Lost) => EffectStop::Terminal(EffectGameOutcome::Lost),
        (_, GameStatus::Won) => EffectStop::Terminal(EffectGameOutcome::Won),
        _ => EffectStop::Stable,
    }
}

fn choice_control_matches_steps(
    control: &EngineControl,
    phases: &[GamePhase],
    participant_positions: &[u8],
) -> bool {
    let last_phase = phases.last().copied();
    match (control.status, &control.decision_point) {
        (
            GameStatus::InProgress,
            Some(DecisionPoint::PlayerIntent {
                responsible_position,
            }),
        ) => {
            control.phase == GamePhase::HeroActions
                && control.queued_phases == [GamePhase::EndTurn]
                && control.queued_effects.is_empty()
                && *responsible_position == control.active_position
                && (phases == [GamePhase::HeroActions]
                    || phases == [GamePhase::Villains]
                    || phases == [GamePhase::DarkArts, GamePhase::Villains])
        }
        (GameStatus::InProgress, Some(DecisionPoint::EffectChoice(choice))) => {
            last_phase == Some(control.phase)
                && matches!(
                    control.phase,
                    GamePhase::DarkArts | GamePhase::Villains | GamePhase::HeroActions
                )
                && queued_phases_match_phase(control.phase, &control.queued_phases)
                && choice.is_valid_for_positions(participant_positions)
                && control.queued_effects.as_slice() == choice.continuation.queue.as_slice()
        }
        (GameStatus::Lost | GameStatus::Won, None) => {
            last_phase == Some(control.phase)
                && matches!(
                    control.phase,
                    GamePhase::DarkArts | GamePhase::Villains | GamePhase::HeroActions
                )
                && control.queued_phases.is_empty()
                && control.queued_effects.is_empty()
        }
        _ => false,
    }
}

fn queued_phases_match_phase(phase: GamePhase, queued_phases: &[GamePhase]) -> bool {
    match phase {
        GamePhase::DarkArts => {
            queued_phases
                == [
                    GamePhase::Villains,
                    GamePhase::HeroActions,
                    GamePhase::EndTurn,
                ]
        }
        GamePhase::Villains => queued_phases == [GamePhase::HeroActions, GamePhase::EndTurn],
        GamePhase::HeroActions => queued_phases == [GamePhase::EndTurn],
        GamePhase::EndTurn => queued_phases.is_empty(),
    }
}

fn validate_next_event_metadata(
    state: &InitialGameState,
    sequence: u64,
    state_version: u64,
    turn: u32,
) -> Result<(), GameEventError> {
    if turn != state.turn {
        return Err(GameEventError::TurnMismatch);
    }
    if state.sequence.checked_add(1) != Some(sequence) {
        return Err(GameEventError::SequenceMismatch);
    }
    if state.state_version.checked_add(1) != Some(state_version) {
        return Err(GameEventError::StateVersionMismatch);
    }
    Ok(())
}

fn validate_effect_prng_counter(
    previous_counter: u64,
    effects: &[EffectOutcome],
    next_counter: u64,
) -> Result<(), GameEventError> {
    let rolls = effects
        .iter()
        .filter(|outcome| matches!(outcome, EffectOutcome::DieRolled { .. }))
        .count();
    let consumed = u64::try_from(rolls).map_err(|_| GameEventError::VersionOverflow)?;
    if previous_counter.checked_add(consumed) != Some(next_counter) {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    Ok(())
}

fn apply_turn_completed_event(
    state: &InitialGameState,
    event: &GameEvent,
) -> Result<InitialGameState, GameEventError> {
    let GameEvent::TurnCompleted {
        sequence,
        state_version,
        turn,
        actor_position,
        end_turn,
        steps,
        control,
        prng_counter,
    } = event
    else {
        return Err(GameEventError::EventNotApplicable);
    };
    validate_turn_event_metadata(
        state,
        *sequence,
        *state_version,
        *turn,
        *actor_position,
        control,
    )?;
    let participant_positions = state
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    validate_turn_steps(steps, control, &participant_positions)?;

    let mut next = state.clone();
    apply_end_turn_outcomes(
        &mut next.effect_world,
        *actor_position,
        state.active_villain_limit,
        end_turn,
    )?;
    if control.phase == GamePhase::EndTurn
        && (control.status != GameStatus::Lost
            || next.effect_world.structural_game_outcome() != Some(EffectGameOutcome::Lost))
    {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    for step in steps.iter().skip(1) {
        effects::apply_effect_outcomes(&mut next.effect_world, &step.effects)
            .map_err(|_| GameEventError::EffectTransitionInvalid)?;
    }
    let consumed = random_samples_consumed(end_turn, steps)?;
    if state.prng_counter.checked_add(consumed) != Some(*prng_counter) {
        return Err(GameEventError::EffectTransitionInvalid);
    }

    validate_effect_world(&next)?;
    next.sequence = *sequence;
    next.state_version = *state_version;
    next.prng_counter = *prng_counter;
    apply_engine_control(&mut next, control);
    next.last_effects = steps
        .iter()
        .flat_map(|step| step.effects.iter().cloned())
        .collect();
    next.last_turn_steps.clone_from(steps);
    Ok(next)
}

fn validate_turn_event_metadata(
    state: &InitialGameState,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
    control: &EngineControl,
) -> Result<(), GameEventError> {
    if state.status != GameStatus::InProgress
        || state.phase != GamePhase::HeroActions
        || state.pending_choice.is_some()
        || state.decision_point
            != Some(DecisionPoint::PlayerIntent {
                responsible_position: actor_position,
            })
    {
        return Err(GameEventError::EventNotApplicable);
    }
    if actor_position != state.active_position {
        return Err(GameEventError::ActorNotActive);
    }
    if turn != state.turn {
        return Err(GameEventError::TurnMismatch);
    }
    if state.sequence.checked_add(1) != Some(sequence) {
        return Err(GameEventError::SequenceMismatch);
    }
    if state.state_version.checked_add(1) != Some(state_version) {
        return Err(GameEventError::StateVersionMismatch);
    }
    if control.phase != GamePhase::EndTurn {
        let expected_turn = state
            .turn
            .checked_add(1)
            .ok_or(GameEventError::VersionOverflow)?;
        let player_index = state
            .players
            .iter()
            .position(|player| player.position == actor_position)
            .ok_or(GameEventError::ActorNotActive)?;
        let expected_active = state.players[(player_index + 1) % state.players.len()].position;
        if control.turn != expected_turn || control.active_position != expected_active {
            return Err(GameEventError::EffectTransitionInvalid);
        }
    } else if control.turn != state.turn || control.active_position != actor_position {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    Ok(())
}

fn random_samples_consumed(
    end_turn: &[EndTurnOutcome],
    steps: &[TurnStep],
) -> Result<u64, GameEventError> {
    end_turn
        .iter()
        .map(|outcome| match outcome {
            EndTurnOutcome::PileShuffled { bottom_to_top, .. } => {
                bottom_to_top.len().saturating_sub(1)
            }
            EndTurnOutcome::LocationAdvanced { .. }
            | EndTurnOutcome::VillainRevealed { .. }
            | EndTurnOutcome::CardMoved { .. }
            | EndTurnOutcome::ResourceReset { .. }
            | EndTurnOutcome::HeroRecovered { .. } => 0,
        })
        .chain(
            steps
                .iter()
                .flat_map(|step| &step.effects)
                .map(|outcome| usize::from(matches!(outcome, EffectOutcome::DieRolled { .. }))),
        )
        .try_fold(0_u64, |total, consumed| {
            total
                .checked_add(u64::try_from(consumed).map_err(|_| GameEventError::VersionOverflow)?)
                .ok_or(GameEventError::VersionOverflow)
        })
}

fn apply_engine_control(state: &mut InitialGameState, control: &EngineControl) {
    state.status = control.status;
    state.turn = control.turn;
    state.phase = control.phase;
    state.active_position = control.active_position;
    state.queued_phases.clone_from(&control.queued_phases);
    state.queued_effects.clone_from(&control.queued_effects);
    state.decision_point.clone_from(&control.decision_point);
    state.pending_choice = match &control.decision_point {
        Some(DecisionPoint::EffectChoice(choice)) => Some(choice.clone()),
        Some(DecisionPoint::Automatic | DecisionPoint::PlayerIntent { .. }) | None => None,
    };
}

fn validate_turn_steps(
    steps: &[TurnStep],
    control: &EngineControl,
    participant_positions: &[u8],
) -> Result<(), GameEventError> {
    let phases = steps.iter().map(TurnStep::phase).collect::<Vec<_>>();
    let phase_sequence_is_valid = matches!(
        phases.as_slice(),
        [GamePhase::EndTurn, GamePhase::DarkArts]
            | [GamePhase::EndTurn, GamePhase::DarkArts, GamePhase::Villains]
    ) || (control.status != GameStatus::InProgress
        && phases.as_slice() == [GamePhase::EndTurn]);
    let end_turn_effects_valid = steps.first().is_some_and(|step| {
        if steps.len() == 1 {
            step.effects
                == [EffectOutcome::Terminal {
                    rule_id: STRUCTURAL_OUTCOME_RULE_ID.to_owned(),
                    outcome: EffectGameOutcome::Lost,
                }]
        } else {
            step.effects.is_empty()
        }
    });
    if !phase_sequence_is_valid || !end_turn_effects_valid {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    for (index, step) in steps.iter().enumerate().skip(1) {
        let is_last = index + 1 == steps.len();
        let stop = if is_last {
            effect_stop_from_control(control)
        } else {
            EffectStop::Stable
        };
        if !effects::effect_transition_is_valid(&step.effects, &stop, participant_positions) {
            return Err(GameEventError::EffectTransitionInvalid);
        }
    }
    if !event_control_matches_steps(control, &phases, participant_positions) {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    Ok(())
}

fn event_control_matches_steps(
    control: &EngineControl,
    phases: &[GamePhase],
    participant_positions: &[u8],
) -> bool {
    let last_phase = phases.last().copied();
    match (control.status, &control.decision_point) {
        (
            GameStatus::InProgress,
            Some(DecisionPoint::PlayerIntent {
                responsible_position,
            }),
        ) => {
            phases.len() == 3
                && control.phase == GamePhase::HeroActions
                && control.queued_phases == [GamePhase::EndTurn]
                && control.queued_effects.is_empty()
                && *responsible_position == control.active_position
        }
        (GameStatus::InProgress, Some(DecisionPoint::EffectChoice(choice))) => {
            last_phase == Some(control.phase)
                && queued_phases_match_phase(control.phase, &control.queued_phases)
                && choice.is_valid_for_positions(participant_positions)
                && control.queued_effects.as_slice() == choice.continuation.queue.as_slice()
        }
        (GameStatus::Lost | GameStatus::Won, None) => {
            last_phase == Some(control.phase)
                && matches!(
                    control.phase,
                    GamePhase::DarkArts | GamePhase::Villains | GamePhase::EndTurn
                )
                && control.queued_phases.is_empty()
                && control.queued_effects.is_empty()
        }
        _ => false,
    }
}

fn apply_end_turn_outcomes(
    world: &mut EffectWorld,
    actor_position: u8,
    active_villain_limit: u8,
    outcomes: &[EndTurnOutcome],
) -> Result<(), GameEventError> {
    let mut supplied = outcomes.iter();
    for outcome in advance_shared_table(world, active_villain_limit)
        .map_err(|_| GameEventError::EffectTransitionInvalid)?
    {
        expect_end_turn_outcome(&mut supplied, &outcome)?;
    }
    for from in [EffectZone::HeroPlayArea, EffectZone::HeroHand] {
        for card_id in world.card_ids_in_zone(actor_position, from) {
            expect_end_turn_outcome(
                &mut supplied,
                &EndTurnOutcome::CardMoved {
                    card_id: card_id.clone(),
                    from,
                    to: EffectZone::HeroDiscardPile,
                },
            )?;
            world
                .move_card(&card_id, from, EffectZone::HeroDiscardPile)
                .map_err(|_| GameEventError::EffectTransitionInvalid)?;
        }
    }

    for resource in [EffectResource::Attack, EffectResource::Influence] {
        let before = world
            .hero_resource(actor_position, resource)
            .ok_or(GameEventError::EffectTransitionInvalid)?;
        expect_end_turn_outcome(
            &mut supplied,
            &EndTurnOutcome::ResourceReset { resource, before },
        )?;
        world
            .reset_hero_resource(actor_position, resource, before)
            .map_err(|_| GameEventError::EffectTransitionInvalid)?;
    }

    replay_hero_recovery(world, &mut supplied)?;

    while world
        .card_ids_in_zone(actor_position, EffectZone::HeroHand)
        .len()
        < 5
    {
        if let Some(card_id) = world.top_card_id(actor_position, EffectZone::HeroDrawPile) {
            expect_end_turn_outcome(
                &mut supplied,
                &EndTurnOutcome::CardMoved {
                    card_id: card_id.clone(),
                    from: EffectZone::HeroDrawPile,
                    to: EffectZone::HeroHand,
                },
            )?;
            world
                .move_card(&card_id, EffectZone::HeroDrawPile, EffectZone::HeroHand)
                .map_err(|_| GameEventError::EffectTransitionInvalid)?;
            continue;
        }

        let mut current = world.card_ids_in_zone(actor_position, EffectZone::HeroDiscardPile);
        if current.is_empty() {
            break;
        }
        let Some(EndTurnOutcome::PileShuffled {
            owner_position,
            zone,
            bottom_to_top,
        }) = supplied.next()
        else {
            return Err(GameEventError::EffectTransitionInvalid);
        };
        if *owner_position != actor_position || *zone != EffectZone::HeroDrawPile {
            return Err(GameEventError::EffectTransitionInvalid);
        }
        let mut ordered = bottom_to_top.clone();
        current.sort();
        ordered.sort();
        if current != ordered || bottom_to_top.is_empty() {
            return Err(GameEventError::EffectTransitionInvalid);
        }
        for card_id in bottom_to_top {
            world
                .move_card(
                    card_id,
                    EffectZone::HeroDiscardPile,
                    EffectZone::HeroDrawPile,
                )
                .map_err(|_| GameEventError::EffectTransitionInvalid)?;
        }
        world
            .set_card_order(actor_position, EffectZone::HeroDrawPile, bottom_to_top)
            .map_err(|_| GameEventError::EffectTransitionInvalid)?;
    }

    if supplied.next().is_some() {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    Ok(())
}

fn expect_end_turn_outcome(
    supplied: &mut std::slice::Iter<'_, EndTurnOutcome>,
    expected: &EndTurnOutcome,
) -> Result<(), GameEventError> {
    if supplied.next() == Some(expected) {
        Ok(())
    } else {
        Err(GameEventError::EffectTransitionInvalid)
    }
}

fn replay_hero_recovery(
    world: &mut EffectWorld,
    supplied: &mut std::slice::Iter<'_, EndTurnOutcome>,
) -> Result<(), GameEventError> {
    for (position, after) in world
        .recover_stunned_heroes()
        .map_err(|_| GameEventError::EffectTransitionInvalid)?
    {
        expect_end_turn_outcome(
            supplied,
            &EndTurnOutcome::HeroRecovered {
                position,
                before: 0,
                after,
            },
        )?;
    }
    Ok(())
}

fn apply_effect_stop(state: &mut InitialGameState, stop: &EffectStop) {
    match stop {
        EffectStop::Choice(choice) => {
            state.queued_phases = vec![GamePhase::EndTurn];
            state.queued_effects.clone_from(&choice.continuation.queue);
            state.pending_choice = Some(choice.clone());
            state.decision_point = Some(DecisionPoint::EffectChoice(choice.clone()));
        }
        EffectStop::Stable => {
            state.phase = GamePhase::HeroActions;
            state.queued_phases = vec![GamePhase::EndTurn];
            state.queued_effects.clear();
            state.pending_choice = None;
            state.decision_point = Some(DecisionPoint::PlayerIntent {
                responsible_position: state.active_position,
            });
        }
        EffectStop::Terminal(outcome) => finish_terminal_state(state, *outcome),
    }
}

fn apply_card_played_event(
    state: &InitialGameState,
    metadata: GameEventMetadata,
    card_id: &str,
    targets: &[EffectTargetBinding],
    event_effects: &[EffectOutcome],
    stop: &EffectStop,
    prng_counter: u64,
) -> Result<InitialGameState, GameEventError> {
    if state.status != GameStatus::InProgress
        || state.phase != GamePhase::HeroActions
        || state.pending_choice.is_some()
    {
        return Err(GameEventError::EventNotApplicable);
    }
    validate_event_metadata(
        state,
        metadata.sequence,
        metadata.state_version,
        metadata.turn,
        metadata.actor_position,
    )?;
    let (_, card) = state
        .effect_world
        .entity(card_id)
        .filter(|(zone, card)| {
            *zone == EffectZone::HeroHand
                && card.owner_position() == Some(metadata.actor_position)
                && matches!(
                    card.kind(),
                    EffectEntityKind::HogwartsCard | EffectEntityKind::StarterCard
                )
        })
        .ok_or(GameEventError::EventNotApplicable)?;
    let card_rule_id = card
        .effect_rule_id()
        .ok_or(GameEventError::EventNotApplicable)?
        .to_owned();
    let Some((first, resolved_effects)) = event_effects.split_first() else {
        return Err(GameEventError::EffectTransitionInvalid);
    };
    let participant_positions = state
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    if first
        != &(EffectOutcome::Moved {
            rule_id: "system:play-card".to_owned(),
            target_id: card_id.to_owned(),
            target_position: Some(metadata.actor_position),
            from: EffectZone::HeroHand,
            to: EffectZone::HeroPlayArea,
        })
        || !outcomes_belong_to_window(&state.effect_world, resolved_effects, &card_rule_id)
        || !target_bindings_are_well_formed(targets)
        || !effects::effect_transition_is_valid(event_effects, stop, &participant_positions)
    {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    validate_event_prng_counter(state, event_effects, prng_counter)?;

    let mut next = state.clone();
    effects::apply_effect_outcomes(&mut next.effect_world, event_effects)
        .map_err(|_| GameEventError::EffectTransitionInvalid)?;
    validate_structural_outcome(
        &next.effect_world,
        split_structural_outcome(event_effects).1,
        stop,
    )?;
    let positions = next
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    if !next.effect_world.is_valid_for_positions(&positions) {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    next.sequence = metadata.sequence;
    next.state_version = metadata.state_version;
    next.prng_counter = prng_counter;
    next.last_effects.extend_from_slice(event_effects);
    if next.last_effects.len() > 4_096 {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    append_phase_effects(
        &mut next.last_turn_steps,
        GamePhase::HeroActions,
        event_effects,
    );
    apply_effect_stop(&mut next, stop);
    Ok(next)
}

fn validate_structural_outcome(
    world: &EffectWorld,
    structural_terminal: Option<EffectGameOutcome>,
    stop: &EffectStop,
) -> Result<(), GameEventError> {
    let expected_structural = world.structural_game_outcome();
    if structural_terminal.is_some_and(|outcome| {
        expected_structural != Some(outcome) || stop != &EffectStop::Terminal(outcome)
    }) || (structural_terminal.is_none()
        && matches!(stop, EffectStop::Stable)
        && expected_structural.is_some())
    {
        return Err(GameEventError::EffectTransitionInvalid);
    }

    Ok(())
}

fn split_structural_outcome(
    outcomes: &[EffectOutcome],
) -> (&[EffectOutcome], Option<EffectGameOutcome>) {
    match outcomes.split_last() {
        Some((EffectOutcome::Terminal { rule_id, outcome }, preceding))
            if rule_id == STRUCTURAL_OUTCOME_RULE_ID =>
        {
            (preceding, Some(*outcome))
        }
        _ => (outcomes, None),
    }
}

fn apply_attack_assigned_event(
    state: &InitialGameState,
    metadata: GameEventMetadata,
    villain_id: &str,
    amount: u16,
    event_effects: &[EffectOutcome],
    stop: &EffectStop,
    prng_counter: u64,
) -> Result<InitialGameState, GameEventError> {
    if state.status != GameStatus::InProgress
        || state.phase != GamePhase::HeroActions
        || state.pending_choice.is_some()
        || amount == 0
    {
        return Err(GameEventError::EventNotApplicable);
    }
    validate_event_metadata(
        state,
        metadata.sequence,
        metadata.state_version,
        metadata.turn,
        metadata.actor_position,
    )?;
    let hero = state
        .effect_world
        .entities_in(EffectZone::Heroes)
        .iter()
        .find(|entity| entity.owner_position() == Some(metadata.actor_position))
        .ok_or(GameEventError::EventNotApplicable)?;
    let available_attack = hero.resource(EffectResource::Attack);
    let (_, villain) = state
        .effect_world
        .entity(villain_id)
        .filter(|(zone, entity)| {
            *zone == EffectZone::ActiveVillains && entity.kind() == EffectEntityKind::Villain
        })
        .ok_or(GameEventError::EventNotApplicable)?;
    let villain_health = villain.resource(EffectResource::Health);
    if amount > available_attack || amount > villain_health {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    let reward_rule_id = villain.reward_rule_id().map(str::to_owned);
    let defeated = amount == villain_health;
    let mut expected_effects =
        attack_assignment_effects(hero, metadata.actor_position, villain, amount);
    if defeated {
        expected_effects.push(EffectOutcome::Moved {
            rule_id: "system:defeat-villain".to_owned(),
            target_id: villain_id.to_owned(),
            target_position: None,
            from: EffectZone::ActiveVillains,
            to: EffectZone::VillainDiscard,
        });
    }
    let Some((committed_effects, remaining_effects)) =
        event_effects.split_at_checked(expected_effects.len())
    else {
        return Err(GameEventError::EffectTransitionInvalid);
    };
    let (reward_effects, structural_terminal) = split_structural_outcome(remaining_effects);
    let participant_positions = state
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    if committed_effects != expected_effects
        || (!reward_effects.is_empty()
            && reward_rule_id.as_deref().is_none_or(|rule_id| {
                !outcomes_belong_to_window(&state.effect_world, reward_effects, rule_id)
            }))
        || (!defeated && !reward_effects.is_empty())
        || !effects::effect_transition_is_valid(event_effects, stop, &participant_positions)
    {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    validate_event_prng_counter(state, event_effects, prng_counter)?;
    let mut next = state.clone();
    effects::apply_effect_outcomes(&mut next.effect_world, event_effects)
        .map_err(|_| GameEventError::EffectTransitionInvalid)?;
    validate_structural_outcome(&next.effect_world, structural_terminal, stop)?;
    validate_effect_world(&next)?;
    next.sequence = metadata.sequence;
    next.state_version = metadata.state_version;
    next.prng_counter = prng_counter;
    next.last_effects.extend_from_slice(event_effects);
    if next.last_effects.len() > 4_096 {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    append_phase_effects(
        &mut next.last_turn_steps,
        GamePhase::HeroActions,
        event_effects,
    );
    apply_effect_stop(&mut next, stop);
    Ok(next)
}

fn apply_card_acquired_event(
    state: &InitialGameState,
    metadata: GameEventMetadata,
    card_id: &str,
    cost: u16,
    refill_card_id: Option<&str>,
    event_effects: &[EffectOutcome],
) -> Result<InitialGameState, GameEventError> {
    if state.status != GameStatus::InProgress
        || state.phase != GamePhase::HeroActions
        || state.pending_choice.is_some()
    {
        return Err(GameEventError::EventNotApplicable);
    }
    validate_event_metadata(
        state,
        metadata.sequence,
        metadata.state_version,
        metadata.turn,
        metadata.actor_position,
    )?;
    let hero = state
        .effect_world
        .entities_in(EffectZone::Heroes)
        .iter()
        .find(|entity| entity.owner_position() == Some(metadata.actor_position))
        .ok_or(GameEventError::EventNotApplicable)?;
    let available_influence = hero.resource(EffectResource::Influence);
    let (_, card) = state
        .effect_world
        .entity(card_id)
        .filter(|(zone, entity)| {
            *zone == EffectZone::Market
                && entity.kind() == EffectEntityKind::HogwartsCard
                && entity.owner_position().is_none()
        })
        .ok_or(GameEventError::EventNotApplicable)?;
    if card.influence_cost() != Some(cost) || cost > available_influence {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    let expected_refill = state
        .effect_world
        .entities_in(EffectZone::HogwartsDeck)
        .first()
        .map(|entity| entity.id().to_owned());
    if refill_card_id != expected_refill.as_deref() {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    let expected_effects = card_acquisition_effects(
        hero,
        metadata.actor_position,
        card,
        cost,
        expected_refill.as_deref(),
    );
    if event_effects != expected_effects {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    let mut next = state.clone();
    effects::apply_effect_outcomes(&mut next.effect_world, event_effects)
        .map_err(|_| GameEventError::EffectTransitionInvalid)?;
    validate_effect_world(&next)?;
    next.sequence = metadata.sequence;
    next.state_version = metadata.state_version;
    next.last_effects.extend_from_slice(event_effects);
    if next.last_effects.len() > 4_096 {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    append_phase_effects(
        &mut next.last_turn_steps,
        GamePhase::HeroActions,
        event_effects,
    );
    Ok(next)
}

fn validate_event_metadata(
    state: &InitialGameState,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
) -> Result<(), GameEventError> {
    if actor_position != state.active_position {
        return Err(GameEventError::ActorNotActive);
    }
    if turn != state.turn {
        return Err(GameEventError::TurnMismatch);
    }
    if state.sequence.checked_add(1) != Some(sequence) {
        return Err(GameEventError::SequenceMismatch);
    }
    if state.state_version.checked_add(1) != Some(state_version) {
        return Err(GameEventError::StateVersionMismatch);
    }
    Ok(())
}

fn validate_event_prng_counter(
    state: &InitialGameState,
    effects: &[EffectOutcome],
    prng_counter: u64,
) -> Result<(), GameEventError> {
    let rolled = effects
        .iter()
        .filter(|outcome| matches!(outcome, EffectOutcome::DieRolled { .. }))
        .count();
    let expected = state
        .prng_counter
        .checked_add(u64::try_from(rolled).map_err(|_| GameEventError::VersionOverflow)?)
        .ok_or(GameEventError::VersionOverflow)?;
    if prng_counter != expected {
        return Err(GameEventError::EffectTransitionInvalid);
    }
    Ok(())
}

fn target_bindings_are_well_formed(bindings: &[EffectTargetBinding]) -> bool {
    let selector_ids = bindings
        .iter()
        .map(|binding| binding.selector_id.as_str())
        .collect::<BTreeSet<_>>();
    selector_ids.len() == bindings.len()
        && bindings.iter().all(|binding| {
            !binding.selector_id.is_empty()
                && binding
                    .target_ids
                    .iter()
                    .all(|target_id| !target_id.is_empty())
                && binding.target_ids.iter().collect::<BTreeSet<_>>().len()
                    == binding.target_ids.len()
        })
}

fn outcomes_belong_to_window(
    world: &EffectWorld,
    outcomes: &[EffectOutcome],
    primary_rule: &str,
) -> bool {
    let rewards = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            EffectOutcome::Moved {
                rule_id,
                target_id,
                from: EffectZone::ActiveVillains,
                to: EffectZone::VillainDiscard,
                target_position: None,
            } if rule_id == "system:defeat-villain" => world
                .entity(target_id)
                .and_then(|(_, entity)| entity.reward_rule_id()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    outcomes.iter().all(|outcome| outcome_belongs_to_rule(outcome, primary_rule)
        || rewards.contains(effect_outcome_rule_id(outcome))
        || matches!(outcome,
            EffectOutcome::Moved { rule_id, from: EffectZone::ActiveVillains, to: EffectZone::VillainDiscard, target_position: None, .. }
                if rule_id == "system:defeat-villain")
        || matches!(outcome, EffectOutcome::Terminal { rule_id, .. } if rule_id == STRUCTURAL_OUTCOME_RULE_ID))
}

fn outcome_belongs_to_rule(outcome: &EffectOutcome, rule_id: &str) -> bool {
    if effect_outcome_rule_id(outcome) == rule_id {
        return true;
    }
    if effect_outcome_rule_id(outcome) != "system:stunned" {
        return false;
    }
    matches!(
        outcome,
        EffectOutcome::Moved {
            from: EffectZone::HeroHand,
            to: EffectZone::HeroDiscardPile,
            target_position: Some(_),
            ..
        } | EffectOutcome::ResourceChanged {
            resource: EffectResource::Attack | EffectResource::Influence,
            after: 0,
            cause: EffectChangeCause::Effect,
            target_position: Some(_),
            ..
        } | EffectOutcome::ResourceChanged {
            resource: EffectResource::Control,
            cause: EffectChangeCause::Effect,
            target_position: None,
            ..
        }
    )
}

fn effect_outcome_rule_id(outcome: &EffectOutcome) -> &str {
    match outcome {
        EffectOutcome::DieRolled { rule_id, .. }
        | EffectOutcome::Moved { rule_id, .. }
        | EffectOutcome::NoOp { rule_id, .. }
        | EffectOutcome::ResourceChanged { rule_id, .. }
        | EffectOutcome::Terminal { rule_id, .. } => rule_id,
    }
}

fn validate_effect_world(state: &InitialGameState) -> Result<(), GameEventError> {
    let positions = state
        .players
        .iter()
        .map(InitialPlayer::position)
        .collect::<Vec<_>>();
    if state.effect_world.is_valid_for_positions(&positions) {
        Ok(())
    } else {
        Err(GameEventError::EffectTransitionInvalid)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct ScriptedRoller {
        rolls: VecDeque<u8>,
        samples: VecDeque<u32>,
    }

    impl ScriptedRoller {
        fn new(rolls: &[u8]) -> Self {
            Self {
                rolls: rolls.iter().copied().collect(),
                samples: VecDeque::new(),
            }
        }

        fn with_samples(samples: &[u32]) -> Self {
            Self {
                rolls: VecDeque::new(),
                samples: samples.iter().copied().collect(),
            }
        }
    }

    impl EffectRoller for ScriptedRoller {
        fn roll(&mut self, _die: EffectDie) -> Option<u8> {
            self.rolls.pop_front()
        }

        fn sample_below(&mut self, upper_exclusive: u32) -> Option<u32> {
            self.samples
                .pop_front()
                .filter(|sample| *sample < upper_exclusive)
        }
    }

    const CONTENT: ContentSelection<'static> = ContentSelection {
        adventure_id: "adventure:001",
        content_version: "fixture-v1",
        ruleset_version: "fixture-rules-v1",
        manifest_digest: "blake3:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        manifest_version: 1,
        playable: true,
        initial_entities: &[],
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
            active_villain_limit: state.active_villain_limit(),
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
            queued_phases: state.queued_phases().to_vec(),
            queued_effects: state.queued_effects().to_vec(),
            decision_point: state.decision_point().cloned(),
            last_turn_steps: state.last_turn_steps().to_vec(),
        }
    }

    fn four_participants() -> Vec<LobbyParticipant> {
        [
            (ParticipantRole::Host, HeroId::Harry),
            (ParticipantRole::Guest, HeroId::Hermione),
            (ParticipantRole::Guest, HeroId::Neville),
            (ParticipantRole::Guest, HeroId::Ron),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (role, hero))| LobbyParticipant {
            role,
            position: u8::try_from(index + 1).expect("four fixture positions fit in u8"),
            hero: Some(hero),
            ready: true,
        })
        .collect()
    }

    fn ordered_automatic_rules() -> Vec<EffectRule> {
        vec![
            automatic_resource_rule(
                "rule:dark-arts:second",
                EffectTrigger::DarkArts,
                2,
                EffectResource::Attack,
                1,
            ),
            automatic_resource_rule(
                "rule:villains:first",
                EffectTrigger::Villains,
                1,
                EffectResource::Influence,
                1,
            ),
            automatic_resource_rule(
                "rule:dark-arts:first",
                EffectTrigger::DarkArts,
                1,
                EffectResource::Health,
                -1,
            ),
            automatic_resource_rule(
                "rule:villains:second",
                EffectTrigger::Villains,
                2,
                EffectResource::Health,
                -1,
            ),
        ]
    }

    fn automatic_resource_rule(
        id: &str,
        trigger: EffectTrigger,
        order: u16,
        resource: EffectResource,
        amount: i16,
    ) -> EffectRule {
        EffectRule {
            id: id.to_owned(),
            trigger,
            order,
            cost: Vec::new(),
            effect: EffectDefinition::Apply {
                target: actor_selector(EffectZone::Heroes),
                operation: EffectOperation::ModifyResource { resource, amount },
            },
        }
    }

    fn interrupted_sequence_rules() -> Vec<EffectRule> {
        vec![
            EffectRule {
                id: "rule:dark-arts:choice".to_owned(),
                trigger: EffectTrigger::DarkArts,
                order: 1,
                cost: Vec::new(),
                effect: EffectDefinition::Sequence {
                    effects: vec![
                        EffectDefinition::Apply {
                            target: actor_selector(EffectZone::Heroes),
                            operation: EffectOperation::ModifyResource {
                                resource: EffectResource::Attack,
                                amount: 1,
                            },
                        },
                        EffectDefinition::Choice {
                            audience: EffectChoiceAudience::Actor,
                            options: vec![
                                EffectDefinition::NoOp,
                                EffectDefinition::Apply {
                                    target: actor_selector(EffectZone::Heroes),
                                    operation: EffectOperation::ModifyResource {
                                        resource: EffectResource::Health,
                                        amount: -1,
                                    },
                                },
                            ],
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
            },
            EffectRule {
                id: "rule:dark-arts:after-choice".to_owned(),
                trigger: EffectTrigger::DarkArts,
                order: 2,
                cost: Vec::new(),
                effect: EffectDefinition::NoOp,
            },
        ]
    }

    fn each_hero_automatic_rules() -> ValidatedGameRules {
        ValidatedGameRules::new(vec![
            EffectRule {
                id: "rule:dark-arts:each-hero".to_owned(),
                trigger: EffectTrigger::DarkArts,
                order: 1,
                cost: Vec::new(),
                effect: EffectDefinition::Choice {
                    audience: EffectChoiceAudience::EachHero,
                    options: vec![
                        EffectDefinition::Apply {
                            target: actor_selector(EffectZone::Heroes),
                            operation: EffectOperation::ModifyResource {
                                resource: EffectResource::Attack,
                                amount: 1,
                            },
                        },
                        EffectDefinition::NoOp,
                    ],
                },
            },
            automatic_resource_rule(
                "rule:villains:after-choices",
                EffectTrigger::Villains,
                1,
                EffectResource::Influence,
                1,
            ),
        ])
        .expect("the participant-choice rules should be valid")
    }

    fn resolve_first_pending_option(
        engine: &GameEngine<'_>,
        state: &InitialGameState,
        actor_position: u8,
    ) -> Result<GameIntentDecision, GameIntentError> {
        let choice = state
            .pending_choice()
            .expect("the fixture should have a pending choice");
        let mut random = ScriptedRoller::new(&[]);
        engine.decide(
            GameIntentInput {
                state,
                actor_position,
                expected_state_version: state.state_version(),
                intent: PlayerIntent::ResolveChoice {
                    choice_id: choice.id.clone(),
                    selected_options: vec![choice.options[0].clone()],
                },
            },
            &mut random,
        )
    }

    fn snapshot_restore_input(state: &InitialGameState) -> GameStateRestoreInput<'_> {
        GameStateRestoreInput {
            snapshot_version: state.snapshot_version(),
            state_version: state.state_version(),
            sequence: state.sequence(),
            status: state.status(),
            turn: state.turn(),
            phase: state.phase(),
            active_position: state.active_position(),
            active_villain_limit: state.active_villain_limit(),
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
            queued_phases: state.queued_phases().to_vec(),
            queued_effects: state.queued_effects().to_vec(),
            decision_point: state.decision_point().cloned(),
            last_turn_steps: state.last_turn_steps().to_vec(),
        }
    }

    fn restore_snapshot(state: &InitialGameState) -> InitialGameState {
        restore_game_state(snapshot_restore_input(state))
            .expect("the valid snapshot should restore")
    }

    fn hero_actions_card_fixture(started: &InitialGameState) -> InitialGameState {
        let world = EffectWorld::new(vec![
            EffectEntityPlacement::new(
                EffectEntity::hero(1)
                    .with_resource(EffectResource::Attack, 2)
                    .with_resource(EffectResource::Influence, 3),
                EffectZone::Heroes,
            ),
            EffectEntityPlacement::new(EffectEntity::hero(2), EffectZone::Heroes),
            EffectEntityPlacement::new(
                EffectEntity::new("played", Some(1)),
                EffectZone::HeroPlayArea,
            ),
            EffectEntityPlacement::new(EffectEntity::new("hand-a", Some(1)), EffectZone::HeroHand),
            EffectEntityPlacement::new(EffectEntity::new("hand-b", Some(1)), EffectZone::HeroHand),
            EffectEntityPlacement::new(
                EffectEntity::new("draw-bottom", Some(1)),
                EffectZone::HeroDrawPile,
            ),
            EffectEntityPlacement::new(
                EffectEntity::new("draw-top", Some(1)),
                EffectZone::HeroDrawPile,
            ),
            EffectEntityPlacement::new(
                EffectEntity::new("discarded", Some(1)),
                EffectZone::HeroDiscardPile,
            ),
        ]);
        restore_game_state(GameStateRestoreInput {
            snapshot_version: started.snapshot_version(),
            state_version: started.state_version(),
            sequence: started.sequence(),
            status: started.status(),
            turn: started.turn(),
            phase: started.phase(),
            active_position: started.active_position(),
            active_villain_limit: started.active_villain_limit(),
            adventure_id: started.adventure_id(),
            content_version: started.content_version(),
            ruleset_version: started.ruleset_version(),
            manifest_digest: started.manifest_digest(),
            manifest_version: started.manifest_version(),
            prng_algorithm: started.prng_algorithm(),
            shuffle_algorithm: started.shuffle_algorithm(),
            sampling_algorithm: started.sampling_algorithm(),
            prng_counter: started.prng_counter(),
            players: started.players().to_vec(),
            effect_world: world,
            last_effects: started.last_effects().to_vec(),
            pending_choice: None,
            queued_phases: vec![GamePhase::EndTurn],
            queued_effects: Vec::new(),
            decision_point: Some(DecisionPoint::PlayerIntent {
                responsible_position: 1,
            }),
            last_turn_steps: started.last_turn_steps().to_vec(),
        })
        .expect("the hero-actions fixture should restore")
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
        assert_eq!(decision.state.phase, GamePhase::HeroActions);
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
    fn game_start_resolves_dark_arts_and_villains_in_declared_order_before_player_intent() {
        let participants = valid_participants();
        let rules = ValidatedGameRules::new(ordered_automatic_rules())
            .expect("the fixture rules should have an unambiguous phase order");
        let engine = GameEngine::new(&rules);
        let mut random = ScriptedRoller::new(&[]);

        let state = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut random,
            )
            .expect("automatic phases should reach the first player intent");

        assert_eq!(state.phase(), GamePhase::HeroActions);
        assert_eq!(
            state.decision_point(),
            Some(&DecisionPoint::PlayerIntent {
                responsible_position: 1,
            })
        );
        assert_eq!(state.queued_phases(), &[GamePhase::EndTurn]);
        assert_eq!(
            state
                .last_turn_steps()
                .iter()
                .map(TurnStep::phase)
                .collect::<Vec<_>>(),
            [GamePhase::DarkArts, GamePhase::Villains]
        );
        assert_eq!(
            state
                .last_turn_steps()
                .iter()
                .flat_map(TurnStep::effects)
                .map(EffectOutcome::rule_id)
                .collect::<Vec<_>>(),
            [
                "rule:dark-arts:first",
                "rule:dark-arts:second",
                "rule:villains:first",
                "rule:villains:second",
            ]
        );
        assert_eq!(
            engine.legal_intent_types(&state, 1),
            vec![PlayerIntentType::EndHeroActions]
        );
        assert!(engine.legal_intent_types(&state, 2).is_empty());
        assert_eq!(
            state
                .effect_world()
                .hero_resource(1, EffectResource::Health),
            Some(8)
        );
    }

    #[test]
    fn player_intents_reject_stale_versions_and_non_responsible_players() {
        let participants = valid_participants();
        let rules = ValidatedGameRules::new(Vec::new()).expect("empty rules should be valid");
        let engine = GameEngine::new(&rules);
        let mut start_random = ScriptedRoller::new(&[]);
        let state = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut start_random,
            )
            .expect("automatic phases should reach hero actions");

        let mut unauthorized_random = ScriptedRoller::new(&[]);
        assert_eq!(
            engine.decide(
                GameIntentInput {
                    state: &state,
                    actor_position: 2,
                    expected_state_version: state.state_version(),
                    intent: PlayerIntent::EndHeroActions,
                },
                &mut unauthorized_random,
            ),
            Err(GameIntentError::ActorNotResponsible)
        );
        let mut stale_random = ScriptedRoller::new(&[]);
        assert_eq!(
            engine.decide(
                GameIntentInput {
                    state: &state,
                    actor_position: 1,
                    expected_state_version: state.state_version() - 1,
                    intent: PlayerIntent::EndHeroActions,
                },
                &mut stale_random,
            ),
            Err(GameIntentError::StaleStateVersion)
        );
        assert_eq!(state.turn(), 1);
        assert_eq!(state.active_position(), 1);
    }

    #[test]
    fn end_turn_handoff_wraps_from_the_last_position_to_the_first() {
        let participants = four_participants();
        let rules = ValidatedGameRules::new(Vec::new()).expect("empty rules should be valid");
        let engine = GameEngine::new(&rules);
        let mut start_random = ScriptedRoller::new(&[]);
        let started = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut start_random,
            )
            .expect("automatic phases should reach hero actions");
        let state = restore_game_state(GameStateRestoreInput {
            snapshot_version: started.snapshot_version(),
            state_version: started.state_version(),
            sequence: started.sequence(),
            status: started.status(),
            turn: started.turn(),
            phase: started.phase(),
            active_position: 4,
            active_villain_limit: started.active_villain_limit(),
            adventure_id: started.adventure_id(),
            content_version: started.content_version(),
            ruleset_version: started.ruleset_version(),
            manifest_digest: started.manifest_digest(),
            manifest_version: started.manifest_version(),
            prng_algorithm: started.prng_algorithm(),
            shuffle_algorithm: started.shuffle_algorithm(),
            sampling_algorithm: started.sampling_algorithm(),
            prng_counter: started.prng_counter(),
            players: started.players().to_vec(),
            effect_world: started.effect_world().clone(),
            last_effects: started.last_effects().to_vec(),
            pending_choice: None,
            queued_phases: vec![GamePhase::EndTurn],
            queued_effects: Vec::new(),
            decision_point: Some(DecisionPoint::PlayerIntent {
                responsible_position: 4,
            }),
            last_turn_steps: started.last_turn_steps().to_vec(),
        })
        .expect("the last participant should be a valid responsible actor");
        let mut random = ScriptedRoller::new(&[]);

        let decision = engine
            .decide(
                GameIntentInput {
                    state: &state,
                    actor_position: 4,
                    expected_state_version: state.state_version(),
                    intent: PlayerIntent::EndHeroActions,
                },
                &mut random,
            )
            .expect("the last participant should hand off to the first");

        assert_eq!(decision.state.turn(), 2);
        assert_eq!(decision.state.active_position(), 1);
        assert_eq!(decision.state.phase(), GamePhase::HeroActions);
    }

    #[test]
    fn restored_player_order_cannot_change_the_circular_handoff() {
        let participants = four_participants();
        let rules = ValidatedGameRules::new(Vec::new()).expect("empty rules should be valid");
        let engine = GameEngine::new(&rules);
        let mut start_random = ScriptedRoller::new(&[]);
        let started = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut start_random,
            )
            .expect("automatic phases should reach hero actions");
        let mut shuffled_players = started.players().to_vec();
        shuffled_players.swap(1, 2);
        let state = restore_game_state(GameStateRestoreInput {
            snapshot_version: started.snapshot_version(),
            state_version: started.state_version(),
            sequence: started.sequence(),
            status: started.status(),
            turn: started.turn(),
            phase: started.phase(),
            active_position: 1,
            active_villain_limit: started.active_villain_limit(),
            adventure_id: started.adventure_id(),
            content_version: started.content_version(),
            ruleset_version: started.ruleset_version(),
            manifest_digest: started.manifest_digest(),
            manifest_version: started.manifest_version(),
            prng_algorithm: started.prng_algorithm(),
            shuffle_algorithm: started.shuffle_algorithm(),
            sampling_algorithm: started.sampling_algorithm(),
            prng_counter: started.prng_counter(),
            players: shuffled_players,
            effect_world: started.effect_world().clone(),
            last_effects: started.last_effects().to_vec(),
            pending_choice: None,
            queued_phases: vec![GamePhase::EndTurn],
            queued_effects: Vec::new(),
            decision_point: Some(DecisionPoint::PlayerIntent {
                responsible_position: 1,
            }),
            last_turn_steps: started.last_turn_steps().to_vec(),
        })
        .expect("player storage order must not redefine table order");
        assert_eq!(
            state
                .players()
                .iter()
                .map(InitialPlayer::position)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );

        let mut random = ScriptedRoller::new(&[]);
        let decision = engine
            .decide(
                GameIntentInput {
                    state: &state,
                    actor_position: 1,
                    expected_state_version: state.state_version(),
                    intent: PlayerIntent::EndHeroActions,
                },
                &mut random,
            )
            .expect("position two must follow position one");

        assert_eq!(decision.state.active_position(), 2);
    }

    #[test]
    fn snapshot_round_trip_preserves_the_fifo_after_an_effect_choice() {
        let participants = valid_participants();
        let rules = ValidatedGameRules::new(interrupted_sequence_rules())
            .expect("the fixture rules should be ordered");
        let engine = GameEngine::new(&rules);
        let mut random = ScriptedRoller::new(&[]);
        let state = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut random,
            )
            .expect("the automatic phase should stop at its choice");

        assert_eq!(state.phase(), GamePhase::DarkArts);
        assert!(matches!(
            state.decision_point(),
            Some(DecisionPoint::EffectChoice(choice))
                if choice.continuation.choice_cursor.rule_id == "rule:dark-arts:choice"
                    && choice.continuation.choice_cursor.path
                        == [EffectPathSegment::SequenceEffect(1)]
        ));
        assert_eq!(
            state
                .queued_effects()
                .iter()
                .map(|queued| (queued.rule_id(), queued.path()))
                .collect::<Vec<_>>(),
            [
                (
                    "rule:dark-arts:choice",
                    &[EffectPathSegment::SequenceEffect(2)][..],
                ),
                ("rule:dark-arts:after-choice", &[][..]),
            ]
        );
        assert_eq!(
            state.queued_phases(),
            &[
                GamePhase::Villains,
                GamePhase::HeroActions,
                GamePhase::EndTurn,
            ]
        );

        let restored = restore_snapshot(&state);

        assert_eq!(restored, state);
    }

    #[test]
    fn game_engine_resolves_each_hero_choices_in_position_order_then_runs_villains() {
        let participants = valid_participants();
        let rules = each_hero_automatic_rules();
        let engine = GameEngine::new(&rules);
        let mut random = ScriptedRoller::new(&[]);
        let initial = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut random,
            )
            .expect("the turn should stop for the first participant");
        let first = initial
            .pending_choice()
            .expect("position one should receive the first choice");
        assert_eq!(first.responsible_position, 1);

        let first_decision = resolve_first_pending_option(&engine, &initial, 1)
            .expect("position one should resolve its assigned choice");
        assert_eq!(
            apply_game_event(&initial, &first_decision.event),
            Ok(first_decision.state.clone())
        );
        let second = first_decision
            .state
            .pending_choice()
            .expect("position two should receive the next choice");
        assert_eq!(second.responsible_position, 2);
        assert_eq!(first_decision.state.active_position(), 1);
        assert_eq!(
            first_decision
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Attack),
            Some(1)
        );
        let GameEvent::ChoiceResolved { steps, control, .. } = &first_decision.event else {
            panic!("the engine should publish a choice event");
        };
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].phase(), GamePhase::DarkArts);
        assert!(matches!(
            control.decision_point,
            Some(DecisionPoint::EffectChoice(ref choice)) if choice.responsible_position == 2
        ));

        assert_eq!(
            resolve_first_pending_option(&engine, &first_decision.state, 1),
            Err(GameIntentError::ActorNotChoiceResponsible)
        );

        let completed = resolve_first_pending_option(&engine, &first_decision.state, 2)
            .expect("position two should complete the participant sequence");
        assert_eq!(
            apply_game_event(&first_decision.state, &completed.event),
            Ok(completed.state.clone())
        );

        assert_eq!(completed.state.phase(), GamePhase::HeroActions);
        assert_eq!(completed.state.active_position(), 1);
        assert_eq!(
            completed
                .state
                .effect_world()
                .hero_resource(2, EffectResource::Attack),
            Some(1)
        );
        assert_eq!(
            completed
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Influence),
            Some(1)
        );
        let GameEvent::ChoiceResolved { steps, .. } = &completed.event else {
            panic!("the engine should publish the final choice event");
        };
        assert_eq!(
            steps.iter().map(TurnStep::phase).collect::<Vec<_>>(),
            [GamePhase::DarkArts, GamePhase::Villains]
        );
        assert_eq!(completed.state.last_turn_steps().len(), 2);
    }

    #[test]
    fn legal_intent_types_expose_effect_choice_only_to_its_current_responsible_participant() {
        let participants = valid_participants();
        let rules = each_hero_automatic_rules();
        let engine = GameEngine::new(&rules);
        let mut random = ScriptedRoller::new(&[]);
        let active_choice_state = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut random,
            )
            .expect("the active participant should receive the first choice");
        let active_choice = active_choice_state
            .pending_choice()
            .expect("the first choice should be pending");

        assert_eq!(active_choice_state.active_position(), 1);
        assert_eq!(active_choice.responsible_position, 1);
        assert_eq!(
            engine.legal_intent_types(&active_choice_state, 1),
            vec![PlayerIntentType::ResolveChoice]
        );
        assert!(
            engine
                .legal_intent_types(&active_choice_state, 2)
                .is_empty()
        );

        let other_choice_state = resolve_first_pending_option(&engine, &active_choice_state, 1)
            .expect("the first resolution should advance to the other participant")
            .state;
        let other_choice = other_choice_state
            .pending_choice()
            .expect("the other participant's choice should be pending");

        assert_eq!(other_choice_state.active_position(), 1);
        assert_eq!(other_choice.responsible_position, 2);
        assert!(engine.legal_intent_types(&other_choice_state, 1).is_empty());
        assert_eq!(
            engine.legal_intent_types(&other_choice_state, 2),
            vec![PlayerIntentType::ResolveChoice]
        );
    }

    #[test]
    fn restored_continuations_and_history_must_match_the_active_turn() {
        let participants = valid_participants();
        let rules = ValidatedGameRules::new(interrupted_sequence_rules())
            .expect("the fixture rules should be ordered");
        let engine = GameEngine::new(&rules);
        let mut random = ScriptedRoller::new(&[]);
        let state = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut random,
            )
            .expect("the automatic phase should stop at its choice");

        let mut wrong_actor = snapshot_restore_input(&state);
        let queued = wrong_actor
            .queued_effects
            .first()
            .expect("the interrupted sequence must preserve its queue");
        wrong_actor.queued_effects[0] = QueuedEffect::Definition {
            cursor: EffectCursor {
                rule_id: queued.rule_id().to_owned(),
                path: queued.path().to_vec(),
            },
            actor_position: 2,
        };
        assert_eq!(
            restore_game_state(wrong_actor),
            Err(GameStateRestoreError::InvalidControlState)
        );

        let mut incomplete_history = snapshot_restore_input(&state);
        incomplete_history.last_effects.clear();
        assert_eq!(
            restore_game_state(incomplete_history),
            Err(GameStateRestoreError::InvalidControlState)
        );

        let mut wrong_choice_actor = snapshot_restore_input(&state);
        let choice = wrong_choice_actor
            .pending_choice
            .as_mut()
            .expect("the interrupted sequence must preserve its choice");
        choice.responsible_position = 3;
        wrong_choice_actor.decision_point = Some(DecisionPoint::EffectChoice(choice.clone()));
        assert_eq!(
            restore_game_state(wrong_choice_actor),
            Err(GameStateRestoreError::InvalidPlayers)
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
        let restored_pending = restore_game_state(restore_input(
            &pending.state,
            PARTICIPANT_CHOICE_SNAPSHOT_VERSION,
        ))
        .expect("a version two participant choice should remain resumable");
        assert_eq!(restored_pending, pending.state);
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
    fn persisted_effect_cursor_indices_match_the_transport_bounds() {
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

        for (segment, should_restore) in [
            (
                EffectPathSegment::ChoiceOption(MAX_EFFECT_BRANCH_INDEX),
                true,
            ),
            (
                EffectPathSegment::SequenceEffect(MAX_EFFECT_BRANCH_INDEX),
                true,
            ),
            (EffectPathSegment::RollOutcome(MAX_EFFECT_ROLL_INDEX), true),
            (
                EffectPathSegment::ChoiceOption(MAX_EFFECT_BRANCH_INDEX + 1),
                false,
            ),
            (
                EffectPathSegment::SequenceEffect(MAX_EFFECT_BRANCH_INDEX + 1),
                false,
            ),
            (
                EffectPathSegment::RollOutcome(MAX_EFFECT_ROLL_INDEX + 1),
                false,
            ),
        ] {
            let mut candidate = pending.state.clone();
            let choice = candidate
                .pending_choice
                .as_mut()
                .expect("the fixture choice should remain pending");
            choice.continuation.choice_cursor.path = vec![segment];
            candidate.decision_point = Some(DecisionPoint::EffectChoice(choice.clone()));

            assert_eq!(
                restore_game_state(restore_input(&candidate, SNAPSHOT_VERSION)).is_ok(),
                should_restore,
                "unexpected restore result for {segment:?}"
            );
        }
    }

    #[test]
    fn persisted_turn_history_uses_the_same_three_step_limit_as_the_transport() {
        let participants = valid_participants();
        let rules = ValidatedGameRules::new(Vec::new()).expect("empty rules should be valid");
        let mut random = ScriptedRoller::new(&[]);
        let state = GameEngine::new(&rules)
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut random,
            )
            .expect("automatic phases should reach the first player intent");
        let mut invalid = state.clone();
        let extra_step = TurnStep::new(GamePhase::DarkArts, Vec::new());
        while invalid.last_turn_steps.len() <= MAX_TURN_STEPS {
            invalid.last_turn_steps.push(extra_step.clone());
        }
        invalid.last_effects = invalid
            .last_turn_steps
            .iter()
            .flat_map(|step| step.effects.iter().cloned())
            .collect();

        assert_eq!(
            restore_game_state(restore_input(&invalid, SNAPSHOT_VERSION)),
            Err(GameStateRestoreError::InvalidControlState)
        );
    }

    #[test]
    fn persisted_choice_requires_an_in_progress_coherent_phase_stop() {
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
        wrong_phase.phase = GamePhase::HeroActions;
        assert_eq!(
            restore_game_state(restore_input(&wrong_phase, SNAPSHOT_VERSION)),
            Err(GameStateRestoreError::InvalidControlState)
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
        let mut placements = initial
            .effect_world
            .entities()
            .map(|(zone, entity)| EffectEntityPlacement::new(entity.clone(), zone))
            .collect::<Vec<_>>();
        placements.push(EffectEntityPlacement::new(
            EffectEntity::new("x".repeat(257), Some(1)),
            EffectZone::HeroHand,
        ));
        initial.effect_world = EffectWorld::new(placements);

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
                active_villain_limit: 0,
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
                        .map(|player| {
                            EffectEntityPlacement::new(
                                EffectEntity::hero(player.position()),
                                EffectZone::Heroes,
                            )
                        })
                        .collect(),
                ),
                last_effects: Vec::new(),
                pending_choice: None,
                queued_phases: vec![
                    GamePhase::Villains,
                    GamePhase::HeroActions,
                    GamePhase::EndTurn,
                ],
                queued_effects: Vec::new(),
                decision_point: Some(DecisionPoint::Automatic),
                last_turn_steps: Vec::new(),
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
            id: None,
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
            order: 0,
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
            EffectEntityPlacement::new(EffectEntity::hero(1), EffectZone::Heroes),
            EffectEntityPlacement::new(EffectEntity::hero(2), EffectZone::Heroes),
            EffectEntityPlacement::new(
                EffectEntity::new("card:synthetic", Some(1)),
                EffectZone::HeroHand,
            ),
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

        assert_eq!(decision.state.phase(), GamePhase::HeroActions);
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
                .any(|(zone, entity)| {
                    entity.id() == "card:synthetic" && zone == EffectZone::HeroDiscardPile
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
                    id: None,
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

        assert_eq!(no_op.state.phase(), GamePhase::HeroActions);
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
                            id: None,
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
        assert_eq!(stable.state.phase(), GamePhase::HeroActions);
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
                    id: None,
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

        assert_eq!(stable.state.phase(), GamePhase::HeroActions);
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

    fn assert_end_turn_replay_rejects_noncanonical_payloads(
        state: &InitialGameState,
        valid_event: &GameEvent,
    ) {
        let mut omitted_end_turn = valid_event.clone();
        let GameEvent::TurnCompleted { end_turn, .. } = &mut omitted_end_turn else {
            panic!("ending Hero actions must create a turn-completed event");
        };
        end_turn.clear();
        assert_eq!(
            apply_game_event(state, &omitted_end_turn),
            Err(GameEventError::EffectTransitionInvalid)
        );

        let mut reordered_end_turn = valid_event.clone();
        let GameEvent::TurnCompleted { end_turn, .. } = &mut reordered_end_turn else {
            panic!("ending Hero actions must create a turn-completed event");
        };
        end_turn.swap(0, 1);
        assert_eq!(
            apply_game_event(state, &reordered_end_turn),
            Err(GameEventError::EffectTransitionInvalid)
        );

        let mut invalid_control = valid_event.clone();
        let GameEvent::TurnCompleted { control, .. } = &mut invalid_control else {
            panic!("ending Hero actions must create a turn-completed event");
        };
        control.phase = GamePhase::DarkArts;
        control.queued_phases = vec![
            GamePhase::Villains,
            GamePhase::HeroActions,
            GamePhase::EndTurn,
        ];
        control.decision_point = Some(DecisionPoint::Automatic);
        assert_eq!(
            apply_game_event(state, &invalid_control),
            Err(GameEventError::EffectTransitionInvalid)
        );
    }

    #[test]
    fn ending_hero_actions_discards_replenishes_and_passes_the_turn_circularly() {
        let participants = valid_participants();
        let rules = ValidatedGameRules::new(Vec::new()).expect("empty rules should be valid");
        let engine = GameEngine::new(&rules);
        let mut start_random = ScriptedRoller::new(&[]);
        let started = engine
            .start(
                StartGameInput {
                    actor_role: ParticipantRole::Host,
                    participants: &participants,
                    content: CONTENT,
                },
                &mut start_random,
            )
            .expect("the automatic phases should settle");
        let state = hero_actions_card_fixture(&started);
        let mut random = ScriptedRoller::with_samples(&[2, 1, 1]);

        let decision = engine
            .decide(
                GameIntentInput {
                    state: &state,
                    actor_position: 1,
                    expected_state_version: 1,
                    intent: PlayerIntent::EndHeroActions,
                },
                &mut random,
            )
            .expect("the active hero should end their actions");

        assert_eq!(decision.state.turn(), 2);
        assert_eq!(decision.state.active_position(), 2);
        assert_eq!(decision.state.phase(), GamePhase::HeroActions);
        assert_eq!(
            decision
                .state
                .last_turn_steps()
                .iter()
                .map(TurnStep::phase)
                .collect::<Vec<_>>(),
            [GamePhase::EndTurn, GamePhase::DarkArts, GamePhase::Villains]
        );
        assert_eq!(
            decision
                .state
                .effect_world()
                .cards_in_zone(1, EffectZone::HeroHand),
            ["draw-top", "draw-bottom", "hand-a", "played", "hand-b"]
        );
        assert_eq!(
            decision
                .state
                .effect_world()
                .cards_in_zone(1, EffectZone::HeroDrawPile),
            ["discarded"]
        );
        assert_eq!(
            decision
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Attack),
            Some(0)
        );
        assert_eq!(
            decision
                .state
                .effect_world()
                .hero_resource(1, EffectResource::Influence),
            Some(0)
        );
        assert_eq!(
            apply_game_event(&state, &decision.event),
            Ok(decision.state)
        );
        assert_end_turn_replay_rejects_noncanonical_payloads(&state, &decision.event);
    }

    #[test]
    fn mandatory_phase_roots_cannot_charge_a_player_or_share_identity() {
        let automatic_with_cost = EffectRule {
            id: "rule:automatic".to_owned(),
            trigger: EffectTrigger::DarkArts,
            order: 1,
            cost: vec![EffectResourceCost {
                resource: EffectResource::Influence,
                amount: 1,
            }],
            effect: EffectDefinition::NoOp,
        };
        assert_eq!(
            ValidatedGameRules::new(vec![automatic_with_cost]),
            Err(ValidatedGameRulesError::AutomaticRuleHasCost)
        );

        let duplicate = EffectRule {
            id: "rule:duplicate".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 1,
            cost: Vec::new(),
            effect: EffectDefinition::NoOp,
        };
        assert_eq!(
            ValidatedGameRules::new(vec![duplicate.clone(), duplicate]),
            Err(ValidatedGameRulesError::DuplicateRuleId)
        );
    }

    #[test]
    fn current_phase_choices_use_bounded_ids_while_legacy_choices_keep_their_identity() {
        let long_rule_id = "r".repeat(256);
        let current_effect_choice = [EffectRule {
            id: long_rule_id.clone(),
            trigger: EffectTrigger::DarkArts,
            order: 0,
            cost: Vec::new(),
            effect: EffectDefinition::Choice {
                audience: EffectChoiceAudience::Actor,
                options: vec![EffectDefinition::NoOp, EffectDefinition::NoOp],
            },
        }];
        let mut world = EffectWorld::new(vec![
            EffectEntityPlacement::new(EffectEntity::hero(1), EffectZone::Heroes),
            EffectEntityPlacement::new(EffectEntity::hero(2), EffectZone::Heroes),
        ]);
        let mut roller = ScriptedRoller::new(&[]);
        let resolution = effects::execute_effects(
            &mut world,
            1,
            &current_effect_choice,
            EffectTrigger::DarkArts,
            &mut roller,
        )
        .expect("a current effect choice should resolve to a bounded decision point");
        let EffectStop::Choice(effect_choice) = resolution.stop else {
            panic!("the effect should stop at its explicit choice");
        };
        assert_eq!(effect_choice.id, "choice:effect:0");
        assert!(effect_choice.id.len() <= 256);
        assert_eq!(effect_choice.cause, long_rule_id);

        let current_target_choice = [EffectRule {
            id: "rule:current-target".to_owned(),
            trigger: EffectTrigger::Villains,
            order: 0,
            cost: Vec::new(),
            effect: EffectDefinition::Apply {
                target: EffectSelector {
                    id: Some("target:any-hero".to_owned()),
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
        }];
        let mut roller = ScriptedRoller::new(&[]);
        let resolution = effects::execute_effects(
            &mut world,
            1,
            &current_target_choice,
            EffectTrigger::Villains,
            &mut roller,
        )
        .expect("a current target choice should resolve to a bounded decision point");
        let EffectStop::Choice(target_choice) = resolution.stop else {
            panic!("multiple targets should require a target choice");
        };
        assert_eq!(target_choice.id, "choice:target:0");
        assert!(target_choice.id.len() <= 256);

        let legacy_rule = [EffectRule {
            id: "rule:legacy".to_owned(),
            trigger: EffectTrigger::DarkArtsCompleted,
            order: 0,
            cost: Vec::new(),
            effect: EffectDefinition::Choice {
                audience: EffectChoiceAudience::Actor,
                options: vec![EffectDefinition::NoOp, EffectDefinition::NoOp],
            },
        }];
        let mut roller = ScriptedRoller::new(&[]);
        let resolution = effects::execute_effects(
            &mut world,
            1,
            &legacy_rule,
            EffectTrigger::DarkArtsCompleted,
            &mut roller,
        )
        .expect("the legacy decision harness should still expose its replay identity");
        let EffectStop::Choice(legacy_choice) = resolution.stop else {
            panic!("the legacy effect should stop at its explicit choice");
        };
        assert_eq!(legacy_choice.id, "rule:legacy:effect:0");
    }

    #[test]
    fn outcome_overflow_fails_without_mutating_the_effect_world() {
        let mut entities = vec![EffectEntityPlacement::new(
            EffectEntity::hero(1),
            EffectZone::Heroes,
        )];
        entities.extend((0..32).map(|index| {
            EffectEntityPlacement::new(
                EffectEntity::new(format!("villain:{index:02}"), None)
                    .with_resource(EffectResource::Health, 200),
                EffectZone::ActiveVillains,
            )
        }));
        let mut world = EffectWorld::new(entities);
        let original = world.clone();
        let rules = [EffectRule {
            id: "rule:outcome-overflow".to_owned(),
            trigger: EffectTrigger::Manual,
            order: 0,
            cost: Vec::new(),
            effect: EffectDefinition::Repeat {
                times: 129,
                effect: Box::new(EffectDefinition::Apply {
                    target: EffectSelector {
                        id: Some("target:all-villains".to_owned()),
                        zone: EffectZone::ActiveVillains,
                        owner: EffectTargetOwner::Any,
                        min: 1,
                        max: 32,
                        eligibility: Vec::new(),
                    },
                    operation: EffectOperation::ModifyResource {
                        resource: EffectResource::Health,
                        amount: -1,
                    },
                }),
            },
        }];
        let mut roller = ScriptedRoller::new(&[]);

        assert_eq!(
            effects::execute_effects(&mut world, 1, &rules, EffectTrigger::Manual, &mut roller,),
            Err(EffectExecutionError::StepLimitExceeded)
        );
        assert_eq!(world, original);
    }
}
