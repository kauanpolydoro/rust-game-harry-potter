use game_domain::{
    DecisionPoint, EffectChangeCause, EffectContinuation, EffectCursor, EffectDie, EffectEntity,
    EffectEntityKind, EffectEntityPlacement, EffectGameOutcome, EffectNoOpReason, EffectOutcome,
    EffectPathSegment, EffectResource, EffectStop, EffectTargetBinding, EffectWorld, EffectZone,
    EndTurnOutcome, EngineControl, GameEvent, GamePhase, GameStateRestoreInput, GameStatus,
    InitialGameState, InitialPlayer, MAX_EFFECT_BRANCH_INDEX, MAX_EFFECT_PATH_DEPTH,
    MAX_EFFECT_ROLL_INDEX, MAX_TURN_STEPS, PendingEffectChoice, PendingEffectChoiceKind,
    QueuedEffect, SNAPSHOT_VERSION, TurnStep, restore_game_state,
};
use serde::Deserialize;

use super::{
    GAME_EVENT_VERSION, PersistedDecisionPoint, PersistedEffectChoice, PersistedEffectContinuation,
    PersistedEffectCursor, PersistedEffectEntity, PersistedEffectOutcome,
    PersistedEffectPathSegment, PersistedEffectTargetBinding, PersistedEffects,
    PersistedEndTurnOutcome, PersistedEngineControl, PersistedEventChoice, PersistedGameEvent,
    PersistedLegacyEffectChoice, PersistedPlayer, PersistedPrng, PersistedQueuedEffect,
    PersistedSnapshot, PersistedTurn, PersistedTurnStep, PersistedVersions, StoredCommandGame,
    StoredGame, StoredRoomParticipant, hero_id,
};
use crate::http_support::ApiError;

const CLOSED_EFFECT_EVENT_VERSION: u16 = 2;
const CHOICE_EVENT_VERSION: u16 = 3;
const MAX_PERSISTED_JSON_BYTES: usize = 4 * 1_024 * 1_024;
const MAX_PERSISTED_JSON_TRANSPORT_BYTES: usize = 2 * MAX_PERSISTED_JSON_BYTES;
const MAX_EFFECT_ENTITIES: usize = 4_096;
const MAX_EFFECT_OUTCOMES: usize = 4_096;
const MAX_EFFECT_QUEUE: usize = 4_096;
const MAX_CHOICE_OPTIONS: usize = 4_096;
const MAX_END_TURN_OUTCOMES: usize = MAX_EFFECT_ENTITIES + 6;
const MAX_PILE_CARDS: usize = 4_096;
const MAX_IDENTIFIER_BYTES: usize = 256;

pub(super) struct StoredSnapshotMetadata<'a> {
    state_digest: &'a str,
    snapshot_version: i16,
    state_version: i64,
    sequence: i64,
    status: &'a str,
    adventure_id: &'a str,
    manifest_version: i16,
    content_version: &'a str,
    ruleset_version: &'a str,
    manifest_digest: &'a str,
    prng_algorithm: &'a str,
    prng_counter: i64,
    shuffle_algorithm: &'a str,
    sampling_algorithm: &'a str,
}

pub(super) trait StoredSnapshotRecord {
    fn snapshot_metadata(&self) -> StoredSnapshotMetadata<'_>;
}

macro_rules! impl_stored_snapshot_record {
    ($($record:ty),+ $(,)?) => {
        $(
            impl StoredSnapshotRecord for $record {
                fn snapshot_metadata(&self) -> StoredSnapshotMetadata<'_> {
                    StoredSnapshotMetadata {
                        state_digest: &self.state_digest,
                        snapshot_version: self.snapshot_version,
                        state_version: self.state_version,
                        sequence: self.sequence,
                        status: &self.status,
                        adventure_id: &self.adventure_id,
                        manifest_version: self.manifest_version,
                        content_version: &self.content_version,
                        ruleset_version: &self.ruleset_version,
                        manifest_digest: &self.manifest_digest,
                        prng_algorithm: &self.prng_algorithm,
                        prng_counter: self.prng_counter,
                        shuffle_algorithm: &self.shuffle_algorithm,
                        sampling_algorithm: &self.sampling_algorithm,
                    }
                }
            }
        )+
    };
}

impl_stored_snapshot_record!(StoredGame, StoredCommandGame);

pub(super) fn verify_persisted_snapshot(
    game: &impl StoredSnapshotRecord,
    persisted: &PersistedSnapshot,
) -> Result<(), ApiError> {
    let game = game.snapshot_metadata();
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
    validate_persisted_snapshot(persisted)?;
    let status = domain_game_status(&persisted.status)?;
    let phase = domain_game_phase(&persisted.turn.phase)?;
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
    let effect_world = domain_effect_world(&persisted.effects, &players)?;
    let last_effects = persisted
        .effects
        .outcomes
        .iter()
        .map(domain_effect_outcome)
        .collect::<Result<Vec<_>, _>>()?;
    let pending_choice = persisted
        .effects
        .choice
        .as_ref()
        .map(domain_effect_choice)
        .transpose()?;
    let control = domain_snapshot_control(
        persisted,
        status,
        phase,
        pending_choice.as_ref(),
        &last_effects,
    )?;

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
        effect_world,
        last_effects,
        pending_choice,
        queued_phases: control.queued_phases,
        queued_effects: control.queued_effects,
        decision_point: control.decision_point,
        last_turn_steps: control.last_turn_steps,
    })
    .map_err(|error| ApiError::internal_with("match application operation", error))
}

pub(super) fn decode_persisted_snapshot(serialized: &str) -> Result<PersistedSnapshot, ApiError> {
    validate_persisted_json_size(serialized)?;
    let snapshot: PersistedSnapshot = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    if !matches!(snapshot.snapshot_version, 1 | 2 | SNAPSHOT_VERSION)
        || (snapshot.snapshot_version == 1 && snapshot.effects.choice.is_some())
    {
        return Err(ApiError::internal());
    }
    validate_persisted_snapshot(&snapshot)?;
    Ok(snapshot)
}

pub(super) fn decode_persisted_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    validate_persisted_json_size(serialized)?;
    let header: PersistedEventHeader = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    let event = match (header.event_version, header.event_type.as_str()) {
        (1, "dark_arts_completed") => decode_v1_event(serialized)?,
        (CLOSED_EFFECT_EVENT_VERSION, "dark_arts_completed") => decode_v2_event(serialized)?,
        (CHOICE_EVENT_VERSION, "dark_arts_completed") => decode_v3_dark_arts_event(serialized)?,
        (CHOICE_EVENT_VERSION, "choice_resolved") => decode_v3_choice_event(serialized)?,
        (GAME_EVENT_VERSION, "turn_completed") => decode_v4_turn_event(serialized)?,
        (GAME_EVENT_VERSION, "choice_resolved") => decode_v4_choice_event(serialized)?,
        (
            CHOICE_EVENT_VERSION | GAME_EVENT_VERSION,
            "card_played" | "attack_assigned" | "card_acquired",
        ) => serde_json::from_str(serialized)
            .map_err(|error| ApiError::internal_with("match application operation", error))?,
        _ => return Err(ApiError::internal()),
    };
    validate_persisted_event(&event)?;
    Ok(event)
}

pub(super) fn validate_persisted_json_size(serialized: &str) -> Result<(), ApiError> {
    if serialized.len() > MAX_PERSISTED_JSON_TRANSPORT_BYTES
        || compact_json_size(serialized) > MAX_PERSISTED_JSON_BYTES
    {
        return Err(ApiError::internal());
    }
    Ok(())
}

fn valid_target_bindings(bindings: &[PersistedEffectTargetBinding]) -> bool {
    bindings.len() <= 4096
        && bindings.iter().all(|binding| {
            valid_choice_value(&binding.selector_id)
                && binding.target_ids.len() <= 4096
                && binding
                    .target_ids
                    .iter()
                    .all(|target_id| valid_choice_value(target_id))
                && binding
                    .target_ids
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == binding.target_ids.len()
        })
        && bindings
            .iter()
            .map(|binding| &binding.selector_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == bindings.len()
}

fn valid_choice_value(value: &str) -> bool {
    (1..=256).contains(&value.chars().count())
}

fn compact_json_size(serialized: &str) -> usize {
    let mut escaped = false;
    let mut in_string = false;
    serialized.bytes().fold(0_usize, |size, byte| {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            size.saturating_add(1)
        } else if byte.is_ascii_whitespace() {
            size
        } else {
            if byte == b'"' {
                in_string = true;
            }
            size.saturating_add(1)
        }
    })
}

#[derive(Deserialize)]
struct PersistedEventHeader {
    event_version: u16,
    #[serde(rename = "type")]
    event_type: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedV1GameEvent {
    event_version: u16,
    #[serde(rename = "type")]
    event_type: String,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedV2GameEvent {
    event_version: u16,
    #[serde(rename = "type")]
    event_type: String,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
    effects: Vec<PersistedEffectOutcome>,
    effect_stop: String,
    choice: Option<PersistedLegacyEffectChoice>,
    prng_counter: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedV3DarkArtsGameEvent {
    event_version: u16,
    #[serde(rename = "type")]
    event_type: String,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
    effects: Vec<PersistedEffectOutcome>,
    effect_stop: String,
    choice: Option<PersistedEffectChoice>,
    prng_counter: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedV3ChoiceGameEvent {
    event_version: u16,
    #[serde(rename = "type")]
    event_type: String,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
    choice_id: String,
    choice_cause: String,
    selected_options: Vec<String>,
    effects: Vec<PersistedEffectOutcome>,
    effect_stop: String,
    choice: Option<PersistedEffectChoice>,
    prng_counter: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedV4TurnGameEvent {
    event_version: u16,
    #[serde(rename = "type")]
    event_type: String,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
    end_turn: Vec<PersistedEndTurnOutcome>,
    steps: Vec<PersistedTurnStep>,
    control: PersistedEngineControl,
    prng_counter: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedV4ChoiceGameEvent {
    event_version: u16,
    #[serde(rename = "type")]
    event_type: String,
    sequence: u64,
    state_version: u64,
    turn: u32,
    actor_position: u8,
    choice_id: String,
    choice_cause: String,
    selected_options: Vec<String>,
    steps: Vec<PersistedTurnStep>,
    control: PersistedEngineControl,
    prng_counter: u64,
}

fn decode_v1_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedV1GameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    Ok(PersistedGameEvent {
        event_version: event.event_version,
        event_type: event.event_type,
        sequence: event.sequence,
        state_version: event.state_version,
        turn: event.turn,
        actor_position: event.actor_position,
        card_id: None,
        targets: Vec::new(),
        villain_id: None,
        amount: None,
        cost: None,
        refill_card_id: None,
        effects: Vec::new(),
        effect_stop: Some("stable".to_owned()),
        choice: None,
        choice_id: None,
        choice_cause: None,
        selected_options: None,
        end_turn: None,
        steps: None,
        control: None,
        prng_counter: Some(0),
    })
}

fn decode_v2_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedV2GameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    Ok(PersistedGameEvent {
        event_version: event.event_version,
        event_type: event.event_type,
        sequence: event.sequence,
        state_version: event.state_version,
        turn: event.turn,
        actor_position: event.actor_position,
        card_id: None,
        targets: Vec::new(),
        villain_id: None,
        amount: None,
        cost: None,
        refill_card_id: None,
        effects: event.effects,
        effect_stop: Some(event.effect_stop),
        choice: event.choice.map(PersistedEventChoice::Legacy),
        choice_id: None,
        choice_cause: None,
        selected_options: None,
        end_turn: None,
        steps: None,
        control: None,
        prng_counter: Some(event.prng_counter),
    })
}

fn decode_v3_dark_arts_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedV3DarkArtsGameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    Ok(PersistedGameEvent {
        event_version: event.event_version,
        event_type: event.event_type,
        sequence: event.sequence,
        state_version: event.state_version,
        turn: event.turn,
        actor_position: event.actor_position,
        card_id: None,
        targets: Vec::new(),
        villain_id: None,
        amount: None,
        cost: None,
        refill_card_id: None,
        effects: event.effects,
        effect_stop: Some(event.effect_stop),
        choice: event.choice.map(PersistedEventChoice::Current),
        choice_id: None,
        choice_cause: None,
        selected_options: None,
        end_turn: None,
        steps: None,
        control: None,
        prng_counter: Some(event.prng_counter),
    })
}

fn decode_v3_choice_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedV3ChoiceGameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    Ok(PersistedGameEvent {
        event_version: event.event_version,
        event_type: event.event_type,
        sequence: event.sequence,
        state_version: event.state_version,
        turn: event.turn,
        actor_position: event.actor_position,
        card_id: None,
        targets: Vec::new(),
        villain_id: None,
        amount: None,
        cost: None,
        refill_card_id: None,
        effects: event.effects,
        effect_stop: Some(event.effect_stop),
        choice: event.choice.map(PersistedEventChoice::Current),
        choice_id: Some(event.choice_id),
        choice_cause: Some(event.choice_cause),
        selected_options: Some(event.selected_options),
        end_turn: None,
        steps: None,
        control: None,
        prng_counter: Some(event.prng_counter),
    })
}

fn decode_v4_turn_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedV4TurnGameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    Ok(PersistedGameEvent {
        event_version: event.event_version,
        event_type: event.event_type,
        sequence: event.sequence,
        state_version: event.state_version,
        turn: event.turn,
        actor_position: event.actor_position,
        card_id: None,
        targets: Vec::new(),
        villain_id: None,
        amount: None,
        cost: None,
        refill_card_id: None,
        effects: Vec::new(),
        effect_stop: Some("stable".to_owned()),
        choice: None,
        choice_id: None,
        choice_cause: None,
        selected_options: None,
        end_turn: Some(event.end_turn),
        steps: Some(event.steps),
        control: Some(event.control),
        prng_counter: Some(event.prng_counter),
    })
}

fn decode_v4_choice_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedV4ChoiceGameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    Ok(PersistedGameEvent {
        event_version: event.event_version,
        event_type: event.event_type,
        sequence: event.sequence,
        state_version: event.state_version,
        turn: event.turn,
        actor_position: event.actor_position,
        card_id: None,
        targets: Vec::new(),
        villain_id: None,
        amount: None,
        cost: None,
        refill_card_id: None,
        effects: Vec::new(),
        effect_stop: Some("stable".to_owned()),
        choice: None,
        choice_id: Some(event.choice_id),
        choice_cause: Some(event.choice_cause),
        selected_options: Some(event.selected_options),
        end_turn: None,
        steps: Some(event.steps),
        control: Some(event.control),
        prng_counter: Some(event.prng_counter),
    })
}

fn validate_persisted_snapshot(snapshot: &PersistedSnapshot) -> Result<(), ApiError> {
    let control_field_count = [
        snapshot.queued_phases.is_some(),
        snapshot.queued_effects.is_some(),
        snapshot.decision_point.is_some(),
        snapshot.last_turn_steps.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    let has_structured_control = match control_field_count {
        0 => false,
        4 => true,
        _ => return Err(ApiError::internal()),
    };
    let version_shape_is_valid = match snapshot.snapshot_version {
        1 => !has_structured_control && snapshot.effects.choice.is_none(),
        2 => !has_structured_control,
        SNAPSHOT_VERSION => has_structured_control,
        _ => false,
    };
    if !version_shape_is_valid {
        return Err(ApiError::internal());
    }
    let bounded_identifiers = snapshot.snapshot_version == SNAPSHOT_VERSION;
    let valid_snapshot_identifier = |value: &str| {
        if bounded_identifiers {
            valid_identifier(value)
        } else {
            valid_legacy_identifier(value)
        }
    };
    let base_is_valid = snapshot.state_version > 0
        && snapshot.sequence.checked_add(1) == Some(snapshot.state_version)
        && snapshot.turn.number > 0
        && valid_position(snapshot.turn.active_position)
        && domain_game_status(&snapshot.status).is_ok()
        && domain_game_phase(&snapshot.turn.phase).is_ok()
        && valid_snapshot_identifier(&snapshot.adventure_id)
        && valid_snapshot_identifier(&snapshot.versions.content)
        && valid_snapshot_identifier(&snapshot.versions.ruleset)
        && snapshot.versions.manifest > 0
        && snapshot.versions.manifest_digest.len() <= MAX_IDENTIFIER_BYTES
        && valid_snapshot_identifier(&snapshot.versions.prng)
        && valid_snapshot_identifier(&snapshot.versions.shuffle)
        && valid_snapshot_identifier(&snapshot.versions.sampling)
        && snapshot.prng.algorithm == snapshot.versions.prng
        && snapshot.participants.len() <= 4
        && snapshot.participants.iter().all(|player| {
            valid_snapshot_identifier(&player.participant_id)
                && valid_position(player.position)
                && valid_snapshot_identifier(&player.hero_id)
        })
        && validate_persisted_effects(&snapshot.effects, bounded_identifiers);
    if !base_is_valid {
        return Err(ApiError::internal());
    }
    if has_structured_control {
        validate_structured_snapshot(snapshot)
    } else {
        Ok(())
    }
}

fn validate_structured_snapshot(snapshot: &PersistedSnapshot) -> Result<(), ApiError> {
    let (Some(queued_phases), Some(queued_effects), Some(decision_point), Some(last_turn_steps)) = (
        &snapshot.queued_phases,
        &snapshot.queued_effects,
        &snapshot.decision_point,
        &snapshot.last_turn_steps,
    ) else {
        return Err(ApiError::internal());
    };
    let flattened_effects = last_turn_steps
        .iter()
        .flat_map(|step| step.effects.iter().cloned())
        .collect::<Vec<_>>();
    let control = PersistedEngineControl {
        status: snapshot.status.clone(),
        turn: snapshot.turn.number,
        phase: snapshot.turn.phase.clone(),
        active_position: snapshot.turn.active_position,
        queued_phases: queued_phases.clone(),
        queued_effects: queued_effects.clone(),
        decision_point: decision_point.clone(),
    };
    let participant_positions = snapshot
        .participants
        .iter()
        .map(|participant| participant.position)
        .collect::<std::collections::BTreeSet<_>>();
    let queue_belongs_to_game = queued_effects.iter().all(|queued| match queued {
        PersistedQueuedEffect::Definition { actor_position, .. } => {
            participant_positions.contains(actor_position)
        }
        PersistedQueuedEffect::EffectChoice {
            responsible_position,
            ..
        } => participant_positions.contains(responsible_position),
    });
    let choice_state_is_coherent = match (decision_point, &snapshot.effects.choice) {
        (
            PersistedDecisionPoint::EffectChoice {
                choice: decision_choice,
            },
            Some(effect_choice),
        ) => {
            decision_choice == effect_choice
                && decision_choice.continuation.queue.as_slice() == queued_effects.as_slice()
                && participant_positions.contains(&decision_choice.responsible_position)
        }
        (PersistedDecisionPoint::EffectChoice { .. }, None) | (_, Some(_)) => false,
        (_, None) => true,
    };
    if !valid_engine_control(&control)
        || !queue_belongs_to_game
        || !choice_state_is_coherent
        || flattened_effects != snapshot.effects.outcomes
        || last_turn_steps.len() > MAX_TURN_STEPS
        || last_turn_steps.iter().any(|step| {
            !canonical_game_phase(&step.phase)
                || step.effects.len() > MAX_EFFECT_OUTCOMES
                || step
                    .effects
                    .iter()
                    .any(|outcome| !valid_effect_outcome(outcome, true))
        })
    {
        return Err(ApiError::internal());
    }
    Ok(())
}

fn validate_persisted_effects(effects: &PersistedEffects, require_address: bool) -> bool {
    effects.entities.len() <= MAX_EFFECT_ENTITIES
        && effects.outcomes.len() <= MAX_EFFECT_OUTCOMES
        && effects
            .entities
            .iter()
            .all(|entity| valid_effect_entity(entity, require_address))
        && effects
            .outcomes
            .iter()
            .all(|outcome| valid_effect_outcome(outcome, require_address))
        && effects.choice.as_ref().is_none_or(|choice| {
            valid_effect_choice(
                choice,
                require_address,
                if require_address {
                    MAX_EFFECT_PATH_DEPTH
                } else {
                    MAX_EFFECT_QUEUE
                },
            )
        })
}

fn valid_effect_entity(entity: &PersistedEffectEntity, bounded_identifiers: bool) -> bool {
    valid_identifier_for_version(&entity.id, bounded_identifiers)
        && entity.owner_position.is_none_or(valid_position)
        && domain_effect_zone(&entity.zone).is_ok()
        && entity
            .resources
            .keys()
            .all(|resource| domain_effect_resource(resource).is_ok())
}

fn valid_effect_choice(
    choice: &PersistedEffectChoice,
    bounded_identifiers: bool,
    max_path: usize,
) -> bool {
    let distinct_options = choice
        .options
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    valid_identifier_for_version(&choice.id, bounded_identifiers)
        && valid_identifier_for_version(&choice.cause, bounded_identifiers)
        && valid_position(choice.responsible_position)
        && match choice.kind.as_str() {
            "effect" => choice.min == 1 && choice.max == 1,
            "target" => choice.max > 0 && usize::from(choice.max) < choice.options.len(),
            _ => false,
        }
        && (2..=MAX_CHOICE_OPTIONS).contains(&choice.options.len())
        && distinct_options.len() == choice.options.len()
        && choice
            .options
            .iter()
            .all(|option| valid_identifier_for_version(option, bounded_identifiers))
        && choice.min <= choice.max
        && choice.max <= 32
        && usize::from(choice.max) <= choice.options.len()
        && choice.cause == choice.continuation.choice_cursor.rule_id
        && valid_effect_continuation(&choice.continuation, bounded_identifiers, max_path)
}

fn valid_legacy_effect_choice(choice: &PersistedLegacyEffectChoice) -> bool {
    let distinct_options = choice
        .options
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    valid_legacy_identifier(&choice.id)
        && valid_position(choice.responsible_position)
        && matches!(choice.kind.as_str(), "effect" | "target")
        && (2..=MAX_CHOICE_OPTIONS).contains(&choice.options.len())
        && distinct_options.len() == choice.options.len()
        && choice.options.iter().all(|option| !option.is_empty())
        && choice.min <= choice.max
        && usize::from(choice.max) <= choice.options.len()
        && legacy_choice_rule_id(choice).is_ok()
}

fn valid_effect_continuation(
    continuation: &PersistedEffectContinuation,
    bounded_identifiers: bool,
    max_path: usize,
) -> bool {
    valid_effect_cursor(&continuation.choice_cursor, bounded_identifiers, max_path)
        && continuation.queue.len() <= MAX_EFFECT_QUEUE
        && (1..=MAX_EFFECT_QUEUE).contains(&continuation.steps_completed)
        && continuation
            .queue
            .iter()
            .all(|queued| valid_queued_effect(queued, bounded_identifiers, max_path))
}

fn valid_queued_effect(
    queued: &PersistedQueuedEffect,
    bounded_identifiers: bool,
    max_path: usize,
) -> bool {
    match queued {
        PersistedQueuedEffect::Definition {
            cursor,
            actor_position,
        } => {
            valid_position(*actor_position)
                && valid_effect_cursor(cursor, bounded_identifiers, max_path)
        }
        PersistedQueuedEffect::EffectChoice {
            cursor,
            responsible_position,
        } => {
            valid_position(*responsible_position)
                && valid_effect_cursor(cursor, bounded_identifiers, max_path)
        }
    }
}

fn valid_effect_cursor(
    cursor: &PersistedEffectCursor,
    bounded_identifiers: bool,
    max_path: usize,
) -> bool {
    valid_identifier_for_version(&cursor.rule_id, bounded_identifiers)
        && cursor.path.len() <= max_path
        && (max_path != MAX_EFFECT_PATH_DEPTH
            || cursor.path.iter().all(|segment| match segment {
                PersistedEffectPathSegment::ChoiceOption { index }
                | PersistedEffectPathSegment::SequenceEffect { index } => {
                    *index <= MAX_EFFECT_BRANCH_INDEX
                }
                PersistedEffectPathSegment::RollOutcome { index } => {
                    *index <= MAX_EFFECT_ROLL_INDEX
                }
                PersistedEffectPathSegment::ConditionThen
                | PersistedEffectPathSegment::ConditionOtherwise
                | PersistedEffectPathSegment::RepeatEffect => true,
            }))
}

fn valid_decision_point(decision: &PersistedDecisionPoint, require_address: bool) -> bool {
    match decision {
        PersistedDecisionPoint::None | PersistedDecisionPoint::Automatic => true,
        PersistedDecisionPoint::PlayerIntent {
            responsible_position,
        } => valid_position(*responsible_position),
        PersistedDecisionPoint::EffectChoice { choice } => {
            valid_effect_choice(choice, require_address, MAX_EFFECT_PATH_DEPTH)
        }
    }
}

fn valid_effect_outcome(outcome: &PersistedEffectOutcome, bounded_identifiers: bool) -> bool {
    match outcome {
        PersistedEffectOutcome::DieRolled {
            rule_id,
            die,
            result,
        } => {
            valid_identifier_for_version(rule_id, bounded_identifiers)
                && domain_effect_die(die)
                    .is_ok_and(|effect_die| (1..=effect_die.sides()).contains(result))
        }
        PersistedEffectOutcome::Moved {
            rule_id,
            target_id,
            target_position,
            from,
            to,
        } => {
            valid_identifier_for_version(rule_id, bounded_identifiers)
                && valid_identifier_for_version(target_id, bounded_identifiers)
                && target_position.is_none_or(valid_position)
                && from != to
                && card_zone(from)
                && card_zone(to)
        }
        PersistedEffectOutcome::NoOp { rule_id, reason } => {
            valid_identifier_for_version(rule_id, bounded_identifiers)
                && matches!(
                    reason.as_str(),
                    "explicit" | "no_eligible_target" | "zero_cardinality"
                )
        }
        PersistedEffectOutcome::ResourceChanged {
            rule_id,
            target_id,
            target_position,
            resource,
            cause,
            ..
        } => {
            valid_identifier_for_version(rule_id, bounded_identifiers)
                && valid_identifier_for_version(target_id, bounded_identifiers)
                && target_position.is_none_or(valid_position)
                && domain_effect_resource(resource).is_ok()
                && matches!(cause.as_str(), "cost" | "effect")
        }
        PersistedEffectOutcome::Terminal { rule_id, outcome } => {
            valid_identifier_for_version(rule_id, bounded_identifiers)
                && matches!(outcome.as_str(), "lost" | "won")
        }
    }
}

fn validate_persisted_event(event: &PersistedGameEvent) -> Result<(), ApiError> {
    if event.sequence == 0
        || event.state_version == 0
        || event.turn == 0
        || !valid_position(event.actor_position)
        || event
            .prng_counter
            .is_some_and(|counter| counter > i64::MAX.cast_unsigned())
    {
        return Err(ApiError::internal());
    }
    let valid = match event.event_version {
        1 => {
            event.event_type == "dark_arts_completed"
                && event.effects.is_empty()
                && event.effect_stop.as_deref() == Some("stable")
                && event.choice.is_none()
                && event.card_id.is_none()
                && event.targets.is_empty()
                && event.villain_id.is_none()
                && event.amount.is_none()
                && event.cost.is_none()
                && event.refill_card_id.is_none()
                && event.choice_id.is_none()
                && event.choice_cause.is_none()
                && event.selected_options.is_none()
                && event.end_turn.is_none()
                && event.steps.is_none()
                && event.control.is_none()
                && event.prng_counter == Some(0)
        }
        CLOSED_EFFECT_EVENT_VERSION => valid_closed_effect_event(event, false),
        CHOICE_EVENT_VERSION => {
            if matches!(
                event.event_type.as_str(),
                "card_played" | "attack_assigned" | "card_acquired"
            ) {
                valid_hero_action_event(event, false)
            } else {
                valid_closed_effect_event(event, true)
            }
        }
        GAME_EVENT_VERSION => {
            if matches!(
                event.event_type.as_str(),
                "card_played" | "attack_assigned" | "card_acquired"
            ) {
                valid_hero_action_event(event, true)
            } else {
                valid_v4_event(event)
            }
        }
        _ => false,
    };
    if !valid {
        return Err(ApiError::internal());
    }
    Ok(())
}

fn valid_closed_effect_event(event: &PersistedGameEvent, current_choice: bool) -> bool {
    let event_metadata_is_valid = match event.event_type.as_str() {
        "dark_arts_completed" => {
            event.choice_id.is_none()
                && event.choice_cause.is_none()
                && event.selected_options.is_none()
        }
        "choice_resolved" if current_choice => {
            event.choice_id.as_deref().is_some_and(valid_choice_value)
                && event
                    .choice_cause
                    .as_deref()
                    .is_some_and(valid_choice_value)
                && event
                    .selected_options
                    .as_deref()
                    .is_some_and(valid_choice_selection)
        }
        _ => false,
    };
    if !event_metadata_is_valid
        || event.effects.len() > MAX_EFFECT_OUTCOMES
        || event
            .effects
            .iter()
            .any(|outcome| !valid_effect_outcome(outcome, current_choice))
        || event.card_id.is_some()
        || !event.targets.is_empty()
        || event.villain_id.is_some()
        || event.amount.is_some()
        || event.cost.is_some()
        || event.refill_card_id.is_some()
        || event.end_turn.is_some()
        || event.steps.is_some()
        || event.control.is_some()
        || event.prng_counter.is_none()
    {
        return false;
    }
    let terminal_count = event
        .effects
        .iter()
        .filter(|outcome| matches!(outcome, PersistedEffectOutcome::Terminal { .. }))
        .count();
    match event.effect_stop.as_deref() {
        Some("stable") => event.choice.is_none() && terminal_count == 0,
        Some("choice") => {
            terminal_count == 0
                && event.choice.as_ref().is_some_and(|choice| match choice {
                    PersistedEventChoice::Current(choice) if current_choice => {
                        valid_effect_choice(choice, true, MAX_EFFECT_QUEUE)
                    }
                    PersistedEventChoice::Legacy(choice) if !current_choice => {
                        valid_legacy_effect_choice(choice)
                    }
                    PersistedEventChoice::Current(_) | PersistedEventChoice::Legacy(_) => false,
                })
        }
        Some("terminal") => {
            event.choice.is_none()
                && terminal_count == 1
                && matches!(
                    event.effects.last(),
                    Some(PersistedEffectOutcome::Terminal { .. })
                )
        }
        _ => false,
    }
}

fn valid_hero_action_event(event: &PersistedGameEvent, bounded_identifiers: bool) -> bool {
    if event.effects.len() > MAX_EFFECT_OUTCOMES
        || event
            .effects
            .iter()
            .any(|outcome| !valid_effect_outcome(outcome, bounded_identifiers))
        || event.end_turn.is_some()
        || event.steps.is_some()
        || event.control.is_some()
        || !no_choice_resolution_fields(event)
    {
        return false;
    }

    match event.event_type.as_str() {
        "card_played" => {
            event.card_id.as_deref().is_some_and(valid_choice_value)
                && event.villain_id.is_none()
                && event.amount.is_none()
                && event.cost.is_none()
                && event.refill_card_id.is_none()
                && valid_target_bindings(&event.targets)
                && valid_effect_progress(event, bounded_identifiers)
        }
        "attack_assigned" => {
            event.card_id.is_none()
                && event.targets.is_empty()
                && event.villain_id.as_deref().is_some_and(valid_choice_value)
                && event.amount.is_some_and(|amount| amount > 0)
                && event.cost.is_none()
                && event.refill_card_id.is_none()
                && event.effect_stop.is_none()
                && event.choice.is_none()
                && event.prng_counter.is_none()
        }
        "card_acquired" => {
            event.card_id.as_deref().is_some_and(valid_choice_value)
                && event.targets.is_empty()
                && event.villain_id.is_none()
                && event.amount.is_none()
                && event.cost.is_some()
                && event
                    .refill_card_id
                    .as_deref()
                    .is_none_or(valid_choice_value)
                && event.effect_stop.is_none()
                && event.choice.is_none()
                && event.prng_counter.is_none()
        }
        _ => false,
    }
}

fn no_choice_resolution_fields(event: &PersistedGameEvent) -> bool {
    event.choice_id.is_none() && event.choice_cause.is_none() && event.selected_options.is_none()
}

fn valid_effect_progress(event: &PersistedGameEvent, bounded_identifiers: bool) -> bool {
    let terminal_count = event
        .effects
        .iter()
        .filter(|outcome| matches!(outcome, PersistedEffectOutcome::Terminal { .. }))
        .count();
    let progress_is_valid = match event.effect_stop.as_deref() {
        Some("stable") => event.choice.is_none() && terminal_count == 0,
        Some("choice") => {
            terminal_count == 0
                && event.choice.as_ref().is_some_and(|choice| match choice {
                    PersistedEventChoice::Current(choice) => {
                        valid_effect_choice(choice, bounded_identifiers, MAX_EFFECT_QUEUE)
                    }
                    PersistedEventChoice::Legacy(choice) if !bounded_identifiers => {
                        valid_legacy_effect_choice(choice)
                    }
                    PersistedEventChoice::Legacy(_) => false,
                })
        }
        Some("terminal") => {
            event.choice.is_none()
                && terminal_count == 1
                && matches!(
                    event.effects.last(),
                    Some(PersistedEffectOutcome::Terminal { .. })
                )
        }
        _ => false,
    };
    progress_is_valid && event.prng_counter.is_some()
}

fn valid_v4_event(event: &PersistedGameEvent) -> bool {
    if !event.effects.is_empty()
        || event.effect_stop.as_deref() != Some("stable")
        || event.choice.is_some()
        || event.card_id.is_some()
        || !event.targets.is_empty()
        || event.villain_id.is_some()
        || event.amount.is_some()
        || event.cost.is_some()
        || event.refill_card_id.is_some()
        || event.prng_counter.is_none()
    {
        return false;
    }
    match event.event_type.as_str() {
        "turn_completed" => {
            if event.choice_id.is_some()
                || event.choice_cause.is_some()
                || event.selected_options.is_some()
            {
                return false;
            }
            let (Some(end_turn), Some(steps), Some(control)) =
                (&event.end_turn, &event.steps, &event.control)
            else {
                return false;
            };
            valid_end_turn_sequence(end_turn, event.actor_position)
                && valid_turn_steps(steps)
                && valid_engine_control(control)
                && control.turn == event.turn.checked_add(1).unwrap_or(0)
                && event_control_matches_steps(steps, control)
                && terminal_effects_match_control(steps, control)
        }
        "choice_resolved" => {
            if event.end_turn.is_some()
                || !event.choice_id.as_deref().is_some_and(valid_choice_value)
                || !event
                    .choice_cause
                    .as_deref()
                    .is_some_and(valid_choice_value)
                || !event
                    .selected_options
                    .as_deref()
                    .is_some_and(valid_choice_selection)
            {
                return false;
            }
            let (Some(steps), Some(control)) = (&event.steps, &event.control) else {
                return false;
            };
            valid_choice_steps(steps)
                && valid_engine_control(control)
                && control.turn == event.turn
                && event_control_matches_steps(steps, control)
                && terminal_effects_match_control(steps, control)
        }
        _ => false,
    }
}

fn valid_choice_selection(selected: &[String]) -> bool {
    selected.len() <= 32
        && selected.iter().all(|value| valid_choice_value(value))
        && selected
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == selected.len()
}

fn valid_end_turn_sequence(outcomes: &[PersistedEndTurnOutcome], actor_position: u8) -> bool {
    if !(2..=MAX_END_TURN_OUTCOMES).contains(&outcomes.len())
        || outcomes
            .iter()
            .any(|outcome| !valid_end_turn_outcome(outcome, actor_position))
    {
        return false;
    }

    let mut remaining = outcomes;
    for source in ["hero_play_area", "hero_hand"] {
        while let Some(PersistedEndTurnOutcome::CardMoved { from, to, .. }) = remaining.first() {
            if from != source || to != "hero_discard_pile" {
                break;
            }
            remaining = &remaining[1..];
        }
    }
    let [
        PersistedEndTurnOutcome::ResourceReset {
            resource: attack, ..
        },
        PersistedEndTurnOutcome::ResourceReset {
            resource: influence,
            ..
        },
        refill @ ..,
    ] = remaining
    else {
        return false;
    };
    if attack != "attack" || influence != "influence" {
        return false;
    }

    let mut shuffled = false;
    refill.iter().all(|outcome| match outcome {
        PersistedEndTurnOutcome::CardMoved { from, to, .. } => {
            from == "hero_draw_pile" && to == "hero_hand"
        }
        PersistedEndTurnOutcome::PileShuffled { .. } if !shuffled => {
            shuffled = true;
            true
        }
        PersistedEndTurnOutcome::PileShuffled { .. }
        | PersistedEndTurnOutcome::ResourceReset { .. } => false,
    })
}

fn terminal_effects_match_control(
    steps: &[PersistedTurnStep],
    control: &PersistedEngineControl,
) -> bool {
    let terminal_count = steps
        .iter()
        .flat_map(|step| &step.effects)
        .filter(|outcome| matches!(outcome, PersistedEffectOutcome::Terminal { .. }))
        .count();
    match control.status.as_str() {
        "in_progress" => terminal_count == 0,
        expected @ ("lost" | "won") => {
            terminal_count == 1
                && matches!(
                    steps.last().and_then(|step| step.effects.last()),
                    Some(PersistedEffectOutcome::Terminal { outcome, .. })
                        if outcome == expected
                )
        }
        _ => false,
    }
}

fn event_control_matches_steps(
    steps: &[PersistedTurnStep],
    control: &PersistedEngineControl,
) -> bool {
    let last_phase = steps.last().map(|step| step.phase.as_str());
    match (control.status.as_str(), &control.decision_point) {
        ("in_progress", PersistedDecisionPoint::PlayerIntent { .. }) => {
            last_phase == Some("villains") && control.phase == "hero_actions"
        }
        ("in_progress", PersistedDecisionPoint::EffectChoice { .. })
        | ("lost" | "won", PersistedDecisionPoint::None) => {
            last_phase == Some(control.phase.as_str())
        }
        _ => false,
    }
}

fn valid_turn_steps(steps: &[PersistedTurnStep]) -> bool {
    let phases = steps
        .iter()
        .map(|step| step.phase.as_str())
        .collect::<Vec<_>>();
    matches!(
        phases.as_slice(),
        ["end_turn", "dark_arts"] | ["end_turn", "dark_arts", "villains"]
    ) && steps.first().is_some_and(|step| step.effects.is_empty())
        && steps.len() <= MAX_TURN_STEPS
        && total_step_effects_are_bounded(steps)
        && steps.iter().all(|step| {
            step.effects.len() <= MAX_EFFECT_OUTCOMES
                && step
                    .effects
                    .iter()
                    .all(|outcome| valid_effect_outcome(outcome, true))
        })
}

fn valid_choice_steps(steps: &[PersistedTurnStep]) -> bool {
    let phases = steps
        .iter()
        .map(|step| step.phase.as_str())
        .collect::<Vec<_>>();
    matches!(
        phases.as_slice(),
        ["dark_arts" | "villains"] | ["dark_arts", "villains"]
    ) && total_step_effects_are_bounded(steps)
        && steps.iter().all(|step| {
            step.effects.len() <= MAX_EFFECT_OUTCOMES
                && step
                    .effects
                    .iter()
                    .all(|outcome| valid_effect_outcome(outcome, true))
        })
}

fn total_step_effects_are_bounded(steps: &[PersistedTurnStep]) -> bool {
    steps
        .iter()
        .try_fold(0_usize, |total, step| total.checked_add(step.effects.len()))
        .is_some_and(|total| total <= MAX_EFFECT_OUTCOMES)
}

fn valid_engine_control(control: &PersistedEngineControl) -> bool {
    if control.turn == 0
        || !valid_position(control.active_position)
        || domain_game_status(&control.status).is_err()
        || !canonical_game_phase(&control.phase)
        || control.queued_phases.len() > 3
        || control
            .queued_phases
            .iter()
            .any(|phase| !canonical_game_phase(phase))
        || control.queued_effects.len() > MAX_EFFECT_QUEUE
        || control
            .queued_effects
            .iter()
            .any(|queued| !valid_queued_effect(queued, true, MAX_EFFECT_PATH_DEPTH))
        || !valid_decision_point(&control.decision_point, true)
    {
        return false;
    }
    match (control.status.as_str(), control.phase.as_str()) {
        ("lost" | "won", _) => {
            control.queued_phases.is_empty()
                && control.queued_effects.is_empty()
                && matches!(control.decision_point, PersistedDecisionPoint::None)
        }
        ("in_progress", "dark_arts") => {
            control.queued_phases == ["villains", "hero_actions", "end_turn"]
                && valid_automatic_control(control)
        }
        ("in_progress", "villains") => {
            control.queued_phases == ["hero_actions", "end_turn"]
                && valid_automatic_control(control)
        }
        ("in_progress", "hero_actions") => {
            control.queued_phases == ["end_turn"]
                && control.queued_effects.is_empty()
                && matches!(
                    control.decision_point,
                    PersistedDecisionPoint::PlayerIntent {
                        responsible_position
                    } if responsible_position == control.active_position
                )
        }
        ("in_progress", "end_turn") => {
            control.queued_phases.is_empty()
                && control.queued_effects.is_empty()
                && matches!(control.decision_point, PersistedDecisionPoint::Automatic)
        }
        _ => false,
    }
}

fn valid_automatic_control(control: &PersistedEngineControl) -> bool {
    match &control.decision_point {
        PersistedDecisionPoint::Automatic => control.queued_effects.is_empty(),
        PersistedDecisionPoint::EffectChoice { choice } => {
            choice.continuation.queue == control.queued_effects
        }
        PersistedDecisionPoint::None | PersistedDecisionPoint::PlayerIntent { .. } => false,
    }
}

fn valid_end_turn_outcome(outcome: &PersistedEndTurnOutcome, actor_position: u8) -> bool {
    match outcome {
        PersistedEndTurnOutcome::CardMoved { card_id, from, to } => {
            valid_identifier(card_id)
                && matches!(
                    (from.as_str(), to.as_str()),
                    ("hero_play_area" | "hero_hand", "hero_discard_pile")
                        | ("hero_draw_pile", "hero_hand")
                )
        }
        PersistedEndTurnOutcome::PileShuffled {
            owner_position,
            zone,
            bottom_to_top,
        } => {
            let unique_cards = bottom_to_top
                .iter()
                .collect::<std::collections::BTreeSet<_>>();
            *owner_position == actor_position
                && zone == "hero_draw_pile"
                && (1..=MAX_PILE_CARDS).contains(&bottom_to_top.len())
                && unique_cards.len() == bottom_to_top.len()
                && bottom_to_top
                    .iter()
                    .all(|card_id| valid_identifier(card_id))
        }
        PersistedEndTurnOutcome::ResourceReset { resource, .. } => {
            matches!(resource.as_str(), "attack" | "influence")
        }
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES
}

fn valid_legacy_identifier(value: &str) -> bool {
    !value.is_empty()
}

fn valid_identifier_for_version(value: &str, bounded: bool) -> bool {
    if bounded {
        valid_identifier(value)
    } else {
        valid_legacy_identifier(value)
    }
}

const fn valid_position(position: u8) -> bool {
    matches!(position, 1..=4)
}

fn canonical_game_phase(phase: &str) -> bool {
    matches!(
        phase,
        "dark_arts" | "villains" | "hero_actions" | "end_turn"
    )
}

fn card_zone(zone: &str) -> bool {
    matches!(
        zone,
        "active_villains"
            | "dark_arts_deck"
            | "dark_arts_discard"
            | "hero_discard_pile"
            | "hero_draw_pile"
            | "hero_hand"
            | "hero_play_area"
            | "hogwarts_deck"
            | "market"
            | "villain_deck"
    )
}

pub(super) fn persisted_after_decision(
    current: &PersistedSnapshot,
    state: &InitialGameState,
) -> PersistedSnapshot {
    let mut next = current.clone();
    next.snapshot_version = SNAPSHOT_VERSION;
    next.state_version = state.state_version();
    next.sequence = state.sequence();
    next.status = match state.status() {
        GameStatus::InProgress => "in_progress".to_owned(),
        GameStatus::Lost => "lost".to_owned(),
        GameStatus::Won => "won".to_owned(),
    };
    next.turn.number = state.turn();
    game_phase_name(state.phase()).clone_into(&mut next.turn.phase);
    next.turn.active_position = state.active_position();
    next.queued_phases = Some(
        state
            .queued_phases()
            .iter()
            .map(|phase| game_phase_name(*phase).to_owned())
            .collect(),
    );
    next.queued_effects = Some(
        state
            .queued_effects()
            .iter()
            .map(persisted_queued_effect)
            .collect(),
    );
    next.decision_point = Some(persisted_decision_point(state.decision_point()));
    next.last_turn_steps = Some(
        state
            .last_turn_steps()
            .iter()
            .map(persisted_turn_step)
            .collect(),
    );
    next.prng.counter = state.prng_counter();
    next.effects = persisted_effects(state);
    next
}

pub(super) fn persisted_event(event: GameEvent) -> Result<(u16, &'static str, String), ApiError> {
    let persisted = if matches!(
        &event,
        GameEvent::DarkArtsCompleted { .. }
            | GameEvent::ChoiceResolved { .. }
            | GameEvent::TurnCompleted { .. }
    ) {
        persisted_turn_event(event)?
    } else {
        persisted_hero_action_event(event)?
    };
    decode_persisted_event(&persisted.2)?;
    Ok(persisted)
}

fn persisted_turn_event(event: GameEvent) -> Result<(u16, &'static str, String), ApiError> {
    match event {
        GameEvent::DarkArtsCompleted {
            sequence,
            state_version,
            turn,
            actor_position,
            effects,
            stop,
            prng_counter,
        } => {
            let event_version = if matches!(&stop, EffectStop::Choice(_)) {
                CHOICE_EVENT_VERSION
            } else {
                CLOSED_EFFECT_EVENT_VERSION
            };
            serde_json::to_string(&serde_json::json!({
                "event_version": event_version,
                "type": "dark_arts_completed",
                "sequence": sequence,
                "state_version": state_version,
                "turn": turn,
                "actor_position": actor_position,
                "effects": effects.iter().map(persisted_effect_outcome).collect::<Vec<_>>(),
                "effect_stop": effect_stop_name(&stop),
                "choice": match &stop {
                    EffectStop::Choice(choice) => Some(persisted_effect_choice(choice)),
                    EffectStop::Stable | EffectStop::Terminal(_) => None,
                },
                "prng_counter": prng_counter,
            }))
            .map(|event| (event_version, "dark_arts_completed", event))
            .map_err(|error| ApiError::internal_with("match application operation", error))
        }
        GameEvent::ChoiceResolved {
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
        } => serde_json::to_string(&serde_json::json!({
            "event_version": GAME_EVENT_VERSION,
            "type": "choice_resolved",
            "sequence": sequence,
            "state_version": state_version,
            "turn": turn,
            "actor_position": actor_position,
            "choice_id": choice_id,
            "choice_cause": choice_cause,
            "selected_options": selected_options,
            "steps": steps.iter().map(persisted_turn_step).collect::<Vec<_>>(),
            "control": persisted_engine_control(&control),
            "prng_counter": prng_counter,
        }))
        .map(|event| (GAME_EVENT_VERSION, "choice_resolved", event))
        .map_err(|error| ApiError::internal_with("match application operation", error)),
        GameEvent::TurnCompleted {
            sequence,
            state_version,
            turn,
            actor_position,
            end_turn,
            steps,
            control,
            prng_counter,
        } => serde_json::to_string(&serde_json::json!({
            "event_version": GAME_EVENT_VERSION,
            "type": "turn_completed",
            "sequence": sequence,
            "state_version": state_version,
            "turn": turn,
            "actor_position": actor_position,
            "end_turn": end_turn.iter().map(persisted_end_turn_outcome).collect::<Vec<_>>(),
            "steps": steps.iter().map(persisted_turn_step).collect::<Vec<_>>(),
            "control": persisted_engine_control(&control),
            "prng_counter": prng_counter,
        }))
        .map(|event| (GAME_EVENT_VERSION, "turn_completed", event))
        .map_err(|error| ApiError::internal_with("match application operation", error)),
        GameEvent::CardPlayed { .. }
        | GameEvent::AttackAssigned { .. }
        | GameEvent::CardAcquired { .. } => {
            unreachable!("hero action event routed to the turn event encoder")
        }
    }
}

fn persisted_hero_action_event(event: GameEvent) -> Result<(u16, &'static str, String), ApiError> {
    match event {
        GameEvent::CardPlayed {
            sequence,
            state_version,
            turn,
            actor_position,
            card_id,
            targets,
            effects,
            stop,
            prng_counter,
        } => serde_json::to_string(&serde_json::json!({
            "event_version": GAME_EVENT_VERSION,
            "type": "card_played",
            "sequence": sequence,
            "state_version": state_version,
            "turn": turn,
            "actor_position": actor_position,
            "card_id": card_id,
            "targets": targets.iter().map(persisted_effect_target_binding).collect::<Vec<_>>(),
            "effects": effects.iter().map(persisted_effect_outcome).collect::<Vec<_>>(),
            "effect_stop": effect_stop_name(&stop),
            "choice": match stop {
                EffectStop::Choice(choice) => Some(persisted_effect_choice(&choice)),
                EffectStop::Stable | EffectStop::Terminal(_) => None,
            },
            "prng_counter": prng_counter,
        }))
        .map(|event| (GAME_EVENT_VERSION, "card_played", event))
        .map_err(|error| ApiError::internal_with("match application operation", error)),
        GameEvent::AttackAssigned {
            sequence,
            state_version,
            turn,
            actor_position,
            villain_id,
            amount,
            effects,
        } => serde_json::to_string(&serde_json::json!({
            "event_version": GAME_EVENT_VERSION,
            "type": "attack_assigned",
            "sequence": sequence,
            "state_version": state_version,
            "turn": turn,
            "actor_position": actor_position,
            "villain_id": villain_id,
            "amount": amount,
            "effects": effects.iter().map(persisted_effect_outcome).collect::<Vec<_>>(),
        }))
        .map(|event| (GAME_EVENT_VERSION, "attack_assigned", event))
        .map_err(|error| ApiError::internal_with("match application operation", error)),
        GameEvent::CardAcquired {
            sequence,
            state_version,
            turn,
            actor_position,
            card_id,
            cost,
            refill_card_id,
            effects,
        } => serde_json::to_string(&serde_json::json!({
            "event_version": GAME_EVENT_VERSION,
            "type": "card_acquired",
            "sequence": sequence,
            "state_version": state_version,
            "turn": turn,
            "actor_position": actor_position,
            "card_id": card_id,
            "cost": cost,
            "refill_card_id": refill_card_id,
            "effects": effects.iter().map(persisted_effect_outcome).collect::<Vec<_>>(),
        }))
        .map(|event| (GAME_EVENT_VERSION, "card_acquired", event))
        .map_err(|error| ApiError::internal_with("match application operation", error)),
        GameEvent::DarkArtsCompleted { .. }
        | GameEvent::ChoiceResolved { .. }
        | GameEvent::TurnCompleted { .. } => {
            unreachable!("turn event routed to the hero action event encoder")
        }
    }
}

pub(super) fn persisted_snapshot(
    state: &InitialGameState,
    participants: &[StoredRoomParticipant],
) -> PersistedSnapshot {
    PersistedSnapshot {
        snapshot_version: SNAPSHOT_VERSION,
        state_version: state.state_version(),
        sequence: state.sequence(),
        status: match state.status() {
            GameStatus::InProgress => "in_progress".to_owned(),
            GameStatus::Lost => "lost".to_owned(),
            GameStatus::Won => "won".to_owned(),
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
            phase: game_phase_name(state.phase()).to_owned(),
            active_position: state.active_position(),
        },
        queued_phases: Some(
            state
                .queued_phases()
                .iter()
                .map(|phase| game_phase_name(*phase).to_owned())
                .collect(),
        ),
        queued_effects: Some(
            state
                .queued_effects()
                .iter()
                .map(persisted_queued_effect)
                .collect(),
        ),
        decision_point: Some(persisted_decision_point(state.decision_point())),
        last_turn_steps: Some(
            state
                .last_turn_steps()
                .iter()
                .map(persisted_turn_step)
                .collect(),
        ),
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
        effects: persisted_effects(state),
    }
}

struct DomainSnapshotControl {
    queued_phases: Vec<GamePhase>,
    queued_effects: Vec<QueuedEffect>,
    decision_point: Option<DecisionPoint>,
    last_turn_steps: Vec<TurnStep>,
}

fn domain_snapshot_control(
    persisted: &PersistedSnapshot,
    status: GameStatus,
    phase: GamePhase,
    pending_choice: Option<&PendingEffectChoice>,
    last_effects: &[EffectOutcome],
) -> Result<DomainSnapshotControl, ApiError> {
    match (
        &persisted.queued_phases,
        &persisted.queued_effects,
        &persisted.decision_point,
        &persisted.last_turn_steps,
    ) {
        (
            Some(queued_phases),
            Some(queued_effects),
            Some(decision_point),
            Some(last_turn_steps),
        ) => Ok(DomainSnapshotControl {
            queued_phases: queued_phases
                .iter()
                .map(|phase| domain_game_phase(phase))
                .collect::<Result<_, _>>()?,
            queued_effects: queued_effects.iter().map(domain_queued_effect).collect(),
            decision_point: domain_decision_point(decision_point)?,
            last_turn_steps: last_turn_steps
                .iter()
                .map(domain_turn_step)
                .collect::<Result<_, _>>()?,
        }),
        (None, None, None, None) => legacy_snapshot_control(
            status,
            phase,
            persisted.turn.active_position,
            pending_choice,
            last_effects,
        ),
        _ => Err(ApiError::internal()),
    }
}

fn legacy_snapshot_control(
    status: GameStatus,
    phase: GamePhase,
    active_position: u8,
    pending_choice: Option<&PendingEffectChoice>,
    last_effects: &[EffectOutcome],
) -> Result<DomainSnapshotControl, ApiError> {
    if status != GameStatus::InProgress {
        return Ok(DomainSnapshotControl {
            queued_phases: Vec::new(),
            queued_effects: Vec::new(),
            decision_point: None,
            last_turn_steps: (!last_effects.is_empty())
                .then(|| TurnStep::new(phase, last_effects.to_vec()))
                .into_iter()
                .collect(),
        });
    }
    let (queued_phases, queued_effects, decision_point, last_turn_steps) = match phase {
        GamePhase::DarkArts => (
            vec![
                GamePhase::Villains,
                GamePhase::HeroActions,
                GamePhase::EndTurn,
            ],
            pending_choice
                .map(|choice| choice.continuation.queue.clone())
                .unwrap_or_default(),
            Some(match pending_choice {
                Some(choice) => DecisionPoint::EffectChoice(choice.clone()),
                None => DecisionPoint::Automatic,
            }),
            vec![TurnStep::new(GamePhase::DarkArts, last_effects.to_vec())],
        ),
        GamePhase::HeroActions if pending_choice.is_none() => (
            vec![GamePhase::EndTurn],
            Vec::new(),
            Some(DecisionPoint::PlayerIntent {
                responsible_position: active_position,
            }),
            vec![
                TurnStep::new(GamePhase::DarkArts, last_effects.to_vec()),
                TurnStep::new(GamePhase::Villains, Vec::new()),
            ],
        ),
        GamePhase::Villains | GamePhase::EndTurn | GamePhase::HeroActions => {
            return Err(ApiError::internal());
        }
    };
    Ok(DomainSnapshotControl {
        queued_phases,
        queued_effects,
        decision_point,
        last_turn_steps,
    })
}

fn domain_game_status(status: &str) -> Result<GameStatus, ApiError> {
    match status {
        "in_progress" => Ok(GameStatus::InProgress),
        "lost" => Ok(GameStatus::Lost),
        "won" => Ok(GameStatus::Won),
        _ => Err(ApiError::internal()),
    }
}

fn domain_game_phase(phase: &str) -> Result<GamePhase, ApiError> {
    match phase {
        "dark_arts" => Ok(GamePhase::DarkArts),
        "villains" => Ok(GamePhase::Villains),
        "hero_action" | "hero_actions" => Ok(GamePhase::HeroActions),
        "end_turn" => Ok(GamePhase::EndTurn),
        _ => Err(ApiError::internal()),
    }
}

pub(super) const fn game_phase_name(phase: GamePhase) -> &'static str {
    match phase {
        GamePhase::DarkArts => "dark_arts",
        GamePhase::Villains => "villains",
        GamePhase::HeroActions => "hero_actions",
        GamePhase::EndTurn => "end_turn",
    }
}

fn domain_decision_point(
    decision: &PersistedDecisionPoint,
) -> Result<Option<DecisionPoint>, ApiError> {
    Ok(match decision {
        PersistedDecisionPoint::None => None,
        PersistedDecisionPoint::Automatic => Some(DecisionPoint::Automatic),
        PersistedDecisionPoint::PlayerIntent {
            responsible_position,
        } => Some(DecisionPoint::PlayerIntent {
            responsible_position: *responsible_position,
        }),
        PersistedDecisionPoint::EffectChoice { choice } => {
            Some(DecisionPoint::EffectChoice(domain_effect_choice(choice)?))
        }
    })
}

fn persisted_decision_point(decision: Option<&DecisionPoint>) -> PersistedDecisionPoint {
    match decision {
        None => PersistedDecisionPoint::None,
        Some(DecisionPoint::Automatic) => PersistedDecisionPoint::Automatic,
        Some(DecisionPoint::PlayerIntent {
            responsible_position,
        }) => PersistedDecisionPoint::PlayerIntent {
            responsible_position: *responsible_position,
        },
        Some(DecisionPoint::EffectChoice(choice)) => PersistedDecisionPoint::EffectChoice {
            choice: persisted_effect_choice(choice),
        },
    }
}

fn domain_turn_step(step: &PersistedTurnStep) -> Result<TurnStep, ApiError> {
    Ok(TurnStep::new(
        domain_game_phase(&step.phase)?,
        step.effects
            .iter()
            .map(domain_effect_outcome)
            .collect::<Result<_, _>>()?,
    ))
}

fn persisted_turn_step(step: &TurnStep) -> PersistedTurnStep {
    PersistedTurnStep {
        phase: game_phase_name(step.phase()).to_owned(),
        effects: step
            .effects()
            .iter()
            .map(persisted_effect_outcome)
            .collect(),
    }
}

fn persisted_end_turn_outcome(outcome: &EndTurnOutcome) -> PersistedEndTurnOutcome {
    match outcome {
        EndTurnOutcome::CardMoved { card_id, from, to } => PersistedEndTurnOutcome::CardMoved {
            card_id: card_id.clone(),
            from: effect_zone_name(*from).to_owned(),
            to: effect_zone_name(*to).to_owned(),
        },
        EndTurnOutcome::PileShuffled {
            owner_position,
            zone,
            bottom_to_top,
        } => PersistedEndTurnOutcome::PileShuffled {
            owner_position: *owner_position,
            zone: effect_zone_name(*zone).to_owned(),
            bottom_to_top: bottom_to_top.clone(),
        },
        EndTurnOutcome::ResourceReset { resource, before } => {
            PersistedEndTurnOutcome::ResourceReset {
                resource: effect_resource_name(*resource).to_owned(),
                before: *before,
            }
        }
    }
}

fn persisted_engine_control(control: &EngineControl) -> PersistedEngineControl {
    PersistedEngineControl {
        status: game_status_name(control.status).to_owned(),
        turn: control.turn,
        phase: game_phase_name(control.phase).to_owned(),
        active_position: control.active_position,
        queued_phases: control
            .queued_phases
            .iter()
            .map(|phase| game_phase_name(*phase).to_owned())
            .collect(),
        queued_effects: control
            .queued_effects
            .iter()
            .map(persisted_queued_effect)
            .collect(),
        decision_point: persisted_decision_point(control.decision_point.as_ref()),
    }
}

const fn game_status_name(status: GameStatus) -> &'static str {
    match status {
        GameStatus::InProgress => "in_progress",
        GameStatus::Lost => "lost",
        GameStatus::Won => "won",
    }
}

fn domain_effect_world(
    persisted: &PersistedEffects,
    players: &[InitialPlayer],
) -> Result<EffectWorld, ApiError> {
    if persisted.entities.is_empty() {
        return Ok(EffectWorld::new(
            players
                .iter()
                .map(|player| {
                    EffectEntityPlacement::new(
                        EffectEntity::hero(player.position()),
                        EffectZone::Heroes,
                    )
                })
                .collect(),
        ));
    }

    let mut entities = persisted.entities.iter().enumerate().collect::<Vec<_>>();
    entities.sort_by_key(|(original_index, entity)| {
        (
            entity.zone.as_str(),
            entity.owner_position,
            entity.zone_index.map_or(*original_index, usize::from),
        )
    });
    entities
        .into_iter()
        .map(|(_, entity)| {
            let zone = domain_effect_zone(&entity.zone)?;
            let kind = domain_effect_entity_kind(entity.kind.as_deref(), zone)?;
            let mut domain =
                EffectEntity::new(entity.id.clone(), entity.owner_position).with_kind(kind);
            if let Some(catalog_id) = &entity.catalog_id {
                domain = domain.with_catalog_id(catalog_id.clone());
            }
            if let Some(effect_rule_id) = &entity.effect_rule_id {
                domain = domain.with_effect_rule(effect_rule_id.clone());
            }
            if let Some(influence_cost) = entity.influence_cost {
                domain = domain.with_influence_cost(influence_cost);
            }
            for (resource, amount) in &entity.resources {
                domain = domain.with_resource(domain_effect_resource(resource)?, *amount);
            }
            Ok(EffectEntityPlacement::new(domain, zone))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(EffectWorld::new)
}

fn domain_effect_outcome(outcome: &PersistedEffectOutcome) -> Result<EffectOutcome, ApiError> {
    Ok(match outcome {
        PersistedEffectOutcome::DieRolled {
            rule_id,
            die,
            result,
        } => EffectOutcome::DieRolled {
            rule_id: rule_id.clone(),
            die: domain_effect_die(die)?,
            result: *result,
        },
        PersistedEffectOutcome::Moved {
            rule_id,
            target_id,
            target_position,
            from,
            to,
        } => EffectOutcome::Moved {
            rule_id: rule_id.clone(),
            target_id: target_id.clone(),
            target_position: *target_position,
            from: domain_effect_zone(from)?,
            to: domain_effect_zone(to)?,
        },
        PersistedEffectOutcome::NoOp { rule_id, reason } => EffectOutcome::NoOp {
            rule_id: rule_id.clone(),
            reason: match reason.as_str() {
                "explicit" => EffectNoOpReason::Explicit,
                "no_eligible_target" => EffectNoOpReason::NoEligibleTarget,
                "zero_cardinality" => EffectNoOpReason::ZeroCardinality,
                _ => return Err(ApiError::internal()),
            },
        },
        PersistedEffectOutcome::ResourceChanged {
            rule_id,
            target_id,
            target_position,
            resource,
            before,
            after,
            cause,
        } => EffectOutcome::ResourceChanged {
            rule_id: rule_id.clone(),
            target_id: target_id.clone(),
            target_position: *target_position,
            resource: domain_effect_resource(resource)?,
            before: *before,
            after: *after,
            cause: match cause.as_str() {
                "cost" => EffectChangeCause::Cost,
                "effect" => EffectChangeCause::Effect,
                _ => return Err(ApiError::internal()),
            },
        },
        PersistedEffectOutcome::Terminal { rule_id, outcome } => EffectOutcome::Terminal {
            rule_id: rule_id.clone(),
            outcome: match outcome.as_str() {
                "lost" => EffectGameOutcome::Lost,
                "won" => EffectGameOutcome::Won,
                _ => return Err(ApiError::internal()),
            },
        },
    })
}

fn domain_effect_choice(choice: &PersistedEffectChoice) -> Result<PendingEffectChoice, ApiError> {
    Ok(PendingEffectChoice {
        id: choice.id.clone(),
        cause: choice.cause.clone(),
        responsible_position: choice.responsible_position,
        kind: match choice.kind.as_str() {
            "effect" => PendingEffectChoiceKind::Effect,
            "target" => PendingEffectChoiceKind::Target,
            _ => return Err(ApiError::internal()),
        },
        options: choice.options.clone(),
        min: choice.min,
        max: choice.max,
        continuation: domain_effect_continuation(&choice.continuation),
    })
}

fn domain_effect_continuation(continuation: &PersistedEffectContinuation) -> EffectContinuation {
    EffectContinuation {
        choice_cursor: domain_effect_cursor(&continuation.choice_cursor),
        queue: continuation
            .queue
            .iter()
            .map(domain_queued_effect)
            .collect(),
        steps_completed: continuation.steps_completed,
    }
}

fn domain_effect_cursor(cursor: &PersistedEffectCursor) -> EffectCursor {
    EffectCursor {
        rule_id: cursor.rule_id.clone(),
        path: cursor
            .path
            .iter()
            .map(|segment| match segment {
                PersistedEffectPathSegment::ChoiceOption { index } => {
                    EffectPathSegment::ChoiceOption(*index)
                }
                PersistedEffectPathSegment::ConditionThen => EffectPathSegment::ConditionThen,
                PersistedEffectPathSegment::ConditionOtherwise => {
                    EffectPathSegment::ConditionOtherwise
                }
                PersistedEffectPathSegment::RepeatEffect => EffectPathSegment::RepeatEffect,
                PersistedEffectPathSegment::RollOutcome { index } => {
                    EffectPathSegment::RollOutcome(*index)
                }
                PersistedEffectPathSegment::SequenceEffect { index } => {
                    EffectPathSegment::SequenceEffect(*index)
                }
            })
            .collect(),
    }
}

fn domain_queued_effect(queued: &PersistedQueuedEffect) -> QueuedEffect {
    match queued {
        PersistedQueuedEffect::Definition {
            cursor,
            actor_position,
        } => QueuedEffect::Definition {
            cursor: domain_effect_cursor(cursor),
            actor_position: *actor_position,
        },
        PersistedQueuedEffect::EffectChoice {
            cursor,
            responsible_position,
        } => QueuedEffect::EffectChoice {
            cursor: domain_effect_cursor(cursor),
            responsible_position: *responsible_position,
        },
    }
}

fn persisted_effects(state: &InitialGameState) -> PersistedEffects {
    let mut zone_indices = std::collections::BTreeMap::<(EffectZone, Option<u8>), u16>::new();
    let mut entities = state
        .effect_world()
        .entities()
        .map(|(zone, entity)| {
            let zone_name = effect_zone_name(zone);
            let next_index = zone_indices
                .entry((zone, entity.owner_position()))
                .or_default();
            let zone_index = *next_index;
            *next_index = next_index.saturating_add(1);
            PersistedEffectEntity {
                id: entity.id().to_owned(),
                kind: Some(effect_entity_kind_name(entity.kind()).to_owned()),
                catalog_id: entity.catalog_id().map(str::to_owned),
                owner_position: entity.owner_position(),
                effect_rule_id: entity.effect_rule_id().map(str::to_owned),
                influence_cost: entity.influence_cost(),
                zone: zone_name.to_owned(),
                zone_index: card_zone(zone_name).then_some(zone_index),
                resources: entity
                    .resources()
                    .iter()
                    .map(|(resource, amount)| (effect_resource_name(*resource).to_owned(), *amount))
                    .collect(),
            }
        })
        .collect::<Vec<_>>();
    entities.sort_by(|left, right| left.id.cmp(&right.id));
    PersistedEffects {
        entities,
        outcomes: state
            .last_effects()
            .iter()
            .map(persisted_effect_outcome)
            .collect(),
        choice: state.pending_choice().map(persisted_effect_choice),
    }
}

pub(super) fn persisted_effect_outcome(outcome: &EffectOutcome) -> PersistedEffectOutcome {
    match outcome {
        EffectOutcome::DieRolled {
            rule_id,
            die,
            result,
        } => PersistedEffectOutcome::DieRolled {
            rule_id: rule_id.clone(),
            die: effect_die_name(*die).to_owned(),
            result: *result,
        },
        EffectOutcome::Moved {
            rule_id,
            target_id,
            target_position,
            from,
            to,
        } => PersistedEffectOutcome::Moved {
            rule_id: rule_id.clone(),
            target_id: target_id.clone(),
            target_position: *target_position,
            from: effect_zone_name(*from).to_owned(),
            to: effect_zone_name(*to).to_owned(),
        },
        EffectOutcome::NoOp { rule_id, reason } => PersistedEffectOutcome::NoOp {
            rule_id: rule_id.clone(),
            reason: match reason {
                EffectNoOpReason::Explicit => "explicit",
                EffectNoOpReason::NoEligibleTarget => "no_eligible_target",
                EffectNoOpReason::ZeroCardinality => "zero_cardinality",
            }
            .to_owned(),
        },
        EffectOutcome::ResourceChanged {
            rule_id,
            target_id,
            target_position,
            resource,
            before,
            after,
            cause,
        } => PersistedEffectOutcome::ResourceChanged {
            rule_id: rule_id.clone(),
            target_id: target_id.clone(),
            target_position: *target_position,
            resource: effect_resource_name(*resource).to_owned(),
            before: *before,
            after: *after,
            cause: match cause {
                EffectChangeCause::Cost => "cost",
                EffectChangeCause::Effect => "effect",
            }
            .to_owned(),
        },
        EffectOutcome::Terminal { rule_id, outcome } => PersistedEffectOutcome::Terminal {
            rule_id: rule_id.clone(),
            outcome: match outcome {
                EffectGameOutcome::Lost => "lost",
                EffectGameOutcome::Won => "won",
            }
            .to_owned(),
        },
    }
}

fn persisted_effect_choice(choice: &PendingEffectChoice) -> PersistedEffectChoice {
    PersistedEffectChoice {
        id: choice.id.clone(),
        cause: choice.cause.clone(),
        responsible_position: choice.responsible_position,
        kind: match choice.kind {
            PendingEffectChoiceKind::Effect => "effect",
            PendingEffectChoiceKind::Target => "target",
        }
        .to_owned(),
        options: choice.options.clone(),
        min: choice.min,
        max: choice.max,
        continuation: persisted_effect_continuation(&choice.continuation),
    }
}

fn persisted_effect_continuation(continuation: &EffectContinuation) -> PersistedEffectContinuation {
    PersistedEffectContinuation {
        choice_cursor: persisted_effect_cursor(&continuation.choice_cursor),
        queue: continuation
            .queue
            .iter()
            .map(persisted_queued_effect)
            .collect(),
        steps_completed: continuation.steps_completed,
    }
}

fn persisted_effect_cursor(cursor: &EffectCursor) -> PersistedEffectCursor {
    PersistedEffectCursor {
        rule_id: cursor.rule_id.clone(),
        path: cursor
            .path
            .iter()
            .map(|segment| match segment {
                EffectPathSegment::ChoiceOption(index) => {
                    PersistedEffectPathSegment::ChoiceOption { index: *index }
                }
                EffectPathSegment::ConditionThen => PersistedEffectPathSegment::ConditionThen,
                EffectPathSegment::ConditionOtherwise => {
                    PersistedEffectPathSegment::ConditionOtherwise
                }
                EffectPathSegment::RepeatEffect => PersistedEffectPathSegment::RepeatEffect,
                EffectPathSegment::RollOutcome(index) => {
                    PersistedEffectPathSegment::RollOutcome { index: *index }
                }
                EffectPathSegment::SequenceEffect(index) => {
                    PersistedEffectPathSegment::SequenceEffect { index: *index }
                }
            })
            .collect(),
    }
}

fn persisted_queued_effect(queued: &QueuedEffect) -> PersistedQueuedEffect {
    match queued {
        QueuedEffect::Definition {
            cursor,
            actor_position,
        } => PersistedQueuedEffect::Definition {
            cursor: persisted_effect_cursor(cursor),
            actor_position: *actor_position,
        },
        QueuedEffect::EffectChoice {
            cursor,
            responsible_position,
        } => PersistedQueuedEffect::EffectChoice {
            cursor: persisted_effect_cursor(cursor),
            responsible_position: *responsible_position,
        },
    }
}

fn legacy_choice_rule_id(choice: &PersistedLegacyEffectChoice) -> Result<String, ApiError> {
    let marker = match choice.kind.as_str() {
        "effect" => ":effect:",
        "target" => ":target:",
        _ => return Err(ApiError::internal()),
    };
    let (rule_id, step) = choice
        .id
        .rsplit_once(marker)
        .ok_or_else(ApiError::internal)?;
    if !valid_legacy_identifier(rule_id) || step.parse::<u64>().is_err() {
        return Err(ApiError::internal());
    }
    Ok(rule_id.to_owned())
}

fn persisted_effect_target_binding(binding: &EffectTargetBinding) -> PersistedEffectTargetBinding {
    PersistedEffectTargetBinding {
        selector_id: binding.selector_id.clone(),
        target_ids: binding.target_ids.clone(),
    }
}

fn effect_stop_name(stop: &EffectStop) -> &'static str {
    match stop {
        EffectStop::Choice(_) => "choice",
        EffectStop::Stable => "stable",
        EffectStop::Terminal(_) => "terminal",
    }
}

fn domain_effect_resource(resource: &str) -> Result<EffectResource, ApiError> {
    match resource {
        "attack" => Ok(EffectResource::Attack),
        "control" => Ok(EffectResource::Control),
        "health" => Ok(EffectResource::Health),
        "influence" => Ok(EffectResource::Influence),
        _ => Err(ApiError::internal()),
    }
}

fn effect_resource_name(resource: EffectResource) -> &'static str {
    match resource {
        EffectResource::Attack => "attack",
        EffectResource::Control => "control",
        EffectResource::Health => "health",
        EffectResource::Influence => "influence",
    }
}

fn domain_effect_entity_kind(
    kind: Option<&str>,
    zone: EffectZone,
) -> Result<EffectEntityKind, ApiError> {
    match kind {
        None => Ok(if zone == EffectZone::Heroes {
            EffectEntityKind::Hero
        } else {
            EffectEntityKind::Generic
        }),
        Some("generic") => Ok(EffectEntityKind::Generic),
        Some("hero") => Ok(EffectEntityKind::Hero),
        Some("hogwarts_card") => Ok(EffectEntityKind::HogwartsCard),
        Some("starter_card") => Ok(EffectEntityKind::StarterCard),
        Some("villain") => Ok(EffectEntityKind::Villain),
        Some(_) => Err(ApiError::internal()),
    }
}

fn effect_entity_kind_name(kind: EffectEntityKind) -> &'static str {
    match kind {
        EffectEntityKind::Generic => "generic",
        EffectEntityKind::Hero => "hero",
        EffectEntityKind::HogwartsCard => "hogwarts_card",
        EffectEntityKind::StarterCard => "starter_card",
        EffectEntityKind::Villain => "villain",
    }
}

fn domain_effect_zone(zone: &str) -> Result<EffectZone, ApiError> {
    match zone {
        "active_location" => Ok(EffectZone::ActiveLocation),
        "active_villains" => Ok(EffectZone::ActiveVillains),
        "dark_arts_deck" => Ok(EffectZone::DarkArtsDeck),
        "dark_arts_discard" => Ok(EffectZone::DarkArtsDiscard),
        "hero_discard_pile" => Ok(EffectZone::HeroDiscardPile),
        "hero_draw_pile" => Ok(EffectZone::HeroDrawPile),
        "hero_hand" => Ok(EffectZone::HeroHand),
        "hero_play_area" => Ok(EffectZone::HeroPlayArea),
        "heroes" => Ok(EffectZone::Heroes),
        "hogwarts_deck" => Ok(EffectZone::HogwartsDeck),
        "market" => Ok(EffectZone::Market),
        "villain_deck" => Ok(EffectZone::VillainDeck),
        _ => Err(ApiError::internal()),
    }
}

fn effect_zone_name(zone: EffectZone) -> &'static str {
    match zone {
        EffectZone::ActiveLocation => "active_location",
        EffectZone::ActiveVillains => "active_villains",
        EffectZone::DarkArtsDeck => "dark_arts_deck",
        EffectZone::DarkArtsDiscard => "dark_arts_discard",
        EffectZone::HeroDiscardPile => "hero_discard_pile",
        EffectZone::HeroDrawPile => "hero_draw_pile",
        EffectZone::HeroHand => "hero_hand",
        EffectZone::HeroPlayArea => "hero_play_area",
        EffectZone::Heroes => "heroes",
        EffectZone::HogwartsDeck => "hogwarts_deck",
        EffectZone::Market => "market",
        EffectZone::VillainDeck => "villain_deck",
    }
}

fn domain_effect_die(die: &str) -> Result<EffectDie, ApiError> {
    match die {
        "d4" => Ok(EffectDie::D4),
        "d6" => Ok(EffectDie::D6),
        "d8" => Ok(EffectDie::D8),
        _ => Err(ApiError::internal()),
    }
}

fn effect_die_name(die: EffectDie) -> &'static str {
    match die {
        EffectDie::D4 => "d4",
        EffectDie::D6 => "d6",
        EffectDie::D8 => "d8",
    }
}

#[cfg(test)]
mod tests {
    use game_domain::{
        EffectOutcome, EffectStop, EffectTargetBinding, EffectZone, GameEvent, GamePhase,
    };
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    use super::{
        MAX_PERSISTED_JSON_BYTES, command_domain_state, compact_json_size, decode_persisted_event,
        decode_persisted_snapshot, persisted_after_decision, persisted_event,
        validate_persisted_json_size,
    };
    use crate::match_runtime::{
        PersistedEffectEntity, PersistedEffects, PersistedPlayer, PersistedPrng, PersistedSnapshot,
        PersistedTurn, PersistedVersions,
    };

    fn current_choice() -> Value {
        json!({
            "id": "rule:functional:effect:0",
            "cause": "rule:functional",
            "responsible_position": 1,
            "kind": "effect",
            "options": ["option:1", "option:2"],
            "min": 1,
            "max": 1,
            "continuation": {
                "choice_cursor": {
                    "rule_id": "rule:functional",
                    "path": []
                },
                "queue": [],
                "steps_completed": 1
            }
        })
    }

    fn choice_event(choice: &Value) -> Value {
        json!({
            "event_version": 3,
            "type": "dark_arts_completed",
            "sequence": 1,
            "state_version": 2,
            "turn": 1,
            "actor_position": 1,
            "effects": [],
            "effect_stop": "choice",
            "choice": choice,
            "prng_counter": 0
        })
    }

    #[test]
    fn v3_event_codec_rejects_legacy_and_oversized_choices_while_preserving_v2() {
        let current = choice_event(&current_choice());
        assert!(decode_persisted_event(&current.to_string()).is_ok());

        let mut played_card = choice_event(&current_choice());
        played_card["type"] = json!("card_played");
        played_card["card_id"] = json!("instance:starter:1");
        played_card["targets"] = json!([]);
        assert!(decode_persisted_event(&played_card.to_string()).is_ok());

        let legacy_choice = json!({
            "id": "rule:functional:effect:0",
            "responsible_position": 1,
            "kind": "effect",
            "options": ["option:1", "option:2"],
            "min": 1,
            "max": 1
        });
        let legacy_v3 = choice_event(&legacy_choice);
        assert!(decode_persisted_event(&legacy_v3.to_string()).is_err());

        let mut oversized_path = current.clone();
        oversized_path["choice"]["continuation"]["choice_cursor"]["path"] =
            Value::Array(vec![json!({ "type": "condition_then" }); 4097]);
        assert!(decode_persisted_event(&oversized_path.to_string()).is_err());

        let mut oversized_option = current;
        oversized_option["choice"]["options"][0] = json!("x".repeat(257));
        assert!(decode_persisted_event(&oversized_option.to_string()).is_err());

        let mut invalid_effect = choice_event(&current_choice());
        invalid_effect["choice"]["min"] = json!(0);
        assert!(decode_persisted_event(&invalid_effect.to_string()).is_err());

        let mut target_selects_all = choice_event(&current_choice());
        target_selects_all["choice"]["kind"] = json!("target");
        target_selects_all["choice"]["min"] = json!(0);
        target_selects_all["choice"]["max"] = json!(2);
        assert!(decode_persisted_event(&target_selects_all.to_string()).is_err());

        let mut target_selects_none = choice_event(&current_choice());
        target_selects_none["choice"]["kind"] = json!("target");
        target_selects_none["choice"]["min"] = json!(0);
        target_selects_none["choice"]["max"] = json!(0);
        assert!(decode_persisted_event(&target_selects_none.to_string()).is_err());

        let legacy_v2 = json!({
            "event_version": 2,
            "type": "dark_arts_completed",
            "sequence": 1,
            "state_version": 2,
            "turn": 1,
            "actor_position": 1,
            "effects": [],
            "effect_stop": "choice",
            "choice": legacy_choice,
            "prng_counter": 0
        });
        assert!(decode_persisted_event(&legacy_v2.to_string()).is_ok());
    }

    #[test]
    fn persisted_json_size_ignores_formatting_but_counts_string_whitespace() {
        assert_eq!(compact_json_size(r#"{ "value": "a, b: c" }"#), 19);
        assert!(
            validate_persisted_json_size(&format!(
                "[{}{}]",
                " ".repeat(MAX_PERSISTED_JSON_BYTES),
                "0"
            ))
            .is_ok()
        );
        assert!(
            validate_persisted_json_size(&format!("\"{}\"", " ".repeat(MAX_PERSISTED_JSON_BYTES)))
                .is_err()
        );
    }

    #[test]
    fn v2_hero_action_snapshot_upcasts_with_structural_phase_history() {
        let snapshot = json!({
            "snapshot_version": 2,
            "state_version": 3,
            "sequence": 2,
            "status": "in_progress",
            "adventure_id": "adventure:test",
            "versions": {
                "content": "1.0.0",
                "ruleset": "1.0.0",
                "manifest": 1,
                "manifest_digest": format!("blake3:{}", "0".repeat(64)),
                "prng": "chacha20-v1",
                "shuffle": "fisher-yates-v1",
                "sampling": "rejection-sampling-v1"
            },
            "turn": {
                "number": 1,
                "phase": "hero_action",
                "active_position": 1
            },
            "participants": [
                {
                    "participant_id": "00000000-0000-0000-0000-000000000001",
                    "position": 1,
                    "hero_id": "harry"
                },
                {
                    "participant_id": "00000000-0000-0000-0000-000000000002",
                    "position": 2,
                    "hero_id": "hermione"
                }
            ],
            "prng": { "algorithm": "chacha20-v1", "counter": 0 },
            "effects": {
                "outcomes": [{
                    "type": "no_op",
                    "rule_id": "rule:legacy",
                    "reason": "explicit"
                }]
            }
        });

        let persisted = decode_persisted_snapshot(&snapshot.to_string())
            .unwrap_or_else(|_| panic!("the v2 snapshot should decode"));
        let state = command_domain_state(&persisted)
            .unwrap_or_else(|_| panic!("the v2 snapshot should restore"));

        assert_eq!(state.phase(), GamePhase::HeroActions);
        assert_eq!(state.queued_phases(), [GamePhase::EndTurn]);
        assert_eq!(state.last_turn_steps().len(), 2);
        assert_eq!(state.last_turn_steps()[0].phase(), GamePhase::DarkArts);
        assert_eq!(state.last_turn_steps()[1].phase(), GamePhase::Villains);
    }

    #[test]
    fn v2_pending_choice_upcasts_its_continuation_for_a_non_active_responsible_player() {
        let snapshot = json!({
            "snapshot_version": 2,
            "state_version": 2,
            "sequence": 1,
            "status": "in_progress",
            "adventure_id": "adventure:test",
            "versions": {
                "content": "1.0.0",
                "ruleset": "1.0.0",
                "manifest": 1,
                "manifest_digest": format!("blake3:{}", "0".repeat(64)),
                "prng": "chacha20-v1",
                "shuffle": "fisher-yates-v1",
                "sampling": "rejection-sampling-v1"
            },
            "turn": {
                "number": 1,
                "phase": "dark_arts",
                "active_position": 1
            },
            "participants": [
                {
                    "participant_id": "00000000-0000-0000-0000-000000000001",
                    "position": 1,
                    "hero_id": "harry"
                },
                {
                    "participant_id": "00000000-0000-0000-0000-000000000002",
                    "position": 2,
                    "hero_id": "hermione"
                }
            ],
            "prng": { "algorithm": "chacha20-v1", "counter": 0 },
            "effects": {
                "outcomes": [{
                    "type": "no_op",
                    "rule_id": "rule:legacy",
                    "reason": "explicit"
                }],
                "choice": {
                    "id": "choice:effect:0",
                    "cause": "rule:legacy",
                    "responsible_position": 2,
                    "kind": "effect",
                    "options": ["option:1", "option:2"],
                    "min": 1,
                    "max": 1,
                    "continuation": {
                        "choice_cursor": { "rule_id": "rule:legacy", "path": [] },
                        "queue": [{
                            "type": "definition",
                            "cursor": { "rule_id": "rule:after", "path": [] },
                            "actor_position": 1
                        }],
                        "steps_completed": 1
                    }
                }
            }
        });

        let persisted = decode_persisted_snapshot(&snapshot.to_string())
            .unwrap_or_else(|_| panic!("the v2 snapshot should decode"));
        let state = command_domain_state(&persisted)
            .unwrap_or_else(|_| panic!("the v2 choice should restore"));

        assert_eq!(state.active_position(), 1);
        assert_eq!(
            state
                .pending_choice()
                .expect("the choice should remain pending")
                .responsible_position,
            2
        );
        assert_eq!(state.queued_effects().len(), 1);
        assert_eq!(state.last_turn_steps().len(), 1);
    }

    #[test]
    fn v3_structured_choice_accepts_a_queued_definition_for_the_responsible_player() {
        let choice = json!({
            "id": "choice:nested:effect:0",
            "cause": "rule:nested",
            "responsible_position": 2,
            "kind": "effect",
            "options": ["option:1", "option:2"],
            "min": 1,
            "max": 1,
            "continuation": {
                "choice_cursor": {
                    "rule_id": "rule:nested",
                    "path": [
                        { "type": "choice_option", "index": 0 },
                        { "type": "sequence_effect", "index": 0 }
                    ]
                },
                "queue": [{
                    "type": "definition",
                    "cursor": {
                        "rule_id": "rule:nested",
                        "path": [
                            { "type": "choice_option", "index": 0 },
                            { "type": "sequence_effect", "index": 1 }
                        ]
                    },
                    "actor_position": 2
                }],
                "steps_completed": 1
            }
        });
        let snapshot = json!({
            "snapshot_version": 3,
            "state_version": 2,
            "sequence": 1,
            "status": "in_progress",
            "adventure_id": "adventure:test",
            "versions": {
                "content": "1.0.0",
                "ruleset": "1.0.0",
                "manifest": 1,
                "manifest_digest": format!("blake3:{}", "0".repeat(64)),
                "prng": "chacha20-v1",
                "shuffle": "fisher-yates-v1",
                "sampling": "rejection-sampling-v1"
            },
            "turn": {
                "number": 1,
                "phase": "dark_arts",
                "active_position": 1
            },
            "queued_phases": ["villains", "hero_actions", "end_turn"],
            "queued_effects": choice["continuation"]["queue"].clone(),
            "decision_point": {
                "type": "effect_choice",
                "choice": choice.clone()
            },
            "last_turn_steps": [{ "phase": "dark_arts", "effects": [] }],
            "participants": [
                {
                    "participant_id": "00000000-0000-0000-0000-000000000001",
                    "position": 1,
                    "hero_id": "harry"
                },
                {
                    "participant_id": "00000000-0000-0000-0000-000000000002",
                    "position": 2,
                    "hero_id": "hermione"
                }
            ],
            "prng": { "algorithm": "chacha20-v1", "counter": 0 },
            "effects": { "choice": choice }
        });

        let persisted = decode_persisted_snapshot(&snapshot.to_string())
            .unwrap_or_else(|_| panic!("the v3 structured snapshot should decode"));
        let state = command_domain_state(&persisted)
            .unwrap_or_else(|_| panic!("the nested non-active choice should restore"));

        assert_eq!(state.active_position(), 1);
        assert_eq!(
            state
                .pending_choice()
                .expect("the choice should remain pending")
                .responsible_position,
            2
        );
        assert_eq!(state.queued_effects().len(), 1);
        assert_eq!(state.queued_effects()[0].actor_position(), 2);
    }

    #[test]
    fn a_played_card_event_persists_every_explicit_fact_in_v4() {
        let result = persisted_event(GameEvent::CardPlayed {
            sequence: 7,
            state_version: 8,
            turn: 2,
            actor_position: 1,
            card_id: "instance:starter:1".to_owned(),
            targets: vec![EffectTargetBinding {
                selector_id: "target:ally".to_owned(),
                target_ids: vec!["hero:2".to_owned()],
            }],
            effects: vec![EffectOutcome::Moved {
                rule_id: "system:play-card".to_owned(),
                target_id: "instance:starter:1".to_owned(),
                target_position: Some(1),
                from: EffectZone::HeroHand,
                to: EffectZone::HeroPlayArea,
            }],
            stop: EffectStop::Stable,
            prng_counter: 3,
        });
        let (event_version, event_type, serialized) =
            result.ok().expect("the event must serialize");

        assert_eq!(event_version, 4);
        assert_eq!(event_type, "card_played");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&serialized)
                .expect("the persisted event must be JSON"),
            json!({
                "event_version": 4,
                "type": "card_played",
                "sequence": 7,
                "state_version": 8,
                "turn": 2,
                "actor_position": 1,
                "card_id": "instance:starter:1",
                "targets": [{
                    "selector_id": "target:ally",
                    "target_ids": ["hero:2"]
                }],
                "effects": [{
                    "type": "moved",
                    "rule_id": "system:play-card",
                    "target_id": "instance:starter:1",
                    "target_position": 1,
                    "from": "hero_hand",
                    "to": "hero_play_area"
                }],
                "effect_stop": "stable",
                "choice": null,
                "prng_counter": 3
            })
        );
    }

    #[test]
    fn attack_and_acquisition_events_persist_only_their_domain_facts() {
        let attack = serialized_event(GameEvent::AttackAssigned {
            sequence: 8,
            state_version: 9,
            turn: 2,
            actor_position: 1,
            villain_id: "instance:villain:1".to_owned(),
            amount: 2,
            effects: Vec::new(),
        });
        assert_eq!(
            attack,
            json!({
                "event_version": 4,
                "type": "attack_assigned",
                "sequence": 8,
                "state_version": 9,
                "turn": 2,
                "actor_position": 1,
                "villain_id": "instance:villain:1",
                "amount": 2,
                "effects": []
            })
        );
        assert!(attack.get("effect_stop").is_none());
        assert!(attack.get("prng_counter").is_none());

        let acquisition = serialized_event(GameEvent::CardAcquired {
            sequence: 9,
            state_version: 10,
            turn: 2,
            actor_position: 1,
            card_id: "instance:market:1".to_owned(),
            cost: 3,
            refill_card_id: None,
            effects: Vec::new(),
        });
        assert_eq!(
            acquisition,
            json!({
                "event_version": 4,
                "type": "card_acquired",
                "sequence": 9,
                "state_version": 10,
                "turn": 2,
                "actor_position": 1,
                "card_id": "instance:market:1",
                "cost": 3,
                "refill_card_id": null,
                "effects": []
            })
        );
        assert!(acquisition.get("effect_stop").is_none());
        assert!(acquisition.get("prng_counter").is_none());
    }

    #[test]
    fn persisted_event_decoder_accepts_v1_through_v4_but_rejects_future_versions() {
        for serialized in [
            json!({
                "event_version": 1,
                "type": "dark_arts_completed",
                "sequence": 1,
                "state_version": 2,
                "turn": 1,
                "actor_position": 1
            }),
            json!({
                "event_version": 2,
                "type": "dark_arts_completed",
                "sequence": 2,
                "state_version": 3,
                "turn": 1,
                "actor_position": 1,
                "effects": [],
                "effect_stop": "stable",
                "choice": null,
                "prng_counter": 0
            }),
            json!({
                "event_version": 4,
                "type": "attack_assigned",
                "sequence": 3,
                "state_version": 4,
                "turn": 1,
                "actor_position": 1,
                "villain_id": "instance:villain:1",
                "amount": 1,
                "effects": []
            }),
        ] {
            assert!(decode_persisted_event(&serialized.to_string()).is_ok());
        }

        let future = json!({
            "event_version": 5,
            "type": "attack_assigned",
            "sequence": 4,
            "state_version": 5,
            "turn": 1,
            "actor_position": 1,
            "villain_id": "instance:villain:1",
            "amount": 1,
            "effects": []
        });
        assert!(decode_persisted_event(&future.to_string()).is_err());
    }

    #[test]
    fn a_snapshot_restores_legacy_kinds_and_preserves_stack_order_and_card_metadata() {
        let snapshot = PersistedSnapshot {
            snapshot_version: 1,
            state_version: 1,
            sequence: 0,
            status: "in_progress".to_owned(),
            adventure_id: "adventure:codec".to_owned(),
            versions: PersistedVersions {
                content: "content-v1".to_owned(),
                ruleset: "rules-v1".to_owned(),
                manifest: 1,
                manifest_digest: format!("blake3:{}", "a".repeat(64)),
                prng: "chacha20-v1".to_owned(),
                shuffle: "fisher-yates-v1".to_owned(),
                sampling: "rejection-sampling-v1".to_owned(),
            },
            turn: PersistedTurn {
                number: 1,
                phase: "hero_action".to_owned(),
                active_position: 1,
            },
            queued_phases: None,
            queued_effects: None,
            decision_point: None,
            last_turn_steps: None,
            participants: vec![
                PersistedPlayer {
                    participant_id: "participant:1".to_owned(),
                    position: 1,
                    hero_id: "harry".to_owned(),
                },
                PersistedPlayer {
                    participant_id: "participant:2".to_owned(),
                    position: 2,
                    hero_id: "hermione".to_owned(),
                },
            ],
            prng: PersistedPrng {
                algorithm: "chacha20-v1".to_owned(),
                counter: 0,
            },
            effects: PersistedEffects {
                entities: vec![
                    legacy_entity("hero:1", Some(1), "heroes"),
                    legacy_entity("hero:2", Some(2), "heroes"),
                    persisted_hogwarts_card("instance:deck:b", "card:b", 4),
                    persisted_hogwarts_card("instance:deck:a", "card:a", 2),
                    legacy_entity("instance:legacy", None, "dark_arts_deck"),
                ],
                outcomes: Vec::new(),
                choice: None,
            },
        };

        let state = command_domain_state(&snapshot)
            .ok()
            .expect("the compatible snapshot must restore");
        let deck = state.effect_world().entities_in(EffectZone::HogwartsDeck);
        assert_eq!(
            deck.iter()
                .map(game_domain::EffectEntity::id)
                .collect::<Vec<_>>(),
            vec!["instance:deck:b", "instance:deck:a"]
        );
        assert_eq!(deck[0].catalog_id(), Some("card:b"));
        assert_eq!(deck[0].effect_rule_id(), Some("rule:card:b"));
        assert_eq!(deck[0].influence_cost(), Some(4));
        assert_eq!(
            state
                .effect_world()
                .entity("hero:1")
                .map(|(_, entity)| entity.kind()),
            Some(game_domain::EffectEntityKind::Hero)
        );
        assert_eq!(
            state
                .effect_world()
                .entity("instance:legacy")
                .map(|(_, entity)| entity.kind()),
            Some(game_domain::EffectEntityKind::Generic)
        );

        let repersisted = persisted_after_decision(&snapshot, &state);
        let repersisted_deck = sorted_zone_entities(&repersisted, "hogwarts_deck");
        assert_eq!(
            repersisted_deck
                .iter()
                .map(|entity| entity.id.as_str())
                .collect::<Vec<_>>(),
            vec!["instance:deck:b", "instance:deck:a"]
        );
        assert_eq!(repersisted_deck[0].kind.as_deref(), Some("hogwarts_card"));
        assert_eq!(repersisted_deck[0].catalog_id.as_deref(), Some("card:b"));
        assert_eq!(
            repersisted_deck[0].effect_rule_id.as_deref(),
            Some("rule:card:b")
        );
        assert_eq!(repersisted_deck[0].influence_cost, Some(4));
    }

    fn sorted_zone_entities<'a>(
        snapshot: &'a PersistedSnapshot,
        zone: &str,
    ) -> Vec<&'a PersistedEffectEntity> {
        let mut entities = snapshot
            .effects
            .entities
            .iter()
            .filter(|entity| entity.zone == zone)
            .collect::<Vec<_>>();
        entities.sort_by_key(|entity| entity.zone_index);
        entities
    }

    fn legacy_entity(id: &str, owner_position: Option<u8>, zone: &str) -> PersistedEffectEntity {
        PersistedEffectEntity {
            id: id.to_owned(),
            kind: None,
            catalog_id: None,
            owner_position,
            effect_rule_id: None,
            influence_cost: None,
            zone: zone.to_owned(),
            zone_index: None,
            resources: BTreeMap::new(),
        }
    }

    fn persisted_hogwarts_card(
        id: &str,
        catalog_id: &str,
        influence_cost: u16,
    ) -> PersistedEffectEntity {
        PersistedEffectEntity {
            id: id.to_owned(),
            kind: Some("hogwarts_card".to_owned()),
            catalog_id: Some(catalog_id.to_owned()),
            owner_position: None,
            effect_rule_id: Some(format!("rule:{catalog_id}")),
            influence_cost: Some(influence_cost),
            zone: "hogwarts_deck".to_owned(),
            zone_index: None,
            resources: BTreeMap::new(),
        }
    }

    fn serialized_event(event: GameEvent) -> serde_json::Value {
        let result = persisted_event(event);
        let (_, _, serialized) = result.ok().expect("the event must serialize");
        serde_json::from_str(&serialized).expect("the persisted event must be JSON")
    }
}
