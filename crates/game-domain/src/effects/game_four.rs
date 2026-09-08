use super::{
    EffectCardType, EffectDefinition, EffectExecutionError, EffectExecutor, EffectOutcome,
    EffectResource, EffectRule, EffectWorld, EffectZone, game_three, modify_entity_resource,
    reactions,
};

fn ally_bonus(effect: &EffectDefinition) -> Option<u8> {
    match effect {
        EffectDefinition::OtherAllyBonus { health } => Some(*health),
        EffectDefinition::Sequence { effects } => effects.iter().find_map(ally_bonus),
        _ => None,
    }
}

fn source_rule<'a>(
    world: &EffectWorld,
    card: &'a super::EffectEntity,
    rules: &'a [EffectRule],
) -> Option<&'a EffectRule> {
    let id = card
        .copied_ally_id
        .as_deref()
        .and_then(|id| world.entity(id))
        .and_then(|(_, ally)| ally.effect_rule_id())
        .or_else(|| card.effect_rule_id())?;
    rules.iter().find(|rule| rule.id == id)
}

impl EffectExecutor<'_> {
    pub(super) fn execute_other_ally_bonus(
        &mut self,
        rule_id: &str,
        owner: u8,
        health: u8,
    ) -> Result<(), EffectExecutionError> {
        if !(1..=10).contains(&health) {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        let card_id = self
            .world
            .entities_in(EffectZone::HeroPlayArea)
            .iter()
            .rev()
            .find(|card| {
                card.owner_position == Some(owner)
                    && source_rule(self.world, card, self.rules)
                        .is_some_and(|rule| rule.id == rule_id)
            })
            .ok_or(EffectExecutionError::InvalidDefinition)?
            .id
            .clone();
        self.apply_other_ally_bonus(&card_id, rule_id, owner, health)
    }

    pub(super) fn enqueue_other_ally_bonuses(
        &mut self,
        start: usize,
    ) -> Result<(), EffectExecutionError> {
        let mut bonuses = Vec::new();
        for outcome in &self.outcomes[start..] {
            let EffectOutcome::Moved {
                rule_id,
                target_id,
                target_position: Some(owner),
                from: EffectZone::HeroHand,
                to: EffectZone::HeroPlayArea,
            } = outcome
            else {
                continue;
            };
            if rule_id != "system:play-card"
                || !self.world.entity(target_id).is_some_and(|(_, card)| {
                    reactions::card_type(card, self.rules) == Some(EffectCardType::Ally)
                })
            {
                continue;
            }
            for card in self.world.entities_in(EffectZone::HeroPlayArea) {
                if card.id == *target_id || card.owner_position != Some(*owner) {
                    continue;
                }
                if let Some(rule) = source_rule(self.world, card, self.rules)
                    && let Some(health) = ally_bonus(&rule.effect)
                {
                    bonuses.push((card.id.clone(), rule.id.clone(), *owner, health));
                }
            }
        }
        for (id, rule, owner, health) in bonuses {
            self.apply_other_ally_bonus(&id, &rule, owner, health)?;
        }
        Ok(())
    }

    fn apply_other_ally_bonus(
        &mut self,
        card_id: &str,
        rule_id: &str,
        owner: u8,
        health: u8,
    ) -> Result<(), EffectExecutionError> {
        let mut state = self
            .world
            .entity(card_id)
            .and_then(|(_, card)| card.turn_state.clone())
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        if state.ability_used
            || !self
                .world
                .entities_in(EffectZone::HeroPlayArea)
                .iter()
                .any(|card| {
                    card.id != card_id
                        && card.owner_position == Some(owner)
                        && reactions::card_type(card, self.rules) == Some(EffectCardType::Ally)
                })
        {
            return Ok(());
        }
        state.ability_used = true;
        game_three::change_turn_state(self.world, card_id, rule_id, state, &mut self.outcomes)?;
        let hero_id = self
            .world
            .entities_in(EffectZone::Heroes)
            .iter()
            .find(|hero| hero.owner_position == Some(owner))
            .ok_or(EffectExecutionError::InvalidDefinition)?
            .id
            .clone();
        modify_entity_resource(
            self.world,
            &hero_id,
            rule_id,
            EffectResource::Health,
            i16::from(health),
            &mut self.outcomes,
        )?;
        Ok(())
    }
}
