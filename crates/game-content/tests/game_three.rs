use game_content::GameSetupOwner::{Harry, Hermione, Neville, Ron};
use game_content::{EntryKind, import_game_three_bundle};

const BUNDLE: &[u8] = include_bytes!("../../../content/bundles/game-three-en-v1.json");

fn trusted(bundle: &serde_json::Value) -> game_content::ContentManifest {
    let sources = ["one", "two", "three"].map(|game| game_content::ProvenanceSource {
        id: format!("game-{game}-adaptation-v1"),
        uri: format!("https://github.com/kauanpolydoro/rust-game-harry-potter/blob/main/content/game-{game}-rules-v1.md"),
        kind: game_content::SourceKind::Adaptation,
    });
    let bytes = serde_json::to_vec(bundle).expect("JSON");
    let supported = game_content::inspect_game_three_rules(&bytes)
        .expect("closed AST")
        .into_iter()
        .map(|rule| rule.id)
        .collect();
    game_content::import_game_three_bundle_with_runtime_rules(&bytes, &sources, &supported)
        .expect("valid cumulative bundle")
}

#[test]
fn every_game_three_rule_requires_provenance_and_every_hero_has_exactly_one_setup_identity() {
    let bundle = serde_json::from_slice(BUNDLE).expect("JSON");
    let manifest = trusted(&bundle);
    assert!(manifest.playable, "gaps: {:?}", manifest.gaps);
    let setup = &manifest.game_setups[0];
    assert_eq!(setup.adventure_id.as_str(), "adventure:003");
    let heroes = setup
        .entities
        .iter()
        .filter(|entry| entry.zone == game_content::Zone::Heroes)
        .map(|entry| (entry.catalog_id.as_str(), entry.copies, entry.owner))
        .collect::<Vec<_>>();
    assert_eq!(
        heroes,
        [
            ("hero:002", 1, Harry),
            ("hero:005", 1, Hermione),
            ("hero:008", 1, Neville),
            ("hero:011", 1, Ron)
        ]
    );
}

#[test]
fn game_three_replaces_hero_versions_and_closes_the_official_cumulative_inventory() {
    let manifest = import_game_three_bundle(BUNDLE).expect("valid Game 3 inventory");
    assert_eq!((manifest.record_count, manifest.card_count), (77, 138));
    assert_eq!(manifest.manifest_version, 6);
    assert!(
        !manifest.playable,
        "a bundle cannot grant itself source trust"
    );
    let count = |kind| {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| u32::from(entry.copies))
            .sum::<u32>()
    };
    assert_eq!(count(EntryKind::HogwartsCard), 60);
    assert_eq!(count(EntryKind::DarkArts), 19);
    assert_eq!(count(EntryKind::Villain), 8);
    assert_eq!(count(EntryKind::Location), 3);
    assert_eq!(count(EntryKind::StarterCard), 40);
    let heroes = manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == EntryKind::Hero)
        .map(|entry| entry.catalog_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(heroes, ["hero:002", "hero:005", "hero:008", "hero:011"]);
    let sirius = manifest
        .entries
        .iter()
        .find(|entry| entry.catalog_id.as_str() == "hogwarts-card:061")
        .expect("Sirius Black is present in the Game 3 manual");
    assert_eq!(sirius.names["en"], "Sirius Black");
    assert_eq!(sirius.introduced_in, 3);
    assert!(!manifest.entries.iter().any(|entry| matches!(
        entry.catalog_id.as_str(),
        "hogwarts-card:012"
            | "location:001"
            | "location:002"
            | "location:003"
            | "location:004"
            | "location:005"
    )));
}

#[test]
fn replacement_heroes_cannot_silently_lose_their_abilities() {
    for id in ["002", "005", "008", "011"] {
        let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
        let rule_id = format!("rule:g3-hero-{id}-ability");
        let rule = bundle["rules"]
            .as_array_mut()
            .expect("rules")
            .iter_mut()
            .find(|rule| rule["id"] == rule_id)
            .expect("ability");
        rule["effect"] = serde_json::json!({"type":"no_op"});
        assert!(
            import_game_three_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err(),
            "{rule_id}"
        );
    }
}

#[test]
fn game_three_mechanics_cannot_be_smuggled_into_an_older_schema() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    bundle["schema_version"] = serde_json::json!(4);
    assert!(import_game_three_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err());
}

#[test]
fn replaced_heroes_cannot_take_another_heros_strategy_or_setup_seat() {
    for mutation in ["strategy", "seat", "duplicate"] {
        let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
        if mutation == "strategy" {
            let rule = bundle["rules"]
                .as_array_mut()
                .expect("rules")
                .iter_mut()
                .find(|rule| rule["id"] == "rule:g3-hero-002-ability")
                .expect("ability");
            rule["effect"]["strategy"] = serde_json::json!("ron_game_three_v1");
        } else {
            let hero = bundle["game_setups"][0]["entities"]
                .as_array_mut()
                .expect("setup")
                .iter_mut()
                .find(|entry| entry["catalog_id"] == "hero:002")
                .expect("Harry setup");
            if mutation == "seat" {
                hero["owner"] = serde_json::json!("hermione");
            } else {
                hero["copies"] = serde_json::json!(2);
            }
        }
        assert!(
            import_game_three_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn publication_bounds_all_neville_activations_and_the_triggering_effect_together() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    let rule = bundle["rules"]
        .as_array_mut()
        .expect("rules")
        .iter_mut()
        .find(|rule| rule["id"] == "rule:g3-hero-008-ability")
        .expect("Neville");
    rule["effect"]["effect"] = serde_json::json!({"type":"repeat","times":16,"effect":{
        "type":"repeat","times":16,"effect":{"type":"apply","target":{"zone":"heroes","owner":"any","cardinality":{"min":0,"max":4}},
        "operation":{"type":"modify_resource","resource":"health","amount":1}}
    }});
    let bytes = serde_json::to_vec(&bundle).expect("JSON");
    let supported = game_content::inspect_game_three_rules(&bytes)
        .expect("structurally valid rules")
        .into_iter()
        .map(|rule| rule.id)
        .collect();
    let sources = trusted(&serde_json::from_slice(BUNDLE).expect("JSON")).sources;
    assert!(
        game_content::import_game_three_bundle_with_runtime_rules(&bytes, &sources, &supported)
            .is_err(),
        "four activations plus their trigger must fit one continuation"
    );
}
