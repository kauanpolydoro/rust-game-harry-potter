use crate::{
    EffectDie, EffectOutcome, EndTurnOutcome, GAME_FOUR_SNAPSHOT_VERSION, GameEvent,
    GameStateRestoreInput, InitialGameState,
};

/// One immutable House die result. Counter identifies its `ChaCha` stream;
/// the result is in the inclusive interval 1..=sides and purpose is the rule ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HouseDieRoll {
    pub purpose: String,
    pub counter: u64,
    pub die: EffectDie,
    pub sides: u8,
    pub result: u8,
}

impl HouseDieRoll {
    /// Derives audit records from ordered outcomes and their final stream counter.
    /// Returns `None` for inconsistent counters or out-of-range House results.
    #[must_use]
    pub fn from_effects<'a>(
        effects: impl IntoIterator<Item = &'a EffectOutcome>,
        ending_counter: u64,
    ) -> Option<Vec<Self>> {
        let effects = effects.into_iter().collect::<Vec<_>>();
        let consumed = effects
            .iter()
            .filter(|effect| effect.consumes_random_sample())
            .count();
        let start = ending_counter.checked_sub(u64::try_from(consumed).ok()?)?;
        let mut history = Vec::new();
        append_rolls(&mut history, start, effects.into_iter())?;
        Some(history)
    }
}

fn append_rolls<'a>(
    history: &mut Vec<HouseDieRoll>,
    mut counter: u64,
    effects: impl Iterator<Item = &'a EffectOutcome>,
) -> Option<u64> {
    for effect in effects {
        if let EffectOutcome::DieRolled {
            rule_id,
            die,
            result,
        } = effect
            && die.is_house()
        {
            if *result == 0 || *result > die.sides() {
                return None;
            }
            history.push(HouseDieRoll {
                purpose: rule_id.clone(),
                counter,
                die: *die,
                sides: die.sides(),
                result: *result,
            });
        }
        if effect.consumes_random_sample() {
            counter = counter.checked_add(1)?;
        }
    }
    Some(counter)
}

impl GameEvent {
    /// Returns the House die audit records carried by this event's persisted facts.
    /// Returns `None` if its counter or a result is inconsistent.
    #[must_use]
    pub fn house_die_rolls(&self) -> Option<Vec<HouseDieRoll>> {
        let (effects, counter): (Vec<_>, u64) = match self {
            Self::ChoiceResolved {
                steps,
                prng_counter,
                ..
            }
            | Self::TurnCompleted {
                steps,
                prng_counter,
                ..
            } => (
                steps.iter().flat_map(|step| &step.effects).collect(),
                *prng_counter,
            ),
            Self::DarkArtsCompleted {
                effects,
                prng_counter,
                ..
            }
            | Self::CardPlayed {
                effects,
                prng_counter,
                ..
            }
            | Self::AttackAssigned {
                effects,
                prng_counter,
                ..
            } => (effects.iter().collect(), *prng_counter),
            Self::CardAcquired { effects, .. } => (effects.iter().collect(), 0),
        };
        HouseDieRoll::from_effects(effects, counter)
    }
}

pub(crate) fn record_event(
    previous: &InitialGameState,
    next: &mut InitialGameState,
    event: &GameEvent,
) -> Option<()> {
    if next.snapshot_version != GAME_FOUR_SNAPSHOT_VERSION {
        return Some(());
    }
    let mut minimum_counter = previous.prng_counter;
    if let GameEvent::TurnCompleted { end_turn, .. } = event {
        for outcome in end_turn {
            if let EndTurnOutcome::PileShuffled { samples, .. } = outcome {
                minimum_counter =
                    minimum_counter.checked_add(u64::try_from(samples.len()).ok()?)?;
            }
        }
    }
    let rolls = event.house_die_rolls()?;
    if rolls
        .iter()
        .any(|roll| roll.counter < minimum_counter || roll.counter >= next.prng_counter)
    {
        return None;
    }
    next.house_die_rolls.clone_from(&previous.house_die_rolls);
    next.house_die_rolls.extend(rolls);
    Some(())
}

pub(crate) fn record_initial(state: &mut InitialGameState, starting_counter: u64) -> Option<()> {
    if state.snapshot_version != GAME_FOUR_SNAPSHOT_VERSION {
        return Some(());
    }
    let counter = append_rolls(
        &mut state.house_die_rolls,
        starting_counter,
        state.last_effects.iter(),
    )?;
    (counter == state.prng_counter).then_some(())
}

pub(crate) fn valid_history(input: &GameStateRestoreInput<'_>) -> bool {
    if input.snapshot_version != GAME_FOUR_SNAPSHOT_VERSION {
        return input.house_die_rolls.is_empty();
    }
    let valid = input
        .house_die_rolls
        .windows(2)
        .all(|pair| pair[0].counter < pair[1].counter)
        && input.house_die_rolls.iter().all(|roll| {
            roll.die.is_house()
                && roll.sides == roll.die.sides()
                && (1..=roll.sides).contains(&roll.result)
                && !roll.purpose.is_empty()
                && roll.purpose.len() <= 256
                && roll.counter >= input.preparation_samples.len() as u64
                && roll.counter < input.prng_counter
        });
    if !valid {
        return false;
    }
    let Some(recent) = HouseDieRoll::from_effects(&input.last_effects, input.prng_counter) else {
        return false;
    };
    input.house_die_rolls.ends_with(&recent)
        && (input.sequence != 0 || input.house_die_rolls == recent)
}
