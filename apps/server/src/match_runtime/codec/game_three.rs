use super::{
    EffectOutcome, PersistedEffectOutcome, PersistedEffectTurnState, valid_identifier_for_version,
    valid_position,
};
use crate::http_support::ApiError;

pub(super) fn valid_turn_state(state: &PersistedEffectTurnState) -> bool {
    [&state.healed_positions, &state.suppressed_by]
        .iter()
        .all(|positions| {
            positions.len() <= 4
                && positions.iter().all(|position| valid_position(*position))
                && positions.windows(2).all(|pair| pair[0] < pair[1])
        })
        && state
            .attack_limit
            .is_none_or(|limit| (1..=16).contains(&limit))
}

impl From<&PersistedEffectTurnState> for game_domain::EffectTurnState {
    fn from(state: &PersistedEffectTurnState) -> Self {
        Self {
            ability_used: state.ability_used,
            healed_positions: state.healed_positions.clone(),
            attack_assigned: state.attack_assigned,
            attack_limit: state.attack_limit,
            suppressed_by: state.suppressed_by.clone(),
        }
    }
}

impl From<&game_domain::EffectTurnState> for PersistedEffectTurnState {
    fn from(state: &game_domain::EffectTurnState) -> Self {
        Self {
            ability_used: state.ability_used,
            healed_positions: state.healed_positions.clone(),
            attack_assigned: state.attack_assigned,
            attack_limit: state.attack_limit,
            suppressed_by: state.suppressed_by.clone(),
        }
    }
}

pub(super) fn valid_outcome(outcome: &PersistedEffectOutcome, bounded_identifiers: bool) -> bool {
    match outcome {
        PersistedEffectOutcome::TurnStateChanged {
            rule_id,
            target_id,
            before,
            after,
        } => {
            valid_identifier_for_version(rule_id, bounded_identifiers)
                && valid_identifier_for_version(target_id, bounded_identifiers)
                && before != after
                && valid_turn_state(before)
                && valid_turn_state(after)
        }
        PersistedEffectOutcome::TopCardRevealed {
            rule_id,
            card_id,
            owner_position,
        } => {
            valid_identifier_for_version(rule_id, bounded_identifiers)
                && valid_identifier_for_version(card_id, bounded_identifiers)
                && valid_position(*owner_position)
        }
        _ => false,
    }
}

pub(super) fn domain_outcome(outcome: &PersistedEffectOutcome) -> Result<EffectOutcome, ApiError> {
    Ok(match outcome {
        PersistedEffectOutcome::TurnStateChanged {
            rule_id,
            target_id,
            before,
            after,
        } => EffectOutcome::TurnStateChanged {
            rule_id: rule_id.clone(),
            target_id: target_id.clone(),
            before: before.into(),
            after: after.into(),
        },
        PersistedEffectOutcome::TopCardRevealed {
            rule_id,
            card_id,
            owner_position,
        } => EffectOutcome::TopCardRevealed {
            rule_id: rule_id.clone(),
            card_id: card_id.clone(),
            owner_position: *owner_position,
        },
        _ => return Err(ApiError::internal()),
    })
}

pub(super) fn persisted_outcome(outcome: &EffectOutcome) -> PersistedEffectOutcome {
    match outcome {
        EffectOutcome::TurnStateChanged {
            rule_id,
            target_id,
            before,
            after,
        } => PersistedEffectOutcome::TurnStateChanged {
            rule_id: rule_id.clone(),
            target_id: target_id.clone(),
            before: before.into(),
            after: after.into(),
        },
        EffectOutcome::TopCardRevealed {
            rule_id,
            card_id,
            owner_position,
        } => PersistedEffectOutcome::TopCardRevealed {
            rule_id: rule_id.clone(),
            card_id: card_id.clone(),
            owner_position: *owner_position,
        },
        _ => unreachable!("only Game 3 outcomes are delegated"),
    }
}
