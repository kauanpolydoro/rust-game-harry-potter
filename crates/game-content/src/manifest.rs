use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{CatalogId, EffectRule, RuleId, Zone};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ContentManifest {
    pub manifest_version: u16,
    pub content_version: String,
    pub ruleset_version: String,
    pub digest: String,
    pub record_count: usize,
    pub card_count: u32,
    pub playable: bool,
    pub gaps: Vec<ContentGap>,
    pub entries: Vec<ManifestEntry>,
    pub executable_rules: BTreeSet<RuleId>,
    pub rules: Vec<EffectRule>,
    pub game_setups: Vec<GameSetup>,
    pub sources: Vec<ProvenanceSource>,
}

/// An externally trusted, ordered setup for one adventure.
///
/// Publication proves the setup provenance and structural validity only.
/// Consumers must also require `ContentManifest::playable` and each referenced
/// `ManifestEntry::playable` before creating a game.
/// Every referenced rule ID must belong to `ContentManifest::executable_rules`, and its definition
/// must resolve from `ContentManifest::rules`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GameSetup {
    pub adventure_id: CatalogId,
    pub confidence: FunctionalConfidence,
    pub sources: Vec<String>,
    pub entities: Vec<GameSetupEntity>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GameSetupEntity {
    pub catalog_id: CatalogId,
    pub copies: u16,
    pub zone: Zone,
    pub owner: GameSetupOwner,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameSetupOwner {
    None,
    EachParticipant,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManifestEntry {
    pub catalog_id: CatalogId,
    pub kind: EntryKind,
    pub set: ContentSet,
    pub copies: u16,
    pub introduced_in: u8,
    pub names: BTreeMap<String, String>,
    pub provenance: BTreeMap<String, Vec<String>>,
    pub functional_provenance: BTreeMap<FunctionalField, FunctionalProvenance>,
    pub playable: bool,
    pub gaps: Vec<FunctionalField>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ContentGap {
    pub entry_id: CatalogId,
    pub field: FunctionalField,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProvenanceSource {
    pub id: String,
    pub uri: String,
    pub kind: SourceKind,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FunctionalProvenance {
    pub confidence: FunctionalConfidence,
    pub sources: Vec<String>,
    pub rule_id: Option<RuleId>,
    pub value: Option<u16>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FunctionalField {
    Ability,
    ControlLimit,
    Cost,
    DarkArtsCount,
    Effect,
    Health,
    Precedence,
    Reward,
    Setup,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentSet {
    Base,
    Expansion,
    Promo,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Adventure,
    Catalog,
    DarkArts,
    Hero,
    HogwartsCard,
    Horcrux,
    Location,
    Proficiency,
    Ruleset,
    StarterCard,
    TurnOrder,
    Villain,
}

impl EntryKind {
    pub(crate) fn required_functional_fields(self) -> BTreeSet<FunctionalField> {
        use FunctionalField::{
            Ability, ControlLimit, Cost, DarkArtsCount, Effect, Health, Precedence, Reward, Setup,
        };

        let fields: &[FunctionalField] = match self {
            Self::Adventure => &[Precedence, Setup],
            Self::Catalog | Self::TurnOrder => &[],
            Self::DarkArts | Self::StarterCard => &[Effect],
            Self::Hero | Self::Proficiency => &[Ability],
            Self::HogwartsCard => &[Cost, Effect],
            Self::Horcrux => &[Effect, Precedence, Reward],
            Self::Location => &[ControlLimit, DarkArtsCount, Effect],
            Self::Ruleset => &[Precedence],
            Self::Villain => &[Effect, Health, Reward],
        };

        fields.iter().copied().collect()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Adaptation,
    Candidate,
    Official,
    Validated,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FunctionalConfidence {
    Adaptation,
    Candidate,
    Official,
    Unknown,
    Validated,
}
