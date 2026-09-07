use super::{
    EffectCardType, EffectChangeCause, EffectCursor, EffectDefinition, EffectEntity,
    EffectEntityKind, EffectExecutionError, EffectExecutor, EffectOutcome, EffectPathSegment,
    EffectReactionTrigger, EffectResource, EffectRule, EffectSource, EffectTrigger, EffectWorld,
    EffectZone, MAX_EFFECT_PATH_DEPTH, MAX_EXECUTION_STEPS, QueuedEffect,
};

pub(super) fn protected_health_loss(
    world: &EffectWorld,
    hero_id: &str,
    source: EffectSource<'_>,
    amount: i16,
) -> i16 {
    let Some((EffectZone::Heroes, hero)) = world.entity(hero_id) else {
        return amount;
    };
    let harmful_source = is_harmful_rule(world, source.rule_id, source.rules);
    if !harmful_source {
        return amount;
    }
    world
        .entities_in(EffectZone::HeroHand)
        .iter()
        .filter(|card| card.owner_position() == hero.owner_position())
        .filter_map(|card| {
            source
                .rules
                .iter()
                .find(|rule| Some(rule.id.as_str()) == card.effect_rule_id())
        })
        .filter_map(|rule| hand_damage_limit(&rule.effect))
        .fold(amount, |loss, maximum| loss.max(-i16::from(maximum)))
}

fn is_harmful_rule(world: &EffectWorld, rule_id: &str, rules: &[EffectRule]) -> bool {
    rules.iter().any(|rule| {
        rule.id == rule_id
            && matches!(
                rule.trigger,
                EffectTrigger::DarkArts
                    | EffectTrigger::DarkArtsCompleted
                    | EffectTrigger::Villains
            )
    }) || world.entities().any(|(zone, entity)| {
        matches!(
            (zone, entity.kind()),
            (EffectZone::DarkArtsDiscard, EffectEntityKind::DarkArts)
                | (EffectZone::ActiveVillains, EffectEntityKind::Villain)
        ) && entity.effect_rule_id() == Some(rule_id)
    })
}

fn hand_damage_limit(effect: &EffectDefinition) -> Option<u8> {
    match effect {
        EffectDefinition::HandDamageLimit { maximum } if *maximum > 0 => Some(*maximum),
        EffectDefinition::Sequence { effects } => {
            effects.iter().filter_map(hand_damage_limit).min()
        }
        _ => None,
    }
}

impl EffectExecutor<'_> {
    pub(super) fn enqueue_reactions(
        &mut self,
        outcome_start: usize,
    ) -> Result<(), EffectExecutionError> {
        let mut reactions = Vec::new();
        for outcome in &self.outcomes[outcome_start..] {
            for (zone, source) in self.world.entities() {
                if source.villain_ability_suppressed() {
                    continue;
                }
                let copied_rule = source
                    .copied_ally_id
                    .as_deref()
                    .and_then(|id| self.world.entity(id))
                    .and_then(|(_, ally)| ally.effect_rule_id());
                let mut declarations = Vec::new();
                for id in [source.effect_rule_id(), copied_rule].into_iter().flatten() {
                    let Some(rule) = self.rules.iter().find(|rule| rule.id == id) else {
                        continue;
                    };
                    collect_reactions(
                        &rule.effect,
                        &EffectCursor::root(&rule.id),
                        &mut declarations,
                    )?;
                }
                for (trigger, cursor) in declarations {
                    if let Some((position, count)) = reaction_context(
                        trigger,
                        zone,
                        source,
                        outcome,
                        self.actor_position,
                        self.world,
                        self.rules,
                    ) {
                        for _ in 0..count {
                            reactions.push(QueuedEffect::Definition {
                                cursor: cursor.clone(),
                                actor_position: position,
                            });
                        }
                    }
                }
            }
        }
        if reactions.len() + self.queue.len() > MAX_EXECUTION_STEPS {
            return Err(EffectExecutionError::StepLimitExceeded);
        }
        for reaction in reactions.into_iter().rev() {
            self.queue.push_front(reaction);
        }
        Ok(())
    }
}

fn collect_reactions(
    definition: &EffectDefinition,
    cursor: &EffectCursor,
    declarations: &mut Vec<(EffectReactionTrigger, EffectCursor)>,
) -> Result<(), EffectExecutionError> {
    if cursor.path.len() >= MAX_EFFECT_PATH_DEPTH {
        return Err(EffectExecutionError::InvalidDefinition);
    }
    match definition {
        EffectDefinition::Reaction { trigger, .. } => {
            declarations.push((*trigger, cursor.child(EffectPathSegment::ReactionEffect)));
        }
        EffectDefinition::Sequence { effects } => {
            for (index, effect) in effects.iter().enumerate() {
                let index =
                    u16::try_from(index).map_err(|_| EffectExecutionError::InvalidDefinition)?;
                collect_reactions(
                    effect,
                    &cursor.child(EffectPathSegment::SequenceEffect(index)),
                    declarations,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn reaction_context(
    trigger: EffectReactionTrigger,
    zone: EffectZone,
    source: &EffectEntity,
    outcome: &EffectOutcome,
    active_position: u8,
    world: &EffectWorld,
    rules: &[EffectRule],
) -> Option<(u8, u16)> {
    match (trigger, outcome) {
        (
            EffectReactionTrigger::OwnerPlaysAlly,
            EffectOutcome::Moved {
                rule_id,
                target_id,
                target_position: Some(owner),
                from: EffectZone::HeroHand,
                to: EffectZone::HeroPlayArea,
            },
        ) if rule_id == "system:play-card"
            && zone == EffectZone::HeroPlayArea
            && source.owner_position() == Some(*owner)
            && world
                .entity(target_id)
                .is_some_and(|(_, card)| card_type(card, rules) == Some(EffectCardType::Ally)) =>
        {
            Some((*owner, 1))
        }
        (
            EffectReactionTrigger::ControlAdded,
            EffectOutcome::ResourceChanged {
                resource: EffectResource::Control,
                before,
                after,
                cause: EffectChangeCause::Effect,
                ..
            },
        ) if zone == EffectZone::ActiveVillains
            && source.kind() == EffectEntityKind::Villain
            && after > before =>
        {
            Some((active_position, after - before))
        }
        (
            EffectReactionTrigger::HeroForcedDiscard
            | EffectReactionTrigger::SelfForcedDiscard
            | EffectReactionTrigger::SelfHarmfulDiscard,
            EffectOutcome::Moved {
                rule_id,
                target_id,
                target_position: Some(position),
                from: EffectZone::HeroHand | EffectZone::HeroDrawPile,
                to: EffectZone::HeroDiscardPile,
                ..
            },
        ) if (trigger == EffectReactionTrigger::HeroForcedDiscard
            && zone == EffectZone::ActiveVillains
            && source.kind() == EffectEntityKind::Villain
            && rule_id != "system:voluntary-discard")
            || (trigger == EffectReactionTrigger::SelfForcedDiscard
                && source.id() == target_id
                && zone == EffectZone::HeroDiscardPile)
            || (trigger == EffectReactionTrigger::SelfHarmfulDiscard
                && source.id() == target_id
                && zone == EffectZone::HeroDiscardPile
                && rule_id != "system:voluntary-discard"
                && (rule_id == "system:stunned" || is_harmful_rule(world, rule_id, rules))) =>
        {
            Some((*position, 1))
        }
        (
            EffectReactionTrigger::OwnerDefeatsVillain,
            EffectOutcome::Moved {
                rule_id,
                from: EffectZone::ActiveVillains,
                to: EffectZone::VillainDiscard,
                ..
            },
        ) if rule_id == "system:defeat-villain"
            && zone == EffectZone::HeroPlayArea
            && source.owner_position() == Some(active_position) =>
        {
            Some((active_position, 1))
        }
        _ => None,
    }
}

pub(super) fn card_type(card: &EffectEntity, rules: &[EffectRule]) -> Option<EffectCardType> {
    let rule = rules
        .iter()
        .find(|rule| Some(rule.id.as_str()) == card.effect_rule_id())?;
    definition_card_type(&rule.effect)
}

fn definition_card_type(effect: &EffectDefinition) -> Option<EffectCardType> {
    match effect {
        EffectDefinition::CardType { card_type } => Some(*card_type),
        EffectDefinition::Sequence { effects } => effects.iter().find_map(definition_card_type),
        _ => None,
    }
}

pub(crate) fn can_acquire_on_deck(
    world: &EffectWorld,
    owner: u8,
    card: &EffectEntity,
    rules: &[EffectRule],
) -> bool {
    let Some(category) = card_type(card, rules) else {
        return false;
    };
    world
        .entities_in(EffectZone::HeroPlayArea)
        .iter()
        .filter(|card| card.owner_position() == Some(owner))
        .filter_map(|card| {
            rules
                .iter()
                .find(|rule| Some(rule.id.as_str()) == card.effect_rule_id())
        })
        .any(|rule| permits_top_deck(&rule.effect, category))
}

fn permits_top_deck(effect: &EffectDefinition, category: EffectCardType) -> bool {
    match effect {
        EffectDefinition::TopDeckAcquisition { card_type } => *card_type == category,
        EffectDefinition::Sequence { effects } => effects
            .iter()
            .any(|effect| permits_top_deck(effect, category)),
        _ => false,
    }
}

pub(super) fn drawing_prevented(world: &EffectWorld, rules: &[EffectRule]) -> bool {
    world
        .entities_in(EffectZone::ActiveVillains)
        .iter()
        .filter(|villain| !villain.villain_ability_suppressed())
        .any(|villain| {
            rules
                .iter()
                .find(|rule| Some(rule.id.as_str()) == villain.effect_rule_id())
                .is_some_and(|rule| prevents_drawing(&rule.effect))
        })
}

fn prevents_drawing(effect: &EffectDefinition) -> bool {
    match effect {
        EffectDefinition::PreventExtraDrawing => true,
        EffectDefinition::Sequence { effects } => effects.iter().any(prevents_drawing),
        _ => false,
    }
}
