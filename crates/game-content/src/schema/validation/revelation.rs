use std::collections::BTreeSet;

use crate::{Effect, EffectTrigger, ImportFailure};

use super::CandidateBundle;

pub(super) fn validate(bundle: &CandidateBundle) -> Result<(), ImportFailure> {
    let roots = bundle
        .rules
        .iter()
        .filter(|rule| contains_revelation(&rule.effect))
        .map(|rule| &rule.id)
        .collect::<BTreeSet<_>>();
    for rule in &bundle.rules {
        if (roots.contains(&rule.id)
            && (rule.effect != Effect::RevealDarkArts
                || rule.trigger != EffectTrigger::DarkArts
                || !rule.cost.is_empty()))
            || rule.effect.references().iter().any(|id| roots.contains(id))
        {
            return Err(ImportFailure {
                message: format!(
                    "rule {} must not nest or reference a Dark Arts revelation root",
                    rule.id
                ),
            });
        }
    }
    if bundle.entries.iter().any(|entry| {
        entry
            .functional
            .values()
            .any(|field| field.rule.as_ref().is_some_and(|id| roots.contains(id)))
    }) {
        return Err(ImportFailure {
            message: "Dark Arts revelation is a phase root, not an entry effect".to_owned(),
        });
    }
    Ok(())
}

fn contains_revelation(effect: &Effect) -> bool {
    match effect {
        Effect::RevealDarkArts => true,
        Effect::Sequence { effects }
        | Effect::Choice {
            options: effects, ..
        }
        | Effect::Roll {
            outcomes: effects, ..
        } => effects.iter().any(contains_revelation),
        Effect::Condition {
            then, otherwise, ..
        } => contains_revelation(then) || otherwise.as_deref().is_some_and(contains_revelation),
        Effect::HeroAbility { effect, .. }
        | Effect::RevealTopCard { effect, .. }
        | Effect::ForEachTarget { effect, .. }
        | Effect::Repeat { effect, .. }
        | Effect::Reaction { effect, .. } => contains_revelation(effect),
        _ => false,
    }
}
