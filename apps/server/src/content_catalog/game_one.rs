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
    adventure_manifest(super::GamePreparation::One)
}

/// Loads the cumulative Game 2 adaptation with independently granted provenance.
///
/// # Panics
/// Panics if the shipped bundle is invalid, unsupported, or incomplete.
#[must_use]
pub fn game_two_manifest() -> ContentManifest {
    adventure_manifest(super::GamePreparation::Two)
}

/// Loads the cumulative Game 3 adaptation with replacement Hero abilities.
///
/// # Panics
/// Panics if the shipped bundle is invalid, unsupported, or incomplete.
#[must_use]
pub fn game_three_manifest() -> ContentManifest {
    adventure_manifest(super::GamePreparation::Three)
}

fn adventure_manifest(game: super::GamePreparation) -> ContentManifest {
    use super::GamePreparation;
    let bytes: &[u8] = match game {
        GamePreparation::One => include_bytes!("../../../../content/bundles/game-one-en-v1.json"),
        GamePreparation::Two => include_bytes!("../../../../content/bundles/game-two-en-v1.json"),
        GamePreparation::Three => {
            include_bytes!("../../../../content/bundles/game-three-en-v1.json")
        }
    };
    let inspect = match game {
        GamePreparation::One => game_content::inspect_game_one_rules,
        GamePreparation::Two => game_content::inspect_game_two_rules,
        GamePreparation::Three => game_content::inspect_game_three_rules,
    };
    let import = match game {
        GamePreparation::One => game_content::import_game_one_bundle_with_runtime_rules,
        GamePreparation::Two => game_content::import_game_two_bundle_with_runtime_rules,
        GamePreparation::Three => game_content::import_game_three_bundle_with_runtime_rules,
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
    let sources = ["one", "two", "three"].into_iter().take(match game { GamePreparation::One => 1, GamePreparation::Two => 2, GamePreparation::Three => 3 }).map(|game| ProvenanceSource {
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
