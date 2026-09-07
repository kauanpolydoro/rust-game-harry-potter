use std::collections::{BTreeMap, BTreeSet, VecDeque};

mod reactions;
pub(crate) use reactions::can_acquire_on_deck;

const MAX_EXECUTION_STEPS: usize = 4_096;
const MAX_CHOICE_OPTIONS: usize = 4_096;
const MAX_CHOICE_SELECTIONS: u16 = 32;
const MAX_CHOICE_VALUE_LENGTH: usize = 256;
const MAX_RUNTIME_RULE_ID_LENGTH: usize = 244;
const MAX_EFFECT_OUTCOMES: usize = 4_096;
pub const HERO_MAX_HEALTH: u16 = 10;
const STUN_RULE_ID: &str = "system:stunned";
pub const MAX_EFFECT_PATH_DEPTH: usize = 32;
pub const MAX_EFFECT_BRANCH_INDEX: u16 = 1_023;
pub const MAX_EFFECT_ROLL_INDEX: u16 = 7;

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
    LocationDeck,
    LocationDiscard,
    Market,
    VillainDeck,
    VillainDiscard,
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
                | Self::VillainDiscard
        )
    }

    const fn supports_resource(self, resource: EffectResource) -> bool {
        matches!(
            (self, resource),
            (
                Self::Heroes,
                EffectResource::Attack | EffectResource::Health | EffectResource::Influence
            ) | (
                Self::ActiveVillains | Self::VillainDeck | Self::VillainDiscard,
                EffectResource::Health
            ) | (
                Self::ActiveLocation | Self::LocationDeck | Self::LocationDiscard,
                EffectResource::Control
            )
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
    GainAttackPerAllyPlayed {
        amount: u8,
    },
    Discard,
    PreventDrawing,
    Draw {
        amount: u8,
    },
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
    TopDeckAcquisition {
        card_type: EffectCardType,
    },
    CardType {
        card_type: EffectCardType,
    },
    HandDamageLimit {
        maximum: u8,
    },
    Reaction {
        trigger: EffectReactionTrigger,
        effect: Box<Self>,
    },
    RevealDarkArts,
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
pub enum EffectReactionTrigger {
    OwnerPlaysAlly,
    ControlAdded,
    HeroForcedDiscard,
    SelfForcedDiscard,
    OwnerDefeatsVillain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectCardType {
    Ally,
    Item,
    Spell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectChoiceAudience {
    Actor,
    EachHero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectTrigger {
    DarkArts,
    DarkArtsCompleted,
    Manual,
    VillainReward,
    Villains,
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
    pub order: u16,
    pub cost: Vec<EffectResourceCost>,
    pub effect: EffectDefinition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectEntityKind {
    DarkArts,
    Generic,
    Hero,
    HogwartsCard,
    Location,
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
    reward_rule_id: Option<String>,
    influence_cost: Option<u16>,
    resources: BTreeMap<EffectResource, u16>,
    resource_limits: BTreeMap<EffectResource, u16>,
    dark_arts_count: Option<u8>,
    drawing_blocked: bool,
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
            reward_rule_id: None,
            influence_cost: None,
            resources: BTreeMap::new(),
            resource_limits: BTreeMap::new(),
            dark_arts_count: None,
            drawing_blocked: false,
        }
    }

    #[must_use]
    pub fn hero(position: u8) -> Self {
        Self {
            kind: EffectEntityKind::Hero,
            resource_limits: BTreeMap::from([(EffectResource::Health, HERO_MAX_HEALTH)]),
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
            reward_rule_id: None,
            influence_cost,
            resources: BTreeMap::new(),
            resource_limits: BTreeMap::new(),
            dark_arts_count: None,
            drawing_blocked: false,
        }
    }

    #[must_use]
    pub fn location(
        id: impl Into<String>,
        catalog_id: impl Into<String>,
        effect_rule_id: impl Into<String>,
        control_limit: u16,
        dark_arts_count: u8,
    ) -> Self {
        Self {
            id: id.into(),
            kind: EffectEntityKind::Location,
            catalog_id: Some(catalog_id.into()),
            owner_position: None,
            effect_rule_id: Some(effect_rule_id.into()),
            reward_rule_id: None,
            influence_cost: None,
            resources: BTreeMap::from([(EffectResource::Control, 0)]),
            resource_limits: BTreeMap::from([(EffectResource::Control, control_limit)]),
            dark_arts_count: Some(dark_arts_count),
            drawing_blocked: false,
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
            reward_rule_id: None,
            influence_cost: None,
            resources: BTreeMap::from([(EffectResource::Health, health)]),
            resource_limits: BTreeMap::new(),
            dark_arts_count: None,
            drawing_blocked: false,
        }
    }

    #[must_use]
    pub fn with_kind(mut self, kind: EffectEntityKind) -> Self {
        self.kind = kind;
        if kind == EffectEntityKind::Hero {
            self.resource_limits
                .insert(EffectResource::Health, HERO_MAX_HEALTH);
        }
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
    pub fn with_reward_rule(mut self, reward_rule_id: impl Into<String>) -> Self {
        self.reward_rule_id = Some(reward_rule_id.into());
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
    pub fn reward_rule_id(&self) -> Option<&str> {
        self.reward_rule_id.as_deref()
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
    pub fn resource_limits(&self) -> &BTreeMap<EffectResource, u16> {
        &self.resource_limits
    }

    #[must_use]
    pub fn resource(&self, resource: EffectResource) -> u16 {
        self.resources.get(&resource).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn resource_limit(&self, resource: EffectResource) -> Option<u16> {
        self.resource_limits.get(&resource).copied()
    }

    #[must_use]
    pub const fn dark_arts_count(&self) -> Option<u8> {
        self.dark_arts_count
    }

    #[must_use]
    pub const fn drawing_blocked(&self) -> bool {
        self.drawing_blocked
    }

    #[must_use]
    pub const fn with_drawing_blocked(mut self, blocked: bool) -> Self {
        self.drawing_blocked = blocked;
        self
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

    #[must_use]
    pub fn cards_in_zone(&self, owner_position: u8, zone: EffectZone) -> Vec<&str> {
        self.entities_in(zone)
            .iter()
            .filter(|entity| entity.owner_position == Some(owner_position))
            .map(|entity| entity.id.as_str())
            .collect()
    }

    pub(crate) fn card_ids_in_zone(&self, owner_position: u8, zone: EffectZone) -> Vec<String> {
        self.cards_in_zone(owner_position, zone)
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    pub(crate) fn top_card_id(&self, owner_position: u8, zone: EffectZone) -> Option<String> {
        self.entities_in(zone)
            .iter()
            .rev()
            .find(|entity| entity.owner_position == Some(owner_position))
            .map(|entity| entity.id.clone())
    }

    pub(crate) fn move_card(
        &mut self,
        card_id: &str,
        expected_from: EffectZone,
        to: EffectZone,
    ) -> Result<(), EffectExecutionError> {
        let owner_position = self
            .entity(card_id)
            .filter(|(zone, _)| *zone == expected_from)
            .and_then(|(_, entity)| entity.owner_position)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        self.move_to_back(card_id, expected_from, to, Some(owner_position))
    }

    pub(crate) fn set_card_order(
        &mut self,
        owner_position: u8,
        zone: EffectZone,
        bottom_to_top: &[String],
    ) -> Result<(), EffectExecutionError> {
        if !zone.is_card_zone() || bottom_to_top.len() > usize::from(u16::MAX) {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        let mut current = self.card_ids_in_zone(owner_position, zone);
        let mut supplied = bottom_to_top.to_vec();
        current.sort();
        supplied.sort();
        if current != supplied {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        let entities = self
            .zones
            .get_mut(&zone)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let owned_indices = entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| {
                (entity.owner_position == Some(owner_position)).then_some(index)
            })
            .collect::<Vec<_>>();
        let ordered = bottom_to_top
            .iter()
            .map(|card_id| {
                entities
                    .iter()
                    .find(|entity| entity.id == *card_id)
                    .cloned()
                    .ok_or(EffectExecutionError::InvalidDefinition)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (index, entity) in owned_indices.into_iter().zip(ordered) {
            entities[index] = entity;
        }
        Ok(())
    }

    pub(crate) fn reset_hero_resource(
        &mut self,
        owner_position: u8,
        resource: EffectResource,
        expected_before: u16,
    ) -> Result<(), EffectExecutionError> {
        let hero = self
            .zones
            .get_mut(&EffectZone::Heroes)
            .and_then(|heroes| {
                heroes
                    .iter_mut()
                    .find(|entity| entity.owner_position == Some(owner_position))
            })
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        if hero.resource(resource) != expected_before {
            return Err(EffectExecutionError::InvalidDefinition);
        }
        hero.resources.insert(resource, 0);
        Ok(())
    }

    pub(crate) fn shuffle_pile(
        &mut self,
        zone: EffectZone,
        owner: Option<u8>,
        random: &mut dyn EffectRoller,
    ) -> Result<u64, EffectExecutionError> {
        let entities = self
            .zones
            .get_mut(&zone)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let indices = entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| (entity.owner_position == owner).then_some(index))
            .collect::<Vec<_>>();
        crate::shuffling::fisher_yates(indices.len(), random, |index, selected| {
            entities.swap(indices[index], indices[selected]);
        })?;
        u64::try_from(indices.len().saturating_sub(1))
            .map_err(|_| EffectExecutionError::InvalidDefinition)
    }

    pub(crate) fn restore_drawing(&mut self) -> Vec<u8> {
        let Some(heroes) = self.zones.get_mut(&EffectZone::Heroes) else {
            return Vec::new();
        };
        heroes
            .iter_mut()
            .filter_map(|hero| {
                if !hero.drawing_blocked {
                    return None;
                }
                hero.drawing_blocked = false;
                hero.owner_position
            })
            .collect()
    }

    pub(crate) fn recover_stunned_heroes(
        &mut self,
    ) -> Result<Vec<(u8, u16)>, EffectExecutionError> {
        let heroes = self
            .zones
            .get_mut(&EffectZone::Heroes)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let mut recovered = Vec::new();
        for hero in heroes {
            if hero.resource(EffectResource::Health) != 0 {
                continue;
            }
            let position = hero
                .owner_position
                .ok_or(EffectExecutionError::InvalidDefinition)?;
            let health = hero
                .resource_limit(EffectResource::Health)
                .ok_or(EffectExecutionError::InvalidDefinition)?;
            hero.resources.insert(EffectResource::Health, health);
            recovered.push((position, health));
        }
        Ok(recovered)
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

    pub(crate) fn advance_controlled_location(
        &mut self,
    ) -> Result<Option<(String, Option<String>)>, EffectExecutionError> {
        let active = self.entities_in(EffectZone::ActiveLocation);
        let Some(location) = active.first() else {
            return Ok(None);
        };
        if active.len() != 1
            || location.kind != EffectEntityKind::Location
            || location.resource(EffectResource::Control)
                < location
                    .resource_limit(EffectResource::Control)
                    .ok_or(EffectExecutionError::InvalidDefinition)?
        {
            return Ok(None);
        }
        let location_id = location.id.clone();
        self.move_to_back(
            &location_id,
            EffectZone::ActiveLocation,
            EffectZone::LocationDiscard,
            None,
        )?;
        let next_location_id = self
            .entities_in(EffectZone::LocationDeck)
            .first()
            .map(|location| location.id.clone());
        if let Some(next_location_id) = &next_location_id {
            self.move_to_back(
                next_location_id,
                EffectZone::LocationDeck,
                EffectZone::ActiveLocation,
                None,
            )?;
        }
        Ok(Some((location_id, next_location_id)))
    }

    pub(crate) fn structural_game_outcome(&self) -> Option<EffectGameOutcome> {
        let has_villains = self
            .entities()
            .any(|(_, entity)| entity.kind == EffectEntityKind::Villain);
        let villains_defeated = has_villains
            && self.entities_in(EffectZone::ActiveVillains).is_empty()
            && self.entities_in(EffectZone::VillainDeck).is_empty();
        let has_locations = self
            .entities()
            .any(|(_, entity)| entity.kind == EffectEntityKind::Location);
        let locations_exhausted = self.entities_in(EffectZone::ActiveLocation).is_empty()
            && self.entities_in(EffectZone::LocationDeck).is_empty();
        let final_location_controlled = self.entities_in(EffectZone::LocationDeck).is_empty()
            && self
                .entities_in(EffectZone::ActiveLocation)
                .first()
                .is_some_and(|location| {
                    location
                        .resource_limit(EffectResource::Control)
                        .is_some_and(|limit| location.resource(EffectResource::Control) >= limit)
                });
        if has_locations
            && (locations_exhausted || (final_location_controlled && villains_defeated))
        {
            return Some(EffectGameOutcome::Lost);
        }

        villains_defeated.then_some(EffectGameOutcome::Won)
    }

    pub(crate) fn refill_villains(
        &mut self,
        active_limit: u8,
    ) -> Result<Vec<String>, EffectExecutionError> {
        let mut revealed = Vec::new();
        while self.entities_in(EffectZone::ActiveVillains).len() < usize::from(active_limit) {
            let Some(villain_id) = self
                .entities_in(EffectZone::VillainDeck)
                .first()
                .map(|villain| villain.id.clone())
            else {
                break;
            };
            self.move_to_back(
                &villain_id,
                EffectZone::VillainDeck,
                EffectZone::ActiveVillains,
                None,
            )?;
            revealed.push(villain_id);
        }
        Ok(revealed)
    }

    pub(crate) fn is_valid_for_positions(&self, positions: &[u8]) -> bool {
        let mut ids = self.entity_ids().collect::<Vec<_>>();
        ids.sort_unstable();
        if ids
            .iter()
            .any(|id| id.is_empty() || id.len() > MAX_CHOICE_VALUE_LENGTH)
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
                    || entity.resources.iter().any(|(resource, value)| {
                        entity
                            .resource_limit(*resource)
                            .is_some_and(|limit| *value > limit)
                    })
                    || !entity_is_valid_in_zone(entity, zone)
            })
        {
            return false;
        }
        let location_count = self
            .entities()
            .filter(|(_, entity)| entity.kind == EffectEntityKind::Location)
            .count();
        let active_location_count = self.entities_in(EffectZone::ActiveLocation).len();
        if location_count > 0
            && (active_location_count > 1
                || (active_location_count == 0
                    && !self.entities_in(EffectZone::LocationDeck).is_empty()))
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

fn valid_hero_entity(entity: &EffectEntity, zone: EffectZone) -> bool {
    zone == EffectZone::Heroes
        && entity.owner_position.is_some()
        && entity.catalog_id.is_none()
        && entity.effect_rule_id.is_none()
        && entity.reward_rule_id.is_none()
        && entity.influence_cost.is_none()
        && entity.dark_arts_count.is_none()
        && entity.resource_limits == BTreeMap::from([(EffectResource::Health, HERO_MAX_HEALTH)])
}

fn valid_dark_arts_entity(entity: &EffectEntity, zone: EffectZone) -> bool {
    matches!(zone, EffectZone::DarkArtsDeck | EffectZone::DarkArtsDiscard)
        && entity.owner_position.is_none()
        && entity
            .catalog_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
        && entity
            .effect_rule_id
            .as_deref()
            .is_some_and(|id| !id.is_empty())
        && entity.reward_rule_id.is_none()
        && entity.influence_cost.is_none()
        && entity.resources.is_empty()
        && entity.resource_limits.is_empty()
        && entity.dark_arts_count.is_none()
}

fn entity_is_valid_in_zone(entity: &EffectEntity, zone: EffectZone) -> bool {
    if entity.drawing_blocked && entity.kind != EffectEntityKind::Hero {
        return false;
    }
    match entity.kind {
        EffectEntityKind::DarkArts => valid_dark_arts_entity(entity, zone),
        EffectEntityKind::Generic => true,
        EffectEntityKind::Hero => valid_hero_entity(entity, zone),
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
                && entity.reward_rule_id.is_none()
                && entity.resource_limits.is_empty()
                && entity.dark_arts_count.is_none()
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
                && entity.reward_rule_id.is_none()
                && (matches!(zone, EffectZone::HogwartsDeck | EffectZone::Market)
                    == entity.owner_position.is_none())
                && entity.resource_limits.is_empty()
                && entity.dark_arts_count.is_none()
        }
        EffectEntityKind::Location => {
            matches!(
                zone,
                EffectZone::ActiveLocation | EffectZone::LocationDeck | EffectZone::LocationDiscard
            ) && entity.owner_position.is_none()
                && entity
                    .catalog_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                && entity
                    .effect_rule_id
                    .as_deref()
                    .is_some_and(|id| !id.is_empty())
                && entity.influence_cost.is_none()
                && entity.reward_rule_id.is_none()
                && entity.resources.len() == 1
                && entity.resources.contains_key(&EffectResource::Control)
                && entity.resource_limits.len() == 1
                && entity
                    .resource_limit(EffectResource::Control)
                    .is_some_and(|limit| limit > 0)
                && entity.dark_arts_count.is_some_and(|count| count > 0)
        }
        EffectEntityKind::Villain => {
            matches!(
                zone,
                EffectZone::VillainDeck | EffectZone::ActiveVillains | EffectZone::VillainDiscard
            ) && entity.owner_position.is_none()
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
                && entity
                    .reward_rule_id
                    .as_deref()
                    .is_none_or(|id| !id.is_empty())
                && entity.resource_limits.is_empty()
                && entity.dark_arts_count.is_none()
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
        EffectEntityKind::DarkArts | EffectEntityKind::Location | EffectEntityKind::Villain => {
            current.is_none() && destination.is_none()
        }
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
    DrawingBlocked,
    Explicit,
    NoEligibleTarget,
    ZeroCardinality,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectOutcome {
    DrawingBlocked {
        rule_id: String,
        target_id: String,
        target_position: u8,
    },
    RandomSampled {
        rule_id: String,
        upper_exclusive: u32,
        result: u32,
    },
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
    StunDiscard,
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
    ReactionEffect,
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
    FinishStun {
        cursor: EffectCursor,
        responsible_position: u8,
    },
    StunChoice {
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
    pub queue: Vec<QueuedEffect>,
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

    fn sample_below(&mut self, upper_exclusive: u32) -> Option<u32>;
}

struct EffectExecutor<'a> {
    world: &'a mut EffectWorld,
    actor_position: u8,
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
        self.enqueue_reactions(0)?;
        self.enqueue_defeated_villains(self.actor_position)?;
        while let Some(queued) = self.queue.pop_front() {
            let outcome_start = self.outcomes.len();
            let actor_position = queued.actor_position();
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
                QueuedEffect::FinishStun {
                    cursor,
                    responsible_position: _,
                } => {
                    finish_stun(self.world, &mut self.outcomes)?;
                    let _ = cursor;
                    None
                }
                QueuedEffect::StunChoice {
                    cursor,
                    responsible_position,
                } => self.execute_stun_choice(&cursor, responsible_position)?,
            };
            self.enqueue_defeated_villains(actor_position)?;
            self.enqueue_reactions(outcome_start)?;
            self.ensure_outcome_limit()?;
            if let Some(stop) = stop {
                return Ok(self.finish(stop));
            }
        }
        Ok(self.finish(EffectStop::Stable))
    }

    fn enqueue_defeated_villains(
        &mut self,
        actor_position: u8,
    ) -> Result<(), EffectExecutionError> {
        let defeated = self
            .world
            .entities_in(EffectZone::ActiveVillains)
            .iter()
            .filter(|entity| {
                entity.kind == EffectEntityKind::Villain
                    && entity.resource(EffectResource::Health) == 0
            })
            .map(|entity| (entity.id.clone(), entity.reward_rule_id.clone()))
            .collect::<Vec<_>>();
        let mut rewards = Vec::new();
        for (id, reward) in defeated {
            self.world.move_to_back(
                &id,
                EffectZone::ActiveVillains,
                EffectZone::VillainDiscard,
                None,
            )?;
            self.outcomes.push(EffectOutcome::Moved {
                rule_id: "system:defeat-villain".to_owned(),
                target_id: id,
                target_position: None,
                from: EffectZone::ActiveVillains,
                to: EffectZone::VillainDiscard,
            });
            if let Some(reward) = reward {
                if !self
                    .rules
                    .iter()
                    .any(|rule| rule.id == reward && rule.trigger == EffectTrigger::VillainReward)
                {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                rewards.push(reward);
            }
        }
        for reward in rewards.into_iter().rev() {
            self.queue.push_front(QueuedEffect::Definition {
                cursor: EffectCursor::root(&reward),
                actor_position,
            });
            let mut costs = pay_costs(
                self.world,
                actor_position,
                self.rules,
                RuleSelection::Rule(&reward),
            )?;
            resolve_cost_stun(self.world, actor_position, &mut costs, &mut self.queue)?;
            self.outcomes.extend(costs);
        }
        Ok(())
    }

    fn execute_definition(
        &mut self,
        cursor: EffectCursor,
        actor_position: u8,
        effect: EffectDefinition,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        match effect {
            EffectDefinition::Reaction { .. }
            | EffectDefinition::HandDamageLimit { .. }
            | EffectDefinition::TopDeckAcquisition { .. }
            | EffectDefinition::CardType { .. } => Ok(None),
            EffectDefinition::RevealDarkArts => self.reveal_dark_arts(&cursor, actor_position),
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
        let uses_manual_bindings = self
            .rules
            .iter()
            .any(|rule| rule.id == cursor.rule_id && rule.trigger == EffectTrigger::Manual);
        let mut stun_choices = Vec::new();
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
        } else if uses_manual_bindings
            && let Some(selector_id) = target.id.as_deref()
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
                if let Some(position) = apply_operation(
                    self.world,
                    entity_id,
                    EffectSource {
                        rule_id: &cursor.rule_id,
                        rules: self.rules,
                    },
                    operation,
                    &mut self.outcomes,
                    self.roller,
                    &mut self.rolls_consumed,
                )? {
                    stun_choices.push(position);
                }
            }
        } else if target.id.is_some() && self.require_named_target_bindings && uses_manual_bindings
        {
            return Err(EffectExecutionError::InvalidTargetSelection);
        } else if candidates.len() > usize::from(target.max) {
            return Ok(Some(EffectStop::Choice(PendingEffectChoice {
                id: self.pending_choice_id(cursor, PendingEffectChoiceKind::Target)?,
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
                if let Some(position) = apply_operation(
                    self.world,
                    &entity_id,
                    EffectSource {
                        rule_id: &cursor.rule_id,
                        rules: self.rules,
                    },
                    operation,
                    &mut self.outcomes,
                    self.roller,
                    &mut self.rolls_consumed,
                )? {
                    stun_choices.push(position);
                }
            }
        }
        for responsible_position in stun_choices.into_iter().rev() {
            self.queue.push_front(QueuedEffect::StunChoice {
                cursor: cursor.clone(),
                responsible_position,
            });
        }
        Ok(None)
    }

    fn reveal_dark_arts(
        &mut self,
        cursor: &EffectCursor,
        actor_position: u8,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        if self.world.entities_in(EffectZone::DarkArtsDeck).is_empty() {
            self.rolls_consumed += refill_shuffled_pile(
                self.world,
                None,
                (EffectZone::DarkArtsDiscard, EffectZone::DarkArtsDeck),
                &cursor.rule_id,
                &mut self.outcomes,
                self.roller,
            )?;
        }
        let card = self
            .world
            .entities_in(EffectZone::DarkArtsDeck)
            .first()
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let effect_rule = card
            .effect_rule_id
            .as_deref()
            .and_then(|id| {
                self.rules
                    .iter()
                    .find(|rule| rule.id == id && rule.cost.is_empty())
            })
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let card_id = card.id.clone();
        self.world.move_to_back(
            &card_id,
            EffectZone::DarkArtsDeck,
            EffectZone::DarkArtsDiscard,
            None,
        )?;
        self.outcomes.push(EffectOutcome::Moved {
            rule_id: cursor.rule_id.clone(),
            target_id: card_id,
            target_position: None,
            from: EffectZone::DarkArtsDeck,
            to: EffectZone::DarkArtsDiscard,
        });
        self.queue.push_front(QueuedEffect::Definition {
            cursor: EffectCursor::root(&effect_rule.id),
            actor_position,
        });
        Ok(None)
    }

    fn execute_stun_choice(
        &mut self,
        cursor: &EffectCursor,
        responsible_position: u8,
    ) -> Result<Option<EffectStop>, EffectExecutionError> {
        let options = self
            .world
            .card_ids_in_zone(responsible_position, EffectZone::HeroHand);
        let discard_count = options.len() / 2;
        if discard_count == 0 {
            finish_stun(self.world, &mut self.outcomes)?;
            return Ok(None);
        }
        let discard_count =
            u16::try_from(discard_count).map_err(|_| EffectExecutionError::InvalidDefinition)?;
        self.queue.push_front(QueuedEffect::FinishStun {
            cursor: cursor.clone(),
            responsible_position,
        });
        Ok(Some(EffectStop::Choice(PendingEffectChoice {
            id: self.pending_choice_id(cursor, PendingEffectChoiceKind::StunDiscard)?,
            cause: cursor.rule_id.clone(),
            responsible_position,
            kind: PendingEffectChoiceKind::StunDiscard,
            options,
            min: discard_count,
            max: discard_count,
            continuation: self.continuation(cursor),
        })))
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
            id: self.pending_choice_id(cursor, PendingEffectChoiceKind::Effect)?,
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

    fn finish(self, mut stop: EffectStop) -> EffectResolution {
        if let EffectStop::Choice(pending) = &mut stop {
            pending.continuation.queue = self.queue.iter().cloned().collect();
        }
        EffectResolution {
            outcomes: self.outcomes,
            stop,
            rolls_consumed: self.rolls_consumed,
            queue: self.queue.into(),
        }
    }

    fn ensure_outcome_limit(&self) -> Result<(), EffectExecutionError> {
        if self.outcomes.len() > MAX_EFFECT_OUTCOMES {
            Err(EffectExecutionError::StepLimitExceeded)
        } else {
            Ok(())
        }
    }

    fn pending_choice_id(
        &self,
        cursor: &EffectCursor,
        kind: PendingEffectChoiceKind,
    ) -> Result<String, EffectExecutionError> {
        let step = self
            .steps
            .checked_sub(1)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let kind = match kind {
            PendingEffectChoiceKind::Effect => "effect",
            PendingEffectChoiceKind::StunDiscard => "stun-discard",
            PendingEffectChoiceKind::Target => "target",
        };
        let trigger = self
            .rules
            .iter()
            .find(|rule| rule.id == cursor.rule_id)
            .map(|rule| rule.trigger)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        if trigger == EffectTrigger::DarkArtsCompleted {
            Ok(format!("{}:{kind}:{step}", cursor.rule_id))
        } else {
            Ok(format!("choice:{kind}:{step}"))
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
            (EffectDefinition::Reaction { effect, .. }, EffectPathSegment::ReactionEffect) => {
                effect
            }
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

fn discard_stunned_hand(
    world: &mut EffectWorld,
    pending: &PendingEffectChoice,
    selected_options: &[String],
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<(), EffectExecutionError> {
    let candidates = world.card_ids_in_zone(pending.responsible_position, EffectZone::HeroHand);
    if candidates != pending.options {
        return Err(EffectExecutionError::InvalidChoice);
    }
    for option in selected_options {
        world.move_to_back(
            option,
            EffectZone::HeroHand,
            EffectZone::HeroDiscardPile,
            Some(pending.responsible_position),
        )?;
        outcomes.push(EffectOutcome::Moved {
            rule_id: STUN_RULE_ID.to_owned(),
            target_id: option.clone(),
            target_position: Some(pending.responsible_position),
            from: EffectZone::HeroHand,
            to: EffectZone::HeroDiscardPile,
        });
    }
    Ok(())
}

pub(crate) fn resume_effects(
    world: &mut EffectWorld,
    active_position: u8,
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
    let mut next_world = world.clone();
    let mut queue = pending
        .continuation
        .queue
        .iter()
        .cloned()
        .collect::<VecDeque<_>>();
    let mut outcomes = Vec::new();
    let mut stun_choices = Vec::new();
    let mut rolls_consumed = 0;

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
            let candidates = eligible_entity_ids(&next_world, pending.responsible_position, target);
            if candidates != pending.options {
                return Err(EffectExecutionError::InvalidChoice);
            }
            for option in &pending.options {
                if !selected_options.contains(option) {
                    continue;
                }
                if let Some(position) = apply_operation(
                    &mut next_world,
                    option,
                    EffectSource {
                        rule_id: &pending.cause,
                        rules,
                    },
                    operation,
                    &mut outcomes,
                    roller,
                    &mut rolls_consumed,
                )? {
                    stun_choices.push(position);
                }
            }
        }
        (PendingEffectChoiceKind::StunDiscard, _)
            if next_world.hero_resource(pending.responsible_position, EffectResource::Health)
                == Some(0) =>
        {
            discard_stunned_hand(&mut next_world, pending, selected_options, &mut outcomes)?;
        }
        _ => return Err(EffectExecutionError::InvalidChoice),
    }

    for responsible_position in stun_choices.into_iter().rev() {
        queue.push_front(QueuedEffect::StunChoice {
            cursor: pending.continuation.choice_cursor.clone(),
            responsible_position,
        });
    }

    if outcomes.len() > MAX_EFFECT_OUTCOMES {
        return Err(EffectExecutionError::StepLimitExceeded);
    }
    let resolution = EffectExecutor {
        world: &mut next_world,
        actor_position: active_position,
        rules,
        roller,
        target_bindings: &[],
        require_named_target_bindings: false,
        queue,
        outcomes,
        rolls_consumed,
        steps: pending.continuation.steps_completed,
    }
    .run()?;
    *world = next_world;
    Ok(resolution)
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
    selected_costs_are_affordable(
        world,
        actor_position,
        rules,
        RuleSelection::Trigger(trigger),
    )
}

fn selected_costs_are_affordable(
    world: &EffectWorld,
    actor_position: u8,
    rules: &[EffectRule],
    selection: RuleSelection<'_>,
) -> bool {
    let costs = combined_costs(rules, selection);
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
    let active_villain_rules = world
        .entities_in(EffectZone::ActiveVillains)
        .iter()
        .filter_map(|entity| entity.effect_rule_id.clone())
        .collect::<BTreeSet<_>>();
    let selection = if trigger == EffectTrigger::Villains
        && world
            .entities()
            .any(|(_, entity)| entity.kind == EffectEntityKind::Villain)
    {
        RuleSelection::ActiveVillains(&active_villain_rules)
    } else {
        RuleSelection::Trigger(trigger)
    };
    execute_effects_with_targets(
        world,
        rules,
        roller,
        EffectExecutionPlan {
            actor_position,
            selection,
            target_bindings: &[],
            require_named_target_bindings: false,
            initial_outcomes: vec![],
        },
    )
}

pub(crate) fn execute_effect_rule(
    world: &mut EffectWorld,
    actor_position: u8,
    card_id: &str,
    rules: &[EffectRule],
    target_bindings: &[EffectTargetBinding],
    roller: &mut dyn EffectRoller,
) -> Result<EffectResolution, EffectExecutionError> {
    let rule_id = world
        .entity(card_id)
        .and_then(|(_, card)| card.effect_rule_id())
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    let rule = rules
        .iter()
        .find(|rule| rule.id == rule_id && rule.trigger == EffectTrigger::Manual)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    execute_effects_with_targets(
        world,
        rules,
        roller,
        EffectExecutionPlan {
            actor_position,
            selection: RuleSelection::Rule(&rule.id),
            target_bindings,
            require_named_target_bindings: true,
            initial_outcomes: vec![EffectOutcome::Moved {
                rule_id: "system:play-card".to_owned(),
                target_id: card_id.to_owned(),
                target_position: Some(actor_position),
                from: EffectZone::HeroHand,
                to: EffectZone::HeroPlayArea,
            }],
        },
    )
}

pub(crate) fn execute_committed_effects(
    world: &mut EffectWorld,
    actor_position: u8,
    rule: Option<&EffectRule>,
    rules: &[EffectRule],
    initial_outcomes: Vec<EffectOutcome>,
    roller: &mut dyn EffectRoller,
) -> Result<EffectResolution, EffectExecutionError> {
    execute_effects_with_targets(
        world,
        rules,
        roller,
        EffectExecutionPlan {
            actor_position,
            selection: rule.map_or(RuleSelection::None, |rule| RuleSelection::Rule(&rule.id)),
            target_bindings: &[],
            require_named_target_bindings: false,
            initial_outcomes,
        },
    )
}

#[derive(Clone, Copy)]
enum RuleSelection<'a> {
    None,
    Trigger(EffectTrigger),
    Rule(&'a str),
    ActiveVillains(&'a BTreeSet<String>),
}

impl RuleSelection<'_> {
    fn matches(self, rule: &EffectRule) -> bool {
        match self {
            Self::None => false,
            Self::Trigger(trigger) => rule.trigger == trigger,
            Self::Rule(id) => rule.id == id,
            Self::ActiveVillains(ids) => {
                rule.trigger == EffectTrigger::Villains && ids.contains(&rule.id)
            }
        }
    }
}

struct EffectExecutionPlan<'a> {
    actor_position: u8,
    selection: RuleSelection<'a>,
    target_bindings: &'a [EffectTargetBinding],
    require_named_target_bindings: bool,
    initial_outcomes: Vec<EffectOutcome>,
}

fn execute_effects_with_targets(
    world: &mut EffectWorld,
    rules: &[EffectRule],
    roller: &mut dyn EffectRoller,
    plan: EffectExecutionPlan<'_>,
) -> Result<EffectResolution, EffectExecutionError> {
    let EffectExecutionPlan {
        actor_position,
        selection,
        target_bindings,
        require_named_target_bindings,
        initial_outcomes,
    } = plan;
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
    if !selected_costs_are_affordable(world, actor_position, rules, selection) {
        return Err(EffectExecutionError::UnaffordableCost);
    }

    let mut queue = rules
        .iter()
        .filter(|rule| selection.matches(rule))
        .map(|rule| QueuedEffect::Definition {
            cursor: EffectCursor::root(&rule.id),
            actor_position,
        })
        .collect::<VecDeque<_>>();
    let mut next_world = world.clone();
    let mut costs = pay_costs(&mut next_world, actor_position, rules, selection)?;
    resolve_cost_stun(&mut next_world, actor_position, &mut costs, &mut queue)?;
    let mut outcomes = initial_outcomes;
    outcomes.extend(costs);
    if outcomes.len() > MAX_EFFECT_OUTCOMES {
        return Err(EffectExecutionError::StepLimitExceeded);
    }
    let resolution = EffectExecutor {
        world: &mut next_world,
        actor_position,
        rules,
        roller,
        target_bindings,
        require_named_target_bindings,
        queue,
        outcomes,
        rolls_consumed: 0,
        steps: 0,
    }
    .run()?;
    *world = next_world;
    Ok(resolution)
}

fn resolve_cost_stun(
    world: &mut EffectWorld,
    actor_position: u8,
    outcomes: &mut Vec<EffectOutcome>,
    queue: &mut VecDeque<QueuedEffect>,
) -> Result<(), EffectExecutionError> {
    let stun = outcomes.iter().find_map(|outcome| match outcome {
        EffectOutcome::ResourceChanged {
            rule_id,
            target_id,
            resource: EffectResource::Health,
            before,
            after: 0,
            ..
        } if *before > 0 => Some((rule_id.clone(), target_id.clone())),
        _ => None,
    });
    if let Some((rule_id, hero_id)) = stun {
        if apply_stun_resource_reset(world, &hero_id, actor_position, outcomes)? {
            queue.push_front(QueuedEffect::StunChoice {
                cursor: EffectCursor::root(&rule_id),
                responsible_position: actor_position,
            });
        } else {
            finish_stun(world, outcomes)?;
        }
    }
    Ok(())
}

fn effect_rules_are_valid(rules: &[EffectRule]) -> bool {
    let mut ids = BTreeSet::new();
    rules.iter().all(|rule| {
        let max_rule_id_length = if rule.trigger == EffectTrigger::DarkArtsCompleted {
            MAX_RUNTIME_RULE_ID_LENGTH
        } else {
            MAX_CHOICE_VALUE_LENGTH
        };
        !rule.id.is_empty()
            && rule.id.len() <= max_rule_id_length
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
    selection: RuleSelection<'_>,
) -> Result<Vec<EffectOutcome>, EffectExecutionError> {
    let mut outcomes = Vec::new();
    for (rule_id, cost) in rules
        .iter()
        .filter(|rule| selection.matches(rule))
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
            EffectOutcome::DrawingBlocked {
                target_id,
                target_position,
                ..
            } => {
                let (_, hero) = world
                    .entity_mut(target_id)
                    .filter(|(_, entity)| {
                        entity.kind == EffectEntityKind::Hero
                            && entity.owner_position == Some(*target_position)
                    })
                    .ok_or(EffectExecutionError::InvalidDefinition)?;
                hero.drawing_blocked = true;
            }
            EffectOutcome::Moved {
                rule_id,
                target_id,
                target_position,
                from,
                to,
                ..
            } => {
                let (current_zone, entity) = world
                    .entity(target_id)
                    .ok_or(EffectExecutionError::InvalidDefinition)?;
                if current_zone != *from || from == to || !from.is_card_zone() || !to.is_card_zone()
                {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
                if rule_id == "system:defeat-villain"
                    && (entity.kind != EffectEntityKind::Villain
                        || entity.resource(EffectResource::Health) != 0
                        || *from != EffectZone::ActiveVillains
                        || *to != EffectZone::VillainDiscard
                        || target_position.is_some())
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
            EffectOutcome::RandomSampled {
                upper_exclusive,
                result,
                ..
            } => {
                if *upper_exclusive == 0 || result >= upper_exclusive {
                    return Err(EffectExecutionError::InvalidDefinition);
                }
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
    if outcomes.len() > MAX_EFFECT_OUTCOMES || outcomes.iter().any(|outcome| !outcome.is_valid()) {
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
    #[must_use]
    pub const fn consumes_random_sample(&self) -> bool {
        matches!(self, Self::DieRolled { .. } | Self::RandomSampled { .. })
    }

    #[must_use]
    pub fn rule_id(&self) -> &str {
        match self {
            Self::DrawingBlocked { rule_id, .. }
            | Self::RandomSampled { rule_id, .. }
            | Self::DieRolled { rule_id, .. }
            | Self::Moved { rule_id, .. }
            | Self::NoOp { rule_id, .. }
            | Self::ResourceChanged { rule_id, .. }
            | Self::Terminal { rule_id, .. } => rule_id,
        }
    }

    fn is_valid(&self) -> bool {
        let valid_id = |value: &str| !value.is_empty() && value.len() <= MAX_CHOICE_VALUE_LENGTH;
        match self {
            Self::DrawingBlocked {
                rule_id,
                target_id,
                target_position,
            } => valid_id(rule_id) && valid_id(target_id) && (1..=4).contains(target_position),
            Self::RandomSampled {
                rule_id,
                upper_exclusive,
                result,
            } => valid_id(rule_id) && *upper_exclusive > 0 && result < upper_exclusive,
            Self::DieRolled {
                rule_id,
                die,
                result,
            } => valid_id(rule_id) && (1..=die.sides()).contains(result),
            Self::Moved {
                rule_id,
                target_id,
                from,
                to,
                ..
            } => {
                valid_id(rule_id)
                    && valid_id(target_id)
                    && from != to
                    && from.is_card_zone()
                    && to.is_card_zone()
            }
            Self::NoOp { rule_id, .. } | Self::Terminal { rule_id, .. } => valid_id(rule_id),
            Self::ResourceChanged {
                rule_id, target_id, ..
            } => valid_id(rule_id) && valid_id(target_id),
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
            && self.id.len() <= MAX_CHOICE_VALUE_LENGTH
            && !self.cause.is_empty()
            && self.cause.len() <= MAX_CHOICE_VALUE_LENGTH
            && self.options.len() >= 2
            && self.options.len() <= MAX_CHOICE_OPTIONS
            && options.len() == self.options.len()
            && options
                .iter()
                .all(|option| !option.is_empty() && option.len() <= MAX_CHOICE_VALUE_LENGTH)
            && self.min <= self.max
            && self.max <= MAX_CHOICE_SELECTIONS
            && usize::from(self.max) <= self.options.len()
            && match self.kind {
                PendingEffectChoiceKind::Effect => self.min == 1 && self.max == 1,
                PendingEffectChoiceKind::StunDiscard => {
                    self.min > 0
                        && self.min == self.max
                        && usize::from(self.max) == self.options.len() / 2
                }
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
        !self.rule_id.is_empty()
            && self.rule_id.len() <= MAX_CHOICE_VALUE_LENGTH
            && self.path.len() <= MAX_EFFECT_PATH_DEPTH
            && self.path.iter().all(|segment| match segment {
                EffectPathSegment::ChoiceOption(index)
                | EffectPathSegment::SequenceEffect(index) => *index <= MAX_EFFECT_BRANCH_INDEX,
                EffectPathSegment::RollOutcome(index) => *index <= MAX_EFFECT_ROLL_INDEX,
                EffectPathSegment::ConditionThen
                | EffectPathSegment::ConditionOtherwise
                | EffectPathSegment::ReactionEffect
                | EffectPathSegment::RepeatEffect => true,
            })
    }
}

impl QueuedEffect {
    #[must_use]
    pub fn rule_id(&self) -> &str {
        self.cursor().rule_id.as_str()
    }

    #[must_use]
    pub fn path(&self) -> &[EffectPathSegment] {
        &self.cursor().path
    }

    #[must_use]
    pub const fn actor_position(&self) -> u8 {
        match self {
            Self::Definition { actor_position, .. } => *actor_position,
            Self::EffectChoice {
                responsible_position,
                ..
            }
            | Self::FinishStun {
                responsible_position,
                ..
            }
            | Self::StunChoice {
                responsible_position,
                ..
            } => *responsible_position,
        }
    }

    #[must_use]
    pub const fn cursor(&self) -> &EffectCursor {
        match self {
            Self::Definition { cursor, .. }
            | Self::EffectChoice { cursor, .. }
            | Self::FinishStun { cursor, .. }
            | Self::StunChoice { cursor, .. } => cursor,
        }
    }

    pub(crate) fn is_valid_for_positions(&self, participant_positions: &[u8]) -> bool {
        match self {
            Self::Definition {
                cursor,
                actor_position,
            } => cursor.is_structurally_valid() && participant_positions.contains(actor_position),
            Self::EffectChoice {
                cursor,
                responsible_position,
            }
            | Self::FinishStun {
                cursor,
                responsible_position,
            }
            | Self::StunChoice {
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
        EffectOperation::PreventDrawing => zone == EffectZone::Heroes,
        EffectOperation::Draw { amount } | EffectOperation::GainAttackPerAllyPlayed { amount } => {
            (1..=16).contains(amount) && zone == EffectZone::Heroes
        }
        EffectOperation::Discard => zone == EffectZone::HeroHand,
        EffectOperation::ModifyResource { resource, amount } => {
            *amount != 0
                && zone.supports_resource(*resource)
                && (*resource != EffectResource::Control || zone == EffectZone::ActiveLocation)
                && (*resource != EffectResource::Health
                    || !matches!(zone, EffectZone::VillainDeck | EffectZone::VillainDiscard))
        }
        EffectOperation::Move { to } => zone.is_card_zone() && to.is_card_zone() && zone != *to,
    }
}

fn combined_costs(
    rules: &[EffectRule],
    selection: RuleSelection<'_>,
) -> BTreeMap<EffectResource, u64> {
    let mut costs = BTreeMap::new();
    for cost in rules
        .iter()
        .filter(|rule| selection.matches(rule))
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
        EffectDefinition::NoOp
        | EffectDefinition::TopDeckAcquisition { .. }
        | EffectDefinition::CardType { .. }
        | EffectDefinition::RevealDarkArts
        | EffectDefinition::HandDamageLimit { .. }
        | EffectDefinition::Reaction { .. } => Some(true),
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

fn refill_shuffled_pile(
    world: &mut EffectWorld,
    owner: Option<u8>,
    zones: (EffectZone, EffectZone),
    rule_id: &str,
    outcomes: &mut Vec<EffectOutcome>,
    roller: &mut dyn EffectRoller,
) -> Result<u64, EffectExecutionError> {
    let (from, to) = zones;
    let mut cards = world
        .entities_in(from)
        .iter()
        .filter(|entity| entity.owner_position == owner)
        .map(|entity| entity.id.clone())
        .collect::<Vec<_>>();
    let samples = u64::try_from(cards.len().saturating_sub(1))
        .map_err(|_| EffectExecutionError::InvalidDefinition)?;
    let evidence = crate::shuffling::fisher_yates(cards.len(), roller, |index, selected| {
        cards.swap(index, selected);
    })?;
    for sample in evidence {
        outcomes.push(EffectOutcome::RandomSampled {
            rule_id: rule_id.to_owned(),
            upper_exclusive: sample.upper_exclusive,
            result: sample.result,
        });
    }
    for card_id in cards {
        world.move_to_back(&card_id, from, to, owner)?;
        outcomes.push(EffectOutcome::Moved {
            rule_id: rule_id.to_owned(),
            target_id: card_id,
            target_position: owner,
            from,
            to,
        });
    }
    Ok(samples)
}

fn draw_cards(
    world: &mut EffectWorld,
    entity_id: &str,
    rule_id: &str,
    amount: u8,
    outcomes: &mut Vec<EffectOutcome>,
    roller: &mut dyn EffectRoller,
    rolls_consumed: &mut u64,
) -> Result<Option<u8>, EffectExecutionError> {
    if world
        .entity(entity_id)
        .is_some_and(|(_, entity)| entity.drawing_blocked)
    {
        outcomes.push(EffectOutcome::NoOp {
            rule_id: rule_id.to_owned(),
            reason: EffectNoOpReason::DrawingBlocked,
        });
        return Ok(None);
    }
    let position = world
        .entity(entity_id)
        .and_then(|(_, entity)| entity.owner_position)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    for _ in 0..amount {
        if world
            .top_card_id(position, EffectZone::HeroDrawPile)
            .is_none()
        {
            *rolls_consumed += refill_shuffled_pile(
                world,
                Some(position),
                (EffectZone::HeroDiscardPile, EffectZone::HeroDrawPile),
                rule_id,
                outcomes,
                roller,
            )?;
        }
        let Some(card_id) = world.top_card_id(position, EffectZone::HeroDrawPile) else {
            break;
        };
        world.move_card(&card_id, EffectZone::HeroDrawPile, EffectZone::HeroHand)?;
        outcomes.push(EffectOutcome::Moved {
            rule_id: rule_id.to_owned(),
            target_id: card_id,
            target_position: Some(position),
            from: EffectZone::HeroDrawPile,
            to: EffectZone::HeroHand,
        });
    }
    Ok(None)
}

#[derive(Clone, Copy)]
struct EffectSource<'a> {
    rule_id: &'a str,
    rules: &'a [EffectRule],
}

fn apply_operation(
    world: &mut EffectWorld,
    entity_id: &str,
    source: EffectSource<'_>,
    operation: &EffectOperation,
    outcomes: &mut Vec<EffectOutcome>,
    roller: &mut dyn EffectRoller,
    rolls_consumed: &mut u64,
) -> Result<Option<u8>, EffectExecutionError> {
    let rule_id = source.rule_id;
    match operation {
        EffectOperation::GainAttackPerAllyPlayed { amount } => {
            let owner = world
                .entity(entity_id)
                .and_then(|(_, hero)| hero.owner_position())
                .ok_or(EffectExecutionError::InvalidDefinition)?;
            let count = world
                .entities_in(EffectZone::HeroPlayArea)
                .iter()
                .filter(|card| {
                    card.owner_position() == Some(owner)
                        && reactions::card_type(card, source.rules) == Some(EffectCardType::Ally)
                })
                .count();
            let amount = i16::try_from(
                count
                    .checked_mul(usize::from(*amount))
                    .ok_or(EffectExecutionError::InvalidDefinition)?,
            )
            .map_err(|_| EffectExecutionError::InvalidDefinition)?;
            modify_entity_resource(
                world,
                entity_id,
                rule_id,
                EffectResource::Attack,
                amount,
                outcomes,
            )
        }
        EffectOperation::PreventDrawing => {
            let (_, hero) = world
                .entity_mut(entity_id)
                .ok_or(EffectExecutionError::InvalidDefinition)?;
            let target_position = hero
                .owner_position
                .filter(|_| hero.kind == EffectEntityKind::Hero)
                .ok_or(EffectExecutionError::InvalidDefinition)?;
            hero.drawing_blocked = true;
            outcomes.push(EffectOutcome::DrawingBlocked {
                rule_id: rule_id.to_owned(),
                target_id: entity_id.to_owned(),
                target_position,
            });
            Ok(None)
        }
        EffectOperation::Draw { amount } => draw_cards(
            world,
            entity_id,
            rule_id,
            *amount,
            outcomes,
            roller,
            rolls_consumed,
        ),
        EffectOperation::Discard => move_entity(
            world,
            entity_id,
            rule_id,
            EffectZone::HeroDiscardPile,
            outcomes,
        ),
        EffectOperation::ModifyResource { resource, amount } => {
            let amount = if *resource == EffectResource::Health && *amount < 0 {
                reactions::protected_health_loss(world, entity_id, source, *amount)
            } else {
                *amount
            };
            modify_entity_resource(world, entity_id, rule_id, *resource, amount, outcomes)
        }
        EffectOperation::Move { to } => move_entity(world, entity_id, rule_id, *to, outcomes),
    }
}

fn modify_entity_resource(
    world: &mut EffectWorld,
    entity_id: &str,
    rule_id: &str,
    resource: EffectResource,
    amount: i16,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<Option<u8>, EffectExecutionError> {
    let (before, after, target_id, target_position, newly_stunned) = {
        let (_, entity) = world
            .entity_mut(entity_id)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        let before = entity.resource(resource);
        let after = if entity.kind == EffectEntityKind::Hero
            && resource == EffectResource::Health
            && before == 0
        {
            0
        } else if amount < 0 {
            before.saturating_sub(amount.unsigned_abs())
        } else {
            let maximum = entity.resource_limit(resource).unwrap_or(u16::MAX);
            before.saturating_add(amount.unsigned_abs()).min(maximum)
        };
        entity.resources.insert(resource, after);
        (
            before,
            after,
            entity.id.clone(),
            entity.owner_position,
            entity.kind == EffectEntityKind::Hero
                && resource == EffectResource::Health
                && before > 0
                && after == 0,
        )
    };
    outcomes.push(EffectOutcome::ResourceChanged {
        rule_id: rule_id.to_owned(),
        target_id: target_id.clone(),
        target_position,
        resource,
        before,
        after,
        cause: EffectChangeCause::Effect,
    });
    if newly_stunned {
        let hero_position = target_position.ok_or(EffectExecutionError::InvalidDefinition)?;
        let requires_discard =
            apply_stun_resource_reset(world, &target_id, hero_position, outcomes)?;
        if requires_discard {
            return Ok(Some(hero_position));
        }
        finish_stun(world, outcomes)?;
    }
    Ok(None)
}

fn move_entity(
    world: &mut EffectWorld,
    entity_id: &str,
    rule_id: &str,
    to: EffectZone,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<Option<u8>, EffectExecutionError> {
    let (from, entity) = world
        .entity(entity_id)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    let target_id = entity.id.clone();
    let target_position = entity.owner_position;
    world.move_to_back(&target_id, from, to, target_position)?;
    outcomes.push(EffectOutcome::Moved {
        rule_id: rule_id.to_owned(),
        target_id,
        target_position,
        from,
        to,
    });
    Ok(None)
}

fn apply_stun_resource_reset(
    world: &mut EffectWorld,
    hero_id: &str,
    hero_position: u8,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<bool, EffectExecutionError> {
    for resource in [EffectResource::Attack, EffectResource::Influence] {
        let before = world
            .entity(hero_id)
            .map(|(_, hero)| hero.resource(resource))
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        if before == 0 {
            continue;
        }
        let (_, hero) = world
            .entity_mut(hero_id)
            .ok_or(EffectExecutionError::InvalidDefinition)?;
        hero.resources.insert(resource, 0);
        outcomes.push(EffectOutcome::ResourceChanged {
            rule_id: STUN_RULE_ID.to_owned(),
            target_id: hero_id.to_owned(),
            target_position: Some(hero_position),
            resource,
            before,
            after: 0,
            cause: EffectChangeCause::Effect,
        });
    }

    Ok(world
        .cards_in_zone(hero_position, EffectZone::HeroHand)
        .len()
        / 2
        != 0)
}

fn finish_stun(
    world: &mut EffectWorld,
    outcomes: &mut Vec<EffectOutcome>,
) -> Result<(), EffectExecutionError> {
    let active_locations = world.entities_in(EffectZone::ActiveLocation);
    let [location] = active_locations else {
        return Err(EffectExecutionError::InvalidDefinition);
    };
    let location_id = location.id.clone();
    let before = location.resource(EffectResource::Control);
    let maximum = location
        .resource_limit(EffectResource::Control)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    let after = before.saturating_add(1).min(maximum);
    let (_, location) = world
        .entity_mut(&location_id)
        .ok_or(EffectExecutionError::InvalidDefinition)?;
    location.resources.insert(EffectResource::Control, after);
    outcomes.push(EffectOutcome::ResourceChanged {
        rule_id: STUN_RULE_ID.to_owned(),
        target_id: location_id,
        target_position: None,
        resource: EffectResource::Control,
        before,
        after,
        cause: EffectChangeCause::Effect,
    });
    Ok(())
}
