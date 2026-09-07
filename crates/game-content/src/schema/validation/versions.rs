use crate::{Effect, GameSetupOwner, ImportFailure, Operation, Zone};

use super::CandidateBundle;

pub(super) fn validate(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
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
        Effect::TopDeckAcquisition { .. }
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
