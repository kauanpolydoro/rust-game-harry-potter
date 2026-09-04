use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Condition, ContentSet, Effect, EffectChoiceAudience, Eligibility, EntryKind, FunctionalField,
    GameSetupOwner, ImportFailure, Operation, Resource, RuleId, Selector, Zone,
};

use super::CandidateBundle;

const MAX_EFFECT_DEPTH: usize = 32;
const MAX_EFFECT_NODES: usize = 1_024;
const MAX_BRANCHES: usize = 64;
const MAX_REPEAT: u8 = 16;
const MAX_RUNTIME_NODES: usize = 4_096;
const MAX_RUNTIME_RULE_ID_LENGTH: usize = 244;
const MAX_TARGETS: u16 = 32;
const MAX_PARTICIPANT_HEROES: usize = 4;
const MAX_PARTICIPANTS: u32 = 4;

pub(super) fn validate(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    validate_metadata(bundle)?;
    validate_inventory(bundle)?;
    validate_provenance(bundle)?;
    validate_game_setups(bundle)?;
    validate_effects(bundle)?;
    validate_references(bundle)
}

fn validate_metadata(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    for (label, version) in [
        ("content version", bundle.content_version.as_str()),
        ("ruleset version", bundle.ruleset_version.as_str()),
    ] {
        if !valid_version(version) {
            return Err(ImportFailure {
                message: format!("{label} must be a non-empty lowercase version identifier"),
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

fn validate_inventory(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
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

    if bundle.entries.len() != super::BASE_RECORD_COUNT || card_count != super::BASE_CARD_COUNT {
        return Err(ImportFailure {
            message: format!(
                "base catalog must contain {} records and {} cards; found {} records and {card_count} cards",
                super::BASE_RECORD_COUNT,
                super::BASE_CARD_COUNT,
                bundle.entries.len()
            ),
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
            .filter(|_| !matches!(field, FunctionalField::Cost | FunctionalField::Health))
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
            GameSetupOwner::None => 1,
            GameSetupOwner::EachParticipant => MAX_PARTICIPANTS,
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
        EntryKind::StarterCard => Ok(()),
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
            GameSetupOwner::EachParticipant,
        ) | (
            EntryKind::HogwartsCard,
            Zone::HogwartsDeck | Zone::Market,
            GameSetupOwner::None,
        ) | (
            EntryKind::Villain,
            Zone::VillainDeck | Zone::ActiveVillains,
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
}

pub(super) fn validate_runtime_rules(
    bundle: &CandidateBundle,
    roots: &BTreeSet<RuleId>,
) -> Result<(), ImportFailure> {
    let rules = bundle
        .rules
        .iter()
        .map(|rule| (&rule.id, &rule.effect))
        .collect::<BTreeMap<_, _>>();
    let mut memo = BTreeMap::new();
    let mut compiled_nodes = 0;
    let mut runtime_nodes = 0;
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
    }
    Ok(())
}

fn rule_stats(
    rule_id: &RuleId,
    rules: &BTreeMap<&RuleId, &Effect>,
    memo: &mut BTreeMap<RuleId, EffectStats>,
) -> Option<EffectStats> {
    if let Some(stats) = memo.get(rule_id) {
        return Some(*stats);
    }
    let stats = effect_stats(rules.get(rule_id)?, rules, memo)?;
    memo.insert(rule_id.clone(), stats);
    Some(stats)
}

fn effect_stats(
    effect: &Effect,
    rules: &BTreeMap<&RuleId, &Effect>,
    memo: &mut BTreeMap<RuleId, EffectStats>,
) -> Option<EffectStats> {
    match effect {
        Effect::Apply { .. } | Effect::NoOp | Effect::Terminal { .. } => Some(EffectStats {
            compiled_nodes: 1,
            reference_depth: 0,
            runtime_nodes: 1,
        }),
        Effect::Choice { audience, options } => {
            let mut stats = branch_stats(options, 1, true, rules, memo)?;
            if *audience == EffectChoiceAudience::EachHero {
                stats.runtime_nodes = stats
                    .runtime_nodes
                    .checked_mul(MAX_PARTICIPANT_HEROES)
                    .filter(|total| *total <= MAX_RUNTIME_NODES)?;
            }
            Some(stats)
        }
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
            })
        }
        Effect::Roll { outcomes, .. } => branch_stats(outcomes, 1, true, rules, memo),
        Effect::Sequence { effects } => branch_stats(effects, 1, false, rules, memo),
    }
}

fn branch_stats(
    effects: &[Effect],
    base_runtime_nodes: usize,
    runtime_uses_largest_branch: bool,
    rules: &BTreeMap<&RuleId, &Effect>,
    memo: &mut BTreeMap<RuleId, EffectStats>,
) -> Option<EffectStats> {
    let mut combined = EffectStats {
        compiled_nodes: 1,
        reference_depth: 0,
        runtime_nodes: base_runtime_nodes,
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
        if combined.runtime_nodes > MAX_RUNTIME_NODES {
            return None;
        }
    }
    Some(combined)
}

fn checked_total(current: usize, additional: usize) -> Option<usize> {
    current
        .checked_add(additional)
        .filter(|total| *total <= MAX_RUNTIME_NODES)
}

fn validate_references(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    if let Some(duplicate) = bundle
        .rules
        .windows(2)
        .find(|pair| pair[0].id == pair[1].id)
        .map(|pair| &pair[0].id)
    {
        return Err(ImportFailure {
            message: format!("duplicate rule ID {duplicate}"),
        });
    }

    let rule_ids = bundle
        .rules
        .iter()
        .map(|rule| &rule.id)
        .collect::<BTreeSet<_>>();

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
            Self::NoOp | Self::Reference { .. } | Self::Terminal { .. } => {}
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
            Self::Market => "market",
            Self::VillainDeck => "villain_deck",
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
