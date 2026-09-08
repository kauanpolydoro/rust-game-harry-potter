mod validation;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    CatalogId, ContentGap, ContentManifest, ContentSet, Effect, EffectRule, EntryKind,
    FunctionalConfidence, FunctionalField, FunctionalProvenance, GameSetup, GameSetupEntity,
    GameSetupOwner, ImportFailure, ManifestEntry, ProvenanceSource, RuleId, SourceKind, Zone,
};

const BASE_RECORD_COUNT: usize = 171;
const BASE_CARD_COUNT: u32 = 252;

/// Returns the validated Game 1 AST for the host to check runtime capabilities.
/// This inspection does not grant functional trust or publish a playable manifest.
///
/// # Errors
///
/// Returns a closed validation failure for invalid inventory, provenance, or rules.
pub fn inspect_game_one_rules(bytes: &[u8]) -> Result<Vec<EffectRule>, ImportFailure> {
    let mut bundle = parse_bundle(bytes, Inventory::GameOne)?;
    bundle.canonicalize();
    validation::validate(&bundle, Inventory::GameOne)?;
    Ok(bundle.rules)
}

/// Imports the Game 1 inventory without granting trust to functional sources.
///
/// # Errors
///
/// Returns a closed validation failure for an invalid Game 1 bundle.
pub fn import_game_one_bundle(bytes: &[u8]) -> Result<ContentManifest, ImportFailure> {
    import_bundle(bytes, &[], &BTreeSet::new(), Inventory::GameOne)
}

/// Imports Game 1 with explicit source trust and runtime capabilities.
///
/// # Errors
///
/// Returns a closed validation failure for invalid inventory, provenance, or rules.
pub fn import_game_one_bundle_with_runtime_rules(
    bytes: &[u8],
    trusted_sources: &[ProvenanceSource],
    executable_rules: &BTreeSet<RuleId>,
) -> Result<ContentManifest, ImportFailure> {
    import_bundle(bytes, trusted_sources, executable_rules, Inventory::GameOne)
}

/// Inspects Game 2 without granting source trust or runtime support.
///
/// # Errors
/// Returns a validation failure for an invalid cumulative bundle.
pub fn inspect_game_two_rules(bytes: &[u8]) -> Result<Vec<EffectRule>, ImportFailure> {
    let mut bundle = parse_bundle(bytes, Inventory::GameTwo)?;
    bundle.canonicalize();
    validation::validate(&bundle, Inventory::GameTwo)?;
    Ok(bundle.rules)
}

/// Imports Game 2 without granting functional trust.
///
/// # Errors
/// Returns a validation failure for an invalid cumulative bundle.
pub fn import_game_two_bundle(bytes: &[u8]) -> Result<ContentManifest, ImportFailure> {
    import_bundle(bytes, &[], &BTreeSet::new(), Inventory::GameTwo)
}

/// Imports Game 2 with externally granted provenance and runtime capabilities.
///
/// # Errors
/// Returns a validation failure for invalid inventory, provenance, or rules.
pub fn import_game_two_bundle_with_runtime_rules(
    bytes: &[u8],
    trusted_sources: &[ProvenanceSource],
    executable_rules: &BTreeSet<RuleId>,
) -> Result<ContentManifest, ImportFailure> {
    import_bundle(bytes, trusted_sources, executable_rules, Inventory::GameTwo)
}

/// Inspects Game 3 without granting source trust or runtime support.
///
/// # Errors
/// Returns a validation failure for an invalid cumulative bundle.
pub fn inspect_game_three_rules(bytes: &[u8]) -> Result<Vec<EffectRule>, ImportFailure> {
    let mut bundle = parse_bundle(bytes, Inventory::GameThree)?;
    bundle.canonicalize();
    validation::validate(&bundle, Inventory::GameThree)?;
    Ok(bundle.rules)
}

/// Imports Game 3 without granting functional trust.
///
/// # Errors
/// Returns a validation failure for an invalid cumulative bundle.
pub fn import_game_three_bundle(bytes: &[u8]) -> Result<ContentManifest, ImportFailure> {
    import_bundle(bytes, &[], &BTreeSet::new(), Inventory::GameThree)
}

/// Imports Game 3 with externally granted provenance and runtime capabilities.
///
/// # Errors
/// Returns a validation failure for invalid inventory, provenance, or rules.
pub fn import_game_three_bundle_with_runtime_rules(
    bytes: &[u8],
    trusted_sources: &[ProvenanceSource],
    executable_rules: &BTreeSet<RuleId>,
) -> Result<ContentManifest, ImportFailure> {
    import_bundle(
        bytes,
        trusted_sources,
        executable_rules,
        Inventory::GameThree,
    )
}

#[derive(Clone, Copy)]
enum Inventory {
    Base,
    GameOne,
    GameTwo,
    GameThree,
}

impl Inventory {
    const fn schema_version(self) -> u16 {
        match self {
            Self::Base => 2,
            Self::GameOne => 3,
            Self::GameTwo => 4,
            Self::GameThree => 5,
        }
    }

    const fn manifest_version(self) -> u16 {
        match self {
            Self::Base => 3,
            Self::GameOne => 4,
            Self::GameTwo => 5,
            Self::GameThree => 6,
        }
    }

    const fn counts(self) -> (usize, u32) {
        match self {
            Self::Base => (BASE_RECORD_COUNT, BASE_CARD_COUNT),
            Self::GameOne => (45, 93),
            Self::GameTwo => (63, 116),
            Self::GameThree => (77, 138),
        }
    }
}
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
    import_bundle(bytes, trusted_sources, executable_rules, Inventory::Base)
}

fn import_bundle(
    bytes: &[u8],
    trusted_sources: &[ProvenanceSource],
    executable_rules: &BTreeSet<RuleId>,
    inventory: Inventory,
) -> Result<ContentManifest, ImportFailure> {
    let mut bundle = parse_bundle(bytes, inventory)?;

    bundle.canonicalize();
    validation::validate(&bundle, inventory)?;

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
    let substantive_rules = bundle.supported_roots(inventory, executable_rules, &source_kinds);
    validation::validate_runtime_rules(&bundle, &substantive_rules)?;
    let runtime_rules = bundle.rule_closure(&substantive_rules);
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
        .filter(|kind| {
            matches!(inventory, Inventory::Base)
                || !matches!(kind, EntryKind::Horcrux | EntryKind::Proficiency)
        })
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
    let game_setups: Vec<_> = bundle
        .game_setups
        .iter()
        .filter(|setup| setup.is_proven(&source_kinds))
        .map(CandidateGameSetup::to_manifest_setup)
        .collect();
    let has_required_setup = matches!(inventory, Inventory::Base)
        || (game_setups.len() == 1
            && bundle
                .rules
                .iter()
                .all(|rule| substantive_rules.contains(&rule.id)));
    let digest_input = ManifestDigestInput {
        manifest_version: inventory.manifest_version(),
        bundle: &bundle,
        trusted_source_kinds: &source_kinds,
        executable_rules: &substantive_rules,
    };
    let canonical = serde_json::to_vec(&digest_input).map_err(|error| ImportFailure {
        message: format!("manifest identity could not be canonicalized: {error}"),
    })?;

    Ok(ContentManifest {
        manifest_version: inventory.manifest_version(),
        content_version: bundle.content_version,
        ruleset_version: bundle.ruleset_version,
        digest: format!("blake3:{}", blake3::hash(&canonical).to_hex()),
        record_count: inventory.counts().0,
        card_count: inventory.counts().1,
        playable: gaps.is_empty()
            && has_required_catalog_shape
            && has_executable_rules
            && has_required_setup,
        gaps,
        entries,
        executable_rules: substantive_rules,
        rules: bundle
            .rules
            .iter()
            .filter(|rule| runtime_rules.contains(&rule.id))
            .cloned()
            .collect(),
        game_setups,
        sources,
    })
}

fn parse_bundle(bytes: &[u8], inventory: Inventory) -> Result<CandidateBundle, ImportFailure> {
    let header: CandidateBundleHeader =
        serde_json::from_slice(bytes).map_err(|error| ImportFailure {
            message: format!("bundle is not valid JSON: {error}"),
        })?;

    if header.schema_version != inventory.schema_version() {
        return Err(ImportFailure {
            message: format!(
                "unsupported bundle schema version: {}",
                header.schema_version
            ),
        });
    }

    let bundle: CandidateBundle = serde_json::from_slice(bytes).map_err(|error| ImportFailure {
        message: format!(
            "bundle is not valid schema v{} JSON: {error}",
            inventory.schema_version()
        ),
    })?;

    Ok(bundle)
}

#[derive(Serialize)]
struct ManifestDigestInput<'a> {
    manifest_version: u16,
    bundle: &'a CandidateBundle,
    trusted_source_kinds: &'a BTreeMap<String, SourceKind>,
    executable_rules: &'a BTreeSet<RuleId>,
}

#[derive(Deserialize)]
struct CandidateBundleHeader {
    schema_version: u16,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateBundle {
    schema_version: u16,
    content_version: String,
    ruleset_version: String,
    locale: String,
    sources: Vec<Source>,
    rules: Vec<EffectRule>,
    #[serde(default)]
    game_setups: Vec<CandidateGameSetup>,
    entries: Vec<CandidateEntry>,
}

impl CandidateBundle {
    fn supported_roots(
        &self,
        inventory: Inventory,
        executable_rules: &BTreeSet<RuleId>,
        source_kinds: &BTreeMap<String, SourceKind>,
    ) -> BTreeSet<RuleId> {
        self.substantive_rules()
            .intersection(executable_rules)
            .filter(|id| {
                matches!(inventory, Inventory::Base)
                    || self.rules.iter().any(|rule| {
                        &rule.id == *id
                            && rule.provenance.as_ref().is_some_and(|provenance| {
                                has_trusted_provenance(
                                    provenance.confidence,
                                    &provenance.sources,
                                    source_kinds,
                                )
                            })
                    })
            })
            .cloned()
            .collect::<BTreeSet<_>>()
    }

    fn canonicalize(&mut self) {
        self.sources.sort_by(|left, right| left.id.cmp(&right.id));
        self.entries.sort_by(|left, right| left.id.cmp(&right.id));
        self.rules.sort_by(|left, right| {
            left.trigger
                .phase_order()
                .cmp(&right.trigger.phase_order())
                .then(left.order.cmp(&right.order))
                .then(left.id.cmp(&right.id))
        });
        for rule in &mut self.rules {
            if let Some(provenance) = &mut rule.provenance {
                provenance.sources.sort();
                provenance.sources.dedup();
            }
        }
        self.game_setups
            .sort_by(|left, right| left.adventure_id.cmp(&right.adventure_id));
        for setup in &mut self.game_setups {
            setup.sources.sort();
        }
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

    fn rule_closure(&self, roots: &BTreeSet<RuleId>) -> BTreeSet<RuleId> {
        let rules = self
            .rules
            .iter()
            .map(|rule| (&rule.id, &rule.effect))
            .collect::<BTreeMap<_, _>>();
        let mut pending = roots.iter().collect::<Vec<_>>();
        let mut closure = roots.clone();
        while let Some(rule_id) = pending.pop() {
            if let Some(effect) = rules.get(rule_id) {
                for reference in effect.references() {
                    if closure.insert(reference.clone()) {
                        pending.push(reference);
                    }
                }
            }
        }
        closure
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateGameSetup {
    adventure_id: CatalogId,
    confidence: FunctionalConfidence,
    sources: Vec<String>,
    entities: Vec<CandidateGameSetupEntity>,
}

impl CandidateGameSetup {
    fn is_proven(&self, source_kinds: &BTreeMap<String, SourceKind>) -> bool {
        has_trusted_provenance(self.confidence, &self.sources, source_kinds)
    }

    fn to_manifest_setup(&self) -> GameSetup {
        GameSetup {
            adventure_id: self.adventure_id.clone(),
            confidence: self.confidence,
            sources: self.sources.clone(),
            entities: self
                .entities
                .iter()
                .map(|entity| GameSetupEntity {
                    catalog_id: entity.catalog_id.clone(),
                    copies: entity.copies,
                    zone: entity.zone,
                    owner: entity.owner,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateGameSetupEntity {
    catalog_id: CatalogId,
    copies: u16,
    zone: Zone,
    owner: GameSetupOwner,
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
                        value: definition.value,
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
            .is_some_and(|definition| definition.is_proven(field, source_kinds, substantive_rules))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FunctionalDefinition {
    confidence: FunctionalConfidence,
    sources: Vec<String>,
    rule: Option<RuleId>,
    #[serde(default)]
    value: Option<u16>,
}

impl FunctionalDefinition {
    fn is_proven(
        &self,
        field: FunctionalField,
        source_kinds: &BTreeMap<String, SourceKind>,
        substantive_rules: &BTreeSet<RuleId>,
    ) -> bool {
        let supported_definition = match field {
            FunctionalField::Cost
            | FunctionalField::Health
            | FunctionalField::ControlLimit
            | FunctionalField::DarkArtsCount => {
                self.value.is_some()
                    || self
                        .rule
                        .as_ref()
                        .is_some_and(|rule| substantive_rules.contains(rule))
            }
            _ => self
                .rule
                .as_ref()
                .is_some_and(|rule| substantive_rules.contains(rule)),
        };

        supported_definition && has_trusted_provenance(self.confidence, &self.sources, source_kinds)
    }
}

fn has_trusted_provenance(
    confidence: FunctionalConfidence,
    sources: &[String],
    source_kinds: &BTreeMap<String, SourceKind>,
) -> bool {
    let expected_source_kind = match confidence {
        FunctionalConfidence::Adaptation => Some(SourceKind::Adaptation),
        FunctionalConfidence::Official => Some(SourceKind::Official),
        FunctionalConfidence::Validated => Some(SourceKind::Validated),
        FunctionalConfidence::Candidate | FunctionalConfidence::Unknown => None,
    };

    expected_source_kind.is_some_and(|expected| {
        sources
            .iter()
            .any(|source_id| source_kinds.get(source_id) == Some(&expected))
    })
}

impl Effect {
    fn references(&self) -> Vec<&RuleId> {
        match self {
            Self::Choice { options, .. }
            | Self::Roll {
                outcomes: options, ..
            }
            | Self::Sequence { effects: options } => {
                options.iter().flat_map(Self::references).collect()
            }
            Self::Condition {
                then, otherwise, ..
            } => {
                let mut references = then.references();
                if let Some(otherwise) = otherwise {
                    references.extend(otherwise.references());
                }
                references
            }
            Self::HeroAbility { effect, .. }
            | Self::RevealTopCard { effect, .. }
            | Self::ForEachTarget { effect, .. }
            | Self::Repeat { effect, .. }
            | Self::Reaction { effect, .. } => effect.references(),
            Self::Reference { rule } => vec![rule],
            Self::LimitVillainAttack { .. }
            | Self::PreventExtraDrawing
            | Self::Apply { .. }
            | Self::TopDeckAcquisition { .. }
            | Self::CardType { .. }
            | Self::HandDamageLimit { .. }
            | Self::NoOp
            | Self::Structural { .. }
            | Self::RevealDarkArts
            | Self::Terminal { .. } => Vec::new(),
        }
    }

    fn has_operation(
        &self,
        rules: &BTreeMap<&RuleId, &Self>,
        visited: &mut BTreeSet<RuleId>,
    ) -> bool {
        match self {
            Self::LimitVillainAttack { .. }
            | Self::PreventExtraDrawing
            | Self::Apply { .. }
            | Self::TopDeckAcquisition { .. }
            | Self::HandDamageLimit { .. }
            | Self::Structural { .. }
            | Self::RevealDarkArts
            | Self::Terminal { .. } => true,
            Self::Choice { options, .. } => options
                .iter()
                .any(|option| option.has_operation(rules, visited)),
            Self::Condition {
                then, otherwise, ..
            } => {
                then.has_operation(rules, visited)
                    || otherwise
                        .as_deref()
                        .is_some_and(|effect| effect.has_operation(rules, visited))
            }
            Self::NoOp | Self::CardType { .. } => false,
            Self::Reference { rule } => {
                visited.insert(rule.clone())
                    && rules
                        .get(rule)
                        .is_some_and(|effect| effect.has_operation(rules, visited))
            }
            Self::HeroAbility { effect, .. }
            | Self::RevealTopCard { effect, .. }
            | Self::ForEachTarget { effect, .. }
            | Self::Repeat { effect, .. }
            | Self::Reaction { effect, .. } => effect.has_operation(rules, visited),
            Self::Roll { outcomes, .. } => outcomes
                .iter()
                .any(|outcome| outcome.has_operation(rules, visited)),
            Self::Sequence { effects } => effects
                .iter()
                .any(|effect| effect.has_operation(rules, visited)),
        }
    }
}
