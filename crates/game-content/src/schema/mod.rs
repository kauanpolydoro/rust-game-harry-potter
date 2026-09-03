mod validation;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    CatalogId, ContentGap, ContentManifest, ContentSet, EntryKind, FunctionalConfidence,
    FunctionalField, FunctionalProvenance, ImportFailure, ManifestEntry, ProvenanceSource, RuleId,
    SourceKind,
};

const BASE_RECORD_COUNT: usize = 171;
const BASE_CARD_COUNT: u32 = 252;

/// Imports a complete base-game candidate bundle.
///
/// # Errors
///
/// Returns a closed validation failure when the bundle cannot be published.
pub fn import_base_bundle(bytes: &[u8]) -> Result<ContentManifest, ImportFailure> {
    let mut bundle: CandidateBundle =
        serde_json::from_slice(bytes).map_err(|error| ImportFailure {
            message: format!("bundle is not valid schema v1 JSON: {error}"),
        })?;

    if bundle.schema_version != 1 {
        return Err(ImportFailure {
            message: format!(
                "unsupported bundle schema version: {}",
                bundle.schema_version
            ),
        });
    }

    bundle.canonicalize();
    validation::validate(&bundle)?;

    let canonical = serde_json::to_vec(&bundle).map_err(|error| ImportFailure {
        message: format!("bundle could not be canonicalized: {error}"),
    })?;
    let source_kinds = bundle
        .sources
        .iter()
        .map(|source| (source.id.clone(), source.kind))
        .collect::<BTreeMap<_, _>>();
    let entries = bundle
        .entries
        .iter()
        .map(|entry| entry.to_manifest_entry(&source_kinds))
        .collect::<Vec<_>>();
    let gaps = entries
        .iter()
        .flat_map(|entry| {
            entry.gaps.iter().map(|field| ContentGap {
                entry_id: entry.catalog_id.clone(),
                field: *field,
            })
        })
        .collect::<Vec<_>>();
    let sources = bundle
        .sources
        .iter()
        .map(|source| ProvenanceSource {
            id: source.id.clone(),
            uri: source.uri.clone(),
            kind: source.kind,
        })
        .collect();

    Ok(ContentManifest {
        manifest_version: 1,
        content_version: bundle.content_version,
        ruleset_version: bundle.ruleset_version,
        digest: format!("blake3:{}", blake3::hash(&canonical).to_hex()),
        record_count: BASE_RECORD_COUNT,
        card_count: BASE_CARD_COUNT,
        playable: gaps.is_empty(),
        gaps,
        entries,
        sources,
    })
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateBundle {
    schema_version: u16,
    content_version: String,
    ruleset_version: String,
    locale: String,
    sources: Vec<Source>,
    rules: Vec<RuleDefinition>,
    entries: Vec<CandidateEntry>,
}

impl CandidateBundle {
    fn canonicalize(&mut self) {
        self.sources.sort_by(|left, right| left.id.cmp(&right.id));
        self.entries.sort_by(|left, right| left.id.cmp(&right.id));
        self.rules.sort_by(|left, right| left.id.cmp(&right.id));
        for entry in &mut self.entries {
            for source_ids in entry.provenance.values_mut() {
                source_ids.sort();
                source_ids.dedup();
            }
            for definition in entry.functional.values_mut() {
                definition.sources.sort();
                definition.sources.dedup();
            }
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Source {
    id: String,
    uri: String,
    kind: SourceKind,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RuleDefinition {
    id: RuleId,
    effect: Effect,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Effect {
    Apply {
        target: Selector,
        operation: Operation,
    },
    Choice {
        options: Vec<Effect>,
    },
    NoOp,
    Reference {
        rule: RuleId,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selector {
    zone: Zone,
    cardinality: Cardinality,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cardinality {
    min: u16,
    max: u16,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Zone {
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Discard,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateEntry {
    id: CatalogId,
    kind: EntryKind,
    set: ContentSet,
    copies: u16,
    introduced_in: u8,
    names: BTreeMap<String, String>,
    provenance: BTreeMap<String, Vec<String>>,
    required_functional_fields: BTreeSet<FunctionalField>,
    #[serde(default)]
    functional: BTreeMap<FunctionalField, FunctionalDefinition>,
}

impl CandidateEntry {
    fn to_manifest_entry(&self, source_kinds: &BTreeMap<String, SourceKind>) -> ManifestEntry {
        let gaps = self
            .required_functional_fields
            .iter()
            .filter(|field| !self.field_is_proven(**field, source_kinds))
            .copied()
            .collect::<Vec<_>>();
        let functional_provenance = self
            .functional
            .iter()
            .map(|(field, definition)| {
                (
                    *field,
                    FunctionalProvenance {
                        confidence: definition.confidence,
                        sources: definition.sources.clone(),
                        rule_id: definition.rule.clone(),
                    },
                )
            })
            .collect();

        ManifestEntry {
            catalog_id: self.id.clone(),
            kind: self.kind,
            set: self.set,
            copies: self.copies,
            introduced_in: self.introduced_in,
            names: self.names.clone(),
            provenance: self.provenance.clone(),
            functional_provenance,
            playable: gaps.is_empty(),
            gaps,
        }
    }

    fn field_is_proven(
        &self,
        field: FunctionalField,
        source_kinds: &BTreeMap<String, SourceKind>,
    ) -> bool {
        self.functional
            .get(&field)
            .is_some_and(|definition| definition.is_proven(source_kinds))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FunctionalDefinition {
    confidence: FunctionalConfidence,
    sources: Vec<String>,
    rule: Option<RuleId>,
}

impl FunctionalDefinition {
    fn is_proven(&self, source_kinds: &BTreeMap<String, SourceKind>) -> bool {
        let expected_source_kind = match self.confidence {
            FunctionalConfidence::Adaptation => Some(SourceKind::Adaptation),
            FunctionalConfidence::Official => Some(SourceKind::Official),
            FunctionalConfidence::Validated => Some(SourceKind::Validated),
            FunctionalConfidence::Candidate | FunctionalConfidence::Unknown => None,
        };

        self.rule.is_some()
            && expected_source_kind.is_some_and(|expected| {
                self.sources
                    .iter()
                    .any(|source_id| source_kinds.get(source_id) == Some(&expected))
            })
    }
}
