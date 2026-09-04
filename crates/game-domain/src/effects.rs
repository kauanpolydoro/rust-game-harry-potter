use std::collections::{BTreeMap, VecDeque};

const MAX_EXECUTION_STEPS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectResource {
    Attack,
    Control,
    Health,
    Influence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectZone {
    ActiveLocation,
    ActiveVillains,
    DarkArtsDeck,
    DarkArtsDiscard,
    HeroDiscardPile,
    HeroDrawPile,
    HeroHand,
    HeroPlayArea,
    Heroes,
    HogwartsDeck,
    Market,
    VillainDeck,
}

impl EffectZone {
    const fn is_card_zone(self) -> bool {
        matches!(
            self,
            Self::DarkArtsDeck
                | Self::DarkArtsDiscard
                | Self::HeroDiscardPile
                | Self::HeroDrawPile
                | Self::HeroHand
                | Self::HeroPlayArea
                | Self::HogwartsDeck
                | Self::Market
                | Self::VillainDeck
        )
    }

    const fn supports_resource(self, resource: EffectResource) -> bool {
        matches!(
            (self, resource),
            (
                Self::Heroes,
                EffectResource::Attack | EffectResource::Health | EffectResource::Influence
            ) | (Self::ActiveVillains, EffectResource::Health)
                | (Self::ActiveLocation, EffectResource::Control)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTargetOwner {
    Actor,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectSelector {
    pub zone: EffectZone,
    pub owner: EffectTargetOwner,
    pub min: u16,
    pub max: u16,
    pub eligibility: Vec<EffectEligibility>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectEligibility {
    ResourceAtLeast {
        resource: EffectResource,
        amount: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectCondition {
    HasEligibleTarget {
        target: EffectSelector,
    },
    ResourceAtLeast {
        target: EffectSelector,
        resource: EffectResource,
        amount: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectOperation {
    Discard,
    ModifyResource {
        resource: EffectResource,
        amount: i16,
    },
    Move {
        to: EffectZone,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectDie {
    D4,
    D6,
    D8,
}

impl EffectDie {
    #[must_use]
    pub const fn sides(self) -> u8 {
        match self {
            Self::D4 => 4,
            Self::D6 => 6,
            Self::D8 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectGameOutcome {
    Lost,
    Won,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectDefinition {
    Apply {
        target: EffectSelector,
        operation: EffectOperation,
    },
    Choice {
        options: Vec<Self>,
    },
    Condition {
        condition: EffectCondition,
        then: Box<Self>,
        otherwise: Option<Box<Self>>,
    },
    NoOp,
    Repeat {
        times: u8,
        effect: Box<Self>,
    },
    Roll {
        die: EffectDie,
        outcomes: Vec<Self>,
    },
    Sequence {
        effects: Vec<Self>,
    },
    Terminal {
        outcome: EffectGameOutcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTrigger {
    DarkArtsCompleted,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectResourceCost {
    pub resource: EffectResource,
    pub amount: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRule {
    pub id: String,
    pub trigger: EffectTrigger,
    pub cost: Vec<EffectResourceCost>,
    pub effect: EffectDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntity {
    id: String,
    owner_position: Option<u8>,
    zone: EffectZone,
    resources: BTreeMap<EffectResource, u16>,
}

impl EffectEntity {
    #[must_use]
    pub fn new(id: impl Into<String>, owner_position: Option<u8>, zone: EffectZone) -> Self {
        Self {
            id: id.into(),
            owner_position,
            zone,
            resources: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn hero(position: u8) -> Self {
        Self::new(
            format!("hero:{position}"),
            Some(position),
            EffectZone::Heroes,
        )
        .with_resource(EffectResource::Health, 10)
    }

    #[must_use]
    pub fn with_resource(mut self, resource: EffectResource, amount: u16) -> Self {
        self.resources.insert(resource, amount);
        self
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn owner_position(&self) -> Option<u8> {
        self.owner_position
    }

    #[must_use]
    pub const fn zone(&self) -> EffectZone {
        self.zone
    }

    #[must_use]
    pub fn resources(&self) -> &BTreeMap<EffectResource, u16> {
        &self.resources
    }

    #[must_use]
    pub fn resource(&self, resource: EffectResource) -> u16 {
        self.resources.get(&resource).copied().unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectWorld {
    entities: Vec<EffectEntity>,
}

impl EffectWorld {
    #[must_use]
    pub fn new(mut entities: Vec<EffectEntity>) -> Self {
        entities.sort_by(|left, right| left.id.cmp(&right.id));
        Self { entities }
    }

    #[must_use]
    pub fn entities(&self) -> &[EffectEntity] {
        &self.entities
    }

    #[must_use]
    pub fn hero_resource(&self, position: u8, resource: EffectResource) -> Option<u16> {
        self.entities
            .iter()
            .find(|entity| {
                entity.zone == EffectZone::Heroes && entity.owner_position == Some(position)
            })
            .map(|entity| entity.resource(resource))
    }

    pub(crate) fn is_valid_for_positions(&self, positions: &[u8]) -> bool {
        let mut ids = self
            .entities
            .iter()
            .map(|entity| entity.id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        if ids.iter().any(|id| id.is_empty())
            || ids.windows(2).any(|pair| pair[0] == pair[1])
            || self.entities.iter().any(|entity| {
                entity
                    .owner_position
                    .is_some_and(|position| !positions.contains(&position))
                    || (entity.zone == EffectZone::Heroes && entity.owner_position.is_none())
                    || entity
                        .resources
                        .keys()
                        .any(|resource| !entity.zone.supports_resource(*resource))
            })
        {
            return false;
        }
        positions.iter().all(|position| {
            self.entities
                .iter()
                .filter(|entity| {
                    entity.zone == EffectZone::Heroes && entity.owner_position == Some(*position)
                })
                .count()
                == 1
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectChangeCause {
    Cost,
    Effect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectNoOpReason {
    Explicit,
    NoEligibleTarget,
    ZeroCardinality,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectOutcome {
    DieRolled {
        rule_id: String,
        die: EffectDie,
        result: u8,
    },
    Moved {
        rule_id: String,
        target_id: String,
        target_position: Option<u8>,
        from: EffectZone,
        to: EffectZone,
    },
    NoOp {
        rule_id: String,
        reason: EffectNoOpReason,
    },
    ResourceChanged {
        rule_id: String,
        target_id: String,
        target_position: Option<u8>,
        resource: EffectResource,
        before: u16,
        after: u16,
        cause: EffectChangeCause,
    },
    Terminal {
        rule_id: String,
        outcome: EffectGameOutcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingEffectChoiceKind {
    Effect,
    Target,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEffectChoice {
    pub id: String,
    pub responsible_position: u8,
    pub kind: PendingEffectChoiceKind,
    pub options: Vec<String>,
    pub min: u16,
    pub max: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectStop {
    Choice(PendingEffectChoice),
    Stable,
    Terminal(EffectGameOutcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectResolution {
    pub outcomes: Vec<EffectOutcome>,
    pub stop: EffectStop,
    pub rolls_consumed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectExecutionError {
    InvalidDefinition,
    InvalidRoll,
    StepLimitExceeded,
    UnaffordableCost,
}

pub trait EffectRoller {
    fn roll(&mut self, die: EffectDie) -> Option<u8>;
}

struct EffectExecutor<'a> {
    world: &'a mut EffectWorld,
    actor_position: u8,
    roller: &'a mut dyn EffectRoller,
    queue: VecDeque<(String, EffectDefinition)>,
    outcomes: Vec<EffectOutcome>,
    rolls_consumed: u64,
    steps: usize,
}

impl EffectExecutor<'_> {
    fn run(mut self) -> Result<EffectResolution, EffectExecutionError> {
        while let Some((rule_id, effect)) = self.queue.pop_front() {
            self.steps += 1;
            if self.steps > MAX_EXECUTION_STEPS {
                return Err(EffectExecutionError::StepLimitExceeded);
            }
            if let Some(stop) = self.execute_definition(rule_id, effect)? {
                return Ok(self.finish(stop));
            }
        }
        Ok(self.finish(EffectStop::Stable))
    }

    fn execute_definition(
        &mut self,
        rule_id: String,
        effect: EffectDefinition,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        match effect {
            EffectDefinition::Apply { target, operation } => {
                self.execute_apply(rule_id, &target, &operation)
            }
            EffectDefinition::Choice { options } => self.execute_choice(&rule_id, &options),
            EffectDefinition::Condition {
                condition,
                then,
                otherwise,
            } => {
                if !condition_is_valid(&condition) {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                let selected = if condition_is_true(self.world, self.actor_position, &condition) {
                    Some(*then)
                } else {
                    otherwise.map(|effect| *effect)
                };
                if let Some(selected) = selected {
                    self.queue.push_front((rule_id, selected));
                }
                Ok(None)
            }
            EffectDefinition::NoOp => {
                self.outcomes.push(EffectOutcome::NoOp {
                    rule_id,
                    reason: EffectNoOpReason::Explicit,
                });
                Ok(None)
            }
            EffectDefinition::Repeat { times, effect } => {
                if times == 0 {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                for _ in 0..times {
                    self.queue.push_front((rule_id.clone(), (*effect).clone()));
                }
                Ok(None)
            }
            EffectDefinition::Roll {
                die,
                outcomes: roll_outcomes,
            } => self.execute_roll(rule_id, die, &roll_outcomes),
            EffectDefinition::Sequence { effects } => {
                if effects.is_empty() {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                for effect in effects.into_iter().rev() {
                    self.queue.push_front((rule_id.clone(), effect));
                }
                Ok(None)
            }
            EffectDefinition::Terminal { outcome } => {
                self.outcomes
                    .push(EffectOutcome::Terminal { rule_id, outcome });
                Ok(Some(EffectStop::Terminal(outcome)))
            }
        }
    }

    fn execute_apply(
        &mut self,
        rule_id: String,
        target: &EffectSelector,
        operation: &EffectOperation,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if !selector_is_valid(target) || !operation_is_valid_for_zone(operation, target.zone) {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        let candidates = eligible_entity_indices(self.world, self.actor_position, target);
        if target.max == 0 {
            self.outcomes.push(EffectOutcome::NoOp {
                rule_id,
                reason: EffectNoOpReason::ZeroCardinality,
            });
        } else if candidates.len() < usize::from(target.min) {
            self.outcomes.push(EffectOutcome::NoOp {
                rule_id,
                reason: EffectNoOpReason::NoEligibleTarget,
            });
        } else if candidates.len() > usize::from(target.max) {
            let options = candidates
                .into_iter()
                .map(|index| self.world.entities[index].id.clone())
                .collect();
            return Ok(Some(EffectStop::Choice(PendingEffectChoice {
                id: format!("{rule_id}:target:{}", self.steps - 1),
                responsible_position: self.actor_position,
                kind: PendingEffectChoiceKind::Target,
                options,
                min: target.min,
                max: target.max,
            })));
        } else {
            for index in candidates {
                apply_operation(self.world, index, &rule_id, operation, &mut self.outcomes)?;
            }
        }
        Ok(None)
    }

    fn execute_choice(
        &self,
        rule_id: &str,
        options: &[EffectDefinition],
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if options.len() < 2 {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        Ok(Some(EffectStop::Choice(PendingEffectChoice {
            id: format!("{rule_id}:effect:{}", self.steps - 1),
            responsible_position: self.actor_position,
            kind: PendingEffectChoiceKind::Effect,
            options: (1..=options.len())
                .map(|index| format!("option:{index}"))
                .collect(),
            min: 1,
            max: 1,
        })))
    }

    fn execute_roll(
        &mut self,
        rule_id: String,
        die: EffectDie,
        roll_outcomes: &[EffectDefinition],
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if roll_outcomes.len() != usize::from(die.sides()) {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        let result = self
            .roller
            .roll(die)
            .ok_or(EffectExecutionError::InvalidRoll)?;
        if result == 0 || result > die.sides() {
            return Err(EffectExecutionError::InvalidRoll);
        }
        self.rolls_consumed = self
            .rolls_consumed
            .checked_add(1)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        self.outcomes.push(EffectOutcome::DieRolled {
            rule_id: rule_id.clone(),
            die,
            result,
        });
        self.queue
            .push_front((rule_id, roll_outcomes[usize::from(result - 1)].clone()));
        Ok(None)
    }

    fn finish(self, stop: EffectStop) -> EffectResolution {
        EffectResolution {
            outcomes: self.outcomes,
            stop,
            rolls_consumed: self.rolls_consumed,
        }
    }
}

#[must_use]
pub fn effect_action_is_affordable(
    world: &EffectWorld,
    actor_position: u8,
    rules: &[EffectRule],
    trigger: EffectTrigger,
) -> bool {
    let costs = combined_costs(rules, trigger);
    costs.into_iter().all(|(resource, amount)| {
        world
            .hero_resource(actor_position, resource)
            .is_some_and(|available| u64::from(available) >= amount)
    })
}

pub(crate) fn execute_effects(
    world: &mut EffectWorld,
    actor_position: u8,
    rules: &[EffectRule],
    trigger: EffectTrigger,
    roller: &mut dyn EffectRoller,
) -> Result<EffectResolution, EffectExecutionError> {
    if rules.iter().any(|rule| {
        rule.id.is_empty()
            || rule
                .cost
                .iter()
                .any(|cost| cost.amount == 0 || cost.resource == EffectResource::Control)
    }) {
        return Err(EffectExecutionError::InvalidDefinition);
    }
    if !effect_action_is_affordable(world, actor_position, rules, trigger) {
        return Err(EffectExecutionError::UnaffordableCost);
    }

    let outcomes = pay_costs(world, actor_position, rules, trigger)?;
    let queue = rules
        .iter()
        .filter(|rule| rule.trigger == trigger)
        .map(|rule| (rule.id.clone(), rule.effect.clone()))
        .collect::<VecDeque<_>>();
    EffectExecutor {
        world,
        actor_position,
        roller,
        queue,
        outcomes,
        rolls_consumed: 0,
        steps: 0,
    }
    .run()
}

fn pay_costs(
    world: &mut EffectWorld,
    actor_position: u8,
    rules: &[EffectRule],
    trigger: EffectTrigger,
) -> Result<Vec<EffectOutcome>, EffectExecutionError> {
    let mut outcomes = Vec::new();
    for (rule_id, cost) in rules
        .iter()
        .filter(|rule| rule.trigger == trigger)
        .flat_map(|rule| rule.cost.iter().map(|cost| (&rule.id, cost)))
    {
        let hero = world
            .entities
            .iter_mut()
            .find(|entity| {
                entity.zone == EffectZone::Heroes && entity.owner_position == Some(actor_position)
            })
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let before = hero.resource(cost.resource);
        let after = before
            .checked_sub(cost.amount)
            .ok_or(EffectExecutionError::UnaffordableCost)?;
        hero.resources.insert(cost.resource, after);
        outcomes.push(EffectOutcome::ResourceChanged {
            rule_id: rule_id.clone(),
            target_id: hero.id.clone(),
            target_position: hero.owner_position,
            resource: cost.resource,
            before,
            after,
            cause: EffectChangeCause::Cost,
        });
    }
    Ok(outcomes)
}

pub(crate) fn apply_effect_outcomes(
    world: &mut EffectWorld,
    outcomes: &[EffectOutcome],
) -> Result<(), EffectExecutionError> {
    for outcome in outcomes {
        match outcome {
            EffectOutcome::Moved {
                target_id,
                target_position,
                from,
                to,
                ..
            } => {
                let entity = world
                    .entities
                    .iter_mut()
                    .find(|entity| entity.id == *target_id)
                    .ok_or(EffectExecutionError::InvalidDefinition)?;
                if entity.zone != *from
                    || entity.owner_position != *target_position
                    || from == to
                    || !from.is_card_zone()
                    || !to.is_card_zone()
                {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                entity.zone = *to;
            }
            EffectOutcome::ResourceChanged {
                target_id,
                target_position,
                resource,
                before,
                after,
                ..
            } => {
                let entity = world
                    .entities
                    .iter_mut()
                    .find(|entity| entity.id == *target_id)
                    .ok_or(EffectExecutionError::InvalidDefinition)?;
                if entity.owner_position != *target_position
                    || !entity.zone.supports_resource(*resource)
                    || entity.resource(*resource) != *before
                {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                entity.resources.insert(*resource, *after);
            }
            EffectOutcome::DieRolled { .. }
            | EffectOutcome::NoOp { .. }
            | EffectOutcome::Terminal { .. } => {}
        }
    }
    Ok(())
}

pub(crate) fn effect_transition_is_valid(
    outcomes: &[EffectOutcome],
    stop: &EffectStop,
    actor_position: u8,
) -> bool {
    if outcomes.len() > MAX_EXECUTION_STEPS || outcomes.iter().any(|outcome| !outcome.is_valid()) {
        return false;
    }
    let terminal_count = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, EffectOutcome::Terminal { .. }))
        .count();
    match stop {
        EffectStop::Choice(choice) => {
            terminal_count == 0 && choice.is_valid_for_actor(actor_position)
        }
        EffectStop::Stable => terminal_count == 0,
        EffectStop::Terminal(expected) => {
            terminal_count == 1
                && matches!(
                    outcomes.last(),
                    Some(EffectOutcome::Terminal { outcome, .. }) if outcome == expected
                )
        }
    }
}

impl EffectOutcome {
    fn is_valid(&self) -> bool {
        match self {
            Self::DieRolled {
                rule_id,
                die,
                result,
            } => !rule_id.is_empty() && (1..=die.sides()).contains(result),
            Self::Moved {
                rule_id,
                target_id,
                from,
                to,
                ..
            } => {
                !rule_id.is_empty()
                    && !target_id.is_empty()
                    && from != to
                    && from.is_card_zone()
                    && to.is_card_zone()
            }
            Self::NoOp { rule_id, .. } | Self::Terminal { rule_id, .. } => !rule_id.is_empty(),
            Self::ResourceChanged {
                rule_id, target_id, ..
            } => !rule_id.is_empty() && !target_id.is_empty(),
        }
    }
}

impl PendingEffectChoice {
    fn is_valid_for_actor(&self, actor_position: u8) -> bool {
        let options = self
            .options
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        self.responsible_position == actor_position
            && !self.id.is_empty()
            && self.options.len() >= 2
            && options.len() == self.options.len()
            && options.iter().all(|option| !option.is_empty())
            && self.min <= self.max
            && usize::from(self.max) <= self.options.len()
    }
}

fn selector_is_valid(selector: &EffectSelector) -> bool {
    selector.min <= selector.max
        && selector
            .eligibility
            .iter()
            .all(|eligibility| match eligibility {
                EffectEligibility::ResourceAtLeast { resource, amount } => {
                    *amount > 0 && selector.zone.supports_resource(*resource)
                }
            })
}

fn condition_is_valid(condition: &EffectCondition) -> bool {
    match condition {
        EffectCondition::HasEligibleTarget { target } => selector_is_valid(target),
        EffectCondition::ResourceAtLeast {
            target,
            resource,
            amount,
        } => selector_is_valid(target) && *amount > 0 && target.zone.supports_resource(*resource),
    }
}

fn operation_is_valid_for_zone(operation: &EffectOperation, zone: EffectZone) -> bool {
    match operation {
        EffectOperation::Discard => zone == EffectZone::HeroHand,
        EffectOperation::ModifyResource { resource, amount } => {
            *amount != 0 && zone.supports_resource(*resource)
        }
        EffectOperation::Move { to } => zone.is_card_zone() && to.is_card_zone() && zone != *to,
    }
}

fn combined_costs(rules: &[EffectRule], trigger: EffectTrigger) -> BTreeMap<EffectResource, u64> {
    let mut costs = BTreeMap::new();
    for cost in rules
        .iter()
        .filter(|rule| rule.trigger == trigger)
        .flat_map(|rule| &rule.cost)
    {
        *costs.entry(cost.resource).or_default() += u64::from(cost.amount);
    }
    costs
}

fn eligible_entity_indices(
    world: &EffectWorld,
    actor_position: u8,
    selector: &EffectSelector,
) -> Vec<usize> {
    world
        .entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| entity.zone == selector.zone)
        .filter(|(_, entity)| {
            selector.owner == EffectTargetOwner::Any
                || entity.owner_position == Some(actor_position)
        })
        .filter(|(_, entity)| {
            selector
                .eligibility
                .iter()
                .all(|eligibility| match eligibility {
                    EffectEligibility::ResourceAtLeast { resource, amount } => {
                        entity.resource(*resource) >= *amount
                    }
                })
        })
        .map(|(index, _)| index)
        .collect()
}

fn condition_is_true(world: &EffectWorld, actor_position: u8, condition: &EffectCondition) -> bool {
    match condition {
        EffectCondition::HasEligibleTarget { target } => {
            eligible_entity_indices(world, actor_position, target).len() >= usize::from(target.min)
        }
        EffectCondition::ResourceAtLeast {
            target,
            resource,
            amount,
        } => {
            let candidates = eligible_entity_indices(world, actor_position, target);
            candidates.len() >= usize::from(target.min)
                && candidates
                    .into_iter()
                    .any(|index| world.entities[index].resource(*resource) >= *amount)
        }
    }
}

fn apply_operation(
    world: &mut EffectWorld,
    index: usize,
    rule_id: &str,
    operation: &EffectOperation,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<(), EffectExecutionError> {
    let entity = world
        .entities
        .get_mut(index)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    match operation {
        EffectOperation::Discard => {
            let from = entity.zone;
            entity.zone = EffectZone::HeroDiscardPile;
            outcomes.push(EffectOutcome::Moved {
                rule_id: rule_id.to_owned(),
                target_id: entity.id.clone(),
                target_position: entity.owner_position,
                from,
                to: entity.zone,
            });
        }
        EffectOperation::ModifyResource { resource, amount } => {
            let before = entity.resource(*resource);
            let after = if *amount < 0 {
                before.saturating_sub(amount.unsigned_abs())
            } else {
                before.saturating_add(amount.unsigned_abs())
            };
            entity.resources.insert(*resource, after);
            outcomes.push(EffectOutcome::ResourceChanged {
                rule_id: rule_id.to_owned(),
                target_id: entity.id.clone(),
                target_position: entity.owner_position,
                resource: *resource,
                before,
                after,
                cause: EffectChangeCause::Effect,
            });
        }
        EffectOperation::Move { to } => {
            let from = entity.zone;
            entity.zone = *to;
            outcomes.push(EffectOutcome::Moved {
                rule_id: rule_id.to_owned(),
                target_id: entity.id.clone(),
                target_position: entity.owner_position,
                from,
                to: *to,
            });
        }
    }
    Ok(())
}
