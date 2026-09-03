use game_domain::{
    GameEvent, GamePhase, GameStateRestoreInput, GameStatus, InitialGameState, InitialPlayer,
    SNAPSHOT_VERSION, restore_game_state,
};

use super::{
    GAME_EVENT_VERSION, PersistedGameEvent, PersistedPlayer, PersistedPrng, PersistedSnapshot,
    PersistedTurn, PersistedVersions, StoredCommandGame, StoredRoomParticipant, hero_id,
};
use crate::http_support::ApiError;

pub(super) fn verify_command_snapshot(
    game: &StoredCommandGame,
    persisted: &PersistedSnapshot,
) -> Result<(), ApiError> {
    let canonical_snapshot = serde_json::to_string(persisted)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
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

pub(super) fn command_domain_state(
    persisted: &PersistedSnapshot,
) -> Result<InitialGameState, ApiError> {
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
            Ok(InitialPlayer::new(
                player.position,
                hero_id(&player.hero_id)?,
            ))
        })
        .collect::<Result<Vec<_>, ApiError>>()?;

    restore_game_state(GameStateRestoreInput {
        snapshot_version: persisted.snapshot_version,
        state_version: persisted.state_version,
        sequence: persisted.sequence,
        status,
        turn: persisted.turn.number,
        phase,
        active_position: persisted.turn.active_position,
        adventure_id: &persisted.adventure_id,
        content_version: &persisted.versions.content,
        ruleset_version: &persisted.versions.ruleset,
        manifest_digest: &persisted.versions.manifest_digest,
        manifest_version: persisted.versions.manifest,
        prng_algorithm: &persisted.versions.prng,
        shuffle_algorithm: &persisted.versions.shuffle,
        sampling_algorithm: &persisted.versions.sampling,
        prng_counter: persisted.prng.counter,
        players,
    })
    .map_err(|error| ApiError::internal_with("match application operation", error))
}

pub(super) fn decode_persisted_snapshot(serialized: &str) -> Result<PersistedSnapshot, ApiError> {
    let snapshot: PersistedSnapshot = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    if snapshot.snapshot_version != SNAPSHOT_VERSION {
        return Err(ApiError::internal());
    }
    Ok(snapshot)
}

pub(super) fn decode_persisted_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedGameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    if event.event_version != GAME_EVENT_VERSION {
        return Err(ApiError::internal());
    }
    Ok(event)
}

pub(super) fn persisted_after_decision(
    current: &PersistedSnapshot,
    state: &InitialGameState,
) -> PersistedSnapshot {
    let mut next = current.clone();
    next.state_version = state.state_version();
    next.sequence = state.sequence();
    next.status = match state.status() {
        GameStatus::InProgress => "in_progress".to_owned(),
    };
    next.turn.number = state.turn();
    next.turn.phase = match state.phase() {
        GamePhase::DarkArts => "dark_arts".to_owned(),
        GamePhase::HeroAction => "hero_action".to_owned(),
    };
    next.turn.active_position = state.active_position();
    next.prng.counter = state.prng_counter();
    next
}

pub(super) fn persisted_event(event: GameEvent) -> Result<(&'static str, String), ApiError> {
    match event {
        GameEvent::DarkArtsCompleted {
            sequence,
            state_version,
            turn,
            actor_position,
        } => serde_json::to_string(&serde_json::json!({
            "event_version": GAME_EVENT_VERSION,
            "type": "dark_arts_completed",
            "sequence": sequence,
            "state_version": state_version,
            "turn": turn,
            "actor_position": actor_position,
        }))
        .map(|event| ("dark_arts_completed", event))
        .map_err(|error| ApiError::internal_with("match application operation", error)),
    }
}

pub(super) fn persisted_snapshot(
    state: &InitialGameState,
    participants: &[StoredRoomParticipant],
) -> PersistedSnapshot {
    PersistedSnapshot {
        snapshot_version: state.snapshot_version(),
        state_version: state.state_version(),
        sequence: state.sequence(),
        status: match state.status() {
            GameStatus::InProgress => "in_progress".to_owned(),
        },
        adventure_id: state.adventure_id().to_owned(),
        versions: PersistedVersions {
            content: state.content_version().to_owned(),
            ruleset: state.ruleset_version().to_owned(),
            manifest: state.manifest_version(),
            manifest_digest: state.manifest_digest().to_owned(),
            prng: state.prng_algorithm().to_owned(),
            shuffle: state.shuffle_algorithm().to_owned(),
            sampling: state.sampling_algorithm().to_owned(),
        },
        turn: PersistedTurn {
            number: state.turn(),
            phase: match state.phase() {
                GamePhase::DarkArts => "dark_arts".to_owned(),
                GamePhase::HeroAction => "hero_action".to_owned(),
            },
            active_position: state.active_position(),
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
            algorithm: state.prng_algorithm().to_owned(),
            counter: state.prng_counter(),
        },
    }
}
