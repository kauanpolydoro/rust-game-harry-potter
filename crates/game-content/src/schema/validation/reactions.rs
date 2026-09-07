use std::collections::BTreeSet;

use crate::{Effect, EffectTrigger, ImportFailure, Operation, ReactionTrigger, Resource};

use super::CandidateBundle;

pub(super) fn validate_reactive_definitions(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    let reactive_rules = bundle
        .rules
        .iter()
        .filter(|rule| contains_reaction(&rule.effect))
        .map(|rule| &rule.id)
        .collect::<BTreeSet<_>>();
    for rule in &bundle.rules {
        let declarations_valid = declarations_are_valid(&rule.effect, rule.trigger, true);
        let references_valid = !rule
            .effect
            .references()
            .iter()
            .any(|id| reactive_rules.contains(id));
        if !declarations_valid
            || !references_valid
            || category_count(&rule.effect) > 1
            || (reactive_rules.contains(&rule.id) && !rule.cost.is_empty())
        {
            return Err(ImportFailure {
                message: format!("rule {} has an invalid reactive declaration", rule.id),
            });
        }
        if category_count(&rule.effect) == 1
            && bundle.entries.iter().any(|entry| {
                entry.functional.iter().any(|(field, definition)| {
                    definition.rule.as_ref() == Some(&rule.id)
                        && (*field != crate::FunctionalField::Effect
                            || !matches!(
                                entry.kind,
                                crate::EntryKind::StarterCard | crate::EntryKind::HogwartsCard
                            ))
                })
            })
        {
            return Err(ImportFailure {
                message: format!(
                    "rule {} attaches a card category to an incompatible field",
                    rule.id
                ),
            });
        }
    }
    Ok(())
}

fn category_count(effect: &Effect) -> usize {
    match effect {
        Effect::CardType { .. } => 1,
        Effect::Sequence { effects } => effects.iter().map(category_count).sum(),
        _ => 0,
    }
}

fn contains_reaction(effect: &Effect) -> bool {
    match effect {
        Effect::Reaction { .. }
        | Effect::HandDamageLimit { .. }
        | Effect::CardType { .. }
        | Effect::TopDeckAcquisition { .. } => true,
        Effect::Sequence { effects }
        | Effect::Choice {
            options: effects, ..
        }
        | Effect::Roll {
            outcomes: effects, ..
        } => effects.iter().any(contains_reaction),
        Effect::Condition {
            then, otherwise, ..
        } => contains_reaction(then) || otherwise.as_deref().is_some_and(contains_reaction),
        Effect::Repeat { effect, .. } => contains_reaction(effect),
        _ => false,
    }
}

fn declarations_are_valid(effect: &Effect, phase: EffectTrigger, allowed: bool) -> bool {
    match effect {
        Effect::CardType { .. } | Effect::TopDeckAcquisition { .. } => {
            allowed && phase == EffectTrigger::Manual
        }
        Effect::HandDamageLimit { maximum } => {
            allowed && phase == EffectTrigger::Manual && (1..=10).contains(maximum)
        }
        Effect::Reaction { trigger, effect } => {
            allowed
                && matches!(
                    (phase, trigger),
                    (
                        EffectTrigger::Villains,
                        ReactionTrigger::ControlAdded | ReactionTrigger::HeroForcedDiscard
                    ) | (
                        EffectTrigger::Manual,
                        ReactionTrigger::SelfForcedDiscard
                            | ReactionTrigger::OwnerDefeatsVillain
                            | ReactionTrigger::OwnerPlaysAlly
                    )
                )
                && reaction_effect_is_bounded(effect)
        }
        Effect::Sequence { effects } => effects
            .iter()
            .all(|effect| declarations_are_valid(effect, phase, allowed)),
        Effect::Choice { options, .. }
        | Effect::Roll {
            outcomes: options, ..
        } => options
            .iter()
            .all(|effect| declarations_are_valid(effect, phase, false)),
        Effect::Condition {
            then, otherwise, ..
        } => {
            declarations_are_valid(then, phase, false)
                && otherwise
                    .as_deref()
                    .is_none_or(|effect| declarations_are_valid(effect, phase, false))
        }
        Effect::Repeat { effect, .. } => declarations_are_valid(effect, phase, false),
        _ => true,
    }
}

// The initial reaction vocabulary only changes hero resources.
// Health loss can cause at most one stun per hero per turn, which bounds cascades.
fn reaction_effect_is_bounded(effect: &Effect) -> bool {
    match effect {
        Effect::Apply {
            target,
            operation: Operation::ModifyResource { resource, .. },
        } => target.zone == crate::Zone::Heroes && *resource != Resource::Control,
        Effect::Sequence { effects }
        | Effect::Choice {
            options: effects, ..
        } => effects.iter().all(reaction_effect_is_bounded),
        _ => false,
    }
}
