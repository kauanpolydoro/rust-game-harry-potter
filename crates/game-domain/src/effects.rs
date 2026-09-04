use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_EXECUTION_STEPS: usize = 4_096;
const MAX_CHOICE_OPTIONS: usize = 4_096;
const MAX_CHOICE_SELECTIONS: u16 = 32;
const MAX_CHOICE_VALUE_LENGTH: usize = 256;
const MAX_RUNTIME_RULE_ID_LENGTH: usize = 244;

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
            Self::ActiveVillains
                | Self::DarkArtsDeck
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
    pub id: Option<String>,
    pub zone: EffectZone,
    pub owner: EffectTargetOwner,
    pub min: u16,
    pub max: u16,
    pub eligibility: Vec<EffectEligibility>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectTargetBinding {
    pub selector_id: String,
    pub target_ids: Vec<String>,
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
        audience: EffectChoiceAudience,
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
pub enum EffectChoiceAudience {
    Actor,
    EachHero,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectEntityKind {
    Generic,
    Hero,
    HogwartsCard,
    StarterCard,
    Villain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntity {
    id: String,
    kind: EffectEntityKind,
    catalog_id: Option<String>,
    owner_position: Option<u8>,
    effect_rule_id: Option<String>,
    influence_cost: Option<u16>,
    resources: BTreeMap<EffectResource, u16>,
}

impl EffectEntity {
    #[must_use]
    pub fn new(id: impl Into<String>, owner_position: Option<u8>) -> Self {
        Self {
            id: id.into(),
            kind: EffectEntityKind::Generic,
            catalog_id: None,
            owner_position,
            effect_rule_id: None,
            influence_cost: None,
            resources: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn hero(position: u8) -> Self {
        Self {
            kind: EffectEntityKind::Hero,
            ..Self::new(format!("hero:{position}"), Some(position))
        }
        .with_resource(EffectResource::Health, 10)
    }

    #[must_use]
    pub fn card(
        id: impl Into<String>,
        catalog_id: impl Into<String>,
        kind: EffectEntityKind,
        owner_position: Option<u8>,
        effect_rule_id: impl Into<String>,
        influence_cost: Option<u16>,
    ) -> Self {
        Self {
            id: id.into(),
            kind,
            catalog_id: Some(catalog_id.into()),
            owner_position,
            effect_rule_id: Some(effect_rule_id.into()),
            influence_cost,
            resources: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn villain(
        id: impl Into<String>,
        catalog_id: impl Into<String>,
        effect_rule_id: impl Into<String>,
        health: u16,
    ) -> Self {
        Self {
            id: id.into(),
            kind: EffectEntityKind::Villain,
            catalog_id: Some(catalog_id.into()),
            owner_position: None,
            effect_rule_id: Some(effect_rule_id.into()),
            influence_cost: None,
            resources: BTreeMap::from([(EffectResource::Health, health)]),
        }
    }

    #[must_use]
    pub fn with_kind(mut self, kind: EffectEntityKind) -> Self {
        self.kind = kind;
        self
    }

    #[must_use]
    pub fn with_catalog_id(mut self, catalog_id: impl Into<String>) -> Self {
        self.catalog_id = Some(catalog_id.into());
        self
    }

    #[must_use]
    pub fn with_effect_rule(mut self, effect_rule_id: impl Into<String>) -> Self {
        self.effect_rule_id = Some(effect_rule_id.into());
        self
    }

    #[must_use]
    pub const fn with_influence_cost(mut self, influence_cost: u16) -> Self {
        self.influence_cost = Some(influence_cost);
        self
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
    pub const fn kind(&self) -> EffectEntityKind {
        self.kind
    }

    #[must_use]
    pub fn catalog_id(&self) -> Option<&str> {
        self.catalog_id.as_deref()
    }

    #[must_use]
    pub const fn owner_position(&self) -> Option<u8> {
        self.owner_position
    }

    #[must_use]
    pub fn effect_rule_id(&self) -> Option<&str> {
        self.effect_rule_id.as_deref()
    }

    #[must_use]
    pub const fn influence_cost(&self) -> Option<u16> {
        self.influence_cost
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectEntityPlacement {
    entity: EffectEntity,
    zone: EffectZone,
}

impl EffectEntityPlacement {
    #[must_use]
    pub const fn new(entity: EffectEntity, zone: EffectZone) -> Self {
        Self { entity, zone }
    }

    #[must_use]
    pub const fn entity(&self) -> &EffectEntity {
        &self.entity
    }

    #[must_use]
    pub const fn zone(&self) -> EffectZone {
        self.zone
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectWorld {
    zones: BTreeMap<EffectZone, Vec<EffectEntity>>,
}

impl EffectWorld {
    #[must_use]
    pub fn new(placements: Vec<EffectEntityPlacement>) -> Self {
        let mut zones = BTreeMap::<EffectZone, Vec<EffectEntity>>::new();
        for placement in placements {
            zones
                .entry(placement.zone)
                .or_default()
                .push(placement.entity);
        }
        Self { zones }
    }

    #[must_use]
    pub fn entities_in(&self, zone: EffectZone) -> &[EffectEntity] {
        self.zones.get(&zone).map_or(&[], Vec::as_slice)
    }

    pub fn entities(&self) -> impl Iterator<Item = (EffectZone, &EffectEntity)> {
        self.zones
            .iter()
            .flat_map(|(zone, entities)| entities.iter().map(move |entity| (*zone, entity)))
    }

    pub fn entity_ids(&self) -> impl Iterator<Item = &str> {
        self.entities().map(|(_, entity)| entity.id())
    }

    #[must_use]
    pub fn entity(&self, id: &str) -> Option<(EffectZone, &EffectEntity)> {
        self.entities().find(|(_, entity)| entity.id == id)
    }

    #[must_use]
    pub fn entity_zone(&self, id: &str) -> Option<EffectZone> {
        self.entity(id).map(|(zone, _)| zone)
    }

    #[must_use]
    pub fn hero_resource(&self, position: u8, resource: EffectResource) -> Option<u16> {
        self.entities_in(EffectZone::Heroes)
            .iter()
            .find(|entity| entity.owner_position == Some(position))
            .map(|entity| entity.resource(resource))
    }

    pub(crate) fn entity_mut(&mut self, id: &str) -> Option<(EffectZone, &mut EffectEntity)> {
        self.zones.iter_mut().find_map(|(zone, entities)| {
            entities
                .iter_mut()
                .find(|entity| entity.id == id)
                .map(|entity| (*zone, entity))
        })
    }

    pub(crate) fn move_to_back(
        &mut self,
        id: &str,
        expected_from: EffectZone,
        to: EffectZone,
        destination_owner: Option<u8>,
    ) -> Result<(), EffectExecutionError> {
        let source = self
            .zones
            .get_mut(&expected_from)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let index = source
            .iter()
            .position(|entity| entity.id == id)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let mut entity = source.remove(index);
        let current_owner = entity.owner_position;
        if !owner_transition_is_valid(
            entity.kind,
            expected_from,
            to,
            current_owner,
            destination_owner,
        ) {
            source.insert(index, entity);
            return Err(EffectExecutionError::InvalidDefinition);
        }
        entity.owner_position = destination_owner;
        if !entity_is_valid_in_zone(&entity, to) {
            entity.owner_position = current_owner;
            source.insert(index, entity);
            return Err(EffectExecutionError::InvalidDefinition);
        }
        self.zones.entry(to).or_default().push(entity);
        Ok(())
    }

    pub(crate) fn is_valid_for_positions(&self, positions: &[u8]) -> bool {
        let mut ids = self.entity_ids().collect::<Vec<_>>();
        ids.sort_unstable();
        if ids
            .iter()
            .any(|id| id.is_empty() || id.chars().count() > MAX_CHOICE_VALUE_LENGTH)
            || ids.windows(2).any(|pair| pair[0] == pair[1])
            || self.entities().any(|(zone, entity)| {
                entity
                    .owner_position
                    .is_some_and(|position| !positions.contains(&position))
                    || (zone == EffectZone::Heroes && entity.owner_position.is_none())
                    || entity
                        .resources
                        .keys()
                        .any(|resource| !zone.supports_resource(*resource))
                    || !entity_is_valid_in_zone(entity, zone)
            })
        {
            return false;
        }
        positions.iter().all(|position| {
            self.entities_in(EffectZone::Heroes)
                .iter()
                .filter(|entity| entity.owner_position == Some(*position))
                .count()
                == 1
        })
    }
}

fn entity_is_valid_in_zone(entity: &EffectEntity, zone: EffectZone) -> bool {
    match entity.kind {
        EffectEntityKind::Generic => true,
        EffectEntityKind::Hero => {
            zone == EffectZone::Heroes
                && entity.owner_position.is_some()
                && entity.catalog_id.is_none()
                && entity.effect_rule_id.is_none()
                && entity.influence_cost.is_none()
        }
        EffectEntityKind::StarterCard => {
            matches!(
                zone,
                EffectZone::HeroDrawPile
                    | EffectZone::HeroHand
                    | EffectZone::HeroPlayArea
                    | EffectZone::HeroDiscardPile
            ) && entity.owner_position.is_some()
                && entity
                    .catalog_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                && entity
                    .effect_rule_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                && entity.influence_cost.is_none()
        }
        EffectEntityKind::HogwartsCard => {
            matches!(
                zone,
                EffectZone::HogwartsDeck
                    | EffectZone::Market
                    | EffectZone::HeroDrawPile
                    | EffectZone::HeroHand
                    | EffectZone::HeroPlayArea
                    | EffectZone::HeroDiscardPile
            ) && entity
                .catalog_id
                .as_deref()
                .is_some_and(|id| !id.is_empty())
                && entity
                    .effect_rule_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                && entity.influence_cost.is_some()
                && (matches!(zone, EffectZone::HogwartsDeck | EffectZone::Market)
                    == entity.owner_position.is_none())
        }
        EffectEntityKind::Villain => {
            matches!(zone, EffectZone::VillainDeck | EffectZone::ActiveVillains)
                && entity.owner_position.is_none()
                && entity
                    .catalog_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                && entity
                    .effect_rule_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                && entity.influence_cost.is_none()
                && entity.resources.contains_key(&EffectResource::Health)
        }
    }
}

fn owner_transition_is_valid(
    kind: EffectEntityKind,
    from: EffectZone,
    to: EffectZone,
    current: Option<u8>,
    destination: Option<u8>,
) -> bool {
    match kind {
        EffectEntityKind::HogwartsCard
            if matches!(from, EffectZone::Market | EffectZone::HogwartsDeck)
                && matches!(
                    to,
                    EffectZone::HeroDrawPile
                        | EffectZone::HeroHand
                        | EffectZone::HeroPlayArea
                        | EffectZone::HeroDiscardPile
                ) =>
        {
            current.is_none() && destination.is_some()
        }
        EffectEntityKind::HogwartsCard
            if matches!(from, EffectZone::HogwartsDeck | EffectZone::Market)
                && matches!(to, EffectZone::HogwartsDeck | EffectZone::Market) =>
        {
            current.is_none() && destination.is_none()
        }
        EffectEntityKind::StarterCard | EffectEntityKind::HogwartsCard => {
            current.is_some() && current == destination
        }
        EffectEntityKind::Villain => current.is_none() && destination.is_none(),
        EffectEntityKind::Generic | EffectEntityKind::Hero => current == destination,
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
    pub cause: String,
    pub responsible_position: u8,
    pub kind: PendingEffectChoiceKind,
    pub options: Vec<String>,
    pub min: u16,
    pub max: u16,
    pub continuation: EffectContinuation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectContinuation {
    pub choice_cursor: EffectCursor,
    pub queue: Vec<QueuedEffect>,
    pub steps_completed: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectCursor {
    pub rule_id: String,
    pub path: Vec<EffectPathSegment>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectPathSegment {
    ChoiceOption(u16),
    ConditionThen,
    ConditionOtherwise,
    RepeatEffect,
    RollOutcome(u16),
    SequenceEffect(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedEffect {
    Definition {
        cursor: EffectCursor,
        actor_position: u8,
    },
    EffectChoice {
        cursor: EffectCursor,
        responsible_position: u8,
    },
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
    InvalidChoice,
    InvalidDefinition,
    InvalidRoll,
    InvalidTargetSelection,
    StepLimitExceeded,
    UnaffordableCost,
}

pub trait EffectRoller {
    fn roll(&mut self, die: EffectDie) -> Option<u8>;
}

struct EffectExecutor<'a> {
    world: &'a mut EffectWorld,
    rules: &'a [EffectRule],
    roller: &'a mut dyn EffectRoller,
    target_bindings: &'a [EffectTargetBinding],
    require_named_target_bindings: bool,
    queue: VecDeque<QueuedEffect>,
    outcomes: Vec<EffectOutcome>,
    rolls_consumed: u64,
    steps: usize,
}

impl EffectCursor {
    fn root(rule_id: &str) -> Self {
        Self {
            rule_id: rule_id.to_owned(),
            path: Vec::new(),
        }
    }

    fn child(&self, segment: EffectPathSegment) -> Self {
        let mut child = self.clone();
        child.path.push(segment);
        child
    }
}

impl EffectExecutor<'_> {
    fn run(mut self) -> Result<EffectResolution, EffectExecutionError> {
        while let Some(queued) = self.queue.pop_front() {
            self.steps += 1;
            if self.steps > MAX_EXECUTION_STEPS {
                return Err(EffectExecutionError::StepLimitExceeded);
            }
            let stop = match queued {
                QueuedEffect::Definition {
                    cursor,
                    actor_position,
                } => {
                    let effect = effect_at_cursor(self.rules, &cursor)
                        .ok_or(EffectExecutionError::InvalidDefinition)?
                        .clone();
                    self.execute_definition(cursor, actor_position, effect)?
                }
                QueuedEffect::EffectChoice {
                    cursor,
                    responsible_position,
                } => {
                    let EffectDefinition::Choice { options, .. } =
                        effect_at_cursor(self.rules, &cursor)
                            .ok_or(EffectExecutionError::InvalidDefinition)?
                    else {
                        return Err(EffectExecutionError::InvalidDefinition);
                    };
                    self.execute_choice(&cursor, responsible_position, options.len())?
                }
            };
            if let Some(stop) = stop {
                return Ok(self.finish(stop));
            }
        }
        Ok(self.finish(EffectStop::Stable))
    }

    fn execute_definition(
        &mut self,
        cursor: EffectCursor,
        actor_position: u8,
        effect: EffectDefinition,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        match effect {
            EffectDefinition::Apply { target, operation } => {
                self.execute_apply(&cursor, actor_position, &target, &operation)
            }
            EffectDefinition::Choice { audience, options } => match audience {
                EffectChoiceAudience::Actor => {
                    self.execute_choice(&cursor, actor_position, options.len())
                }
                EffectChoiceAudience::EachHero => {
                    let positions = hero_positions(self.world);
                    let Some((&first, remaining)) = positions.split_first() else {
                        return Err(EffectExecutionError::InvalidDefinition);
                    };
                    for responsible_position in remaining.iter().rev() {
                        self.queue.push_front(QueuedEffect::EffectChoice {
                            cursor: cursor.clone(),
                            responsible_position: *responsible_position,
                        });
                    }
                    self.execute_choice(&cursor, first, options.len())
                }
            },
            EffectDefinition::Condition {
                condition,
                then: _,
                otherwise,
            } => {
                if !condition_is_valid(&condition) {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                let selected = if condition_is_true(self.world, actor_position, &condition) {
                    Some(EffectPathSegment::ConditionThen)
                } else {
                    otherwise.map(|_| EffectPathSegment::ConditionOtherwise)
                };
                if let Some(selected) = selected {
                    self.queue.push_front(QueuedEffect::Definition {
                        cursor: cursor.child(selected),
                        actor_position,
                    });
                }
                Ok(None)
            }
            EffectDefinition::NoOp => {
                self.outcomes.push(EffectOutcome::NoOp {
                    rule_id: cursor.rule_id,
                    reason: EffectNoOpReason::Explicit,
                });
                Ok(None)
            }
            EffectDefinition::Repeat { times, effect: _ } => {
                if times == 0 {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                let child = cursor.child(EffectPathSegment::RepeatEffect);
                for _ in 0..times {
                    self.queue.push_front(QueuedEffect::Definition {
                        cursor: child.clone(),
                        actor_position,
                    });
                }
                Ok(None)
            }
            EffectDefinition::Roll {
                die,
                outcomes: roll_outcomes,
            } => self.execute_roll(&cursor, actor_position, die, &roll_outcomes),
            EffectDefinition::Sequence { effects } => {
                if effects.is_empty() {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                for index in (0..effects.len()).rev() {
                    let index = u16::try_from(index)
                        .map_err(|_| EffectExecutionError::InvalidDefinition)?;
                    self.queue.push_front(QueuedEffect::Definition {
                        cursor: cursor.child(EffectPathSegment::SequenceEffect(index)),
                        actor_position,
                    });
                }
                Ok(None)
            }
            EffectDefinition::Terminal { outcome } => {
                self.outcomes.push(EffectOutcome::Terminal {
                    rule_id: cursor.rule_id,
                    outcome,
                });
                Ok(Some(EffectStop::Terminal(outcome)))
            }
        }
    }

    fn execute_apply(
        &mut self,
        cursor: &EffectCursor,
        actor_position: u8,
        target: &EffectSelector,
        operation: &EffectOperation,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if !selector_is_valid(target) || !operation_is_valid_for_zone(operation, target.zone) {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        let candidates = eligible_entity_ids(self.world, actor_position, target);
        if target.max == 0 {
            self.outcomes.push(EffectOutcome::NoOp {
                rule_id: cursor.rule_id.clone(),
                reason: EffectNoOpReason::ZeroCardinality,
            });
        } else if candidates.len() < usize::from(target.min) {
            self.outcomes.push(EffectOutcome::NoOp {
                rule_id: cursor.rule_id.clone(),
                reason: EffectNoOpReason::NoEligibleTarget,
            });
        } else if let Some(selector_id) = target.id.as_deref()
            && let Some(binding) = self
                .target_bindings
                .iter()
                .find(|binding| binding.selector_id == selector_id)
        {
            let selected = binding.target_ids.iter().collect::<BTreeSet<_>>();
            if selected.len() != binding.target_ids.len()
                || binding.target_ids.len() < usize::from(target.min)
                || binding.target_ids.len() > usize::from(target.max)
                || binding
                    .target_ids
                    .iter()
                    .any(|selected| !candidates.iter().any(|candidate| candidate == selected))
            {
                return Err(EffectExecutionError::InvalidTargetSelection);
            }
            for entity_id in &binding.target_ids {
                apply_operation(
                    self.world,
                    entity_id,
                    &cursor.rule_id,
                    operation,
                    &mut self.outcomes,
                )?;
            }
        } else if target.id.is_some() && self.require_named_target_bindings {
            return Err(EffectExecutionError::InvalidTargetSelection);
        } else if candidates.len() > usize::from(target.max) {
            return Ok(Some(EffectStop::Choice(PendingEffectChoice {
                id: target
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("{}:target:{}", cursor.rule_id, self.steps - 1)),
                cause: cursor.rule_id.clone(),
                responsible_position: actor_position,
                kind: PendingEffectChoiceKind::Target,
                options: candidates,
                min: target.min,
                max: target.max,
                continuation: self.continuation(cursor),
            })));
        } else {
            for entity_id in candidates {
                apply_operation(
                    self.world,
                    &entity_id,
                    &cursor.rule_id,
                    operation,
                    &mut self.outcomes,
                )?;
            }
        }
        Ok(None)
    }

    fn execute_choice(
        &self,
        cursor: &EffectCursor,
        responsible_position: u8,
        option_count: usize,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if option_count < 2 {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        Ok(Some(EffectStop::Choice(PendingEffectChoice {
            id: format!("{}:effect:{}", cursor.rule_id, self.steps - 1),
            cause: cursor.rule_id.clone(),
            responsible_position,
            kind: PendingEffectChoiceKind::Effect,
            options: (1..=option_count)
                .map(|index| format!("option:{index}"))
                .collect(),
            min: 1,
            max: 1,
            continuation: self.continuation(cursor),
        })))
    }

    fn execute_roll(
        &mut self,
        cursor: &EffectCursor,
        actor_position: u8,
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
            rule_id: cursor.rule_id.clone(),
            die,
            result,
        });
        self.queue.push_front(QueuedEffect::Definition {
            cursor: cursor.child(EffectPathSegment::RollOutcome(u16::from(result - 1))),
            actor_position,
        });
        Ok(None)
    }

    fn continuation(&self, choice_cursor: &EffectCursor) -> EffectContinuation {
        EffectContinuation {
            choice_cursor: choice_cursor.clone(),
            queue: self.queue.iter().cloned().collect(),
            steps_completed: self.steps,
        }
    }

    fn finish(self, stop: EffectStop) -> EffectResolution {
        EffectResolution {
            outcomes: self.outcomes,
            stop,
            rolls_consumed: self.rolls_consumed,
        }
    }
}

fn effect_at_cursor<'a>(
    rules: &'a [EffectRule],
    cursor: &EffectCursor,
) -> Option<&'a EffectDefinition> {
    let rule = rules.iter().find(|rule| rule.id == cursor.rule_id)?;
    let mut effect = &rule.effect;
    for segment in &cursor.path {
        effect = match (effect, segment) {
            (EffectDefinition::Choice { options, .. }, EffectPathSegment::ChoiceOption(index)) => {
                options.get(usize::from(*index))?
            }
            (EffectDefinition::Condition { then, .. }, EffectPathSegment::ConditionThen) => then,
            (
                EffectDefinition::Condition {
                    otherwise: Some(otherwise),
                    ..
                },
                EffectPathSegment::ConditionOtherwise,
            ) => otherwise,
            (EffectDefinition::Repeat { effect, .. }, EffectPathSegment::RepeatEffect) => effect,
            (EffectDefinition::Roll { outcomes, .. }, EffectPathSegment::RollOutcome(index)) => {
                outcomes.get(usize::from(*index))?
            }
            (EffectDefinition::Sequence { effects }, EffectPathSegment::SequenceEffect(index)) => {
                effects.get(usize::from(*index))?
            }
            _ => return None,
        };
    }
    Some(effect)
}

fn hero_positions(world: &EffectWorld) -> Vec<u8> {
    let mut positions = world
        .entities_in(EffectZone::Heroes)
        .iter()
        .filter_map(|entity| entity.owner_position)
        .collect::<Vec<_>>();
    positions.sort_unstable();
    positions.dedup();
    positions
}

pub(crate) fn effect_choice_selection_is_valid(
    choice: &PendingEffectChoice,
    selected_options: &[String],
) -> bool {
    let selected = selected_options
        .iter()
        .collect::<std::collections::BTreeSet<_>>();
    selected.len() == selected_options.len()
        && (usize::from(choice.min)..=usize::from(choice.max)).contains(&selected.len())
        && selected_options
            .iter()
            .all(|option| choice.options.contains(option))
}

pub(crate) fn normalize_effect_choice_selection(
    choice: &PendingEffectChoice,
    selected_options: &[String],
) -> Option<Vec<String>> {
    if !effect_choice_selection_is_valid(choice, selected_options) {
        return None;
    }
    Some(
        choice
            .options
            .iter()
            .filter(|option| selected_options.contains(option))
            .cloned()
            .collect(),
    )
}

pub(crate) fn resume_effects(
    world: &mut EffectWorld,
    pending: &PendingEffectChoice,
    selected_options: &[String],
    rules: &[EffectRule],
    roller: &mut dyn EffectRoller,
) -> Result<EffectResolution, EffectExecutionError> {
    if !effect_rules_are_valid(rules) {
        return Err(EffectExecutionError::InvalidDefinition);
    }
    if !effect_choice_selection_is_valid(pending, selected_options)
        || pending.cause != pending.continuation.choice_cursor.rule_id
        || pending.continuation.steps_completed > MAX_EXECUTION_STEPS
    {
        return Err(EffectExecutionError::InvalidChoice);
    }
    let definition = effect_at_cursor(rules, &pending.continuation.choice_cursor)
        .ok_or(EffectExecutionError::InvalidChoice)?;
    let mut queue = pending
        .continuation
        .queue
        .iter()
        .cloned()
        .collect::<VecDeque<_>>();
    let mut outcomes = Vec::new();

    match (pending.kind, definition) {
        (PendingEffectChoiceKind::Effect, EffectDefinition::Choice { options, .. }) => {
            if pending.options.len() != options.len() || selected_options.len() != 1 {
                return Err(EffectExecutionError::InvalidChoice);
            }
            let selected_index = pending
                .options
                .iter()
                .position(|option| option == &selected_options[0])
                .ok_or(EffectExecutionError::InvalidChoice)?;
            let selected_index =
                u16::try_from(selected_index).map_err(|_| EffectExecutionError::InvalidChoice)?;
            queue.push_front(QueuedEffect::Definition {
                cursor: pending
                    .continuation
                    .choice_cursor
                    .child(EffectPathSegment::ChoiceOption(selected_index)),
                actor_position: pending.responsible_position,
            });
        }
        (PendingEffectChoiceKind::Target, EffectDefinition::Apply { target, operation }) => {
            let candidates = eligible_entity_ids(world, pending.responsible_position, target);
            if candidates != pending.options {
                return Err(EffectExecutionError::InvalidChoice);
            }
            for option in &pending.options {
                if !selected_options.contains(option) {
                    continue;
                }
                apply_operation(world, option, &pending.cause, operation, &mut outcomes)?;
            }
        }
        _ => return Err(EffectExecutionError::InvalidChoice),
    }

    EffectExecutor {
        world,
        rules,
        roller,
        target_bindings: &[],
        require_named_target_bindings: false,
        queue,
        outcomes,
        rolls_consumed: 0,
        steps: pending.continuation.steps_completed,
    }
    .run()
}

#[must_use]
pub fn effect_action_is_affordable(
    world: &EffectWorld,
    actor_position: u8,
    rules: &[EffectRule],
    trigger: EffectTrigger,
) -> bool {
    if !effect_rules_are_valid(rules) {
        return false;
    }
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
    execute_effects_with_targets(world, actor_position, rules, trigger, &[], false, roller)
}

pub(crate) fn execute_effect_rule(
    world: &mut EffectWorld,
    actor_position: u8,
    rule: &EffectRule,
    target_bindings: &[EffectTargetBinding],
    roller: &mut dyn EffectRoller,
) -> Result<EffectResolution, EffectExecutionError> {
    execute_effects_with_targets(
        world,
        actor_position,
        std::slice::from_ref(rule),
        EffectTrigger::Manual,
        target_bindings,
        true,
        roller,
    )
}

fn execute_effects_with_targets(
    world: &mut EffectWorld,
    actor_position: u8,
    rules: &[EffectRule],
    trigger: EffectTrigger,
    target_bindings: &[EffectTargetBinding],
    require_named_target_bindings: bool,
    roller: &mut dyn EffectRoller,
) -> Result<EffectResolution, EffectExecutionError> {
    if !effect_rules_are_valid(rules)
        || target_bindings.iter().any(|binding| {
            binding.selector_id.is_empty()
                || !binding
                    .target_ids
                    .iter()
                    .all(|target_id| !target_id.is_empty())
        })
        || target_bindings
            .iter()
            .map(|binding| &binding.selector_id)
            .collect::<BTreeSet<_>>()
            .len()
            != target_bindings.len()
    {
        return Err(EffectExecutionError::InvalidDefinition);
    }
    if !effect_action_is_affordable(world, actor_position, rules, trigger) {
        return Err(EffectExecutionError::UnaffordableCost);
    }

    let outcomes = pay_costs(world, actor_position, rules, trigger)?;
    let queue = rules
        .iter()
        .filter(|rule| rule.trigger == trigger)
        .map(|rule| QueuedEffect::Definition {
            cursor: EffectCursor::root(&rule.id),
            actor_position,
        })
        .collect::<VecDeque<_>>();
    EffectExecutor {
        world,
        rules,
        roller,
        target_bindings,
        require_named_target_bindings,
        queue,
        outcomes,
        rolls_consumed: 0,
        steps: 0,
    }
    .run()
}

fn effect_rules_are_valid(rules: &[EffectRule]) -> bool {
    let mut ids = BTreeSet::new();
    rules.iter().all(|rule| {
        !rule.id.is_empty()
            && rule.id.chars().count() <= MAX_RUNTIME_RULE_ID_LENGTH
            && ids.insert(rule.id.as_str())
            && rule
                .cost
                .iter()
                .all(|cost| cost.amount > 0 && cost.resource != EffectResource::Control)
    })
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
        let hero_id = world
            .entities_in(EffectZone::Heroes)
            .iter()
            .find(|entity| entity.owner_position == Some(actor_position))
            .map(|entity| entity.id.clone())
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let (_, hero) = world
            .entity_mut(&hero_id)
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
                let (current_zone, _) = world
                    .entity(target_id)
                    .ok_or(EffectExecutionError::InvalidDefinition)?;
                if current_zone != *from || from == to || !from.is_card_zone() || !to.is_card_zone()
                {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                world.move_to_back(target_id, *from, *to, *target_position)?;
            }
            EffectOutcome::ResourceChanged {
                target_id,
                target_position,
                resource,
                before,
                after,
                ..
            } => {
                let (zone, entity) = world
                    .entity_mut(target_id)
                    .ok_or(EffectExecutionError::InvalidDefinition)?;
                if entity.owner_position != *target_position
                    || !zone.supports_resource(*resource)
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
    participant_positions: &[u8],
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
            terminal_count == 0 && choice.is_valid_for_positions(participant_positions)
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
    pub(crate) fn is_valid_for_positions(&self, participant_positions: &[u8]) -> bool {
        let options = self
            .options
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        participant_positions.contains(&self.responsible_position)
            && !self.id.is_empty()
            && self.id.chars().count() <= MAX_CHOICE_VALUE_LENGTH
            && !self.cause.is_empty()
            && self.cause.chars().count() <= MAX_CHOICE_VALUE_LENGTH
            && self.options.len() >= 2
            && self.options.len() <= MAX_CHOICE_OPTIONS
            && options.len() == self.options.len()
            && options.iter().all(|option| {
                !option.is_empty() && option.chars().count() <= MAX_CHOICE_VALUE_LENGTH
            })
            && self.min <= self.max
            && self.max <= MAX_CHOICE_SELECTIONS
            && usize::from(self.max) <= self.options.len()
            && match self.kind {
                PendingEffectChoiceKind::Effect => self.min == 1 && self.max == 1,
                PendingEffectChoiceKind::Target => {
                    self.max > 0 && usize::from(self.max) < self.options.len()
                }
            }
            && self.continuation.steps_completed > 0
            && self.continuation.steps_completed <= MAX_EXECUTION_STEPS
            && self.continuation.choice_cursor.rule_id == self.cause
            && self.continuation.choice_cursor.is_structurally_valid()
            && self.continuation.queue.len() <= MAX_EXECUTION_STEPS
            && self
                .continuation
                .queue
                .iter()
                .all(|queued| queued.is_valid_for_positions(participant_positions))
    }
}

impl EffectCursor {
    fn is_structurally_valid(&self) -> bool {
        !self.rule_id.is_empty() && self.path.len() <= MAX_EXECUTION_STEPS
    }
}

impl QueuedEffect {
    fn is_valid_for_positions(&self, participant_positions: &[u8]) -> bool {
        match self {
            Self::Definition {
                cursor,
                actor_position,
            } => cursor.is_structurally_valid() && participant_positions.contains(actor_position),
            Self::EffectChoice {
                cursor,
                responsible_position,
            } => {
                cursor.is_structurally_valid()
                    && participant_positions.contains(responsible_position)
            }
        }
    }
}

fn selector_is_valid(selector: &EffectSelector) -> bool {
    selector.id.as_deref().is_none_or(|id| !id.is_empty())
        && selector.min <= selector.max
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

pub(crate) fn eligible_entity_ids(
    world: &EffectWorld,
    actor_position: u8,
    selector: &EffectSelector,
) -> Vec<String> {
    world
        .entities_in(selector.zone)
        .iter()
        .filter(|entity| {
            selector.owner == EffectTargetOwner::Any
                || entity.owner_position == Some(actor_position)
        })
        .filter(|entity| {
            selector
                .eligibility
                .iter()
                .all(|eligibility| match eligibility {
                    EffectEligibility::ResourceAtLeast { resource, amount } => {
                        entity.resource(*resource) >= *amount
                    }
                })
        })
        .map(|entity| entity.id.clone())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AtomicTargetSlot {
    pub selector_id: String,
    pub min: u16,
    pub max: u16,
    pub target_ids: Vec<String>,
}

pub(crate) fn atomic_manual_target_slots(
    world: &EffectWorld,
    actor_position: u8,
    rule: &EffectRule,
) -> Option<Vec<AtomicTargetSlot>> {
    if rule.trigger != EffectTrigger::Manual
        || rule.id.is_empty()
        || rule
            .cost
            .iter()
            .any(|cost| cost.amount == 0 || cost.resource == EffectResource::Control)
    {
        return None;
    }
    let mut slots = Vec::new();
    collect_atomic_target_slots(world, actor_position, &rule.effect, &mut slots)?;
    Some(slots)
}

fn collect_atomic_target_slots(
    world: &EffectWorld,
    actor_position: u8,
    definition: &EffectDefinition,
    slots: &mut Vec<AtomicTargetSlot>,
) -> Option<bool> {
    match definition {
        EffectDefinition::Apply { target, operation } => {
            if !selector_is_valid(target) || !operation_is_valid_for_zone(operation, target.zone) {
                return None;
            }
            let target_ids = eligible_entity_ids(world, actor_position, target);
            if target.max == 0 || target_ids.len() < usize::from(target.min) {
                return Some(true);
            }
            let Some(selector_id) = target.id.as_ref() else {
                return (target_ids.len() <= usize::from(target.max)).then_some(true);
            };
            let slot = AtomicTargetSlot {
                selector_id: selector_id.clone(),
                min: target.min,
                max: target
                    .max
                    .min(u16::try_from(target_ids.len()).unwrap_or(u16::MAX)),
                target_ids,
            };
            if let Some(existing) = slots
                .iter()
                .find(|existing| existing.selector_id == slot.selector_id)
            {
                (existing == &slot).then_some(true)
            } else {
                slots.push(slot);
                Some(true)
            }
        }
        EffectDefinition::Choice { .. } | EffectDefinition::Terminal { .. } => Some(false),
        EffectDefinition::Condition {
            condition,
            then,
            otherwise,
        } => {
            if !condition_is_valid(condition) {
                return None;
            }
            let then_continues = collect_atomic_target_slots(world, actor_position, then, slots)?;
            let otherwise_continues = otherwise.as_deref().map_or(Some(true), |otherwise| {
                collect_atomic_target_slots(world, actor_position, otherwise, slots)
            })?;
            Some(then_continues || otherwise_continues)
        }
        EffectDefinition::NoOp => Some(true),
        EffectDefinition::Repeat { times, effect } => (*times > 0)
            .then(|| collect_atomic_target_slots(world, actor_position, effect, slots))?,
        EffectDefinition::Roll { die, outcomes } => {
            if outcomes.len() != usize::from(die.sides()) {
                return None;
            }
            let mut any_continues = false;
            for outcome in outcomes {
                any_continues |=
                    collect_atomic_target_slots(world, actor_position, outcome, slots)?;
            }
            Some(any_continues)
        }
        EffectDefinition::Sequence { effects } => {
            if effects.is_empty() {
                return None;
            }
            let mut can_continue = true;
            for effect in effects {
                if !can_continue {
                    break;
                }
                can_continue = collect_atomic_target_slots(world, actor_position, effect, slots)?;
            }
            Some(can_continue)
        }
    }
}

fn condition_is_true(world: &EffectWorld, actor_position: u8, condition: &EffectCondition) -> bool {
    match condition {
        EffectCondition::HasEligibleTarget { target } => {
            eligible_entity_ids(world, actor_position, target).len() >= usize::from(target.min)
        }
        EffectCondition::ResourceAtLeast {
            target,
            resource,
            amount,
        } => {
            let candidates = eligible_entity_ids(world, actor_position, target);
            candidates.len() >= usize::from(target.min)
                && candidates.into_iter().any(|entity_id| {
                    world
                        .entity(&entity_id)
                        .is_some_and(|(_, entity)| entity.resource(*resource) >= *amount)
                })
        }
    }
}

fn apply_operation(
    world: &mut EffectWorld,
    entity_id: &str,
    rule_id: &str,
    operation: &EffectOperation,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<(), EffectExecutionError> {
    match operation {
        EffectOperation::Discard => {
            let (from, entity) = world
                .entity(entity_id)
                .ok_or(EffectExecutionError::InvalidDefinition)?;
            let target_id = entity.id.clone();
            let target_position = entity.owner_position;
            world.move_to_back(
                &target_id,
                from,
                EffectZone::HeroDiscardPile,
                target_position,
            )?;
            outcomes.push(EffectOutcome::Moved {
                rule_id: rule_id.to_owned(),
                target_id,
                target_position,
                from,
                to: EffectZone::HeroDiscardPile,
            });
        }
        EffectOperation::ModifyResource { resource, amount } => {
            let (_, entity) = world
                .entity_mut(entity_id)
                .ok_or(EffectExecutionError::InvalidDefinition)?;
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
            let (from, entity) = world
                .entity(entity_id)
                .ok_or(EffectExecutionError::InvalidDefinition)?;
            let target_id = entity.id.clone();
            let target_position = entity.owner_position;
            world.move_to_back(&target_id, from, *to, target_position)?;
            outcomes.push(EffectOutcome::Moved {
                rule_id: rule_id.to_owned(),
                target_id,
                target_position,
                from,
                to: *to,
            });
        }
    }
    Ok(())
}
