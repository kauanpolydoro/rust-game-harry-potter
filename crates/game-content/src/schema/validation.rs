use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Condition, ContentSet, Effect, EffectChoiceAudience, Eligibility, EntryKind, FunctionalField,
    GameSetupOwner, ImportFailure, Operation, Resource, RuleId, Selector, Zone,
};

use super::CandidateBundle;

mod game_one;
mod reactions;
mod revelation;
mod versions;
use reactions::validate_reactive_definitions;

const MAX_EFFECT_DEPTH: usize = 32;
const MAX_EFFECT_NODES: usize = 1_024;
const MAX_BRANCHES: usize = 64;
const MAX_REPEAT: u8 = 16;
const MAX_RUNTIME_NODES: usize = 4_096;
const MAX_RUNTIME_RULE_ID_LENGTH: usize = 244;
const MAX_RUNTIME_OUTCOMES: usize = 4_096;
const MAX_TARGETS: u16 = 32;
const MAX_PARTICIPANT_HEROES: usize = 4;
const MAX_VERSION_BYTES: usize = 256;
const MAX_PARTICIPANTS: u32 = 4;

pub(super) fn validate(
    bundle: &CandidateBundle,
    inventory: super::Inventory,
) -> Result<(), ImportFailure> {
    validate_metadata(bundle)?;
    versions::validate(bundle)?;
    validate_inventory(bundle, inventory)?;
    validate_provenance(bundle)?;
    validate_game_setups(bundle)?;
    if matches!(inventory, super::Inventory::GameOne) {
        game_one::validate_setup(bundle)?;
    }
    validate_effects(bundle)?;
    validate_reactive_definitions(bundle)?;
    revelation::validate(bundle)?;
    validate_structural_definitions(bundle)?;
    validate_references(bundle)
}

fn validate_structural_definitions(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    use crate::StructuralRule;
    let structural = bundle
        .rules
        .iter()
        .filter_map(|rule| match rule.effect {
            Effect::Structural { rule: assertion } => Some((&rule.id, assertion)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    for entry in &bundle.entries {
        for (field, definition) in &entry.functional {
            let Some(assertion) = definition.rule.as_ref().and_then(|id| structural.get(id)) else {
                continue;
            };
            if !matches!(
                (entry.kind, field, assertion),
                (
                    EntryKind::Hero,
                    FunctionalField::Ability,
                    StructuralRule::NoHeroAbility
                ) | (
                    EntryKind::Location,
                    FunctionalField::Effect,
                    StructuralRule::NoLocationEffect
                ) | (
                    EntryKind::Adventure,
                    FunctionalField::Setup,
                    StructuralRule::GameOneSetup
                ) | (
                    EntryKind::Adventure | EntryKind::Ruleset,
                    FunctionalField::Precedence,
                    StructuralRule::GameOnePrecedence
                )
            ) {
                return Err(ImportFailure {
                    message: format!(
                        "entry {} has an incompatible structural definition for {field:?}",
                        entry.id
                    ),
                });
            }
        }
    }
    for rule in &bundle.rules {
        if (structural.contains_key(&rule.id)
            && (rule.trigger != crate::EffectTrigger::Manual || !rule.cost.is_empty()))
            || rule
                .effect
                .references()
                .iter()
                .any(|id| structural.contains_key(id))
        {
            return Err(ImportFailure {
                message: format!(
                    "structural definition cannot execute inside rule {}",
                    rule.id
                ),
            });
        }
    }
    Ok(())
}

fn validate_metadata(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    for (label, version) in [
        ("content version", bundle.content_version.as_str()),
        ("ruleset version", bundle.ruleset_version.as_str()),
    ] {
        if !valid_version(version) {
            return Err(ImportFailure {
                message: format!(
                    "{label} must be a non-empty lowercase version identifier of at most {MAX_VERSION_BYTES} bytes"
                ),
            });
        }
    }
    if !valid_locale(&bundle.locale) {
        return Err(ImportFailure {
            message: "locale must be a non-empty language tag".to_owned(),
        });
    }
    for source in &bundle.sources {
        if !valid_slug(&source.id) {
            return Err(ImportFailure {
                message: "source ID must be a non-empty lowercase slug".to_owned(),
            });
        }
        if !valid_https_uri(&source.uri) {
            return Err(ImportFailure {
                message: format!("source {} must use an absolute HTTPS URI", source.id),
            });
        }
    }
    Ok(())
}

fn valid_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_VERSION_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'_')
        })
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value
            .bytes()
            .next_back()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn valid_locale(value: &str) -> bool {
    let mut parts = value.split('-');
    let language = parts.next().unwrap_or_default();
    (2..=3).contains(&language.len())
        && language.bytes().all(|byte| byte.is_ascii_lowercase())
        && parts.all(|part| {
            (2..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

fn valid_https_uri(value: &str) -> bool {
    value
        .strip_prefix("https://")
        .and_then(|remainder| remainder.split('/').next())
        .is_some_and(|authority| {
            !authority.is_empty() && authority.bytes().all(|byte| !byte.is_ascii_whitespace())
        })
}

fn validate_inventory(
    bundle: &CandidateBundle,
    inventory: super::Inventory,
) -> Result<(), ImportFailure> {
    if let Some(duplicate) = bundle
        .entries
        .windows(2)
        .find(|pair| pair[0].id == pair[1].id)
        .map(|pair| &pair[0].id)
    {
        return Err(ImportFailure {
            message: format!("duplicate catalog ID {duplicate}"),
        });
    }

    if bundle
        .entries
        .iter()
        .any(|entry| entry.set != ContentSet::Base)
    {
        return Err(ImportFailure {
            message: "base catalog cannot contain expansion or promo entries".to_owned(),
        });
    }

    for entry in &bundle.entries {
        if !(1..=7).contains(&entry.introduced_in) {
            return Err(ImportFailure {
                message: format!(
                    "entry {} must be introduced in a base game from 1 through 7",
                    entry.id
                ),
            });
        }
        let expected_fields = entry.kind.required_functional_fields();
        if entry.required_functional_fields != expected_fields {
            return Err(ImportFailure {
                message: format!(
                    "entry {} must declare functional fields {expected_fields:?}",
                    entry.id
                ),
            });
        }
        if let Some(unexpected) = entry
            .functional
            .keys()
            .find(|field| !expected_fields.contains(field))
        {
            return Err(ImportFailure {
                message: format!(
                    "entry {} has unexpected functional field {unexpected:?}",
                    entry.id
                ),
            });
        }
        validate_functional_definition_shapes(entry)?;
    }

    let card_count = bundle
        .entries
        .iter()
        .map(|entry| u32::from(entry.copies))
        .sum::<u32>();

    let (record_count, expected_cards) = inventory.counts();
    if bundle.entries.len() != record_count || card_count != expected_cards {
        return Err(ImportFailure {
            message: format!(
                "catalog must contain {record_count} records and {expected_cards} cards; found {} records and {card_count} cards",
                bundle.entries.len()
            ),
        });
    }

    if matches!(inventory, super::Inventory::GameOne) {
        validate_game_one_inventory(bundle)?;
    }

    Ok(())
}

fn validate_game_one_inventory(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    let mut expected = BTreeMap::new();
    for (prefix, kind, copies) in [
        (
            "hogwarts-card",
            EntryKind::HogwartsCard,
            &[1, 2, 4, 1, 4, 2, 1, 4, 6, 1, 1, 0, 3][..],
        ),
        (
            "starter",
            EntryKind::StarterCard,
            &[28, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1][..],
        ),
        ("dark-arts", EntryKind::DarkArts, &[3, 2, 3, 2][..]),
        ("villain", EntryKind::Villain, &[1, 1, 1][..]),
        ("location", EntryKind::Location, &[1, 1][..]),
        ("turn-order", EntryKind::TurnOrder, &[1, 1, 1, 1][..]),
    ] {
        for (index, copies) in copies.iter().enumerate().filter(|(_, count)| **count > 0) {
            expected.insert(format!("{prefix}:{:03}", index + 1), (kind, *copies));
        }
    }
    for index in [1, 4, 7, 10] {
        expected.insert(format!("hero:{index:03}"), (EntryKind::Hero, 1));
    }
    for (id, kind) in [
        ("adventure:001", EntryKind::Adventure),
        ("catalog:game-one-v1", EntryKind::Catalog),
        ("ruleset:game-one-v1", EntryKind::Ruleset),
    ] {
        expected.insert(id.to_owned(), (kind, 0));
    }
    for entry in &bundle.entries {
        if entry.introduced_in != 1
            || expected.remove(entry.id.as_str()) != Some((entry.kind, entry.copies))
        {
            return Err(ImportFailure {
                message: format!("entry {} does not match the Game 1 inventory", entry.id),
            });
        }
    }
    if !expected.is_empty() {
        return Err(ImportFailure {
            message: "Game 1 inventory is incomplete".to_owned(),
        });
    }
    Ok(())
}

fn validate_functional_definition_shapes(
    entry: &super::CandidateEntry,
) -> Result<(), ImportFailure> {
    if let Some(incompatible) = entry.functional.iter().find_map(|(field, definition)| {
        definition
            .value
            .filter(|_| {
                !matches!(
                    field,
                    FunctionalField::Cost
                        | FunctionalField::Health
                        | FunctionalField::ControlLimit
                        | FunctionalField::DarkArtsCount
                )
            })
            .map(|_| field)
    }) {
        return Err(ImportFailure {
            message: format!(
                "entry {} value is incompatible with functional field {incompatible:?}",
                entry.id
            ),
        });
    }
    if entry
        .functional
        .get(&FunctionalField::Health)
        .and_then(|definition| definition.value)
        == Some(0)
    {
        return Err(ImportFailure {
            message: format!("entry {} Health value must be greater than zero", entry.id),
        });
    }

    Ok(())
}

fn validate_rule_provenance(
    bundle: &CandidateBundle,
    sources: &BTreeSet<&str>,
) -> Result<(), ImportFailure> {
    for rule in &bundle.rules {
        if let Some(provenance) = &rule.provenance
            && (provenance.sources.is_empty()
                || provenance
                    .sources
                    .iter()
                    .any(|id| !sources.contains(id.as_str())))
        {
            return Err(ImportFailure {
                message: format!("rule {} has empty or unknown provenance sources", rule.id),
            });
        }
    }

    Ok(())
}

fn validate_provenance(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    if let Some(duplicate) = bundle
        .sources
        .windows(2)
        .find(|pair| pair[0].id == pair[1].id)
        .map(|pair| &pair[0].id)
    {
        return Err(ImportFailure {
            message: format!("duplicate provenance source {duplicate}"),
        });
    }

    let sources = bundle
        .sources
        .iter()
        .map(|source| source.id.as_str())
        .collect::<BTreeSet<_>>();

    validate_rule_provenance(bundle, &sources)?;

    for entry in &bundle.entries {
        for (field, field_sources) in &entry.provenance {
            if field_sources.is_empty() {
                return Err(ImportFailure {
                    message: format!("entry {} has empty provenance for {field}", entry.id),
                });
            }
            if let Some(unknown_source) = field_sources
                .iter()
                .find(|source_id| !sources.contains(source_id.as_str()))
            {
                return Err(ImportFailure {
                    message: format!(
                        "entry {} has unknown provenance source {unknown_source} for {field}",
                        entry.id
                    ),
                });
            }
        }
        for required_field in ["id", "kind", "set", "copies", "introduced_in", "names.en"] {
            if !entry.provenance.contains_key(required_field) {
                return Err(ImportFailure {
                    message: format!("entry {} has no provenance for {required_field}", entry.id),
                });
            }
        }
        if !entry.names.contains_key("en") {
            return Err(ImportFailure {
                message: format!("entry {} has no English fallback name", entry.id),
            });
        }
        for (locale, name) in &entry.names {
            if name.trim().is_empty() {
                return Err(ImportFailure {
                    message: format!("entry {} has an empty name for {locale}", entry.id),
                });
            }
            let field = format!("names.{locale}");
            if !entry.provenance.contains_key(&field) {
                return Err(ImportFailure {
                    message: format!("entry {} has no provenance for {field}", entry.id),
                });
            }
        }
        for definition in entry.functional.values() {
            if let Some(unknown_source) = definition
                .sources
                .iter()
                .find(|source_id| !sources.contains(source_id.as_str()))
            {
                return Err(ImportFailure {
                    message: format!(
                        "entry {} has unknown provenance source {unknown_source}",
                        entry.id
                    ),
                });
            }
        }
    }

    for setup in &bundle.game_setups {
        if setup.sources.is_empty() {
            return Err(ImportFailure {
                message: format!(
                    "game setup {} has no provenance sources",
                    setup.adventure_id
                ),
            });
        }
        if let Some(unknown_source) = setup
            .sources
            .iter()
            .find(|source_id| !sources.contains(source_id.as_str()))
        {
            return Err(ImportFailure {
                message: format!(
                    "game setup {} has unknown provenance source {unknown_source}",
                    setup.adventure_id
                ),
            });
        }
    }

    Ok(())
}

fn validate_game_setups(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    if let Some(duplicate) = bundle
        .game_setups
        .windows(2)
        .find(|pair| pair[0].adventure_id == pair[1].adventure_id)
        .map(|pair| &pair[0].adventure_id)
    {
        return Err(ImportFailure {
            message: format!("duplicate game setup for adventure {duplicate}"),
        });
    }

    let entries = bundle
        .entries
        .iter()
        .map(|entry| (&entry.id, entry))
        .collect::<BTreeMap<_, _>>();

    for setup in &bundle.game_setups {
        validate_game_setup(setup, &entries)?;
    }

    Ok(())
}

fn validate_game_setup(
    setup: &super::CandidateGameSetup,
    entries: &BTreeMap<&crate::CatalogId, &super::CandidateEntry>,
) -> Result<(), ImportFailure> {
    if setup.entities.is_empty() {
        return Err(ImportFailure {
            message: format!(
                "game setup {} must contain at least one entity",
                setup.adventure_id
            ),
        });
    }
    if let Some(duplicate) = setup
        .sources
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| &pair[0])
    {
        return Err(ImportFailure {
            message: format!(
                "game setup {} has duplicate provenance source {duplicate}",
                setup.adventure_id
            ),
        });
    }
    let adventure = entries
        .get(&setup.adventure_id)
        .ok_or_else(|| ImportFailure {
            message: format!(
                "game setup has unknown adventure reference {}",
                setup.adventure_id
            ),
        })?;
    if adventure.kind != EntryKind::Adventure {
        return Err(ImportFailure {
            message: format!(
                "game setup adventure reference {} has kind {:?}",
                setup.adventure_id, adventure.kind
            ),
        });
    }

    let mut entity_ids = BTreeSet::new();
    for entity in &setup.entities {
        validate_setup_entity(&setup.adventure_id, entity, entries, &mut entity_ids)?;
    }

    Ok(())
}

fn validate_setup_entity(
    adventure_id: &crate::CatalogId,
    entity: &super::CandidateGameSetupEntity,
    entries: &BTreeMap<&crate::CatalogId, &super::CandidateEntry>,
    entity_ids: &mut BTreeSet<crate::CatalogId>,
) -> Result<(), ImportFailure> {
    if !entity_ids.insert(entity.catalog_id.clone()) {
        return Err(ImportFailure {
            message: format!(
                "game setup {adventure_id} has duplicate entity reference {}",
                entity.catalog_id
            ),
        });
    }
    let entry = entries
        .get(&entity.catalog_id)
        .ok_or_else(|| ImportFailure {
            message: format!(
                "game setup {adventure_id} has unknown entity reference {}",
                entity.catalog_id
            ),
        })?;
    if !setup_entity_is_compatible(entry.kind, entity.zone, entity.owner) {
        return Err(ImportFailure {
            message: format!(
                "game setup {adventure_id} entity {} kind {:?}, zone {:?}, and owner {:?} are incompatible",
                entity.catalog_id, entry.kind, entity.zone, entity.owner
            ),
        });
    }
    validate_setup_entity_data(adventure_id, &entity.catalog_id, entry)?;
    if entity.copies == 0 {
        return Err(ImportFailure {
            message: format!(
                "game setup {adventure_id} entity {} must request at least one copy",
                entity.catalog_id
            ),
        });
    }
    let required_copies = u32::from(entity.copies)
        * match entity.owner {
            GameSetupOwner::EachParticipant => MAX_PARTICIPANTS,
            GameSetupOwner::None
            | GameSetupOwner::Harry
            | GameSetupOwner::Hermione
            | GameSetupOwner::Neville
            | GameSetupOwner::Ron => 1,
        };
    if required_copies > u32::from(entry.copies) {
        return Err(ImportFailure {
            message: format!(
                "game setup {adventure_id} entity {} requires {required_copies} copies but the catalog declares {}",
                entity.catalog_id, entry.copies
            ),
        });
    }

    Ok(())
}

fn validate_setup_entity_data(
    adventure_id: &crate::CatalogId,
    entity_id: &crate::CatalogId,
    entry: &super::CandidateEntry,
) -> Result<(), ImportFailure> {
    if entry
        .functional
        .get(&FunctionalField::Effect)
        .and_then(|definition| definition.rule.as_ref())
        .is_none()
    {
        return Err(ImportFailure {
            message: format!(
                "game setup {adventure_id} entity {entity_id} requires an Effect rule"
            ),
        });
    }

    match entry.kind {
        EntryKind::HogwartsCard => {
            let has_unambiguous_cost = entry
                .functional
                .get(&FunctionalField::Cost)
                .is_some_and(|definition| definition.value.is_some() && definition.rule.is_none());
            if !has_unambiguous_cost {
                return Err(ImportFailure {
                    message: format!(
                        "game setup {adventure_id} entity {entity_id} requires a Cost value"
                    ),
                });
            }

            Ok(())
        }
        EntryKind::Villain => {
            let has_positive_health =
                entry
                    .functional
                    .get(&FunctionalField::Health)
                    .is_some_and(|definition| {
                        definition.value.is_some_and(|health| health > 0)
                            && definition.rule.is_none()
                    });
            if !has_positive_health {
                return Err(ImportFailure {
                    message: format!(
                        "game setup {adventure_id} entity {entity_id} requires a positive Health value"
                    ),
                });
            }

            Ok(())
        }
        EntryKind::Location => {
            let has_positive_control_limit = entry
                .functional
                .get(&FunctionalField::ControlLimit)
                .is_some_and(|definition| {
                    definition.value.is_some_and(|limit| limit > 0) && definition.rule.is_none()
                });
            let has_valid_dark_arts_count = entry
                .functional
                .get(&FunctionalField::DarkArtsCount)
                .is_some_and(|definition| {
                    definition
                        .value
                        .is_some_and(|count| (1..=u16::from(u8::MAX)).contains(&count))
                        && definition.rule.is_none()
                });
            if !has_positive_control_limit || !has_valid_dark_arts_count {
                return Err(ImportFailure {
                    message: format!(
                        "game setup {adventure_id} entity {entity_id} requires a positive ControlLimit and a DarkArtsCount from 1 through 255"
                    ),
                });
            }

            Ok(())
        }
        EntryKind::StarterCard | EntryKind::DarkArts => Ok(()),
        _ => Err(ImportFailure {
            message: format!(
                "game setup {adventure_id} entity {entity_id} has unsupported kind {:?}",
                entry.kind
            ),
        }),
    }
}

fn setup_entity_is_compatible(kind: EntryKind, zone: Zone, owner: GameSetupOwner) -> bool {
    matches!(
        (kind, zone, owner),
        (
            EntryKind::StarterCard | EntryKind::HogwartsCard,
            Zone::HeroDiscardPile | Zone::HeroDrawPile | Zone::HeroHand | Zone::HeroPlayArea,
            GameSetupOwner::EachParticipant
                | GameSetupOwner::Harry
                | GameSetupOwner::Hermione
                | GameSetupOwner::Neville
                | GameSetupOwner::Ron,
        ) | (
            EntryKind::HogwartsCard,
            Zone::HogwartsDeck | Zone::Market,
            GameSetupOwner::None,
        ) | (
            EntryKind::Villain,
            Zone::VillainDeck | Zone::ActiveVillains | Zone::VillainDiscard,
            GameSetupOwner::None,
        ) | (
            EntryKind::DarkArts,
            Zone::DarkArtsDeck | Zone::DarkArtsDiscard,
            GameSetupOwner::None,
        ) | (
            EntryKind::Location,
            Zone::ActiveLocation | Zone::LocationDeck | Zone::LocationDiscard,
            GameSetupOwner::None,
        )
    )
}

fn validate_effects(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    if bundle.rules.len() > MAX_EFFECT_NODES {
        return Err(ImportFailure {
            message: format!("bundle declares more than {MAX_EFFECT_NODES} effect rules"),
        });
    }
    for rule in &bundle.rules {
        if rule.cost.len() > MAX_BRANCHES {
            return Err(ImportFailure {
                message: format!(
                    "rule {} declares more than {MAX_BRANCHES} resource costs",
                    rule.id
                ),
            });
        }
        for cost in &rule.cost {
            if cost.amount == 0 || cost.resource == Resource::Control {
                return Err(ImportFailure {
                    message: format!(
                        "rule {} has an invalid {:?} resource cost",
                        rule.id, cost.resource
                    ),
                });
            }
        }
        let mut nodes = 0;
        let mut selectors = BTreeMap::new();
        rule.effect
            .validate(&rule.id, 0, &mut nodes, &mut selectors)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct EffectStats {
    compiled_nodes: usize,
    reference_depth: usize,
    runtime_nodes: usize,
    runtime_outcomes: usize,
}

pub(super) fn validate_runtime_rules(
    bundle: &CandidateBundle,
    roots: &BTreeSet<RuleId>,
) -> Result<(), ImportFailure> {
    let mut previous_automatic_root: Option<&crate::EffectRule> = None;
    for rule in bundle
        .rules
        .iter()
        .filter(|rule| roots.contains(&rule.id) && rule.trigger.is_automatic())
    {
        if !rule.cost.is_empty() {
            return Err(ImportFailure {
                message: format!(
                    "executable automatic root {} cannot declare a cost",
                    rule.id
                ),
            });
        }
        if let Some(previous) = previous_automatic_root
            && previous.trigger == rule.trigger
            && previous.order == rule.order
        {
            return Err(ImportFailure {
                message: format!(
                    "executable rules {} and {} share automatic trigger {:?} and order {}",
                    previous.id, rule.id, rule.trigger, rule.order
                ),
            });
        }
        previous_automatic_root = Some(rule);
    }

    let rules = EffectRuleStats::new(bundle);
    let mut memo = BTreeMap::new();
    let mut compiled_nodes = 0;
    let mut runtime_nodes = 0;
    let mut automatic_outcomes = 0;
    let mut manual_outcomes = 0;
    for root in roots {
        if root.as_str().chars().count() > MAX_RUNTIME_RULE_ID_LENGTH {
            return Err(ImportFailure {
                message: format!(
                    "runtime rule ID exceeds {MAX_RUNTIME_RULE_ID_LENGTH} characters: {root}"
                ),
            });
        }
        let stats = rule_stats(root, &rules, &mut memo).ok_or_else(|| ImportFailure {
            message: format!("runtime rule {root} exceeds the closed effect execution limit"),
        })?;
        if stats.reference_depth > MAX_EFFECT_DEPTH {
            return Err(ImportFailure {
                message: format!("runtime rule {root} exceeds the rule reference depth limit"),
            });
        }
        compiled_nodes =
            checked_total(compiled_nodes, stats.compiled_nodes).ok_or_else(|| ImportFailure {
                message: format!("runtime rule {root} exceeds the closed effect complexity limit"),
            })?;
        runtime_nodes =
            checked_total(runtime_nodes, stats.runtime_nodes).ok_or_else(|| ImportFailure {
                message: format!("runtime rule {root} exceeds the execution step limit"),
            })?;
        let root_rule = bundle
            .rules
            .iter()
            .find(|rule| &rule.id == root)
            .ok_or_else(|| ImportFailure {
                message: format!("runtime rule {root} is not declared in the bundle"),
            })?;
        let root_outcomes = outcome_total(root_rule.cost.len(), stats.runtime_outcomes);
        let phase_outcomes = if root_rule.trigger.is_automatic() {
            &mut automatic_outcomes
        } else {
            &mut manual_outcomes
        };
        *phase_outcomes = outcome_total(*phase_outcomes, root_outcomes);
        if *phase_outcomes > MAX_RUNTIME_OUTCOMES {
            return Err(ImportFailure {
                message: format!("runtime rule {root} exceeds the effect outcome limit"),
            });
        }
    }
    Ok(())
}

struct EffectRuleStats<'a> {
    definitions: BTreeMap<&'a RuleId, &'a Effect>,
    hero_card_count: usize,
    dark_arts_card_count: usize,
    dark_arts_rules: Vec<&'a RuleId>,
}

impl<'a> EffectRuleStats<'a> {
    fn new(bundle: &'a CandidateBundle) -> Self {
        Self {
            definitions: bundle
                .rules
                .iter()
                .map(|rule| (&rule.id, &rule.effect))
                .collect(),
            hero_card_count: bundle
                .entries
                .iter()
                .filter(|entry| {
                    matches!(entry.kind, EntryKind::StarterCard | EntryKind::HogwartsCard)
                })
                .map(|entry| usize::from(entry.copies))
                .sum(),
            dark_arts_card_count: bundle
                .entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::DarkArts)
                .map(|entry| usize::from(entry.copies))
                .sum(),
            dark_arts_rules: bundle
                .entries
                .iter()
                .filter(|entry| entry.kind == EntryKind::DarkArts)
                .filter_map(|entry| {
                    entry
                        .functional
                        .get(&FunctionalField::Effect)?
                        .rule
                        .as_ref()
                })
                .collect(),
        }
    }
}

fn revelation_stats(
    rules: &EffectRuleStats<'_>,
    memo: &mut BTreeMap<RuleId, EffectStats>,
) -> Option<EffectStats> {
    let mut stats = EffectStats {
        compiled_nodes: 1,
        reference_depth: 0,
        runtime_nodes: 1,
        runtime_outcomes: rules.dark_arts_card_count.saturating_mul(2).max(1),
    };
    let mut card_nodes = 0;
    let mut card_outcomes = 0;
    for rule in &rules.dark_arts_rules {
        let card = rule_stats(rule, rules, memo)?;
        card_nodes = card_nodes.max(card.runtime_nodes);
        card_outcomes = card_outcomes.max(card.runtime_outcomes);
    }
    stats.runtime_nodes = checked_total(stats.runtime_nodes, card_nodes)?;
    stats.runtime_outcomes = outcome_total(stats.runtime_outcomes, card_outcomes);
    Some(stats)
}

fn rule_stats(
    rule_id: &RuleId,
    rules: &EffectRuleStats<'_>,
    memo: &mut BTreeMap<RuleId, EffectStats>,
) -> Option<EffectStats> {
    if let Some(stats) = memo.get(rule_id) {
        return Some(*stats);
    }
    let stats = effect_stats(rules.definitions.get(rule_id)?, rules, memo)?;
    memo.insert(rule_id.clone(), stats);
    Some(stats)
}

fn application_stats(
    target: &Selector,
    operation: &Operation,
    rules: &EffectRuleStats<'_>,
) -> EffectStats {
    let runtime_outcomes = if let Operation::Draw { amount } = operation {
        repeated_outcomes(
            outcome_total(
                rules.hero_card_count.saturating_mul(2),
                usize::from(*amount),
            ),
            usize::from(target.cardinality.max).clamp(1, MAX_PARTICIPANT_HEROES),
        )
    } else {
        usize::from(target.cardinality.max).max(1)
    };
    EffectStats {
        compiled_nodes: 1,
        reference_depth: 0,
        runtime_nodes: 1,
        runtime_outcomes,
    }
}

fn effect_stats(
    effect: &Effect,
    rules: &EffectRuleStats<'_>,
    memo: &mut BTreeMap<RuleId, EffectStats>,
) -> Option<EffectStats> {
    match effect {
        Effect::CardType { .. } | Effect::TopDeckAcquisition { .. } => Some(EffectStats {
            compiled_nodes: 1,
            reference_depth: 0,
            runtime_nodes: 1,
            runtime_outcomes: 0,
        }),
        Effect::Reaction { effect, .. } => {
            let mut stats = effect_stats(effect, rules, memo)?;
            stats.compiled_nodes = checked_total(stats.compiled_nodes, 1)?;
            stats.reference_depth += 1;
            stats.runtime_nodes = checked_total(stats.runtime_nodes, 1)?;
            Some(stats)
        }
        Effect::RevealDarkArts => revelation_stats(rules, memo),
        Effect::Apply { target, operation } => Some(application_stats(target, operation, rules)),
        Effect::Choice { audience, options } => {
            let mut stats = branch_stats(options, 1, 0, true, rules, memo)?;
            if *audience == EffectChoiceAudience::EachHero {
                stats.runtime_nodes = stats
                    .runtime_nodes
                    .checked_mul(MAX_PARTICIPANT_HEROES)
                    .filter(|total| *total <= MAX_RUNTIME_NODES)?;
                stats.runtime_outcomes =
                    repeated_outcomes(stats.runtime_outcomes, MAX_PARTICIPANT_HEROES);
            }
            Some(stats)
        }
        Effect::NoOp
        | Effect::Structural { .. }
        | Effect::Terminal { .. }
        | Effect::HandDamageLimit { .. } => Some(EffectStats {
            compiled_nodes: 1,
            reference_depth: 0,
            runtime_nodes: 1,
            runtime_outcomes: 1,
        }),
        Effect::Condition {
            then, otherwise, ..
        } => {
            let then_stats = effect_stats(then, rules, memo)?;
            let otherwise_stats = match otherwise.as_deref() {
                Some(effect) => effect_stats(effect, rules, memo)?,
                None => EffectStats {
                    compiled_nodes: 0,
                    reference_depth: 0,
                    runtime_nodes: 0,
                    runtime_outcomes: 0,
                },
            };
            Some(EffectStats {
                compiled_nodes: checked_total(
                    checked_total(1, then_stats.compiled_nodes)?,
                    otherwise_stats.compiled_nodes,
                )?,
                reference_depth: then_stats
                    .reference_depth
                    .max(otherwise_stats.reference_depth),
                runtime_nodes: checked_total(
                    1,
                    then_stats.runtime_nodes.max(otherwise_stats.runtime_nodes),
                )?,
                runtime_outcomes: then_stats
                    .runtime_outcomes
                    .max(otherwise_stats.runtime_outcomes),
            })
        }
        Effect::Reference { rule } => {
            let mut stats = rule_stats(rule, rules, memo)?;
            stats.reference_depth = stats.reference_depth.checked_add(1)?;
            Some(stats)
        }
        Effect::Repeat { times, effect } => {
            let child = effect_stats(effect, rules, memo)?;
            Some(EffectStats {
                compiled_nodes: checked_total(1, child.compiled_nodes)?,
                reference_depth: child.reference_depth,
                runtime_nodes: checked_total(
                    1,
                    child.runtime_nodes.checked_mul(usize::from(*times))?,
                )?,
                runtime_outcomes: repeated_outcomes(child.runtime_outcomes, usize::from(*times)),
            })
        }
        Effect::Roll { outcomes, .. } => branch_stats(outcomes, 1, 1, true, rules, memo),
        Effect::Sequence { effects } => branch_stats(effects, 1, 0, false, rules, memo),
    }
}

fn branch_stats(
    effects: &[Effect],
    base_runtime_nodes: usize,
    base_runtime_outcomes: usize,
    runtime_uses_largest_branch: bool,
    rules: &EffectRuleStats<'_>,
    memo: &mut BTreeMap<RuleId, EffectStats>,
) -> Option<EffectStats> {
    let mut combined = EffectStats {
        compiled_nodes: 1,
        reference_depth: 0,
        runtime_nodes: base_runtime_nodes,
        runtime_outcomes: base_runtime_outcomes,
    };
    for effect in effects {
        let child = effect_stats(effect, rules, memo)?;
        combined.compiled_nodes = checked_total(combined.compiled_nodes, child.compiled_nodes)?;
        combined.reference_depth = combined.reference_depth.max(child.reference_depth);
        combined.runtime_nodes = if runtime_uses_largest_branch {
            base_runtime_nodes.checked_add(
                combined
                    .runtime_nodes
                    .saturating_sub(base_runtime_nodes)
                    .max(child.runtime_nodes),
            )?
        } else {
            checked_total(combined.runtime_nodes, child.runtime_nodes)?
        };
        combined.runtime_outcomes = if runtime_uses_largest_branch {
            outcome_total(
                base_runtime_outcomes,
                combined
                    .runtime_outcomes
                    .saturating_sub(base_runtime_outcomes)
                    .max(child.runtime_outcomes),
            )
        } else {
            outcome_total(combined.runtime_outcomes, child.runtime_outcomes)
        };
        if combined.runtime_nodes > MAX_RUNTIME_NODES {
            return None;
        }
    }
    Some(combined)
}

fn outcome_total(current: usize, additional: usize) -> usize {
    current
        .saturating_add(additional)
        .min(MAX_RUNTIME_OUTCOMES + 1)
}

fn repeated_outcomes(outcomes: usize, times: usize) -> usize {
    outcomes.saturating_mul(times).min(MAX_RUNTIME_OUTCOMES + 1)
}

fn checked_total(current: usize, additional: usize) -> Option<usize> {
    current
        .checked_add(additional)
        .filter(|total| *total <= MAX_RUNTIME_NODES)
}

fn validate_references(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    let mut rule_ids = BTreeSet::new();
    for rule in &bundle.rules {
        if !rule_ids.insert(&rule.id) {
            return Err(ImportFailure {
                message: format!("duplicate rule ID {}", rule.id),
            });
        }
    }

    for entry in &bundle.entries {
        for definition in entry.functional.values() {
            if let Some(rule_id) = definition.rule.as_ref()
                && !rule_ids.contains(rule_id)
            {
                return Err(ImportFailure {
                    message: format!("entry {} has unknown rule reference {rule_id}", entry.id),
                });
            }
        }
    }

    for rule in &bundle.rules {
        for referenced_rule in rule.effect.references() {
            if !rule_ids.contains(referenced_rule) {
                return Err(ImportFailure {
                    message: format!(
                        "rule {} has unknown rule reference {referenced_rule}",
                        rule.id
                    ),
                });
            }
        }
    }

    validate_rule_cycles(bundle)
}

fn validate_rule_cycles(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    let graph = bundle
        .rules
        .iter()
        .map(|rule| (&rule.id, rule.effect.references()))
        .collect::<BTreeMap<_, _>>();
    let mut complete = BTreeSet::new();
    let mut active = BTreeSet::new();

    for rule_id in graph.keys() {
        if let Some(cyclic_rule) = find_rule_cycle(rule_id, &graph, &mut active, &mut complete) {
            return Err(ImportFailure {
                message: format!("rule cycle detected at {cyclic_rule}"),
            });
        }
    }

    Ok(())
}

fn find_rule_cycle<'a>(
    rule_id: &'a RuleId,
    graph: &BTreeMap<&'a RuleId, Vec<&'a RuleId>>,
    active: &mut BTreeSet<&'a RuleId>,
    complete: &mut BTreeSet<&'a RuleId>,
) -> Option<&'a RuleId> {
    if complete.contains(rule_id) {
        return None;
    }
    if !active.insert(rule_id) {
        return Some(rule_id);
    }

    if let Some(references) = graph.get(rule_id) {
        for reference in references {
            if let Some(cyclic_rule) = find_rule_cycle(reference, graph, active, complete) {
                return Some(cyclic_rule);
            }
        }
    }

    active.remove(rule_id);
    complete.insert(rule_id);
    None
}

impl Effect {
    fn validate(
        &self,
        rule_id: &RuleId,
        depth: usize,
        nodes: &mut usize,
        selectors: &mut BTreeMap<String, Selector>,
    ) -> Result<(), ImportFailure> {
        *nodes += 1;
        if depth > MAX_EFFECT_DEPTH || *nodes > MAX_EFFECT_NODES {
            return Err(ImportFailure {
                message: format!("rule {rule_id} exceeds the closed effect complexity limit"),
            });
        }

        match self {
            Self::Reaction { effect, .. } => {
                effect.validate(rule_id, depth + 1, nodes, selectors)?;
            }
            Self::Apply { target, operation } => {
                target.validate(rule_id, selectors)?;
                operation.validate_zone(target.zone, rule_id)?;
            }
            Self::Choice { options, .. } => {
                if !(2..=MAX_BRANCHES).contains(&options.len()) {
                    return Err(ImportFailure {
                        message: format!(
                            "rule {rule_id} choice must have at least two conclusions and at most {MAX_BRANCHES}"
                        ),
                    });
                }
                for option in options {
                    option.validate(rule_id, depth + 1, nodes, selectors)?;
                }
            }
            Self::Condition {
                condition,
                then,
                otherwise,
            } => {
                condition.validate(rule_id, selectors)?;
                then.validate(rule_id, depth + 1, nodes, selectors)?;
                if let Some(otherwise) = otherwise {
                    otherwise.validate(rule_id, depth + 1, nodes, selectors)?;
                }
            }
            Self::Repeat { times, effect } => {
                if !(1..=MAX_REPEAT).contains(times) {
                    return Err(ImportFailure {
                        message: format!(
                            "rule {rule_id} repeat count must be between 1 and {MAX_REPEAT}"
                        ),
                    });
                }
                effect.validate(rule_id, depth + 1, nodes, selectors)?;
            }
            Self::Roll { die, outcomes } => {
                if outcomes.len() != die.sides() {
                    return Err(ImportFailure {
                        message: format!(
                            "rule {rule_id} {:?} must declare exactly {} outcomes",
                            die,
                            die.sides()
                        ),
                    });
                }
                for outcome in outcomes {
                    outcome.validate(rule_id, depth + 1, nodes, selectors)?;
                }
            }
            Self::Sequence { effects } => {
                if effects.is_empty() || effects.len() > MAX_BRANCHES {
                    return Err(ImportFailure {
                        message: format!(
                            "rule {rule_id} sequence must contain between 1 and {MAX_BRANCHES} effects"
                        ),
                    });
                }
                for effect in effects {
                    effect.validate(rule_id, depth + 1, nodes, selectors)?;
                }
            }
            Self::Structural { .. } if depth != 0 => {
                return Err(ImportFailure {
                    message: format!(
                        "structural definition in {rule_id} must be a standalone declaration"
                    ),
                });
            }
            Self::NoOp
            | Self::TopDeckAcquisition { .. }
            | Self::CardType { .. }
            | Self::HandDamageLimit { .. }
            | Self::Structural { .. }
            | Self::RevealDarkArts
            | Self::Reference { .. }
            | Self::Terminal { .. } => {}
        }
        Ok(())
    }
}

impl Condition {
    fn validate(
        &self,
        rule_id: &RuleId,
        selectors: &mut BTreeMap<String, Selector>,
    ) -> Result<(), ImportFailure> {
        match self {
            Self::HasEligibleTarget { target } => target.validate(rule_id, selectors),
            Self::ResourceAtLeast {
                target,
                resource,
                amount,
            } => {
                target.validate(rule_id, selectors)?;
                if *amount == 0 || !zone_supports_resource(target.zone, *resource) {
                    return Err(ImportFailure {
                        message: format!(
                            "rule {rule_id} condition resource {} is incompatible with zone {}",
                            resource.as_str(),
                            target.zone.as_str()
                        ),
                    });
                }
                Ok(())
            }
        }
    }
}

impl Selector {
    fn validate(
        &self,
        rule_id: &RuleId,
        selectors: &mut BTreeMap<String, Self>,
    ) -> Result<(), ImportFailure> {
        if let Some(id) = &self.id {
            if id.is_empty() {
                return Err(ImportFailure {
                    message: format!("rule {rule_id} selector ID must not be empty"),
                });
            }
            if selectors.get(id).is_some_and(|existing| existing != self) {
                return Err(ImportFailure {
                    message: format!("rule {rule_id} selector ID {id} has conflicting definitions"),
                });
            }
            selectors.entry(id.clone()).or_insert_with(|| self.clone());
        }
        if self.cardinality.min > self.cardinality.max {
            return Err(ImportFailure {
                message: format!(
                    "rule {rule_id} cardinality min {} exceeds max {}",
                    self.cardinality.min, self.cardinality.max
                ),
            });
        }
        if self.cardinality.max > MAX_TARGETS {
            return Err(ImportFailure {
                message: format!(
                    "rule {rule_id} selector exceeds the maximum cardinality {MAX_TARGETS}"
                ),
            });
        }
        for eligibility in &self.eligibility {
            match eligibility {
                Eligibility::ResourceAtLeast { resource, amount }
                    if *amount == 0 || !zone_supports_resource(self.zone, *resource) =>
                {
                    return Err(ImportFailure {
                        message: format!(
                            "rule {rule_id} eligibility resource {} is incompatible with zone {}",
                            resource.as_str(),
                            self.zone.as_str()
                        ),
                    });
                }
                Eligibility::ResourceAtLeast { .. } => {}
            }
        }
        Ok(())
    }
}

impl Operation {
    fn validate_zone(&self, zone: Zone, rule_id: &RuleId) -> Result<(), ImportFailure> {
        let compatible = match self {
            Self::Discard => zone == Zone::HeroHand,
            Self::PreventDrawing => zone == Zone::Heroes,
            Self::Draw { amount } | Self::GainAttackPerAllyPlayed { amount } => {
                (1..=16).contains(amount) && zone == Zone::Heroes
            }
            Self::ModifyResource { resource, amount } => {
                *amount != 0 && zone_supports_resource(zone, *resource)
            }
            Self::Move { to } => zone.is_card_zone() && to.is_card_zone() && zone != *to,
        };
        if !compatible {
            return Err(ImportFailure {
                message: format!(
                    "rule {rule_id} operation {} is incompatible with zone {}",
                    self.as_str(),
                    zone.as_str()
                ),
            });
        }
        Ok(())
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Discard => "discard",
            Self::PreventDrawing => "prevent_drawing",
            Self::GainAttackPerAllyPlayed { .. } => "gain_attack_per_ally_played",
            Self::Draw { .. } => "draw",
            Self::ModifyResource { .. } => "modify_resource",
            Self::Move { .. } => "move",
        }
    }
}

impl Resource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Attack => "attack",
            Self::Control => "control",
            Self::Health => "health",
            Self::Influence => "influence",
        }
    }
}

impl Zone {
    fn is_card_zone(self) -> bool {
        matches!(
            self,
            Self::DarkArtsDeck
                | Self::DarkArtsDiscard
                | Self::HeroDiscardPile
                | Self::HeroDrawPile
                | Self::HeroHand
                | Self::HeroPlayArea
                | Self::HogwartsDeck
                | Self::Market
                | Self::VillainDeck
                | Self::VillainDiscard
        )
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::ActiveLocation => "active_location",
            Self::ActiveVillains => "active_villains",
            Self::DarkArtsDeck => "dark_arts_deck",
            Self::DarkArtsDiscard => "dark_arts_discard",
            Self::HeroDiscardPile => "hero_discard_pile",
            Self::HeroDrawPile => "hero_draw_pile",
            Self::HeroHand => "hero_hand",
            Self::HeroPlayArea => "hero_play_area",
            Self::Heroes => "heroes",
            Self::HogwartsDeck => "hogwarts_deck",
            Self::LocationDeck => "location_deck",
            Self::LocationDiscard => "location_discard",
            Self::Market => "market",
            Self::VillainDeck => "villain_deck",
            Self::VillainDiscard => "villain_discard",
        }
    }
}

fn zone_supports_resource(zone: Zone, resource: Resource) -> bool {
    matches!(
        (zone, resource),
        (
            Zone::Heroes,
            Resource::Attack | Resource::Health | Resource::Influence
        ) | (Zone::ActiveVillains, Resource::Health)
            | (Zone::ActiveLocation, Resource::Control)
    )
}
