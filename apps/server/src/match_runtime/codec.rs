use game_domain::{
    EffectChangeCause, EffectContinuation, EffectCursor, EffectDie, EffectEntity,
    EffectGameOutcome, EffectNoOpReason, EffectOutcome, EffectPathSegment, EffectResource,
    EffectStop, EffectWorld, EffectZone, GameEvent, GamePhase, GameStateRestoreInput, GameStatus,
    InitialGameState, InitialPlayer, PendingEffectChoice, PendingEffectChoiceKind, QueuedEffect,
    SNAPSHOT_VERSION, restore_game_state,
};

use super::{
    GAME_EVENT_VERSION, PersistedEffectChoice, PersistedEffectContinuation, PersistedEffectCursor,
    PersistedEffectEntity, PersistedEffectOutcome, PersistedEffectPathSegment, PersistedEffects,
    PersistedGameEvent, PersistedPlayer, PersistedPrng, PersistedQueuedEffect, PersistedSnapshot,
    PersistedTurn, PersistedVersions, StoredCommandGame, StoredGame, StoredRoomParticipant,
    hero_id,
};
use crate::http_support::ApiError;

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
    let status = match persisted.status.as_str() {
        "in_progress" => GameStatus::InProgress,
        "lost" => GameStatus::Lost,
        "won" => GameStatus::Won,
        _ => return Err(ApiError::internal()),
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
    })
    .map_err(|error| ApiError::internal_with("match application operation", error))
}

pub(super) fn decode_persisted_snapshot(serialized: &str) -> Result<PersistedSnapshot, ApiError> {
    let snapshot: PersistedSnapshot = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    if !matches!(snapshot.snapshot_version, 1 | SNAPSHOT_VERSION)
        || (snapshot.snapshot_version == 1 && snapshot.effects.choice.is_some())
    {
        return Err(ApiError::internal());
    }
    Ok(snapshot)
}

pub(super) fn decode_persisted_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedGameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    let stop_and_choice_are_valid = match event.effect_stop.as_str() {
        "choice" => event.choice.as_ref().is_some_and(|choice| {
            event.event_version != GAME_EVENT_VERSION || valid_v3_effect_choice(choice)
        }),
        "stable" | "terminal" => event.choice.is_none(),
        _ => false,
    };
    let version_and_type_are_supported = match event.event_version {
        1 | 2 => {
            event.event_type == "dark_arts_completed"
                && event.choice_id.is_none()
                && event.choice_cause.is_none()
                && event.selected_options.is_none()
        }
        GAME_EVENT_VERSION => match event.event_type.as_str() {
            "dark_arts_completed" => {
                event.choice_id.is_none()
                    && event.choice_cause.is_none()
                    && event.selected_options.is_none()
            }
            "choice_resolved" => {
                event.choice_id.as_deref().is_some_and(valid_choice_value)
                    && event
                        .choice_cause
                        .as_deref()
                        .is_some_and(valid_choice_value)
                    && event.selected_options.as_deref().is_some_and(|selected| {
                        selected.len() <= 32
                            && selected.iter().all(|value| valid_choice_value(value))
                            && selected
                                .iter()
                                .collect::<std::collections::BTreeSet<_>>()
                                .len()
                                == selected.len()
                    })
            }
            _ => false,
        },
        _ => false,
    };
    if !version_and_type_are_supported || !stop_and_choice_are_valid {
        return Err(ApiError::internal());
    }
    Ok(event)
}

fn valid_choice_value(value: &str) -> bool {
    (1..=256).contains(&value.chars().count())
}

fn valid_v3_effect_choice(choice: &super::PersistedEventChoice) -> bool {
    let super::PersistedEventChoice::Current(choice) = choice else {
        return false;
    };
    valid_choice_value(&choice.id)
        && valid_choice_value(&choice.cause)
        && (1..=4).contains(&choice.responsible_position)
        && match choice.kind.as_str() {
            "effect" => choice.min == 1 && choice.max == 1,
            "target" => choice.max > 0 && usize::from(choice.max) < choice.options.len(),
            _ => false,
        }
        && (2..=4096).contains(&choice.options.len())
        && choice
            .options
            .iter()
            .all(|option| valid_choice_value(option))
        && choice
            .options
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == choice.options.len()
        && choice.min <= choice.max
        && choice.max <= 32
        && usize::from(choice.max) <= choice.options.len()
        && choice.cause == choice.continuation.choice_cursor.rule_id
        && valid_v3_effect_continuation(&choice.continuation)
}

fn valid_v3_effect_continuation(continuation: &PersistedEffectContinuation) -> bool {
    valid_v3_effect_cursor(&continuation.choice_cursor)
        && continuation.queue.len() <= 4096
        && (1..=4096).contains(&continuation.steps_completed)
        && continuation.queue.iter().all(|queued| match queued {
            PersistedQueuedEffect::Definition {
                cursor,
                actor_position,
            } => (1..=4).contains(actor_position) && valid_v3_effect_cursor(cursor),
            PersistedQueuedEffect::EffectChoice {
                cursor,
                responsible_position,
            } => (1..=4).contains(responsible_position) && valid_v3_effect_cursor(cursor),
        })
}

fn valid_v3_effect_cursor(cursor: &PersistedEffectCursor) -> bool {
    !cursor.rule_id.is_empty() && cursor.path.len() <= 4096
}

pub(super) fn persisted_after_decision(
    current: &PersistedSnapshot,
    state: &InitialGameState,
) -> PersistedSnapshot {
    let mut next = current.clone();
    next.snapshot_version =
        if current.snapshot_version == SNAPSHOT_VERSION || state.pending_choice().is_some() {
            SNAPSHOT_VERSION
        } else {
            1
        };
    next.state_version = state.state_version();
    next.sequence = state.sequence();
    next.status = match state.status() {
        GameStatus::InProgress => "in_progress".to_owned(),
        GameStatus::Lost => "lost".to_owned(),
        GameStatus::Won => "won".to_owned(),
    };
    next.turn.number = state.turn();
    next.turn.phase = match state.phase() {
        GamePhase::DarkArts => "dark_arts".to_owned(),
        GamePhase::HeroAction => "hero_action".to_owned(),
    };
    next.turn.active_position = state.active_position();
    next.prng.counter = state.prng_counter();
    next.effects = persisted_effects(state);
    next
}

pub(super) fn persisted_event(event: GameEvent) -> Result<(u16, &'static str, String), ApiError> {
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
                GAME_EVENT_VERSION
            } else {
                2
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
            effects,
            stop,
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
            "effects": effects.iter().map(persisted_effect_outcome).collect::<Vec<_>>(),
            "effect_stop": effect_stop_name(&stop),
            "choice": match &stop {
                EffectStop::Choice(choice) => Some(persisted_effect_choice(choice)),
                EffectStop::Stable | EffectStop::Terminal(_) => None,
            },
            "prng_counter": prng_counter,
        }))
        .map(|event| (GAME_EVENT_VERSION, "choice_resolved", event))
        .map_err(|error| ApiError::internal_with("match application operation", error)),
    }
}

pub(super) fn persisted_snapshot(
    state: &InitialGameState,
    participants: &[StoredRoomParticipant],
) -> PersistedSnapshot {
    PersistedSnapshot {
        snapshot_version: if state.pending_choice().is_some() {
            SNAPSHOT_VERSION
        } else {
            1
        },
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
        effects: persisted_effects(state),
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
                .map(|player| EffectEntity::hero(player.position()))
                .collect(),
        ));
    }

    persisted
        .entities
        .iter()
        .map(|entity| {
            let mut domain = EffectEntity::new(
                entity.id.clone(),
                entity.owner_position,
                domain_effect_zone(&entity.zone)?,
            );
            for (resource, amount) in &entity.resources {
                domain = domain.with_resource(domain_effect_resource(resource)?, *amount);
            }
            Ok(domain)
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
    PersistedEffects {
        entities: state
            .effect_world()
            .entities()
            .iter()
            .map(|entity| PersistedEffectEntity {
                id: entity.id().to_owned(),
                owner_position: entity.owner_position(),
                zone: effect_zone_name(entity.zone()).to_owned(),
                resources: entity
                    .resources()
                    .iter()
                    .map(|(resource, amount)| (effect_resource_name(*resource).to_owned(), *amount))
                    .collect(),
            })
            .collect(),
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
    use serde_json::{Value, json};

    use super::decode_persisted_event;

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
}
