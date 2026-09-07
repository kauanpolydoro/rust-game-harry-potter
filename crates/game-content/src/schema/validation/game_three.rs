use std::collections::BTreeSet;

use crate::{
    Effect, EffectTrigger, EntryKind, FunctionalField, HeroAbilityStrategy, ImportFailure,
};

use super::CandidateBundle;

pub(super) fn validate(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    let abilities = bundle
        .rules
        .iter()
        .filter(|rule| matches!(rule.effect, Effect::HeroAbility { .. }))
        .map(|rule| &rule.id)
        .collect::<BTreeSet<_>>();
    for rule in &bundle.rules {
        if abilities.contains(&rule.id)
            && (rule.trigger != EffectTrigger::Manual || !rule.cost.is_empty())
            || rule
                .effect
                .references()
                .iter()
                .any(|id| abilities.contains(id))
        {
            return Err(ImportFailure {
                message: format!(
                    "hero ability {} must be a standalone passive declaration",
                    rule.id
                ),
            });
        }
    }
    for entry in &bundle.entries {
        for (field, definition) in &entry.functional {
            let Some(rule) = definition
                .rule
                .as_ref()
                .and_then(|id| bundle.rules.iter().find(|rule| rule.id == *id))
            else {
                continue;
            };
            if bundle.schema_version >= 5
                && entry.kind == EntryKind::Hero
                && *field == FunctionalField::Ability
                && !matches!(rule.effect, Effect::HeroAbility { .. })
            {
                return Err(ImportFailure {
                    message: format!("replacement hero {} requires a versioned ability", entry.id),
                });
            }
            if let Effect::HeroAbility { strategy, .. } = rule.effect {
                let expected_id = match strategy {
                    HeroAbilityStrategy::HarryGameThreeV1 => "hero:002",
                    HeroAbilityStrategy::HermioneGameThreeV1 => "hero:005",
                    HeroAbilityStrategy::NevilleGameThreeV1 => "hero:008",
                    HeroAbilityStrategy::RonGameThreeV1 => "hero:011",
                };
                if entry.kind != EntryKind::Hero
                    || *field != FunctionalField::Ability
                    || entry.id.as_str() != expected_id
                {
                    return Err(ImportFailure {
                        message: format!(
                            "entry {} cannot use the hero strategy for {expected_id}",
                            entry.id
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}
