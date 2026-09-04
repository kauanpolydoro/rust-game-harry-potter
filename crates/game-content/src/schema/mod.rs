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
const REQUIRED_BASE_ENTRY_KINDS: [EntryKind; 12] = [
    EntryKind::Adventure,
    EntryKind::Catalog,
    EntryKind::DarkArts,
    EntryKind::Hero,
    EntryKind::HogwartsCard,
    EntryKind::Horcrux,
    EntryKind::Location,
    EntryKind::Proficiency,
    EntryKind::Ruleset,
    EntryKind::StarterCard,
    EntryKind::TurnOrder,
    EntryKind::Villain,
];

/// Imports a complete base-game candidate bundle.
///
/// # Errors
///
/// Returns a closed validation failure when the bundle cannot be published.
pub fn import_base_bundle(bytes: &[u8]) -> Result<ContentManifest, ImportFailure> {
    import_base_bundle_with_trusted_sources(bytes, &[])
}

/// Imports a complete base-game bundle using trust decisions supplied outside
/// the bundle itself.
///
/// A declared source contributes to functional playability only when its ID,
/// URI, and kind exactly match an entry in `trusted_sources`.
///
/// # Errors
///
/// Returns a closed validation failure when the bundle cannot be published.
pub fn import_base_bundle_with_trusted_sources(
    bytes: &[u8],
    trusted_sources: &[ProvenanceSource],
) -> Result<ContentManifest, ImportFailure> {
    import_base_bundle_with_runtime_rules(bytes, trusted_sources, &BTreeSet::new())
}

/// Imports a bundle using external source trust and the exact rule IDs that
/// the current runtime can execute.
///
/// Source trust proves the meaning of a rule. Runtime support is a separate
/// requirement because the importer does not publish discarded AST as
/// playable content.
///
/// # Errors
///
/// Returns a closed validation failure when the bundle cannot be published.
pub fn import_base_bundle_with_runtime_rules(
    bytes: &[u8],
    trusted_sources: &[ProvenanceSource],
    executable_rules: &BTreeSet<RuleId>,
) -> Result<ContentManifest, ImportFailure> {
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

    let source_kinds = bundle
        .sources
        .iter()
        .filter(|source| {
            trusted_sources.iter().any(|trusted| {
                trusted.id == source.id && trusted.uri == source.uri && trusted.kind == source.kind
            })
        })
        .map(|source| (source.id.clone(), source.kind))
        .collect::<BTreeMap<_, _>>();
    let substantive_rules = bundle
        .substantive_rules()
        .intersection(executable_rules)
        .cloned()
        .collect::<BTreeSet<_>>();
    let entries = bundle
        .entries
        .iter()
        .map(|entry| entry.to_manifest_entry(&source_kinds, &substantive_rules))
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
    let has_required_catalog_shape = REQUIRED_BASE_ENTRY_KINDS
        .iter()
        .all(|kind| entries.iter().any(|entry| entry.kind == *kind));
    let has_executable_rules = !substantive_rules.is_empty();
    let sources = bundle
        .sources
        .iter()
        .map(|source| ProvenanceSource {
            id: source.id.clone(),
            uri: source.uri.clone(),
            kind: source.kind,
        })
        .collect();
    let digest_input = ManifestDigestInput {
        bundle: &bundle,
        trusted_source_kinds: &source_kinds,
        executable_rules: &substantive_rules,
    };
    let canonical = serde_json::to_vec(&digest_input).map_err(|error| ImportFailure {
        message: format!("manifest identity could not be canonicalized: {error}"),
    })?;

    Ok(ContentManifest {
        manifest_version: 1,
        content_version: bundle.content_version,
        ruleset_version: bundle.ruleset_version,
        digest: format!("blake3:{}", blake3::hash(&canonical).to_hex()),
        record_count: BASE_RECORD_COUNT,
        card_count: BASE_CARD_COUNT,
        playable: gaps.is_empty() && has_required_catalog_shape && has_executable_rules,
        gaps,
        entries,
        sources,
    })
}

#[derive(Serialize)]
struct ManifestDigestInput<'a> {
    bundle: &'a CandidateBundle,
    trusted_source_kinds: &'a BTreeMap<String, SourceKind>,
    executable_rules: &'a BTreeSet<RuleId>,
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

    fn substantive_rules(&self) -> BTreeSet<RuleId> {
        let rules = self
            .rules
            .iter()
            .map(|rule| (&rule.id, &rule.effect))
            .collect::<BTreeMap<_, _>>();

        self.rules
            .iter()
            .filter(|rule| rule.effect.has_operation(&rules, &mut BTreeSet::new()))
            .map(|rule| rule.id.clone())
            .collect()
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
    fn to_manifest_entry(
        &self,
        source_kinds: &BTreeMap<String, SourceKind>,
        substantive_rules: &BTreeSet<RuleId>,
    ) -> ManifestEntry {
        let gaps = self
            .required_functional_fields
            .iter()
            .filter(|field| !self.field_is_proven(**field, source_kinds, substantive_rules))
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
        substantive_rules: &BTreeSet<RuleId>,
    ) -> bool {
        self.functional
            .get(&field)
            .is_some_and(|definition| definition.is_proven(source_kinds, substantive_rules))
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
    fn is_proven(
        &self,
        source_kinds: &BTreeMap<String, SourceKind>,
        substantive_rules: &BTreeSet<RuleId>,
    ) -> bool {
        let expected_source_kind = match self.confidence {
            FunctionalConfidence::Adaptation => Some(SourceKind::Adaptation),
            FunctionalConfidence::Official => Some(SourceKind::Official),
            FunctionalConfidence::Validated => Some(SourceKind::Validated),
            FunctionalConfidence::Candidate | FunctionalConfidence::Unknown => None,
        };

        self.rule
            .as_ref()
            .is_some_and(|rule| substantive_rules.contains(rule))
            && expected_source_kind.is_some_and(|expected| {
                self.sources
                    .iter()
                    .any(|source_id| source_kinds.get(source_id) == Some(&expected))
            })
    }
}

impl Effect {
    fn has_operation(
        &self,
        rules: &BTreeMap<&RuleId, &Self>,
        visited: &mut BTreeSet<RuleId>,
    ) -> bool {
        match self {
            Self::Apply { .. } => true,
            Self::Choice { options } => options
                .iter()
                .any(|option| option.has_operation(rules, visited)),
            Self::NoOp => false,
            Self::Reference { rule } => {
                visited.insert(rule.clone())
                    && rules
                        .get(rule)
                        .is_some_and(|effect| effect.has_operation(rules, visited))
            }
        }
    }
}
