use game_domain::{
    EffectChangeCause, EffectDie, EffectEntity, EffectGameOutcome, EffectNoOpReason, EffectOutcome,
    EffectResource, EffectStop, EffectWorld, EffectZone, GameEvent, GamePhase,
    GameStateRestoreInput, GameStatus, InitialGameState, InitialPlayer, PendingEffectChoice,
    PendingEffectChoiceKind, SNAPSHOT_VERSION, restore_game_state,
};

use super::{
    GAME_EVENT_VERSION, PersistedEffectChoice, PersistedEffectEntity, PersistedEffectOutcome,
    PersistedEffects, PersistedGameEvent, PersistedPlayer, PersistedPrng, PersistedSnapshot,
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
    if snapshot.snapshot_version != SNAPSHOT_VERSION {
        return Err(ApiError::internal());
    }
    Ok(snapshot)
}

pub(super) fn decode_persisted_event(serialized: &str) -> Result<PersistedGameEvent, ApiError> {
    let event: PersistedGameEvent = serde_json::from_str(serialized)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    if event.event_version == 0 || event.event_version > GAME_EVENT_VERSION {
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

pub(super) fn persisted_event(event: GameEvent) -> Result<(&'static str, String), ApiError> {
    match event {
        GameEvent::DarkArtsCompleted {
            sequence,
            state_version,
            turn,
            actor_position,
            effects,
            stop,
            prng_counter,
        } => serde_json::to_string(&serde_json::json!({
            "event_version": GAME_EVENT_VERSION,
            "type": "dark_arts_completed",
            "sequence": sequence,
            "state_version": state_version,
            "turn": turn,
            "actor_position": actor_position,
            "effects": effects.iter().map(persisted_effect_outcome).collect::<Vec<_>>(),
            "effect_stop": effect_stop_name(&stop),
            "choice": match stop {
                EffectStop::Choice(choice) => Some(persisted_effect_choice(&choice)),
                EffectStop::Stable | EffectStop::Terminal(_) => None,
            },
            "prng_counter": prng_counter,
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
        responsible_position: choice.responsible_position,
        kind: match choice.kind.as_str() {
            "effect" => PendingEffectChoiceKind::Effect,
            "target" => PendingEffectChoiceKind::Target,
            _ => return Err(ApiError::internal()),
        },
        options: choice.options.clone(),
        min: choice.min,
        max: choice.max,
    })
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
        responsible_position: choice.responsible_position,
        kind: match choice.kind {
            PendingEffectChoiceKind::Effect => "effect",
            PendingEffectChoiceKind::Target => "target",
        }
        .to_owned(),
        options: choice.options.clone(),
        min: choice.min,
        max: choice.max,
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
