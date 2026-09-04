use std::{collections::BTreeMap, sync::Arc};

use game_content::{
    Condition, ContentManifest, Die, Effect, EffectChoiceAudience as ContentEffectChoiceAudience,
    EffectRule, EffectTrigger, Eligibility, EntryKind, FunctionalField, GameOutcome,
    GameSetupOwner, ManifestEntry, Operation, Resource, Selector, TargetOwner, Zone,
};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct ContentCatalog {
    manifests: Arc<[ContentManifest]>,
}

impl ContentCatalog {
    pub(crate) fn new(manifests: Vec<ContentManifest>) -> Self {
        Self {
            manifests: manifests.into(),
        }
    }

    pub(crate) fn selection(
        &self,
        adventure_id: &str,
        manifest_digest: &str,
        ruleset_version: &str,
    ) -> Option<SelectedContent> {
        let manifest = self.manifests.iter().find(|manifest| {
            manifest.digest == manifest_digest && manifest.ruleset_version == ruleset_version
        })?;
        let adventure = manifest.entries.iter().find(|entry| {
            entry.kind == EntryKind::Adventure && entry.catalog_id.as_str() == adventure_id
        })?;
        let initial_entities = match manifest
            .game_setups
            .iter()
            .find(|setup| setup.adventure_id == adventure.catalog_id)
        {
            Some(setup) => setup
                .entities
                .iter()
                .map(|setup_entity| {
                    let entry = manifest
                        .entries
                        .iter()
                        .find(|entry| entry.catalog_id == setup_entity.catalog_id)?;
                    let effect_rule_id = entry
                        .functional_provenance
                        .get(&FunctionalField::Effect)?
                        .rule_id
                        .as_ref()?
                        .as_str()
                        .to_owned();
                    let influence_cost = entry
                        .functional_provenance
                        .get(&FunctionalField::Cost)
                        .and_then(|definition| definition.value);
                    let health = entry
                        .functional_provenance
                        .get(&FunctionalField::Health)
                        .and_then(|definition| definition.value);
                    if (entry.kind == EntryKind::HogwartsCard && influence_cost.is_none())
                        || (entry.kind == EntryKind::Villain
                            && health.is_none_or(|health| health == 0))
                    {
                        return None;
                    }
                    Some(SelectedInitialEntity {
                        catalog_id: entry.catalog_id.as_str().to_owned(),
                        copies: setup_entity.copies,
                        zone: effect_zone(setup_entity.zone),
                        owner: setup_entity.owner,
                        kind: entry.kind,
                        effect_rule_id,
                        influence_cost,
                        health,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };

        Some(SelectedContent {
            adventure_id: adventure.catalog_id.as_str().to_owned(),
            adventure_name: entry_name(adventure),
            content_version: manifest.content_version.clone(),
            ruleset_version: manifest.ruleset_version.clone(),
            manifest_digest: manifest.digest.clone(),
            manifest_version: manifest.manifest_version,
            playable: manifest.playable && adventure.playable,
            initial_entities,
        })
    }

    pub(crate) fn options(&self) -> Vec<ContentManifestOption> {
        self.manifests
            .iter()
            .map(|manifest| ContentManifestOption {
                manifest_digest: manifest.digest.clone(),
                manifest_version: manifest.manifest_version,
                content_version: manifest.content_version.clone(),
                ruleset_version: manifest.ruleset_version.clone(),
                playable: manifest.playable,
                adventures: manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.kind == EntryKind::Adventure)
                    .map(|entry| AdventureOption {
                        id: entry.catalog_id.as_str().to_owned(),
                        name: entry_name(entry),
                        playable: manifest.playable && entry.playable,
                    })
                    .collect(),
            })
            .collect()
    }

    pub(crate) fn effect_rules(
        &self,
        manifest_digest: &str,
    ) -> Option<Vec<game_domain::EffectRule>> {
        let manifest = self
            .manifests
            .iter()
            .find(|manifest| manifest.digest == manifest_digest)?;
        let rules = manifest
            .rules
            .iter()
            .map(|rule| (&rule.id, rule))
            .collect::<BTreeMap<_, _>>();

        manifest
            .executable_rules
            .iter()
            .map(|rule_id| compile_rule(rules.get(rule_id).copied()?, &rules))
            .collect()
    }

    pub(crate) fn entity_name(&self, manifest_digest: &str, catalog_id: &str) -> Option<String> {
        self.manifests
            .iter()
            .find(|manifest| manifest.digest == manifest_digest)?
            .entries
            .iter()
            .find(|entry| entry.catalog_id.as_str() == catalog_id)
            .map(entry_name)
    }

    pub(crate) async fn publish(&self, database: &PgPool) -> Result<(), sqlx::Error> {
        for manifest in self.manifests.iter() {
            let document = serde_json::to_string(manifest)
                .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
            publish_manifest(database, manifest, &document).await?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub(crate) struct ContentManifestOption {
    manifest_digest: String,
    manifest_version: u16,
    content_version: String,
    ruleset_version: String,
    playable: bool,
    adventures: Vec<AdventureOption>,
}

#[derive(Serialize)]
struct AdventureOption {
    id: String,
    name: String,
    playable: bool,
}

#[derive(Clone)]
pub(crate) struct SelectedContent {
    pub(crate) adventure_id: String,
    pub(crate) adventure_name: String,
    pub(crate) content_version: String,
    pub(crate) ruleset_version: String,
    pub(crate) manifest_digest: String,
    pub(crate) manifest_version: u16,
    pub(crate) playable: bool,
    initial_entities: Vec<SelectedInitialEntity>,
}

#[derive(Clone)]
struct SelectedInitialEntity {
    catalog_id: String,
    copies: u16,
    zone: game_domain::EffectZone,
    owner: GameSetupOwner,
    kind: EntryKind,
    effect_rule_id: String,
    influence_cost: Option<u16>,
    health: Option<u16>,
}

impl SelectedContent {
    pub(crate) fn initial_entities(
        &self,
        participant_positions: &[u8],
    ) -> Vec<game_domain::EffectEntityPlacement> {
        let mut next_instance = 1_u32;
        let mut placements = Vec::new();
        for template in &self.initial_entities {
            let owners = match template.owner {
                GameSetupOwner::None => vec![None],
                GameSetupOwner::EachParticipant => {
                    participant_positions.iter().copied().map(Some).collect()
                }
            };
            for owner_position in owners {
                for _ in 0..template.copies {
                    let instance_id = format!("instance:{next_instance:08}");
                    next_instance += 1;
                    let entity = match template.kind {
                        EntryKind::StarterCard => game_domain::EffectEntity::card(
                            instance_id,
                            &template.catalog_id,
                            game_domain::EffectEntityKind::StarterCard,
                            owner_position,
                            &template.effect_rule_id,
                            None,
                        ),
                        EntryKind::HogwartsCard => game_domain::EffectEntity::card(
                            instance_id,
                            &template.catalog_id,
                            game_domain::EffectEntityKind::HogwartsCard,
                            owner_position,
                            &template.effect_rule_id,
                            template.influence_cost,
                        ),
                        EntryKind::Villain => game_domain::EffectEntity::villain(
                            instance_id,
                            &template.catalog_id,
                            &template.effect_rule_id,
                            template
                                .health
                                .expect("validated villain setup has positive health"),
                        ),
                        EntryKind::Adventure
                        | EntryKind::Catalog
                        | EntryKind::DarkArts
                        | EntryKind::Hero
                        | EntryKind::Horcrux
                        | EntryKind::Location
                        | EntryKind::Proficiency
                        | EntryKind::Ruleset
                        | EntryKind::TurnOrder => {
                            unreachable!("validated game setup has a supported entity kind")
                        }
                    };
                    placements.push(game_domain::EffectEntityPlacement::new(
                        entity,
                        template.zone,
                    ));
                }
            }
        }
        placements
    }
}

async fn publish_manifest(
    database: &PgPool,
    manifest: &ContentManifest,
    document: &str,
) -> Result<(), sqlx::Error> {
    let manifest_version = i16::try_from(manifest.manifest_version).map_err(|_| {
        sqlx::Error::Protocol("manifest version does not fit PostgreSQL SMALLINT".to_owned())
    })?;
    let inserted = sqlx::query_scalar::<_, String>(
        r"
        INSERT INTO content_manifests (
            digest,
            manifest_version,
            content_version,
            ruleset_version,
            playable,
            document
        )
        VALUES ($1, $2, $3, $4, $5, $6::jsonb)
        ON CONFLICT (digest) DO NOTHING
        RETURNING digest
        ",
    )
    .bind(&manifest.digest)
    .bind(manifest_version)
    .bind(&manifest.content_version)
    .bind(&manifest.ruleset_version)
    .bind(manifest.playable)
    .bind(document)
    .fetch_optional(database)
    .await?;

    if inserted.is_none() {
        verify_immutable_manifest(database, manifest, manifest_version, document).await?;
    }

    Ok(())
}

async fn verify_immutable_manifest(
    database: &PgPool,
    manifest: &ContentManifest,
    manifest_version: i16,
    document: &str,
) -> Result<(), sqlx::Error> {
    let stored = sqlx::query_as::<_, (i16, String, String, bool, String)>(
        r"
        SELECT
            manifest_version,
            content_version,
            ruleset_version,
            playable,
            document::text
        FROM content_manifests
        WHERE digest = $1
        ",
    )
    .bind(&manifest.digest)
    .fetch_one(database)
    .await?;
    let requested_document: serde_json::Value =
        serde_json::from_str(document).map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let stored_document: serde_json::Value = serde_json::from_str(&stored.4)
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    if stored.0 != manifest_version
        || stored.1 != manifest.content_version
        || stored.2 != manifest.ruleset_version
        || stored.3 != manifest.playable
        || stored_document != requested_document
    {
        return Err(sqlx::Error::Protocol(
            "content manifest digest collision or immutable document mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn entry_name(entry: &ManifestEntry) -> String {
    entry
        .names
        .get("pt-BR")
        .or_else(|| entry.names.get("en"))
        .or_else(|| entry.names.values().next())
        .cloned()
        .unwrap_or_else(|| entry.catalog_id.as_str().to_owned())
}

fn compile_rule(
    rule: &EffectRule,
    rules: &BTreeMap<&game_content::RuleId, &EffectRule>,
) -> Option<game_domain::EffectRule> {
    Some(game_domain::EffectRule {
        id: rule.id.as_str().to_owned(),
        trigger: effect_trigger(rule.trigger),
        cost: rule
            .cost
            .iter()
            .map(|cost| game_domain::EffectResourceCost {
                resource: effect_resource(cost.resource),
                amount: cost.amount,
            })
            .collect(),
        effect: compile_effect(&rule.effect, rules)?,
    })
}

fn compile_effect(
    effect: &Effect,
    rules: &BTreeMap<&game_content::RuleId, &EffectRule>,
) -> Option<game_domain::EffectDefinition> {
    Some(match effect {
        Effect::Apply { target, operation } => game_domain::EffectDefinition::Apply {
            target: effect_selector(target),
            operation: effect_operation(operation),
        },
        Effect::Choice { audience, options } => game_domain::EffectDefinition::Choice {
            audience: match audience {
                ContentEffectChoiceAudience::Actor => game_domain::EffectChoiceAudience::Actor,
                ContentEffectChoiceAudience::EachHero => {
                    game_domain::EffectChoiceAudience::EachHero
                }
            },
            options: options
                .iter()
                .map(|option| compile_effect(option, rules))
                .collect::<Option<Vec<_>>>()?,
        },
        Effect::Condition {
            condition,
            then,
            otherwise,
        } => game_domain::EffectDefinition::Condition {
            condition: effect_condition(condition),
            then: Box::new(compile_effect(then, rules)?),
            otherwise: match otherwise.as_deref() {
                Some(effect) => Some(Box::new(compile_effect(effect, rules)?)),
                None => None,
            },
        },
        Effect::NoOp => game_domain::EffectDefinition::NoOp,
        Effect::Reference { rule } => compile_effect(&rules.get(rule)?.effect, rules)?,
        Effect::Repeat { times, effect } => game_domain::EffectDefinition::Repeat {
            times: *times,
            effect: Box::new(compile_effect(effect, rules)?),
        },
        Effect::Roll { die, outcomes } => game_domain::EffectDefinition::Roll {
            die: effect_die(*die),
            outcomes: outcomes
                .iter()
                .map(|outcome| compile_effect(outcome, rules))
                .collect::<Option<Vec<_>>>()?,
        },
        Effect::Sequence { effects } => game_domain::EffectDefinition::Sequence {
            effects: effects
                .iter()
                .map(|effect| compile_effect(effect, rules))
                .collect::<Option<Vec<_>>>()?,
        },
        Effect::Terminal { outcome } => game_domain::EffectDefinition::Terminal {
            outcome: effect_game_outcome(*outcome),
        },
    })
}

fn effect_condition(condition: &Condition) -> game_domain::EffectCondition {
    match condition {
        Condition::HasEligibleTarget { target } => {
            game_domain::EffectCondition::HasEligibleTarget {
                target: effect_selector(target),
            }
        }
        Condition::ResourceAtLeast {
            target,
            resource,
            amount,
        } => game_domain::EffectCondition::ResourceAtLeast {
            target: effect_selector(target),
            resource: effect_resource(*resource),
            amount: *amount,
        },
    }
}

fn effect_selector(selector: &Selector) -> game_domain::EffectSelector {
    game_domain::EffectSelector {
        id: selector.id.clone(),
        zone: effect_zone(selector.zone),
        owner: match selector.owner {
            TargetOwner::Actor => game_domain::EffectTargetOwner::Actor,
            TargetOwner::Any => game_domain::EffectTargetOwner::Any,
        },
        min: selector.cardinality.min,
        max: selector.cardinality.max,
        eligibility: selector
            .eligibility
            .iter()
            .map(|eligibility| match eligibility {
                Eligibility::ResourceAtLeast { resource, amount } => {
                    game_domain::EffectEligibility::ResourceAtLeast {
                        resource: effect_resource(*resource),
                        amount: *amount,
                    }
                }
            })
            .collect(),
    }
}

fn effect_operation(operation: &Operation) -> game_domain::EffectOperation {
    match operation {
        Operation::Discard => game_domain::EffectOperation::Discard,
        Operation::ModifyResource { resource, amount } => {
            game_domain::EffectOperation::ModifyResource {
                resource: effect_resource(*resource),
                amount: *amount,
            }
        }
        Operation::Move { to } => game_domain::EffectOperation::Move {
            to: effect_zone(*to),
        },
    }
}

const fn effect_trigger(trigger: EffectTrigger) -> game_domain::EffectTrigger {
    match trigger {
        EffectTrigger::DarkArtsCompleted => game_domain::EffectTrigger::DarkArtsCompleted,
        EffectTrigger::Manual => game_domain::EffectTrigger::Manual,
    }
}

const fn effect_resource(resource: Resource) -> game_domain::EffectResource {
    match resource {
        Resource::Attack => game_domain::EffectResource::Attack,
        Resource::Control => game_domain::EffectResource::Control,
        Resource::Health => game_domain::EffectResource::Health,
        Resource::Influence => game_domain::EffectResource::Influence,
    }
}

const fn effect_zone(zone: Zone) -> game_domain::EffectZone {
    match zone {
        Zone::ActiveLocation => game_domain::EffectZone::ActiveLocation,
        Zone::ActiveVillains => game_domain::EffectZone::ActiveVillains,
        Zone::DarkArtsDeck => game_domain::EffectZone::DarkArtsDeck,
        Zone::DarkArtsDiscard => game_domain::EffectZone::DarkArtsDiscard,
        Zone::HeroDiscardPile => game_domain::EffectZone::HeroDiscardPile,
        Zone::HeroDrawPile => game_domain::EffectZone::HeroDrawPile,
        Zone::HeroHand => game_domain::EffectZone::HeroHand,
        Zone::HeroPlayArea => game_domain::EffectZone::HeroPlayArea,
        Zone::Heroes => game_domain::EffectZone::Heroes,
        Zone::HogwartsDeck => game_domain::EffectZone::HogwartsDeck,
        Zone::Market => game_domain::EffectZone::Market,
        Zone::VillainDeck => game_domain::EffectZone::VillainDeck,
    }
}

const fn effect_die(die: Die) -> game_domain::EffectDie {
    match die {
        Die::D4 => game_domain::EffectDie::D4,
        Die::D6 => game_domain::EffectDie::D6,
        Die::D8 => game_domain::EffectDie::D8,
    }
}

const fn effect_game_outcome(outcome: GameOutcome) -> game_domain::EffectGameOutcome {
    match outcome {
        GameOutcome::Lost => game_domain::EffectGameOutcome::Lost,
        GameOutcome::Won => game_domain::EffectGameOutcome::Won,
    }
}
