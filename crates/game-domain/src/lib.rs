//! Pure game rules.
//!
//! Game decisions enter through typed inputs and leave as typed state without
//! depending on infrastructure, clocks, global randomness, or transport DTOs.

#![forbid(unsafe_code)]

use std::{collections::BTreeSet, str::FromStr};

pub const SNAPSHOT_VERSION: u16 = 1;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameStatus {
    InProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamePhase {
    DarkArts,
    HeroAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCommand {
    CompleteDarkArts,
}

#[derive(Debug, Clone, Copy)]
pub struct GameCommandInput<'a> {
    pub state: &'a InitialGameState,
    pub actor_position: u8,
    pub expected_state_version: u64,
    pub command: GameCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameCommandDecision {
    pub state: InitialGameState,
    pub event: GameEvent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameEvent {
    DarkArtsCompleted {
        sequence: u64,
        state_version: u64,
        turn: u32,
        actor_position: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCommandError {
    StaleStateVersion,
    ActorNotActive,
    CommandNotLegal,
    VersionOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameEventError {
    ActorNotActive,
    EventNotApplicable,
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
    if input.snapshot_version != SNAPSHOT_VERSION {
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
    if !(2..=4).contains(&input.players.len())
        || positions != expected_positions
        || heroes.len() != input.players.len()
        || !positions.contains(&input.active_position)
    {
        return Err(GameStateRestoreError::InvalidPlayers);
    }

    Ok(InitialGameState {
        snapshot_version: input.snapshot_version,
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
    if input.expected_state_version != input.state.state_version {
        return Err(GameCommandError::StaleStateVersion);
    }
    if input.actor_position != input.state.active_position {
        return Err(GameCommandError::ActorNotActive);
    }
    if !legal_game_commands(input.state, input.actor_position).contains(&input.command) {
        return Err(GameCommandError::CommandNotLegal);
    }

    match input.command {
        GameCommand::CompleteDarkArts => {
            let state_version = input
                .state
                .state_version
                .checked_add(1)
                .ok_or(GameCommandError::VersionOverflow)?;
            let sequence = input
                .state
                .sequence
                .checked_add(1)
                .ok_or(GameCommandError::VersionOverflow)?;
            let event = GameEvent::DarkArtsCompleted {
                sequence,
                state_version,
                turn: input.state.turn,
                actor_position: input.actor_position,
            };
            let state = apply_game_event(input.state, event).map_err(|error| match error {
                GameEventError::VersionOverflow => GameCommandError::VersionOverflow,
                GameEventError::ActorNotActive
                | GameEventError::EventNotApplicable
                | GameEventError::SequenceMismatch
                | GameEventError::StateVersionMismatch
                | GameEventError::TurnMismatch => GameCommandError::CommandNotLegal,
            })?;

            Ok(GameCommandDecision { state, event })
        }
    }
}

/// Returns the commands that the current game rules permit for one actor.
///
/// External gates such as database-clock expiration are applied by the
/// application before exposing this result.
#[must_use]
pub fn legal_game_commands(state: &InitialGameState, actor_position: u8) -> Vec<GameCommand> {
    if state.status == GameStatus::InProgress
        && state.phase == GamePhase::DarkArts
        && actor_position == state.active_position
    {
        vec![GameCommand::CompleteDarkArts]
    } else {
        Vec::new()
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
    event: GameEvent,
) -> Result<InitialGameState, GameEventError> {
    match event {
        GameEvent::DarkArtsCompleted {
            sequence,
            state_version,
            turn,
            actor_position,
        } => {
            if state.phase != GamePhase::DarkArts {
                return Err(GameEventError::EventNotApplicable);
            }
            if actor_position != state.active_position {
                return Err(GameEventError::ActorNotActive);
            }
            if turn != state.turn {
                return Err(GameEventError::TurnMismatch);
            }
            let expected_sequence = state
                .sequence
                .checked_add(1)
                .ok_or(GameEventError::VersionOverflow)?;
            if sequence != expected_sequence {
                return Err(GameEventError::SequenceMismatch);
            }
            let expected_state_version = state
                .state_version
                .checked_add(1)
                .ok_or(GameEventError::VersionOverflow)?;
            if state_version != expected_state_version {
                return Err(GameEventError::StateVersionMismatch);
            }

            let mut next = state.clone();
            next.sequence = sequence;
            next.state_version = state_version;
            next.phase = GamePhase::HeroAction;
            Ok(next)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn valid_lobby_creates_the_versioned_initial_state() {
        let participants = valid_participants();
        let state = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");

        assert_eq!(state.snapshot_version, 1);
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
            legal_game_commands(&initial, 1),
            vec![GameCommand::CompleteDarkArts]
        );
        assert!(legal_game_commands(&initial, 2).is_empty());

        let decision = decide_game_command(GameCommandInput {
            state: &initial,
            actor_position: 1,
            expected_state_version: 1,
            command: GameCommand::CompleteDarkArts,
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
            }
        );
        assert_eq!(
            apply_game_event(&initial, decision.event)
                .expect("the official event should reconstruct the decided state"),
            decision.state
        );
        assert!(legal_game_commands(&decision.state, 1).is_empty());
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

        assert_eq!(
            decide_game_command(GameCommandInput {
                state: &initial,
                actor_position: 1,
                expected_state_version: 0,
                command: GameCommand::CompleteDarkArts,
            }),
            Err(GameCommandError::StaleStateVersion)
        );
        assert_eq!(
            decide_game_command(GameCommandInput {
                state: &initial,
                actor_position: 2,
                expected_state_version: 1,
                command: GameCommand::CompleteDarkArts,
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

        let restored = restore_game_state(GameStateRestoreInput {
            snapshot_version: initial.snapshot_version(),
            state_version: initial.state_version(),
            sequence: initial.sequence(),
            status: initial.status(),
            turn: initial.turn(),
            phase: initial.phase(),
            active_position: initial.active_position(),
            adventure_id: initial.adventure_id(),
            content_version: initial.content_version(),
            ruleset_version: initial.ruleset_version(),
            manifest_digest: initial.manifest_digest(),
            manifest_version: initial.manifest_version(),
            prng_algorithm: initial.prng_algorithm(),
            shuffle_algorithm: initial.shuffle_algorithm(),
            sampling_algorithm: initial.sampling_algorithm(),
            prng_counter: initial.prng_counter(),
            players: initial.players().to_vec(),
        })
        .expect("the canonical initial snapshot should restore");

        assert_eq!(restored, initial);
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
}
