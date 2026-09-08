use serde::{Deserialize, Serialize};

use crate::RuleId;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EffectRule {
    pub id: RuleId,
    #[serde(default)]
    pub trigger: EffectTrigger,
    pub order: u16,
    #[serde(default)]
    pub cost: Vec<ResourceCost>,
    pub effect: Effect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<RuleProvenance>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuleProvenance {
    pub confidence: crate::FunctionalConfidence,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectTrigger {
    #[serde(alias = "dark_arts_completed")]
    DarkArts,
    Villains,
    VillainReward,
    #[default]
    Manual,
}

impl EffectTrigger {
    pub(crate) const fn phase_order(self) -> u8 {
        match self {
            Self::DarkArts => 0,
            Self::Villains => 1,
            Self::VillainReward => 2,
            Self::Manual => 3,
        }
    }

    pub(crate) const fn is_automatic(self) -> bool {
        matches!(self, Self::DarkArts | Self::Villains)
    }
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
    RevealExtraDarkArts,
    PreventControlRemoval,
    OtherAllyBonus {
        health: u8,
    },
    HeroAbility {
        strategy: HeroAbilityStrategy,
        effect: Box<Self>,
    },
    RevealTopCard {
        minimum_cost: u16,
        effect: Box<Self>,
    },
    LimitVillainAttack {
        maximum: u8,
    },
    PreventExtraDrawing,
    ForEachTarget {
        target: Selector,
        effect: Box<Self>,
    },
    TopDeckAcquisition {
        card_type: CardType,
    },
    CardType {
        card_type: CardType,
    },
    HandDamageLimit {
        maximum: u8,
    },
    Reaction {
        trigger: ReactionTrigger,
        effect: Box<Self>,
    },
    RevealDarkArts,
    Structural {
        rule: StructuralRule,
    },
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

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReactionTrigger {
    MorsmordreRevealedV1,
    VillainRevealed,
    SelfHarmfulDiscard,
    OwnerPlaysAlly,
    ControlAdded,
    HeroForcedDiscard,
    SelfForcedDiscard,
    OwnerDefeatsVillain,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CardType {
    Ally,
    Item,
    Spell,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EffectChoiceAudience {
    #[default]
    Actor,
    EachHero,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StructuralRule {
    GameOneSetup,
    GameTwoSetup,
    GameThreeSetup,
    GameFourSetup,
    GameOnePrecedence,
    NoHeroAbility,
    NoLocationEffect,
}

/// Closed Rust strategies for the four printed Game 3 abilities.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HeroAbilityStrategy {
    HarryGameThreeV1,
    HermioneGameThreeV1,
    NevilleGameThreeV1,
    RonGameThreeV1,
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
    DrawingAllowed,
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
    /// Stable binding key for a manually selected target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
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
    Other,
    #[default]
    Any,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Eligibility {
    CardType { card_type: CardType },
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
    LocationDeck,
    LocationDiscard,
    Market,
    VillainDeck,
    VillainDiscard,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    GainInfluenceAndDraw { influence: u8, cards: u8 },
    SuppressVillain,
    GainInfluenceAndHealth { influence: u8, health: u8 },
    DiscardForSpellBonus { influence: u8 },
    DiscardVoluntarily,
    CopyPlayedAlly,
    GainAttackPerAllyPlayed { amount: u8 },
    Discard,
    PreventDrawing,
    Draw { amount: u8 },
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
    GryffindorV1,
    HufflepuffV1,
    RavenclawV1,
    SlytherinV1,
    D4,
    D6,
    D8,
}

impl Die {
    #[must_use]
    pub const fn is_house(self) -> bool {
        matches!(
            self,
            Self::GryffindorV1 | Self::HufflepuffV1 | Self::RavenclawV1 | Self::SlytherinV1
        )
    }

    #[must_use]
    pub const fn sides(self) -> usize {
        match self {
            Self::D4 => 4,
            Self::D6
            | Self::GryffindorV1
            | Self::HufflepuffV1
            | Self::RavenclawV1
            | Self::SlytherinV1 => 6,
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
