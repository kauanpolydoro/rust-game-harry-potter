use crate::{EffectExecutionError, EffectRoller, GameIntentError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShuffleSample {
    pub upper_exclusive: u32,
    pub result: u32,
}

pub(crate) fn shuffle(
    cards: &mut [String],
    random: &mut dyn EffectRoller,
) -> Result<Vec<ShuffleSample>, GameIntentError> {
    fisher_yates(cards.len(), random, |index, selected| {
        cards.swap(index, selected);
    })
    .map_err(|error| match error {
        EffectExecutionError::InvalidRoll => GameIntentError::RandomSourceFailed,
        _ => GameIntentError::EffectExecutionFailed,
    })
}

pub(crate) fn fisher_yates(
    length: usize,
    random: &mut dyn EffectRoller,
    mut swap: impl FnMut(usize, usize),
) -> Result<Vec<ShuffleSample>, EffectExecutionError> {
    let mut samples = Vec::new();
    for index in (1..length).rev() {
        let upper_exclusive =
            u32::try_from(index + 1).map_err(|_| EffectExecutionError::InvalidDefinition)?;
        let result = random
            .sample_below(upper_exclusive)
            .filter(|result| *result < upper_exclusive)
            .ok_or(EffectExecutionError::InvalidRoll)?;
        swap(
            index,
            usize::try_from(result).map_err(|_| EffectExecutionError::InvalidRoll)?,
        );
        samples.push(ShuffleSample {
            upper_exclusive,
            result,
        });
    }
    Ok(samples)
}

pub(crate) fn matches(original: &[String], shuffled: &[String], samples: &[ShuffleSample]) -> bool {
    if shuffled.is_empty() || shuffled.len() != original.len() {
        return false;
    }
    let mut expected = original.to_vec();
    // Events predating codec v6 recorded only the committed permutation.
    if samples.is_empty() {
        let mut actual = shuffled.to_vec();
        expected.sort();
        actual.sort();
        return expected == actual;
    }
    if samples.len() != expected.len().saturating_sub(1) {
        return false;
    }
    for (sample, index) in samples.iter().zip((1..expected.len()).rev()) {
        if usize::try_from(sample.upper_exclusive).ok() != Some(index + 1)
            || sample.result >= sample.upper_exclusive
        {
            return false;
        }
        let Ok(selected) = usize::try_from(sample.result) else {
            return false;
        };
        expected.swap(index, selected);
    }
    expected == shuffled
}
