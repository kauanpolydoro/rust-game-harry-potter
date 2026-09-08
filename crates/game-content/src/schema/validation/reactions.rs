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
        validate_copy_declaration(rule)?;
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
        Effect::PreventExtraDrawing
        | Effect::Reaction { .. }
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
        Effect::HeroAbility { effect, .. }
        | Effect::RevealTopCard { effect, .. }
        | Effect::ForEachTarget { effect, .. }
        | Effect::Repeat { effect, .. } => contains_reaction(effect),
        _ => false,
    }
}

fn declarations_are_valid(effect: &Effect, phase: EffectTrigger, allowed: bool) -> bool {
    match effect {
        Effect::PreventExtraDrawing => allowed && phase == EffectTrigger::Villains,
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
                            | ReactionTrigger::SelfHarmfulDiscard
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
        Effect::HeroAbility { effect, .. }
        | Effect::RevealTopCard { effect, .. }
        | Effect::ForEachTarget { effect, .. }
        | Effect::Repeat { effect, .. } => declarations_are_valid(effect, phase, false),
        _ => true,
    }
}

// Reactions may change hero resources, heal villains or draw one card.
// None adds control or discards cards; health loss causes at most one stun per hero per turn.
fn reaction_effect_is_bounded(effect: &Effect) -> bool {
    match effect {
        Effect::Apply {
            target,
            operation: Operation::ModifyResource { resource, amount },
        } => {
            (target.zone == crate::Zone::Heroes && *resource != Resource::Control)
                || (target.zone == crate::Zone::ActiveVillains
                    && *resource == Resource::Health
                    && *amount > 0)
        }
        Effect::Apply {
            target,
            operation: Operation::GainInfluenceAndHealth { .. } | Operation::Draw { amount: 1 },
        } => target.zone == crate::Zone::Heroes,
        Effect::Sequence { effects }
        | Effect::Choice {
            options: effects, ..
        } => effects.iter().all(reaction_effect_is_bounded),
        _ => false,
    }
}

fn validate_copy_declaration(rule: &crate::EffectRule) -> Result<(), ImportFailure> {
    fn copies(effect: &Effect) -> usize {
        match effect {
            Effect::Apply {
                operation: Operation::CopyPlayedAlly,
                ..
            } => 1,
            Effect::Sequence { effects }
            | Effect::Choice {
                options: effects, ..
            }
            | Effect::Roll {
                outcomes: effects, ..
            } => effects.iter().map(copies).sum(),
            Effect::Condition {
                then, otherwise, ..
            } => copies(then) + otherwise.as_deref().map_or(0, copies),
            Effect::Repeat { effect, .. }
            | Effect::HeroAbility { effect, .. }
            | Effect::RevealTopCard { effect, .. }
            | Effect::ForEachTarget { effect, .. }
            | Effect::Reaction { effect, .. } => copies(effect),
            _ => 0,
        }
    }
    if copies(&rule.effect) == 0 {
        return Ok(());
    }
    let valid = matches!(&rule.effect, Effect::Sequence { effects } if matches!(effects.as_slice(), [
        Effect::CardType { card_type: crate::CardType::Item },
        Effect::Apply { target, operation: Operation::CopyPlayedAlly },
    ] if target.zone == crate::Zone::HeroPlayArea
        && target.owner == crate::TargetOwner::Actor
        && target.cardinality == crate::Cardinality { min: 1, max: 1 }
        && target.eligibility == vec![crate::Eligibility::CardType { card_type: crate::CardType::Ally }]))
        && rule.trigger == EffectTrigger::Manual
        && rule.cost.is_empty();
    if valid {
        Ok(())
    } else {
        Err(ImportFailure {
            message: format!(
                "rule {} must copy exactly one owned played Ally from an Item",
                rule.id
            ),
        })
    }
}
