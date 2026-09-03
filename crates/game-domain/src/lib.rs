//! Pure game rules.
//!
//! Game decisions enter through typed inputs and leave as typed state without
//! depending on infrastructure, clocks, global randomness, or transport DTOs.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

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
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Harry => "harry",
            Self::Hermione => "hermione",
            Self::Neville => "neville",
            Self::Ron => "ron",
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
    pub snapshot_version: u16,
    pub state_version: u64,
    pub sequence: u64,
    pub status: GameStatus,
    pub turn: u32,
    pub phase: GamePhase,
    pub active_position: u8,
    pub adventure_id: String,
    pub content_version: String,
    pub ruleset_version: String,
    pub manifest_digest: String,
    pub manifest_version: u16,
    pub prng_algorithm: &'static str,
    pub shuffle_algorithm: &'static str,
    pub sampling_algorithm: &'static str,
    pub prng_counter: u64,
    pub players: Vec<InitialPlayer>,
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
    pub events: Vec<GameEvent>,
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
pub struct InitialPlayer {
    pub position: u8,
    pub hero: HeroId,
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
        || !input.content.manifest_digest.starts_with("blake3:")
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

    match (input.command, input.state.phase) {
        (GameCommand::CompleteDarkArts, GamePhase::DarkArts) => {
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
            let mut state = input.state.clone();
            state.state_version = state_version;
            state.sequence = sequence;
            state.phase = GamePhase::HeroAction;

            Ok(GameCommandDecision {
                state,
                events: vec![GameEvent::DarkArtsCompleted {
                    sequence,
                    state_version,
                    turn: input.state.turn,
                    actor_position: input.actor_position,
                }],
            })
        }
        (GameCommand::CompleteDarkArts, GamePhase::HeroAction) => {
            Err(GameCommandError::CommandNotLegal)
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
        manifest_digest: "blake3:0123456789abcdef",
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
    fn active_participant_completes_dark_arts_at_a_stable_decision_point() {
        let participants = valid_participants();
        let initial = initialize_game(StartGameInput {
            actor_role: ParticipantRole::Host,
            participants: &participants,
            content: CONTENT,
        })
        .expect("the complete lobby should start");

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
            decision.events,
            vec![GameEvent::DarkArtsCompleted {
                sequence: 1,
                state_version: 2,
                turn: 1,
                actor_position: 1,
            }]
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
}
