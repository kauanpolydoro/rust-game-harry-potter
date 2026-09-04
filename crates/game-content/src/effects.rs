use serde::{Deserialize, Serialize};

use crate::RuleId;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectRule {
    pub id: RuleId,
    #[serde(default)]
    pub trigger: EffectTrigger,
    #[serde(default)]
    pub cost: Vec<ResourceCost>,
    pub effect: Effect,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectTrigger {
    DarkArtsCompleted,
    #[default]
    Manual,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceCost {
    pub resource: Resource,
    pub amount: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    Apply {
        target: Selector,
        operation: Operation,
    },
    Choice {
        #[serde(default, skip_serializing_if = "EffectChoiceAudience::is_actor")]
        audience: EffectChoiceAudience,
        options: Vec<Self>,
    },
    Condition {
        condition: Condition,
        then: Box<Self>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        otherwise: Option<Box<Self>>,
    },
    NoOp,
    Reference {
        rule: RuleId,
    },
    Repeat {
        times: u8,
        effect: Box<Self>,
    },
    Roll {
        die: Die,
        outcomes: Vec<Self>,
    },
    Sequence {
        effects: Vec<Self>,
    },
    Terminal {
        outcome: GameOutcome,
    },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectChoiceAudience {
    #[default]
    Actor,
    EachHero,
}

impl EffectChoiceAudience {
    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde skip_serializing_if requires a shared reference"
    )]
    fn is_actor(&self) -> bool {
        *self == Self::Actor
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    HasEligibleTarget {
        target: Selector,
    },
    ResourceAtLeast {
        target: Selector,
        resource: Resource,
        amount: u16,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    pub zone: Zone,
    #[serde(default)]
    pub owner: TargetOwner,
    pub cardinality: Cardinality,
    #[serde(default)]
    pub eligibility: Vec<Eligibility>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Cardinality {
    pub min: u16,
    pub max: u16,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetOwner {
    Actor,
    #[default]
    Any,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Eligibility {
    ResourceAtLeast { resource: Resource, amount: u16 },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Discard,
    ModifyResource { resource: Resource, amount: i16 },
    Move { to: Zone },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Resource {
    Attack,
    Control,
    Health,
    Influence,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Die {
    D4,
    D6,
    D8,
}

impl Die {
    #[must_use]
    pub const fn sides(self) -> usize {
        match self {
            Self::D4 => 4,
            Self::D6 => 6,
            Self::D8 => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameOutcome {
    Lost,
    Won,
}
