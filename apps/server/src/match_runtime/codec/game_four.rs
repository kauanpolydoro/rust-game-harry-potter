use super::{ApiError, domain_effect_die, effect_die_name};
use crate::match_runtime::PersistedHouseDieRoll;

pub(super) fn persisted_roll(roll: &game_domain::HouseDieRoll) -> PersistedHouseDieRoll {
    PersistedHouseDieRoll {
        purpose: roll.purpose.clone(),
        counter: roll.counter,
        die: effect_die_name(roll.die).to_owned(),
        sides: roll.sides,
        result: roll.result,
    }
}

pub(super) fn domain_roll(
    roll: &PersistedHouseDieRoll,
) -> Result<game_domain::HouseDieRoll, ApiError> {
    let die = domain_effect_die(&roll.die)?;
    if !die.is_house() || roll.sides != die.sides() || !(1..=roll.sides).contains(&roll.result) {
        return Err(ApiError::internal());
    }
    Ok(game_domain::HouseDieRoll {
        purpose: roll.purpose.clone(),
        counter: roll.counter,
        die,
        sides: roll.sides,
        result: roll.result,
    })
}

pub(super) fn validate_event_rolls(
    event: &crate::match_runtime::PersistedGameEvent,
) -> Result<(), ApiError> {
    if event.event_version != super::GAME_FOUR_EVENT_VERSION {
        return if event.house_die_rolls.is_none() {
            Ok(())
        } else {
            Err(ApiError::internal())
        };
    }
    let effects = event
        .effects
        .iter()
        .chain(event.steps.iter().flatten().flat_map(|step| &step.effects))
        .map(super::domain_effect_outcome)
        .collect::<Result<Vec<_>, _>>()?;
    let expected =
        game_domain::HouseDieRoll::from_effects(&effects, event.prng_counter.unwrap_or(0))
            .ok_or_else(ApiError::internal)?
            .iter()
            .map(persisted_roll)
            .collect::<Vec<_>>();
    if event.house_die_rolls.as_ref() != Some(&expected) {
        return Err(ApiError::internal());
    }
    Ok(())
}
