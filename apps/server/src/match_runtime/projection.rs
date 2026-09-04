use game_domain::{
    EffectChangeCause, EffectDie, EffectGameOutcome, EffectNoOpReason, EffectOutcome,
    EffectResource, EffectZone, GameEngine, GameStatus, InitialGameState, PendingEffectChoiceKind,
    PlayerIntentType, ValidatedGameRules,
};
use serde::Serialize;
use uuid::Uuid;

use super::{
    StoredRoomParticipant, codec::command_domain_state, codec::decode_persisted_snapshot,
    codec::game_phase_name, codec::verify_persisted_snapshot, hero_name, postgres,
};
use crate::{AppState, http_support::ApiError};

#[derive(Serialize)]
pub(crate) struct GameProjectionResponse {
    pub(super) game: GameSummary,
    pub(super) snapshot: SnapshotSummary,
    turn: TurnSummary,
    participant: GameParticipant,
    participants: Vec<GameParticipant>,
    legal_actions: Vec<String>,
    choice: ChoiceSummary,
    queued_phases: Vec<String>,
    queued_effect_count: usize,
    effects: EffectResolutionSummary,
}

#[derive(Serialize)]
pub(super) struct GameSummary {
    pub(super) id: String,
    status: String,
    adventure: AdventureSummary,
    expires_at: String,
}

#[derive(Serialize)]
struct AdventureSummary {
    id: String,
    name: String,
}

#[derive(Serialize)]
pub(super) struct SnapshotSummary {
    pub(super) snapshot_version: i16,
    state_version: i64,
    sequence: i64,
    pub(super) cursor: i64,
    pub(super) digest: String,
    versions: GameVersions,
}

#[derive(Serialize)]
struct ChoiceSummary {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cause: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    responsible_position: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    options: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<u16>,
}

#[derive(Serialize)]
struct EffectResolutionSummary {
    status: &'static str,
    outcomes: Vec<EffectOutcomeSummary>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum EffectOutcomeSummary {
    DieRolled {
        rule_id: String,
        die: &'static str,
        result: u8,
    },
    Moved {
        rule_id: String,
        target_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_position: Option<u8>,
        from: &'static str,
        to: &'static str,
    },
    NoOp {
        rule_id: String,
        reason: &'static str,
    },
    ResourceChanged {
        rule_id: String,
        target_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_position: Option<u8>,
        resource: &'static str,
        before: u16,
        after: u16,
        cause: &'static str,
    },
    Terminal {
        rule_id: String,
        outcome: &'static str,
    },
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
    resources: GameResources,
}

#[derive(Serialize)]
struct GameHero {
    id: String,
    name: &'static str,
}

#[derive(Serialize)]
struct GameResources {
    health: u16,
    attack: u16,
    influence: u16,
}

pub(crate) async fn projection_for_participant(
    state: &AppState,
    participant_id: Uuid,
) -> Result<Option<GameProjectionResponse>, ApiError> {
    let Some(game) = postgres::game_for_participant(&state.database, participant_id).await? else {
        return Ok(None);
    };
    let persisted = decode_persisted_snapshot(&game.snapshot_json)?;
    verify_persisted_snapshot(&game, &persisted)?;
    let participants = postgres::game_participants(&state.database, game.id).await?;
    let current = participants
        .iter()
        .find(|participant| participant.id == participant_id)
        .ok_or_else(ApiError::internal)?;
    let domain_state = command_domain_state(&persisted)?;
    let actor_position = u8::try_from(current.position)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    let effect_rules = state
        .content
        .effect_rules(&game.manifest_digest)
        .ok_or_else(ApiError::internal)?;
    let rules = ValidatedGameRules::new(effect_rules).map_err(|_| ApiError::internal())?;
    let legal_actions = if game.expired {
        Vec::new()
    } else {
        GameEngine::new(&rules)
            .legal_intent_types(&domain_state, actor_position)
            .into_iter()
            .map(|intent_type| match intent_type {
                PlayerIntentType::EndHeroActions => "end_hero_actions".to_owned(),
                PlayerIntentType::ResolveChoice => "resolve_choice".to_owned(),
            })
            .collect()
    };

    Ok(Some(GameProjectionResponse {
        game: GameSummary {
            id: game.id.to_string(),
            status: game.status,
            adventure: AdventureSummary {
                id: game.adventure_id,
                name: game.adventure_name,
            },
            expires_at: game.expires_at,
        },
        snapshot: SnapshotSummary {
            snapshot_version: game.snapshot_version,
            state_version: game.state_version,
            sequence: game.sequence,
            cursor: game.sequence,
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
            number: domain_state.turn(),
            phase: game_phase_name(domain_state.phase()).to_owned(),
            active_position: domain_state.active_position(),
        },
        participant: game_participant(current, &domain_state)?,
        participants: participants
            .iter()
            .map(|participant| game_participant(participant, &domain_state))
            .collect::<Result<Vec<_>, _>>()?,
        legal_actions,
        choice: choice_summary(&domain_state),
        queued_phases: domain_state
            .queued_phases()
            .iter()
            .map(|phase| game_phase_name(*phase).to_owned())
            .collect(),
        queued_effect_count: domain_state.queued_effects().len(),
        effects: EffectResolutionSummary {
            status: if domain_state.pending_choice().is_some() {
                "choice"
            } else if domain_state.status() != GameStatus::InProgress {
                "terminal"
            } else if domain_state.last_effects().is_empty() {
                "idle"
            } else {
                "resolved"
            },
            outcomes: domain_state
                .last_effects()
                .iter()
                .map(effect_outcome_summary)
                .collect(),
        },
    }))
}

fn game_participant(
    stored: &StoredRoomParticipant,
    state: &InitialGameState,
) -> Result<GameParticipant, ApiError> {
    let hero_id = stored.hero_id.as_deref().ok_or_else(ApiError::internal)?;
    Ok(GameParticipant {
        display_name: stored.display_name.clone(),
        role: stored.role.clone(),
        position: stored.position,
        hero: GameHero {
            id: hero_id.to_owned(),
            name: hero_name(hero_id)?,
        },
        resources: GameResources {
            health: hero_resource(state, stored.position, EffectResource::Health)?,
            attack: hero_resource(state, stored.position, EffectResource::Attack)?,
            influence: hero_resource(state, stored.position, EffectResource::Influence)?,
        },
    })
}

fn hero_resource(
    state: &InitialGameState,
    position: i16,
    resource: EffectResource,
) -> Result<u16, ApiError> {
    let position = u8::try_from(position)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
    state
        .effect_world()
        .hero_resource(position, resource)
        .ok_or_else(ApiError::internal)
}

fn choice_summary(state: &InitialGameState) -> ChoiceSummary {
    let Some(choice) = state.pending_choice() else {
        return ChoiceSummary {
            status: "none",
            id: None,
            cause: None,
            responsible_position: None,
            kind: None,
            options: Vec::new(),
            min: None,
            max: None,
        };
    };
    ChoiceSummary {
        status: "pending",
        id: Some(choice.id.clone()),
        cause: Some(choice.cause.clone()),
        responsible_position: Some(choice.responsible_position),
        kind: Some(match choice.kind {
            PendingEffectChoiceKind::Effect => "effect",
            PendingEffectChoiceKind::Target => "target",
        }),
        options: choice.options.clone(),
        min: Some(choice.min),
        max: Some(choice.max),
    }
}

fn effect_outcome_summary(outcome: &EffectOutcome) -> EffectOutcomeSummary {
    match outcome {
        EffectOutcome::DieRolled {
            rule_id,
            die,
            result,
        } => EffectOutcomeSummary::DieRolled {
            rule_id: rule_id.clone(),
            die: match die {
                EffectDie::D4 => "d4",
                EffectDie::D6 => "d6",
                EffectDie::D8 => "d8",
            },
            result: *result,
        },
        EffectOutcome::Moved {
            rule_id,
            target_id,
            target_position,
            from,
            to,
        } => EffectOutcomeSummary::Moved {
            rule_id: rule_id.clone(),
            target_id: target_id.clone(),
            target_position: *target_position,
            from: effect_zone_name(*from),
            to: effect_zone_name(*to),
        },
        EffectOutcome::NoOp { rule_id, reason } => EffectOutcomeSummary::NoOp {
            rule_id: rule_id.clone(),
            reason: match reason {
                EffectNoOpReason::Explicit => "explicit",
                EffectNoOpReason::NoEligibleTarget => "no_eligible_target",
                EffectNoOpReason::ZeroCardinality => "zero_cardinality",
            },
        },
        EffectOutcome::ResourceChanged {
            rule_id,
            target_id,
            target_position,
            resource,
            before,
            after,
            cause,
        } => EffectOutcomeSummary::ResourceChanged {
            rule_id: rule_id.clone(),
            target_id: target_id.clone(),
            target_position: *target_position,
            resource: match resource {
                EffectResource::Attack => "attack",
                EffectResource::Control => "control",
                EffectResource::Health => "health",
                EffectResource::Influence => "influence",
            },
            before: *before,
            after: *after,
            cause: match cause {
                EffectChangeCause::Cost => "cost",
                EffectChangeCause::Effect => "effect",
            },
        },
        EffectOutcome::Terminal { rule_id, outcome } => EffectOutcomeSummary::Terminal {
            rule_id: rule_id.clone(),
            outcome: match outcome {
                EffectGameOutcome::Lost => "lost",
                EffectGameOutcome::Won => "won",
            },
        },
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
