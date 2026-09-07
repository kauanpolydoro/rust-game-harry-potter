use crate::{Effect, GameSetupOwner, ImportFailure, Operation, Zone};

use super::CandidateBundle;

pub(super) fn validate(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    if bundle.schema_version < 5
        && bundle
            .rules
            .iter()
            .any(|rule| requires_game_three(&rule.effect))
    {
        return Err(ImportFailure {
            message: "Game 3 definitions require bundle schema version 5".to_owned(),
        });
    }
    if bundle.schema_version < 4
        && bundle
            .rules
            .iter()
            .any(|rule| requires_game_two(&rule.effect))
    {
        return Err(ImportFailure {
            message: "Game 2 definitions require bundle schema version 4".to_owned(),
        });
    }
    if bundle.schema_version != 2 {
        return Ok(());
    }
    let new_rules = bundle
        .rules
        .iter()
        .any(|rule| rule.provenance.is_some() || requires_game_one(&rule.effect));
    let new_setup = bundle
        .game_setups
        .iter()
        .flat_map(|setup| &setup.entities)
        .any(|entity| {
            !matches!(
                entity.owner,
                GameSetupOwner::None | GameSetupOwner::EachParticipant
            ) || matches!(entity.zone, Zone::DarkArtsDeck | Zone::DarkArtsDiscard)
        });
    if new_rules || new_setup {
        Err(ImportFailure {
            message: "Game 1 definitions require bundle schema version 3".to_owned(),
        })
    } else {
        Ok(())
    }
}

fn requires_game_one(effect: &Effect) -> bool {
    match effect {
        Effect::HeroAbility { .. }
        | Effect::RevealTopCard { .. }
        | Effect::LimitVillainAttack { .. }
        | Effect::PreventExtraDrawing
        | Effect::ForEachTarget { .. }
        | Effect::TopDeckAcquisition { .. }
        | Effect::CardType { .. }
        | Effect::HandDamageLimit { .. }
        | Effect::Reaction { .. }
        | Effect::RevealDarkArts
        | Effect::Structural { .. } => true,
        Effect::Apply { operation, .. } => matches!(
            operation,
            Operation::Draw { .. }
                | Operation::PreventDrawing
                | Operation::GainAttackPerAllyPlayed { .. }
        ),
        Effect::Choice { options, .. }
        | Effect::Roll {
            outcomes: options, ..
        }
        | Effect::Sequence { effects: options } => options.iter().any(requires_game_one),
        Effect::Condition {
            then, otherwise, ..
        } => requires_game_one(then) || otherwise.as_deref().is_some_and(requires_game_one),
        Effect::Repeat { effect, .. } => requires_game_one(effect),
        Effect::NoOp | Effect::Reference { .. } | Effect::Terminal { .. } => false,
    }
}

fn requires_game_two(effect: &Effect) -> bool {
    let category = |target: &crate::Selector| {
        target
            .eligibility
            .iter()
            .any(|entry| matches!(entry, crate::Eligibility::CardType { .. }))
    };
    match effect {
        Effect::HeroAbility { .. }
        | Effect::RevealTopCard { .. }
        | Effect::LimitVillainAttack { .. }
        | Effect::PreventExtraDrawing
        | Effect::ForEachTarget { .. }
        | Effect::Structural {
            rule: crate::StructuralRule::GameTwoSetup,
        } => true,
        Effect::Apply { target, operation } => {
            category(target)
                || matches!(
                    operation,
                    Operation::CopyPlayedAlly | Operation::DiscardVoluntarily
                )
        }
        Effect::Choice { options, .. }
        | Effect::Roll {
            outcomes: options, ..
        }
        | Effect::Sequence { effects: options } => options.iter().any(requires_game_two),
        Effect::Condition {
            condition,
            then,
            otherwise,
        } => {
            let target = match condition {
                crate::Condition::DrawingAllowed => return true,
                crate::Condition::HasEligibleTarget { target }
                | crate::Condition::ResourceAtLeast { target, .. } => target,
            };
            category(target)
                || requires_game_two(then)
                || otherwise.as_deref().is_some_and(requires_game_two)
        }
        Effect::Repeat { effect, .. } => requires_game_two(effect),
        Effect::Reaction { effect, .. } => new_reaction_effect(effect) || requires_game_two(effect),
        _ => false,
    }
}

fn new_reaction_effect(effect: &Effect) -> bool {
    match effect {
        Effect::Apply {
            operation: Operation::Draw { .. },
            ..
        } => true,
        Effect::Apply {
            target,
            operation:
                Operation::ModifyResource {
                    resource: crate::Resource::Health,
                    amount,
                },
        } => target.zone == Zone::ActiveVillains && *amount > 0,
        Effect::Sequence { effects }
        | Effect::Choice {
            options: effects, ..
        } => effects.iter().any(new_reaction_effect),
        _ => false,
    }
}

fn requires_game_three(effect: &Effect) -> bool {
    let other = |target: &crate::Selector| target.owner == crate::TargetOwner::Other;
    match effect {
        Effect::HeroAbility { .. }
        | Effect::RevealTopCard { .. }
        | Effect::LimitVillainAttack { .. }
        | Effect::Structural {
            rule: crate::StructuralRule::GameThreeSetup,
        } => true,
        Effect::Apply { target, operation } => {
            other(target)
                || matches!(
                    operation,
                    Operation::SuppressVillain
                        | Operation::GainInfluenceAndHealth { .. }
                        | Operation::DiscardForSpellBonus { .. }
                )
        }
        Effect::ForEachTarget { target, effect } => other(target) || requires_game_three(effect),
        Effect::Reaction { trigger, effect } => {
            *trigger == crate::ReactionTrigger::SelfHarmfulDiscard || requires_game_three(effect)
        }
        Effect::Choice { options, .. }
        | Effect::Roll {
            outcomes: options, ..
        }
        | Effect::Sequence { effects: options } => options.iter().any(requires_game_three),
        Effect::Condition {
            condition,
            then,
            otherwise,
        } => {
            let new_condition = match condition {
                crate::Condition::HasEligibleTarget { target }
                | crate::Condition::ResourceAtLeast { target, .. } => other(target),
                crate::Condition::DrawingAllowed => false,
            };
            new_condition
                || requires_game_three(then)
                || otherwise.as_deref().is_some_and(requires_game_three)
        }
        Effect::Repeat { effect, .. } => requires_game_three(effect),
        _ => false,
    }
}
