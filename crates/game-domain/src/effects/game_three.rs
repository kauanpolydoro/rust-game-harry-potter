use super::{
    EffectCardType, EffectChangeCause, EffectCursor, EffectDefinition, EffectEntity,
    EffectEntityKind, EffectExecutionError, EffectExecutor, EffectNoOpReason, EffectOutcome,
    EffectPathSegment, EffectResource, EffectSource, EffectStop, EffectTurnState, EffectWorld,
    EffectZone, HeroAbilityStrategy, QueuedEffect, modify_entity_resource, move_entity, reactions,
    refill_shuffled_pile,
};

pub(super) fn valid_turn_state(
    kind: EffectEntityKind,
    state: &EffectTurnState,
    positions: &[u8],
) -> bool {
    let valid_positions = |values: &[u8]| {
        values.windows(2).all(|pair| pair[0] < pair[1])
            && values.iter().all(|position| positions.contains(position))
    };
    valid_positions(&state.healed_positions)
        && valid_positions(&state.suppressed_by)
        && state
            .attack_limit
            .is_none_or(|limit| (1..=16).contains(&limit))
        && match kind {
            EffectEntityKind::Hero => {
                state.attack_limit.is_none() && state.suppressed_by.is_empty()
            }
            EffectEntityKind::Villain => !state.ability_used && state.healed_positions.is_empty(),
            EffectEntityKind::HogwartsCard => {
                state.healed_positions.is_empty()
                    && state.attack_assigned == 0
                    && state.attack_limit.is_none()
                    && state.suppressed_by.is_empty()
            }
            _ => false,
        }
}

pub(super) fn change_turn_state(
    world: &mut EffectWorld,
    id: &str,
    rule_id: &str,
    after: EffectTurnState,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<(), EffectExecutionError> {
    let (_, entity) = world
        .entity_mut(id)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    let before = entity
        .turn_state
        .replace(after.clone())
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    if before != after {
        outcomes.push(EffectOutcome::TurnStateChanged {
            rule_id: rule_id.to_owned(),
            target_id: id.to_owned(),
            before,
            after,
        });
    }
    Ok(())
}

pub(super) fn begin_turn(
    world: &mut EffectWorld,
    actor_position: u8,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<(), EffectExecutionError> {
    let changes = world
        .entities()
        .filter_map(|(_, entity)| {
            let previous = entity.turn_state.as_ref()?;
            let after = EffectTurnState {
                suppressed_by: previous
                    .suppressed_by
                    .iter()
                    .copied()
                    .filter(|owner| *owner != actor_position)
                    .collect(),
                ..EffectTurnState::default()
            };
            (previous != &after).then(|| (entity.id.clone(), after))
        })
        .collect::<Vec<_>>();
    for (id, after) in changes {
        change_turn_state(world, &id, "system:begin-turn", after, outcomes)?;
    }
    Ok(())
}

pub(super) fn suppress_villain(
    world: &mut EffectWorld,
    id: &str,
    source: EffectSource<'_>,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<Option<u8>, EffectExecutionError> {
    let (zone, villain) = world
        .entity(id)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    if zone != EffectZone::ActiveVillains || villain.kind != EffectEntityKind::Villain {
        return Err(EffectExecutionError::InvalidDefinition);
    }
    let mut state = villain
        .turn_state
        .clone()
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    if !state.suppressed_by.contains(&source.actor_position) {
        state.suppressed_by.push(source.actor_position);
        state.suppressed_by.sort_unstable();
        change_turn_state(world, id, source.rule_id, state, outcomes)?;
    }
    Ok(None)
}

impl EffectExecutor<'_> {
    pub(super) fn limit_villain_attack(
        &mut self,
        cursor: &EffectCursor,
        maximum: u8,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if !(1..=16).contains(&maximum) {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        let villains = self
            .world
            .entities_in(EffectZone::ActiveVillains)
            .iter()
            .map(|villain| villain.id.clone())
            .collect::<Vec<_>>();
        for id in villains {
            let mut state = self
                .world
                .entity(&id)
                .and_then(|(_, villain)| villain.turn_state.clone())
                .ok_or(EffectExecutionError::InvalidDefinition)?;
            state.attack_limit = Some(
                state
                    .attack_limit
                    .map_or(maximum, |previous| previous.min(maximum)),
            );
            change_turn_state(self.world, &id, &cursor.rule_id, state, &mut self.outcomes)?;
        }
        Ok(None)
    }

    pub(super) fn enqueue_hero_abilities(
        &mut self,
        outcome_start: usize,
    ) -> Result<(), EffectExecutionError> {
        let outcomes = self.outcomes[outcome_start..].to_vec();
        let heroes = self
            .world
            .entities_in(EffectZone::Heroes)
            .iter()
            .map(|hero| hero.id.clone())
            .collect::<Vec<_>>();
        let mut activations = Vec::new();
        for outcome in outcomes {
            for id in &heroes {
                let (_, hero) = self
                    .world
                    .entity(id)
                    .ok_or(EffectExecutionError::InvalidDefinition)?;
                let Some(mut state) = hero.turn_state.clone() else {
                    continue;
                };
                let Some(rule) = self
                    .rules
                    .iter()
                    .find(|rule| Some(rule.id.as_str()) == hero.effect_rule_id())
                else {
                    continue;
                };
                let EffectDefinition::HeroAbility { strategy, .. } = rule.effect else {
                    continue;
                };
                let recipient =
                    self.ability_recipient(hero, &mut state, strategy, &rule.id, &outcome)?;
                if let Some(actor_position) = recipient {
                    change_turn_state(self.world, id, &rule.id, state, &mut self.outcomes)?;
                    let activation = QueuedEffect::Definition {
                        cursor: EffectCursor::root(&rule.id)
                            .child(EffectPathSegment::HeroAbilityEffect),
                        actor_position,
                    };
                    if strategy == HeroAbilityStrategy::HermioneGameThreeV1 {
                        self.queue.push_back(activation);
                    } else {
                        activations.push(activation);
                    }
                }
            }
        }
        for activation in activations.into_iter().rev() {
            self.queue.push_front(activation);
        }
        Ok(())
    }

    fn ability_recipient(
        &self,
        hero: &EffectEntity,
        state: &mut EffectTurnState,
        strategy: HeroAbilityStrategy,
        ability_rule_id: &str,
        outcome: &EffectOutcome,
    ) -> Result<Option<u8>, EffectExecutionError> {
        let owner = hero
            .owner_position
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        Ok(match (strategy, outcome) {
            (
                HeroAbilityStrategy::HarryGameThreeV1,
                EffectOutcome::ResourceChanged {
                    resource: EffectResource::Control,
                    before,
                    after,
                    cause: EffectChangeCause::Effect,
                    ..
                },
            ) if after < before && !state.ability_used => {
                state.ability_used = true;
                Some(owner)
            }
            (
                HeroAbilityStrategy::HermioneGameThreeV1,
                EffectOutcome::Moved {
                    rule_id,
                    target_position: Some(position),
                    from: EffectZone::HeroHand,
                    to: EffectZone::HeroPlayArea,
                    ..
                },
            ) if rule_id == "system:play-card"
                && *position == owner
                && owner == self.actor_position
                && !state.ability_used
                && self
                    .world
                    .entities_in(EffectZone::HeroPlayArea)
                    .iter()
                    .filter(|card| {
                        card.owner_position == Some(owner)
                            && reactions::card_type(card, self.rules) == Some(EffectCardType::Spell)
                    })
                    .count()
                    >= 4 =>
            {
                state.ability_used = true;
                Some(owner)
            }
            (
                HeroAbilityStrategy::NevilleGameThreeV1,
                EffectOutcome::ResourceChanged {
                    rule_id,
                    target_position: Some(position),
                    resource: EffectResource::Health,
                    before,
                    after,
                    cause: EffectChangeCause::Effect,
                    ..
                },
            ) if owner == self.actor_position
                && after > before
                && rule_id != ability_rule_id
                && !state.healed_positions.contains(position) =>
            {
                state.healed_positions.push(*position);
                state.healed_positions.sort_unstable();
                Some(*position)
            }
            (
                HeroAbilityStrategy::RonGameThreeV1,
                EffectOutcome::ResourceChanged {
                    rule_id,
                    target_position: Some(position),
                    resource: EffectResource::Attack,
                    cause: EffectChangeCause::Cost,
                    before,
                    after,
                    ..
                },
            ) if rule_id == "system:assign-attack"
                && owner == self.actor_position
                && *position == owner
                && after < before
                && state.attack_assigned >= 3
                && !state.ability_used =>
            {
                state.ability_used = true;
                Some(owner)
            }
            _ => None,
        })
    }

    pub(super) fn reveal_top_card(
        &mut self,
        cursor: &EffectCursor,
        actor_position: u8,
        minimum_cost: u16,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if self
            .world
            .top_card_id(actor_position, EffectZone::HeroDrawPile)
            .is_none()
        {
            self.rolls_consumed += refill_shuffled_pile(
                self.world,
                Some(actor_position),
                (EffectZone::HeroDiscardPile, EffectZone::HeroDrawPile),
                &cursor.rule_id,
                &mut self.outcomes,
                self.roller,
            )?;
        }
        let Some(card_id) = self
            .world
            .top_card_id(actor_position, EffectZone::HeroDrawPile)
        else {
            self.outcomes.push(EffectOutcome::NoOp {
                rule_id: cursor.rule_id.clone(),
                reason: EffectNoOpReason::NoEligibleTarget,
            });
            return Ok(None);
        };
        let cost = self
            .world
            .entity(&card_id)
            .and_then(|(_, card)| card.influence_cost)
            .unwrap_or(0);
        self.outcomes.push(EffectOutcome::TopCardRevealed {
            rule_id: cursor.rule_id.clone(),
            card_id: card_id.clone(),
            owner_position: actor_position,
        });
        if cost >= minimum_cost {
            move_entity(
                self.world,
                &card_id,
                &cursor.rule_id,
                EffectZone::HeroDiscardPile,
                &mut self.outcomes,
            )?;
            self.queue.push_front(QueuedEffect::Definition {
                cursor: cursor.child(EffectPathSegment::RevealedCardEffect),
                actor_position,
            });
        }
        Ok(None)
    }
}

pub(super) fn discard_for_spell_bonus(
    world: &mut EffectWorld,
    entity_id: &str,
    source: EffectSource<'_>,
    influence: u8,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<Option<u8>, EffectExecutionError> {
    let (_, card) = world
        .entity(entity_id)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    let owner = card
        .owner_position
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    let is_spell = reactions::card_type(card, source.rules) == Some(EffectCardType::Spell);
    move_entity(
        world,
        entity_id,
        "system:voluntary-discard",
        EffectZone::HeroDiscardPile,
        outcomes,
    )?;
    if is_spell {
        modify_entity_resource(
            world,
            &format!("hero:{owner}"),
            source.rule_id,
            EffectResource::Influence,
            i16::from(influence),
            outcomes,
        )?;
    }
    Ok(None)
}

pub(super) fn gain_influence_and_health(
    world: &mut EffectWorld,
    entity_id: &str,
    source: EffectSource<'_>,
    influence: u8,
    health: u8,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<Option<u8>, EffectExecutionError> {
    modify_entity_resource(
        world,
        entity_id,
        source.rule_id,
        EffectResource::Influence,
        i16::from(influence),
        outcomes,
    )?;
    modify_entity_resource(
        world,
        entity_id,
        source.rule_id,
        EffectResource::Health,
        i16::from(health),
        outcomes,
    )
}

pub(super) fn apply_turn_state(
    world: &mut EffectWorld,
    target_id: &str,
    before: &EffectTurnState,
    after: &EffectTurnState,
) -> Result<(), EffectExecutionError> {
    let (_, entity) = world
        .entity_mut(target_id)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    if entity.turn_state.as_ref() != Some(before)
        || before == after
        || !valid_turn_state(entity.kind, after, &[1, 2, 3, 4])
    {
        return Err(EffectExecutionError::InvalidDefinition);
    }
    entity.turn_state = Some(after.clone());
    Ok(())
}

pub(super) fn validate_revealed_top(
    world: &EffectWorld,
    card_id: &str,
    owner: u8,
) -> Result<(), EffectExecutionError> {
    if world
        .top_card_id(owner, EffectZone::HeroDrawPile)
        .as_deref()
        != Some(card_id)
    {
        return Err(EffectExecutionError::InvalidDefinition);
    }
    Ok(())
}
