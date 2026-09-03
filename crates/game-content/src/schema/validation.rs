use std::collections::{BTreeMap, BTreeSet};

use crate::{ContentSet, ImportFailure, RuleId};

use super::{CandidateBundle, Effect, Operation, Zone};

pub(super) fn validate(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    validate_metadata(bundle)?;
    validate_inventory(bundle)?;
    validate_provenance(bundle)?;
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

    Ok(())
}

fn validate_effects(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    for rule in &bundle.rules {
        rule.effect.validate(&rule.id)?;
    }
    Ok(())
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
    fn references(&self) -> Vec<&RuleId> {
        match self {
            Self::Choice { options } => options.iter().flat_map(Self::references).collect(),
            Self::Apply { .. } | Self::NoOp => Vec::new(),
            Self::Reference { rule } => vec![rule],
        }
    }

    fn validate(&self, rule_id: &RuleId) -> Result<(), ImportFailure> {
        match self {
            Self::Apply { target, operation } => {
                if target.cardinality.min > target.cardinality.max {
                    return Err(ImportFailure {
                        message: format!(
                            "rule {rule_id} cardinality min {} exceeds max {}",
                            target.cardinality.min, target.cardinality.max
                        ),
                    });
                }
                operation.validate_zone(&target.zone, rule_id)?;
            }
            Self::Choice { options } => {
                if options.len() < 2 {
                    return Err(ImportFailure {
                        message: format!(
                            "rule {rule_id} choice must have at least two conclusions"
                        ),
                    });
                }
                for option in options {
                    option.validate(rule_id)?;
                }
            }
            Self::NoOp | Self::Reference { .. } => {}
        }
        Ok(())
    }
}

impl Operation {
    fn validate_zone(&self, zone: &Zone, rule_id: &RuleId) -> Result<(), ImportFailure> {
        let compatible = matches!((self, zone), (Self::Discard, Zone::HeroHand));
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
        }
    }
}

impl Zone {
    fn as_str(&self) -> &'static str {
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
