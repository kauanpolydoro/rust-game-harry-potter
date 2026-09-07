use std::collections::BTreeMap;

use game_content::{ContentManifest, ProvenanceSource, SourceKind};

/// Loads the checked-in Game 1 adaptation after validating provenance and runtime support.
///
/// # Panics
///
/// Panics if the versioned bundle is invalid, cannot compile, or has functional gaps.
/// Content and runtime tests protect this startup invariant.
#[must_use]
pub fn game_one_manifest() -> ContentManifest {
    let bytes = include_bytes!("../../../../content/bundles/game-one-en-v1.json");
    let inspected = game_content::inspect_game_one_rules(bytes)
        .expect("the shipped Game 1 bundle must contain a valid closed AST");
    let rules = inspected
        .iter()
        .map(|rule| (&rule.id, rule))
        .collect::<BTreeMap<_, _>>();
    let compiled = inspected
        .iter()
        .map(|rule| super::compile_rule(rule, &rules))
        .collect::<Option<Vec<_>>>()
        .expect("every shipped Game 1 rule must compile before receiving runtime support");
    game_domain::ValidatedGameRules::new(compiled)
        .expect("the compiled Game 1 rules must satisfy domain execution limits");
    let supported = inspected.iter().map(|rule| rule.id.clone()).collect();
    let manifest = game_content::import_game_one_bundle_with_runtime_rules(
        bytes,
        &[ProvenanceSource {
            id: "game-one-adaptation-v1".to_owned(),
            uri: "https://github.com/kauanpolydoro/rust-game-harry-potter/blob/main/content/game-one-rules-v1.md".to_owned(),
            kind: SourceKind::Adaptation,
        }],
        &supported,
    ).expect("the shipped Game 1 content must pass the publication boundary");
    assert!(
        manifest.playable,
        "the shipped Game 1 manifest must close every functional gap"
    );
    manifest
}
