//! Candidate content import and validation.

#![forbid(unsafe_code)]

use std::fmt;

mod ids;
mod manifest;
mod schema;

mod effects;

pub use effects::{
    Cardinality, Condition, Die, Effect, EffectChoiceAudience, EffectRule, EffectTrigger,
    Eligibility, GameOutcome, Operation, Resource, ResourceCost, Selector, TargetOwner, Zone,
};
pub use ids::{CardInstanceId, CatalogId, InvalidId, RuleId};
pub use manifest::{
    ContentGap, ContentManifest, ContentSet, EntryKind, FunctionalConfidence, FunctionalField,
    FunctionalProvenance, GameSetup, GameSetupEntity, GameSetupOwner, ManifestEntry,
    ProvenanceSource, SourceKind,
};
pub use schema::{
    import_base_bundle, import_base_bundle_with_runtime_rules,
    import_base_bundle_with_trusted_sources,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFailure {
    pub(crate) message: String,
}

impl fmt::Display for ImportFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ImportFailure {}
