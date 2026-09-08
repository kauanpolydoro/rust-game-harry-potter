use super::{
    EffectCardType, EffectCursor, EffectExecutionError, EffectExecutor, EffectOutcome,
    EffectPathSegment, EffectSelector, EffectSource, EffectStop, EffectWorld, EffectZone,
    QueuedEffect, eligible_entity_ids, reactions, selector_is_valid,
};

pub(super) fn record_copy(
    world: &mut EffectWorld,
    rule_id: &str,
    card_id: &str,
    ally_id: &str,
    owner: u8,
) -> Result<(), EffectExecutionError> {
    let valid_ally = world.entity(ally_id).is_some_and(|(zone, ally)| {
        zone == EffectZone::HeroPlayArea
            && ally.owner_position == Some(owner)
            && ally.effect_rule_id.is_some()
            && ally.copied_ally_id.is_none()
            && card_id != ally_id
    });
    let (_, card) = world
        .entity_mut(card_id)
        .filter(|(zone, card)| {
            valid_ally
                && *zone == EffectZone::HeroPlayArea
                && card.owner_position == Some(owner)
                && card.effect_rule_id.as_deref() == Some(rule_id)
                && card.copied_ally_id.is_none()
        })
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    card.copied_ally_id = Some(ally_id.to_owned());
    Ok(())
}

pub(super) fn copy_played_ally(
    world: &mut EffectWorld,
    ally_id: &str,
    source: EffectSource<'_>,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<Option<u8>, EffectExecutionError> {
    let (_, ally) = world
        .entity(ally_id)
        .filter(|(zone, ally)| {
            *zone == EffectZone::HeroPlayArea
                && reactions::card_type(ally, source.rules) == Some(EffectCardType::Ally)
        })
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    let owner = ally
        .owner_position
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    let card_id = world
        .entities_in(EffectZone::HeroPlayArea)
        .iter()
        .rev()
        .find(|card| {
            card.owner_position == Some(owner)
                && card.effect_rule_id.as_deref() == Some(source.rule_id)
                && card.copied_ally_id.is_none()
        })
        .ok_or(EffectExecutionError::InvalidDefinition)?
        .id
        .clone();
    record_copy(world, source.rule_id, &card_id, ally_id, owner)?;
    outcomes.push(EffectOutcome::AllyCopied {
        rule_id: source.rule_id.to_owned(),
        card_id,
        ally_id: ally_id.to_owned(),
        owner_position: owner,
    });
    Ok(None)
}

impl EffectExecutor<'_> {
    pub(super) fn enqueue_copied_effects(
        &mut self,
        start: usize,
    ) -> Result<(), EffectExecutionError> {
        for outcome in self.outcomes[start..].iter().rev() {
            if let EffectOutcome::AllyCopied {
                ally_id,
                owner_position,
                ..
            } = outcome
            {
                let rule_id = self
                    .world
                    .entity(ally_id)
                    .and_then(|(_, ally)| ally.effect_rule_id())
                    .ok_or(EffectExecutionError::InvalidDefinition)?;
                self.queue.push_front(QueuedEffect::Definition {
                    cursor: EffectCursor::root(rule_id),
                    actor_position: *owner_position,
                });
            }
        }
        Ok(())
    }
}

impl EffectExecutor<'_> {
    pub(super) fn execute_for_each_target(
        &mut self,
        cursor: &EffectCursor,
        actor_position: u8,
        target: &EffectSelector,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if !selector_is_valid(target) {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        let targets = eligible_entity_ids(self.world, actor_position, target, self.rules);
        if targets.len() > usize::from(target.max) {
            return Err(EffectExecutionError::StepLimitExceeded);
        }
        for id in targets.into_iter().rev() {
            let position = if target.zone == EffectZone::Heroes {
                self.world
                    .entity(&id)
                    .and_then(|(_, hero)| hero.owner_position)
                    .ok_or(EffectExecutionError::InvalidDefinition)?
            } else {
                actor_position
            };
            self.queue.push_front(QueuedEffect::Definition {
                cursor: cursor.child(EffectPathSegment::RepeatEffect),
                actor_position: position,
            });
        }
        Ok(None)
    }
}

pub(super) fn prevent_drawing(
    world: &mut super::EffectWorld,
    entity_id: &str,
    rule_id: &str,
    outcomes: &mut Vec<super::EffectOutcome>,
) -> Result<Option<u8>, super::EffectExecutionError> {
    let (_, hero) = world
        .entity_mut(entity_id)
        .ok_or(super::EffectExecutionError::InvalidDefinition)?;
    let target_position = hero
        .owner_position
        .filter(|_| hero.kind == super::EffectEntityKind::Hero)
        .ok_or(super::EffectExecutionError::InvalidDefinition)?;
    hero.drawing_blocked = true;
    outcomes.push(super::EffectOutcome::DrawingBlocked {
        rule_id: rule_id.to_owned(),
        target_id: entity_id.to_owned(),
        target_position,
    });
    Ok(None)
}
