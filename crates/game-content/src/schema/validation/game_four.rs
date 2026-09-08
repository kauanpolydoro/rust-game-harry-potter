use std::collections::BTreeSet;

use crate::{
    Effect, EffectChoiceAudience, EffectTrigger, EntryKind, FunctionalField, ImportFailure,
};

use super::CandidateBundle;

pub(super) fn extra_revelations(effect: &Effect) -> usize {
    match effect {
        Effect::RevealExtraDarkArts => 1,
        Effect::Sequence { effects }
        | Effect::Roll {
            outcomes: effects, ..
        } => effects
            .iter()
            .map(extra_revelations)
            .fold(0, usize::saturating_add),
        Effect::Choice { audience, options } => options
            .iter()
            .map(extra_revelations)
            .fold(0, usize::saturating_add)
            .saturating_mul(if *audience == EffectChoiceAudience::EachHero {
                4
            } else {
                1
            }),
        Effect::Condition {
            then, otherwise, ..
        } => extra_revelations(then)
            .saturating_add(otherwise.as_deref().map_or(0, extra_revelations)),
        Effect::Repeat { times, effect } => {
            usize::from(*times).saturating_mul(extra_revelations(effect))
        }
        Effect::ForEachTarget { target, effect } => {
            usize::from(target.cardinality.max).saturating_mul(extra_revelations(effect))
        }
        Effect::HeroAbility { effect, .. }
        | Effect::RevealTopCard { effect, .. }
        | Effect::Reaction { effect, .. } => extra_revelations(effect),
        _ => 0,
    }
}

pub(super) fn validate(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    validate_ally_bonus(bundle)?;
    let roots = bundle
        .rules
        .iter()
        .filter(|rule| extra_revelations(&rule.effect) > 0)
        .map(|rule| &rule.id)
        .collect::<BTreeSet<_>>();
    if roots.is_empty() {
        return Ok(());
    }
    let mut copies = 0;
    for rule in &bundle.rules {
        if rule.effect.references().iter().any(|id| roots.contains(id)) {
            return Err(ImportFailure {
                message: format!(
                    "extra Dark Arts revelation {} cannot be referenced",
                    rule.id
                ),
            });
        }
        if !roots.contains(&rule.id) {
            continue;
        }
        let entries = bundle
            .entries
            .iter()
            .filter(|entry| {
                entry
                    .functional
                    .values()
                    .any(|definition| definition.rule.as_ref() == Some(&rule.id))
            })
            .collect::<Vec<_>>();
        if rule.trigger != EffectTrigger::Manual
            || !rule.cost.is_empty()
            || extra_revelations(&rule.effect) != 1
            || entries.is_empty()
            || entries
                .iter()
                .any(|entry| entry.kind != EntryKind::DarkArts)
        {
            return Err(ImportFailure {
                message: format!(
                    "rule {} may reveal one extra card only as a Dark Arts effect",
                    rule.id
                ),
            });
        }
        copies += entries
            .iter()
            .map(|entry| usize::from(entry.copies))
            .sum::<usize>();
    }
    let base_reveals = bundle
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Location)
        .filter_map(|entry| entry.functional.get(&FunctionalField::DarkArtsCount)?.value)
        .max()
        .unwrap_or(1);
    let deck = bundle
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::DarkArts)
        .map(|entry| usize::from(entry.copies))
        .sum::<usize>();
    // Even a tail of chaining cards followed by a reshuffle must finish before
    // exhausting that second deck. This proves a finite bound without rerolling.
    if copies * 2 + usize::from(base_reveals) > deck {
        return Err(ImportFailure {
            message: "extra Dark Arts chain has no finite inventory bound".to_owned(),
        });
    }
    Ok(())
}

fn validate_ally_bonus(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    fn count(effect: &Effect) -> usize {
        match effect {
            Effect::OtherAllyBonus { .. } => 1,
            Effect::Sequence { effects } => effects.iter().map(count).sum(),
            // Reactive validation rejects declarations inside other containers.
            _ => 0,
        }
    }
    for rule in &bundle.rules {
        let bonuses = count(&rule.effect);
        if bonuses == 0 {
            continue;
        }
        let owners = bundle
            .entries
            .iter()
            .flat_map(|entry| {
                entry
                    .functional
                    .iter()
                    .filter_map(move |(field, definition)| {
                        (definition.rule.as_ref() == Some(&rule.id)).then_some((entry, field))
                    })
            })
            .collect::<Vec<_>>();
        if bonuses != 1
            || !matches!(owners.as_slice(), [(entry, FunctionalField::Effect)]
            if entry.kind == EntryKind::HogwartsCard && entry.id.as_str() == "hogwarts-card:035")
        {
            return Err(ImportFailure {
                message: format!(
                    "rule {} requires Fleur's persistent Ally bonus state",
                    rule.id
                ),
            });
        }
    }
    Ok(())
}
