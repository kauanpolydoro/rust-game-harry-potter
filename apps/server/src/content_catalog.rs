use std::{collections::BTreeMap, sync::Arc};

use game_content::{
    Condition, ContentManifest, Die, Effect, EffectChoiceAudience as ContentEffectChoiceAudience,
    EffectRule, EffectTrigger, Eligibility, EntryKind, FunctionalField, GameOutcome,
    GameSetupOwner, ManifestEntry, Operation, Resource, Selector, TargetOwner, Zone,
};
use serde::Serialize;
use sqlx::PgPool;

mod descriptions;
mod game_one;
pub use game_one::{game_four_manifest, game_one_manifest, game_three_manifest, game_two_manifest};

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
                        .get(&if entry.kind == EntryKind::Hero {
                            FunctionalField::Ability
                        } else {
                            FunctionalField::Effect
                        })?
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
                    let reward_rule_id = entry
                        .functional_provenance
                        .get(&FunctionalField::Reward)
                        .and_then(|definition| definition.rule_id.as_ref())
                        .map(|id| id.as_str().to_owned());
                    let control_limit = entry
                        .functional_provenance
                        .get(&FunctionalField::ControlLimit)
                        .and_then(|definition| definition.value);
                    let dark_arts_count = entry
                        .functional_provenance
                        .get(&FunctionalField::DarkArtsCount)
                        .and_then(|definition| definition.value)
                        .and_then(|count| u8::try_from(count).ok());
                    if (entry.kind == EntryKind::HogwartsCard && influence_cost.is_none())
                        || (entry.kind == EntryKind::Villain
                            && (health.is_none_or(|health| health == 0)
                                || reward_rule_id.is_none()))
                        || (entry.kind == EntryKind::Location
                            && (control_limit.is_none_or(|limit| limit == 0)
                                || dark_arts_count.is_none_or(|count| count == 0)))
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
                        reward_rule_id,
                        control_limit,
                        dark_arts_count,
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
            preparation: preparation(manifest, adventure),
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

    pub(crate) fn entity_description(
        &self,
        digest: &str,
        catalog_id: &str,
        field: FunctionalField,
    ) -> Option<String> {
        let manifest = self
            .manifests
            .iter()
            .find(|manifest| manifest.digest == digest)?;
        if manifest.manifest_version < 4 {
            return None;
        }
        let entry = manifest
            .entries
            .iter()
            .find(|entry| entry.catalog_id.as_str() == catalog_id)?;
        let id = entry.functional_provenance.get(&field)?.rule_id.as_ref()?;
        let rules = manifest
            .rules
            .iter()
            .map(|rule| (&rule.id, rule))
            .collect::<BTreeMap<_, _>>();
        let rule = compile_rule(rules.get(id).copied()?, &rules)?;
        let actor = if field == FunctionalField::Effect
            && matches!(entry.kind, EntryKind::DarkArts | EntryKind::Villain)
        {
            "O Herói ativo"
        } else {
            "Você"
        };
        descriptions::describe(&rule.effect, actor).filter(|text| !text.is_empty())
    }

    pub(crate) fn rule_name(&self, digest: &str, rule_id: &str) -> Option<String> {
        let manifest = self
            .manifests
            .iter()
            .find(|manifest| manifest.digest == digest && manifest.manifest_version >= 4)?;
        manifest
            .entries
            .iter()
            .find(|entry| {
                entry.functional_provenance.values().any(|field| {
                    field
                        .rule_id
                        .as_ref()
                        .is_some_and(|id| id.as_str() == rule_id)
                })
            })
            .map(entry_name)
    }

    pub(crate) fn choice_option_labels(
        &self,
        digest: &str,
        cursor: &game_domain::EffectCursor,
    ) -> Vec<(String, String)> {
        let Some(rules) = self.effect_rules(digest) else {
            return Vec::new();
        };
        let Some(rule) = rules.iter().find(|rule| rule.id == cursor.rule_id) else {
            return Vec::new();
        };
        let Some(game_domain::EffectDefinition::Choice { options, .. }) =
            descriptions::at_path(&rule.effect, &cursor.path)
        else {
            return Vec::new();
        };
        options
            .iter()
            .enumerate()
            .filter_map(|(index, effect)| {
                descriptions::describe(effect, "Você")
                    .map(|text| {
                        if text.is_empty() {
                            "Não aplicar efeito.".to_owned()
                        } else {
                            text
                        }
                    })
                    .map(|label| (format!("option:{}", index + 1), label))
            })
            .collect()
    }

    pub(crate) fn choice_instruction(
        &self,
        digest: &str,
        cursor: &game_domain::EffectCursor,
    ) -> Option<String> {
        self.rule_name(digest, &cursor.rule_id)?;
        let rules = self.effect_rules(digest)?;
        let rule = rules.iter().find(|rule| rule.id == cursor.rule_id)?;
        descriptions::describe(descriptions::at_path(&rule.effect, &cursor.path)?, "Você")
            .filter(|text| !text.is_empty())
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

fn preparation(manifest: &ContentManifest, adventure: &ManifestEntry) -> Option<GamePreparation> {
    adventure
        .functional_provenance
        .get(&FunctionalField::Setup)
        .and_then(|definition| definition.rule_id.as_ref())
        .and_then(|id| manifest.rules.iter().find(|rule| rule.id == *id))
        .and_then(|rule| match rule.effect {
            Effect::Structural {
                rule: game_content::StructuralRule::GameOneSetup,
            } => Some(GamePreparation::One),
            Effect::Structural {
                rule: game_content::StructuralRule::GameTwoSetup,
            } => Some(GamePreparation::Two),
            Effect::Structural {
                rule: game_content::StructuralRule::GameThreeSetup,
            } => Some(GamePreparation::Three),
            Effect::Structural {
                rule: game_content::StructuralRule::GameFourSetup,
            } => Some(GamePreparation::Four),
            _ => None,
        })
}

#[derive(Clone, Copy)]
enum GamePreparation {
    One,
    Two,
    Three,
    Four,
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
    preparation: Option<GamePreparation>,
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
    reward_rule_id: Option<String>,
    control_limit: Option<u16>,
    dark_arts_count: Option<u8>,
}

impl SelectedInitialEntity {
    fn instance(
        &self,
        instance_id: String,
        owner_position: Option<u8>,
    ) -> game_domain::EffectEntity {
        match self.kind {
            EntryKind::DarkArts => game_domain::EffectEntity::new(instance_id, None)
                .with_kind(game_domain::EffectEntityKind::DarkArts)
                .with_catalog_id(&self.catalog_id)
                .with_effect_rule(&self.effect_rule_id),
            EntryKind::StarterCard => game_domain::EffectEntity::card(
                instance_id,
                &self.catalog_id,
                game_domain::EffectEntityKind::StarterCard,
                owner_position,
                &self.effect_rule_id,
                None,
            ),
            EntryKind::HogwartsCard => game_domain::EffectEntity::card(
                instance_id,
                &self.catalog_id,
                game_domain::EffectEntityKind::HogwartsCard,
                owner_position,
                &self.effect_rule_id,
                self.influence_cost,
            ),
            EntryKind::Villain => game_domain::EffectEntity::villain(
                instance_id,
                &self.catalog_id,
                &self.effect_rule_id,
                self.health
                    .expect("validated villain setup has positive health"),
            )
            .with_reward_rule(
                self.reward_rule_id
                    .as_deref()
                    .expect("validated villain setup has a reward"),
            ),
            EntryKind::Location => game_domain::EffectEntity::location(
                instance_id,
                &self.catalog_id,
                &self.effect_rule_id,
                self.control_limit
                    .expect("validated location has a control limit"),
                self.dark_arts_count
                    .expect("validated location has a Dark Arts count"),
            ),
            EntryKind::Hero => {
                game_domain::EffectEntity::hero(owner_position.expect("validated Hero owner"))
                    .with_catalog_id(&self.catalog_id)
                    .with_effect_rule(&self.effect_rule_id)
                    .with_turn_state(Some(game_domain::EffectTurnState::default()))
            }
            EntryKind::Adventure
            | EntryKind::Catalog
            | EntryKind::Horcrux
            | EntryKind::Proficiency
            | EntryKind::Ruleset
            | EntryKind::TurnOrder => {
                unreachable!("validated game setup has a supported entity kind")
            }
        }
    }
}

impl SelectedContent {
    pub(crate) fn start_game(
        &self,
        actor_role: game_domain::ParticipantRole,
        participants: &[game_domain::LobbyParticipant],
        rules: &game_domain::ValidatedGameRules,
        random: &mut dyn game_domain::EffectRoller,
    ) -> Result<game_domain::InitialGameState, game_domain::StartGameError> {
        let entities = self.initial_entities(participants);
        let input = game_domain::StartGameInput {
            actor_role,
            participants,
            content: game_domain::ContentSelection {
                adventure_id: &self.adventure_id,
                content_version: &self.content_version,
                ruleset_version: &self.ruleset_version,
                manifest_digest: &self.manifest_digest,
                manifest_version: self.manifest_version,
                playable: self.playable,
                initial_entities: &entities,
            },
        };
        let engine = game_domain::GameEngine::new(rules);
        match self.preparation {
            Some(GamePreparation::One) => engine.start_game_one(input, random),
            Some(GamePreparation::Two) => engine.start_game_two(input, random),
            Some(GamePreparation::Three) => engine.start_game_three(input, random),
            Some(GamePreparation::Four) => engine.start_game_four(input, random),
            None => engine.start(input, random),
        }
    }

    pub(crate) fn initial_entities(
        &self,
        participants: &[game_domain::LobbyParticipant],
    ) -> Vec<game_domain::EffectEntityPlacement> {
        let mut next_instance = 1_u32;
        let mut placements = Vec::new();
        for template in &self.initial_entities {
            let owners = match template.owner {
                GameSetupOwner::None => vec![None],
                GameSetupOwner::EachParticipant => participants
                    .iter()
                    .map(|participant| Some(participant.position))
                    .collect(),
                owner => participants
                    .iter()
                    .filter(|participant| {
                        matches!(
                            (owner, participant.hero),
                            (GameSetupOwner::Harry, Some(game_domain::HeroId::Harry))
                                | (
                                    GameSetupOwner::Hermione,
                                    Some(game_domain::HeroId::Hermione)
                                )
                                | (GameSetupOwner::Neville, Some(game_domain::HeroId::Neville))
                                | (GameSetupOwner::Ron, Some(game_domain::HeroId::Ron))
                        )
                    })
                    .map(|participant| Some(participant.position))
                    .collect(),
            };
            for owner_position in owners {
                for _ in 0..template.copies {
                    let instance_id = format!("instance:{next_instance:08}");
                    next_instance += 1;
                    let mut entity = template.instance(instance_id, owner_position);
                    if template.kind == EntryKind::Villain
                        && matches!(
                            self.preparation,
                            Some(
                                GamePreparation::Two
                                    | GamePreparation::Three
                                    | GamePreparation::Four
                            )
                        )
                    {
                        entity = entity
                            .with_max_health(template.health.expect("validated villain health"));
                    }
                    if (template.kind == EntryKind::Villain
                        && matches!(
                            self.preparation,
                            Some(GamePreparation::Three | GamePreparation::Four)
                        ))
                        || (matches!(self.preparation, Some(GamePreparation::Four))
                            && matches!(
                                template.catalog_id.as_str(),
                                "hogwarts-card:023" | "hogwarts-card:035"
                            ))
                    {
                        entity =
                            entity.with_turn_state(Some(game_domain::EffectTurnState::default()));
                    }
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
        order: rule.order,
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
        Effect::RevealExtraDarkArts => game_domain::EffectDefinition::RevealExtraDarkArts,
        Effect::PreventControlRemoval => game_domain::EffectDefinition::PreventControlRemoval,
        Effect::OtherAllyBonus { health } => {
            game_domain::EffectDefinition::OtherAllyBonus { health: *health }
        }
        Effect::HeroAbility { strategy, effect } => game_domain::EffectDefinition::HeroAbility {
            strategy: effect_hero_ability(*strategy),
            effect: Box::new(compile_effect(effect, rules)?),
        },
        Effect::RevealTopCard {
            minimum_cost,
            effect,
        } => game_domain::EffectDefinition::RevealTopCard {
            minimum_cost: *minimum_cost,
            effect: Box::new(compile_effect(effect, rules)?),
        },
        Effect::LimitVillainAttack { maximum } => {
            game_domain::EffectDefinition::LimitVillainAttack { maximum: *maximum }
        }
        Effect::PreventExtraDrawing => game_domain::EffectDefinition::PreventExtraDrawing,
        Effect::ForEachTarget { target, effect } => game_domain::EffectDefinition::ForEachTarget {
            target: effect_selector(target),
            effect: Box::new(compile_effect(effect, rules)?),
        },
        Effect::TopDeckAcquisition { card_type } => {
            game_domain::EffectDefinition::TopDeckAcquisition {
                card_type: effect_card_type(*card_type),
            }
        }
        Effect::CardType { card_type } => game_domain::EffectDefinition::CardType {
            card_type: effect_card_type(*card_type),
        },
        Effect::HandDamageLimit { maximum } => {
            game_domain::EffectDefinition::HandDamageLimit { maximum: *maximum }
        }
        Effect::Reaction { trigger, effect } => game_domain::EffectDefinition::Reaction {
            trigger: effect_reaction_trigger(*trigger),
            effect: Box::new(compile_effect(effect, rules)?),
        },
        Effect::RevealDarkArts => game_domain::EffectDefinition::RevealDarkArts,
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
        Effect::NoOp | Effect::Structural { .. } => game_domain::EffectDefinition::NoOp,
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

const fn effect_hero_ability(
    strategy: game_content::HeroAbilityStrategy,
) -> game_domain::HeroAbilityStrategy {
    match strategy {
        game_content::HeroAbilityStrategy::HarryGameThreeV1 => {
            game_domain::HeroAbilityStrategy::HarryGameThreeV1
        }
        game_content::HeroAbilityStrategy::HermioneGameThreeV1 => {
            game_domain::HeroAbilityStrategy::HermioneGameThreeV1
        }
        game_content::HeroAbilityStrategy::NevilleGameThreeV1 => {
            game_domain::HeroAbilityStrategy::NevilleGameThreeV1
        }
        game_content::HeroAbilityStrategy::RonGameThreeV1 => {
            game_domain::HeroAbilityStrategy::RonGameThreeV1
        }
    }
}

const fn effect_reaction_trigger(
    trigger: game_content::ReactionTrigger,
) -> game_domain::EffectReactionTrigger {
    match trigger {
        game_content::ReactionTrigger::MorsmordreRevealedV1 => {
            game_domain::EffectReactionTrigger::MorsmordreRevealedV1
        }
        game_content::ReactionTrigger::VillainRevealed => {
            game_domain::EffectReactionTrigger::VillainRevealed
        }
        game_content::ReactionTrigger::OwnerPlaysAlly => {
            game_domain::EffectReactionTrigger::OwnerPlaysAlly
        }
        game_content::ReactionTrigger::ControlAdded => {
            game_domain::EffectReactionTrigger::ControlAdded
        }
        game_content::ReactionTrigger::HeroForcedDiscard => {
            game_domain::EffectReactionTrigger::HeroForcedDiscard
        }
        game_content::ReactionTrigger::SelfHarmfulDiscard => {
            game_domain::EffectReactionTrigger::SelfHarmfulDiscard
        }
        game_content::ReactionTrigger::SelfForcedDiscard => {
            game_domain::EffectReactionTrigger::SelfForcedDiscard
        }
        game_content::ReactionTrigger::OwnerDefeatsVillain => {
            game_domain::EffectReactionTrigger::OwnerDefeatsVillain
        }
    }
}

fn effect_condition(condition: &Condition) -> game_domain::EffectCondition {
    match condition {
        Condition::DrawingAllowed => game_domain::EffectCondition::DrawingAllowed,
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
            TargetOwner::Other => game_domain::EffectTargetOwner::Other,
            TargetOwner::Any => game_domain::EffectTargetOwner::Any,
        },
        min: selector.cardinality.min,
        max: selector.cardinality.max,
        eligibility: selector
            .eligibility
            .iter()
            .map(|eligibility| match eligibility {
                Eligibility::CardType { card_type } => game_domain::EffectEligibility::CardType {
                    card_type: effect_card_type(*card_type),
                },
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

fn effect_card_type(card_type: game_content::CardType) -> game_domain::EffectCardType {
    match card_type {
        game_content::CardType::Ally => game_domain::EffectCardType::Ally,
        game_content::CardType::Item => game_domain::EffectCardType::Item,
        game_content::CardType::Spell => game_domain::EffectCardType::Spell,
    }
}

fn effect_operation(operation: &Operation) -> game_domain::EffectOperation {
    match operation {
        Operation::GainInfluenceAndDraw { influence, cards } => {
            game_domain::EffectOperation::GainInfluenceAndDraw {
                influence: *influence,
                cards: *cards,
            }
        }
        Operation::SuppressVillain => game_domain::EffectOperation::SuppressVillain,
        Operation::GainInfluenceAndHealth { influence, health } => {
            game_domain::EffectOperation::GainInfluenceAndHealth {
                influence: *influence,
                health: *health,
            }
        }
        Operation::DiscardForSpellBonus { influence } => {
            game_domain::EffectOperation::DiscardForSpellBonus {
                influence: *influence,
            }
        }
        Operation::DiscardVoluntarily => game_domain::EffectOperation::DiscardVoluntarily,
        Operation::CopyPlayedAlly => game_domain::EffectOperation::CopyPlayedAlly,
        Operation::GainAttackPerAllyPlayed { amount } => {
            game_domain::EffectOperation::GainAttackPerAllyPlayed { amount: *amount }
        }
        Operation::Discard => game_domain::EffectOperation::Discard,
        Operation::PreventDrawing => game_domain::EffectOperation::PreventDrawing,
        Operation::Draw { amount } => game_domain::EffectOperation::Draw { amount: *amount },
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
        EffectTrigger::DarkArts => game_domain::EffectTrigger::DarkArts,
        EffectTrigger::Villains => game_domain::EffectTrigger::Villains,
        EffectTrigger::VillainReward => game_domain::EffectTrigger::VillainReward,
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
        Zone::LocationDeck => game_domain::EffectZone::LocationDeck,
        Zone::LocationDiscard => game_domain::EffectZone::LocationDiscard,
        Zone::Market => game_domain::EffectZone::Market,
        Zone::VillainDeck => game_domain::EffectZone::VillainDeck,
        Zone::VillainDiscard => game_domain::EffectZone::VillainDiscard,
    }
}

const fn effect_die(die: Die) -> game_domain::EffectDie {
    match die {
        Die::GryffindorV1 => game_domain::EffectDie::GryffindorV1,
        Die::HufflepuffV1 => game_domain::EffectDie::HufflepuffV1,
        Die::RavenclawV1 => game_domain::EffectDie::RavenclawV1,
        Die::SlytherinV1 => game_domain::EffectDie::SlytherinV1,
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

#[cfg(test)]
mod tests {
    #[test]
    fn real_reparo_choice_explains_each_option_at_its_continuation_cursor() {
        let manifest = super::game_one_manifest();
        let digest = manifest.digest.clone();
        let catalog = super::ContentCatalog::new(vec![manifest]);
        let cursor = game_domain::EffectCursor {
            rule_id: "rule:g1-hogwarts-009".to_owned(),
            path: vec![game_domain::EffectPathSegment::SequenceEffect(1)],
        };
        assert_eq!(
            catalog.rule_name(&digest, &cursor.rule_id).as_deref(),
            Some("Reparo")
        );
        assert_eq!(
            catalog.choice_option_labels(&digest, &cursor),
            vec![
                (
                    "option:1".to_owned(),
                    "Você recebe 2 de Influência.".to_owned()
                ),
                ("option:2".to_owned(), "Você compra 1 carta.".to_owned()),
            ]
        );
    }

    #[test]
    fn game_one_cards_explain_their_executable_effects_in_portuguese() {
        let manifest = super::game_one_manifest();
        let digest = manifest.digest.clone();
        let ids = manifest
            .entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    game_content::EntryKind::StarterCard
                        | game_content::EntryKind::HogwartsCard
                        | game_content::EntryKind::DarkArts
                        | game_content::EntryKind::Villain
                )
            })
            .map(|entry| entry.catalog_id.as_str().to_owned())
            .collect::<Vec<_>>();
        let catalog = super::ContentCatalog::new(vec![manifest]);
        for id in ids {
            assert!(
                catalog
                    .entity_description(&digest, &id, game_content::FunctionalField::Effect)
                    .is_some_and(|text| !text.is_empty()),
                "missing description for {id}"
            );
        }
        assert_eq!(
            catalog
                .entity_description(
                    &digest,
                    "hogwarts-card:005",
                    game_content::FunctionalField::Effect
                )
                .as_deref(),
            Some("Você recebe 1 de Ataque. Você compra 1 carta.")
        );
        assert_eq!(
            catalog
                .entity_description(
                    &digest,
                    "villain:001",
                    game_content::FunctionalField::Reward
                )
                .as_deref(),
            Some("Cada Herói compra 1 carta.")
        );
    }

    #[test]
    fn game_three_descriptions_identify_nevilles_recipient_and_optional_discard() {
        let manifest = super::game_three_manifest();
        let digest = manifest.digest.clone();
        let catalog = super::ContentCatalog::new(vec![manifest]);
        assert_eq!(
            catalog
                .entity_description(&digest, "hero:008", game_content::FunctionalField::Ability)
                .as_deref(),
            Some(
                "No seu turno, a primeira vez que cada Herói recuperar Vida: Esse Herói recebe 1 de Vida."
            )
        );
        let crystal = catalog
            .entity_description(
                &digest,
                "hogwarts-card:026",
                game_content::FunctionalField::Effect,
            )
            .expect("Crystal Ball description");
        assert!(crystal.ends_with("Ou: Não realizar esta ação."));
    }

    #[test]
    fn shipped_game_one_is_playable_only_after_every_rule_compiles() {
        let manifest = super::game_one_manifest();
        assert!(manifest.playable);
        assert_eq!(manifest.manifest_version, 4);
        assert_eq!((manifest.record_count, manifest.card_count), (45, 93));
        let digest = manifest.digest.clone();
        let expected = manifest.executable_rules.len();
        let catalog = super::ContentCatalog::new(vec![manifest]);
        let compiled = catalog.effect_rules(&digest).expect("compiled rules");
        assert_eq!(compiled.len(), expected);
        game_domain::ValidatedGameRules::new(compiled).expect("domain capabilities");
    }

    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn effect_rules_preserve_phase_triggers_and_order_values() {
        let rules = [
            ("rule:dark-arts", EffectTrigger::DarkArts, 7),
            ("rule:villains", EffectTrigger::Villains, 11),
            ("rule:manual", EffectTrigger::Manual, 13),
        ]
        .into_iter()
        .map(|(id, trigger, order)| EffectRule {
            id: game_content::RuleId::parse(id).expect("fixture rule ID should be valid"),
            trigger,
            order,
            cost: Vec::new(),
            effect: Effect::NoOp,
            provenance: None,
        })
        .collect::<Vec<_>>();
        let executable_rules = rules.iter().map(|rule| rule.id.clone()).collect();
        let catalog = ContentCatalog::new(vec![ContentManifest {
            manifest_version: 2,
            content_version: "fixture-v1".to_owned(),
            ruleset_version: "fixture-rules-v1".to_owned(),
            digest: "blake3:fixture".to_owned(),
            record_count: 0,
            card_count: 0,
            playable: true,
            gaps: Vec::new(),
            entries: Vec::new(),
            executable_rules,
            rules,
            game_setups: Vec::new(),
            sources: Vec::new(),
        }]);

        let compiled = catalog
            .effect_rules("blake3:fixture")
            .expect("the fixture manifest should compile");
        let compiled = compiled
            .iter()
            .map(|rule| (rule.id.as_str(), rule))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(compiled.len(), 3);
        assert_eq!(
            compiled["rule:dark-arts"].trigger,
            game_domain::EffectTrigger::DarkArts
        );
        assert_eq!(compiled["rule:dark-arts"].order, 7);
        assert_eq!(
            compiled["rule:villains"].trigger,
            game_domain::EffectTrigger::Villains
        );
        assert_eq!(compiled["rule:villains"].order, 11);
        assert_eq!(
            compiled["rule:manual"].trigger,
            game_domain::EffectTrigger::Manual
        );
        assert_eq!(compiled["rule:manual"].order, 13);
    }
}
