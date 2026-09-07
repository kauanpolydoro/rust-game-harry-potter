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
    adventure_manifest(false)
}

/// Loads the cumulative Game 2 adaptation with independently granted provenance.
///
/// # Panics
/// Panics if the shipped bundle is invalid, unsupported, or incomplete.
#[must_use]
pub fn game_two_manifest() -> ContentManifest {
    adventure_manifest(true)
}

fn adventure_manifest(game_two: bool) -> ContentManifest {
    let bytes: &[u8] = if game_two {
        include_bytes!("../../../../content/bundles/game-two-en-v1.json")
    } else {
        include_bytes!("../../../../content/bundles/game-one-en-v1.json")
    };
    let inspect = if game_two {
        game_content::inspect_game_two_rules
    } else {
        game_content::inspect_game_one_rules
    };
    let import = if game_two {
        game_content::import_game_two_bundle_with_runtime_rules
    } else {
        game_content::import_game_one_bundle_with_runtime_rules
    };
    let inspected = inspect(bytes).expect("shipped adventure must contain a valid closed AST");
    let rules = inspected
        .iter()
        .map(|rule| (&rule.id, rule))
        .collect::<BTreeMap<_, _>>();
    let compiled = inspected
        .iter()
        .map(|rule| super::compile_rule(rule, &rules))
        .collect::<Option<Vec<_>>>()
        .expect("every shipped rule must compile before receiving runtime support");
    game_domain::ValidatedGameRules::new(compiled).expect("compiled rules satisfy domain limits");
    let supported = inspected.iter().map(|rule| rule.id.clone()).collect();
    let sources = ["one", "two"].into_iter().take(if game_two { 2 } else { 1 }).map(|game| ProvenanceSource {
        id: format!("game-{game}-adaptation-v1"),
        uri: format!("https://github.com/kauanpolydoro/rust-game-harry-potter/blob/main/content/game-{game}-rules-v1.md"),
        kind: SourceKind::Adaptation,
    }).collect::<Vec<_>>();
    let manifest = import(bytes, &sources, &supported)
        .expect("shipped content passes the publication boundary");
    assert!(
        manifest.playable,
        "shipped manifest must close every functional gap"
    );
    manifest
}
