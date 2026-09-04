use game_domain::{
    EffectChangeCause, EffectDie, EffectGameOutcome, EffectNoOpReason, EffectOutcome,
    EffectResource, EffectZone, GameEngine, GameStatus, InitialGameState, LegalGameIntentions,
    PendingEffectChoiceKind, PlayerIntentType, ValidatedGameRules, legal_game_intentions,
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
    legal_intentions: LegalIntentionsSummary,
    table: TableSummary,
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
    hand_count: usize,
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

#[derive(Serialize)]
struct LegalIntentionsSummary {
    end_hero_actions: bool,
    play_cards: Vec<LegalPlayCardSummary>,
    assign_attack: Vec<LegalAttackSummary>,
    acquire_cards: Vec<LegalAcquisitionSummary>,
}

#[derive(Serialize)]
struct LegalPlayCardSummary {
    card_id: String,
    target_slots: Vec<LegalTargetSlotSummary>,
}

#[derive(Serialize)]
struct LegalTargetSlotSummary {
    selector_id: String,
    min: u16,
    max: u16,
    options: Vec<TargetOptionSummary>,
}

#[derive(Serialize)]
struct TargetOptionSummary {
    target_id: String,
    label: String,
}

#[derive(Serialize)]
struct LegalAttackSummary {
    villain_id: String,
    max_amount: u16,
}

#[derive(Serialize)]
struct LegalAcquisitionSummary {
    card_id: String,
    cost: u16,
}

#[derive(Serialize)]
struct TableSummary {
    hand: Vec<CardSummary>,
    play_area: Vec<CardSummary>,
    draw_pile_count: usize,
    discard_pile_count: usize,
    market: Vec<MarketCardSummary>,
    hogwarts_deck_count: usize,
    active_villains: Vec<VillainSummary>,
    villain_deck_count: usize,
}

#[derive(Serialize)]
struct CardSummary {
    instance_id: String,
    catalog_id: String,
    name: String,
}

#[derive(Serialize)]
struct MarketCardSummary {
    instance_id: String,
    catalog_id: String,
    name: String,
    cost: u16,
    affordable: bool,
}

#[derive(Serialize)]
struct VillainSummary {
    instance_id: String,
    catalog_id: String,
    name: String,
    health: u16,
    attackable: bool,
    max_attack: u16,
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
    let rules = ValidatedGameRules::new(effect_rules.clone()).map_err(|_| ApiError::internal())?;
    let (player_intents, legal_intentions) = if game.expired {
        (Vec::new(), LegalGameIntentions::default())
    } else {
        (
            GameEngine::new(&rules).legal_intent_types(&domain_state, actor_position),
            legal_game_intentions(&domain_state, actor_position, &effect_rules),
        )
    };
    let legal_actions = legal_action_names(&player_intents, &legal_intentions);
    let legal_intentions_summary = legal_intentions_summary(
        &player_intents,
        &legal_intentions,
        &domain_state,
        &participants,
        &state.content,
        &game.manifest_digest,
    );
    let table = table_summary(
        &domain_state,
        actor_position,
        &legal_intentions,
        &state.content,
        &game.manifest_digest,
    )?;

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
        legal_intentions: legal_intentions_summary,
        table,
        choice: choice_summary(&domain_state),
        queued_phases: domain_state
            .queued_phases()
            .iter()
            .map(|phase| game_phase_name(*phase).to_owned())
            .collect(),
        queued_effect_count: domain_state.queued_effects().len(),
        effects: effect_resolution_summary(&domain_state),
    }))
}

fn effect_resolution_summary(state: &InitialGameState) -> EffectResolutionSummary {
    let status = if state.pending_choice().is_some() {
        "choice"
    } else if state.status() != GameStatus::InProgress {
        "terminal"
    } else if state.last_effects().is_empty() {
        "idle"
    } else {
        "resolved"
    };
    EffectResolutionSummary {
        status,
        outcomes: state
            .last_effects()
            .iter()
            .map(effect_outcome_summary)
            .collect(),
    }
}

fn legal_action_names(
    player_intents: &[PlayerIntentType],
    intentions: &LegalGameIntentions,
) -> Vec<String> {
    let mut actions = player_intents
        .iter()
        .map(|intent| match intent {
            PlayerIntentType::EndHeroActions => "end_hero_actions".to_owned(),
            PlayerIntentType::ResolveChoice => "resolve_choice".to_owned(),
        })
        .collect::<Vec<_>>();
    if !intentions.playable_cards.is_empty() {
        actions.push("play_card".to_owned());
    }
    if !intentions.attack_targets.is_empty() {
        actions.push("assign_attack".to_owned());
    }
    if !intentions.acquisitions.is_empty() {
        actions.push("acquire_card".to_owned());
    }
    actions
}

fn legal_intentions_summary(
    player_intents: &[PlayerIntentType],
    intentions: &LegalGameIntentions,
    domain_state: &InitialGameState,
    participants: &[StoredRoomParticipant],
    content: &crate::content_catalog::ContentCatalog,
    manifest_digest: &str,
) -> LegalIntentionsSummary {
    LegalIntentionsSummary {
        end_hero_actions: player_intents.contains(&PlayerIntentType::EndHeroActions),
        play_cards: intentions
            .playable_cards
            .iter()
            .map(|card| LegalPlayCardSummary {
                card_id: card.card_id.clone(),
                target_slots: card
                    .target_slots
                    .iter()
                    .map(|slot| LegalTargetSlotSummary {
                        selector_id: slot.selector_id.clone(),
                        min: slot.min,
                        max: slot.max,
                        options: slot
                            .target_ids
                            .iter()
                            .map(|target_id| TargetOptionSummary {
                                target_id: target_id.clone(),
                                label: target_label(
                                    target_id,
                                    domain_state,
                                    participants,
                                    content,
                                    manifest_digest,
                                ),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
        assign_attack: intentions
            .attack_targets
            .iter()
            .map(|target| LegalAttackSummary {
                villain_id: target.villain_id.clone(),
                max_amount: target.max_amount,
            })
            .collect(),
        acquire_cards: intentions
            .acquisitions
            .iter()
            .map(|acquisition| LegalAcquisitionSummary {
                card_id: acquisition.card_id.clone(),
                cost: acquisition.cost,
            })
            .collect(),
    }
}

fn target_label(
    target_id: &str,
    state: &InitialGameState,
    participants: &[StoredRoomParticipant],
    content: &crate::content_catalog::ContentCatalog,
    manifest_digest: &str,
) -> String {
    let Some((_, entity)) = state.effect_world().entity(target_id) else {
        return target_id.to_owned();
    };
    if let Some(position) = entity.owner_position()
        && let Some(participant) = participants
            .iter()
            .find(|participant| i16::from(position) == participant.position)
    {
        return participant
            .hero_id
            .as_deref()
            .and_then(|hero_id| hero_name(hero_id).ok())
            .map_or_else(
                || participant.display_name.clone(),
                |hero| format!("{} - {hero}", participant.display_name),
            );
    }
    entity.catalog_id().map_or_else(
        || target_id.to_owned(),
        |catalog_id| {
            content
                .entity_name(manifest_digest, catalog_id)
                .unwrap_or_else(|| catalog_id.to_owned())
        },
    )
}

fn table_summary(
    state: &InitialGameState,
    actor_position: u8,
    intentions: &LegalGameIntentions,
    content: &crate::content_catalog::ContentCatalog,
    manifest_digest: &str,
) -> Result<TableSummary, ApiError> {
    let cards_in = |zone| {
        state
            .effect_world()
            .entities_in(zone)
            .iter()
            .filter(|entity| entity.owner_position() == Some(actor_position))
            .filter_map(|entity| card_summary(entity, content, manifest_digest))
            .collect::<Vec<_>>()
    };
    let market = state
        .effect_world()
        .entities_in(EffectZone::Market)
        .iter()
        .map(|entity| {
            let card =
                card_summary(entity, content, manifest_digest).ok_or_else(ApiError::internal)?;
            let cost = entity.influence_cost().ok_or_else(ApiError::internal)?;
            Ok(MarketCardSummary {
                instance_id: card.instance_id,
                catalog_id: card.catalog_id,
                name: card.name,
                cost,
                affordable: intentions
                    .acquisitions
                    .iter()
                    .any(|acquisition| acquisition.card_id == entity.id()),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let active_villains = state
        .effect_world()
        .entities_in(EffectZone::ActiveVillains)
        .iter()
        .map(|entity| {
            let catalog_id = entity.catalog_id().ok_or_else(ApiError::internal)?;
            let legal = intentions
                .attack_targets
                .iter()
                .find(|target| target.villain_id == entity.id());
            Ok(VillainSummary {
                instance_id: entity.id().to_owned(),
                catalog_id: catalog_id.to_owned(),
                name: content
                    .entity_name(manifest_digest, catalog_id)
                    .unwrap_or_else(|| catalog_id.to_owned()),
                health: entity.resource(EffectResource::Health),
                attackable: legal.is_some(),
                max_attack: legal.map_or(0, |target| target.max_amount),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    let owned_count = |zone| {
        state
            .effect_world()
            .entities_in(zone)
            .iter()
            .filter(|entity| entity.owner_position() == Some(actor_position))
            .count()
    };

    Ok(TableSummary {
        hand: cards_in(EffectZone::HeroHand),
        play_area: cards_in(EffectZone::HeroPlayArea),
        draw_pile_count: owned_count(EffectZone::HeroDrawPile),
        discard_pile_count: owned_count(EffectZone::HeroDiscardPile),
        market,
        hogwarts_deck_count: state
            .effect_world()
            .entities_in(EffectZone::HogwartsDeck)
            .len(),
        active_villains,
        villain_deck_count: state
            .effect_world()
            .entities_in(EffectZone::VillainDeck)
            .len(),
    })
}

fn card_summary(
    entity: &game_domain::EffectEntity,
    content: &crate::content_catalog::ContentCatalog,
    manifest_digest: &str,
) -> Option<CardSummary> {
    let catalog_id = entity.catalog_id()?;
    Some(CardSummary {
        instance_id: entity.id().to_owned(),
        catalog_id: catalog_id.to_owned(),
        name: content
            .entity_name(manifest_digest, catalog_id)
            .unwrap_or_else(|| catalog_id.to_owned()),
    })
}

fn game_participant(
    stored: &StoredRoomParticipant,
    state: &InitialGameState,
) -> Result<GameParticipant, ApiError> {
    let hero_id = stored.hero_id.as_deref().ok_or_else(ApiError::internal)?;
    let position = u8::try_from(stored.position)
        .map_err(|error| ApiError::internal_with("match application operation", error))?;
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
        hand_count: state
            .effect_world()
            .entities_in(EffectZone::HeroHand)
            .iter()
            .filter(|entity| entity.owner_position() == Some(position))
            .count(),
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
