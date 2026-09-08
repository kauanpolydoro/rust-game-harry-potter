use game_content::{EntryKind, import_game_two_bundle};

const BUNDLE: &[u8] = include_bytes!("../../../content/bundles/game-two-en-v1.json");

fn trusted(bundle: &serde_json::Value) -> game_content::ContentManifest {
    let sources = ["one", "two"].map(|game| game_content::ProvenanceSource {
        id: format!("game-{game}-adaptation-v1"),
        uri: format!("https://github.com/kauanpolydoro/rust-game-harry-potter/blob/main/content/game-{game}-rules-v1.md"),
        kind: game_content::SourceKind::Adaptation,
    });
    let supported = bundle["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .map(|rule| {
            game_content::RuleId::parse(rule["id"].as_str().expect("ID")).expect("valid ID")
        })
        .collect();
    game_content::import_game_two_bundle_with_runtime_rules(
        &serde_json::to_vec(bundle).expect("JSON"),
        &sources,
        &supported,
    )
    .expect("structurally valid Game 2")
}

#[test]
fn trusted_game_two_closes_every_functional_field_and_prepares_only_its_locations() {
    let bundle = serde_json::from_slice(BUNDLE).expect("JSON");
    let manifest = trusted(&bundle);
    assert!(manifest.playable, "gaps: {:?}", manifest.gaps);
    assert!(manifest.entries.iter().all(|entry| entry.playable));
    assert_eq!(manifest.manifest_version, 5);
    assert_eq!(manifest.game_setups.len(), 1);
    assert_eq!(
        manifest.game_setups[0].adventure_id.as_str(),
        "adventure:002"
    );
    let locations = manifest.game_setups[0]
        .entities
        .iter()
        .filter(|entry| entry.catalog_id.as_str().starts_with("location:"))
        .map(|entry| entry.catalog_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        locations,
        vec!["location:003", "location:004", "location:005"]
    );
}

#[test]
fn an_unproven_printing_blocks_its_entry_without_borrowing_another_versions_rule() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    let entry = bundle["entries"]
        .as_array_mut()
        .expect("entries")
        .iter_mut()
        .find(|entry| entry["id"] == "hogwarts-card:018")
        .expect("Finite");
    entry["functional"]["effect"]["confidence"] = serde_json::json!("candidate");
    let manifest = trusted(&bundle);
    assert!(!manifest.playable);
    let blocked = manifest
        .entries
        .iter()
        .filter(|entry| !entry.playable)
        .map(|entry| entry.catalog_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(blocked, vec!["hogwarts-card:018"]);
    assert_eq!(
        manifest.gaps[0].field,
        game_content::FunctionalField::Effect
    );
}

#[test]
fn ally_copy_cannot_target_items_or_turn_an_ally_into_a_recursive_copy() {
    for copy_is_ally in [false, true] {
        let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
        let rule = bundle["rules"]
            .as_array_mut()
            .expect("rules")
            .iter_mut()
            .find(|rule| rule["id"] == "rule:g2-hogwarts-card-023")
            .expect("Polyjuice");
        if copy_is_ally {
            rule["effect"]["effects"][0]["card_type"] = serde_json::json!("ally");
        } else {
            rule["effect"]["effects"][1]["target"]["eligibility"][0]["card_type"] =
                serde_json::json!("item");
        }
        assert!(
            game_content::inspect_game_two_rules(&serde_json::to_vec(&bundle).expect("JSON"))
                .is_err()
        );
    }
}

#[test]
fn cumulative_game_two_inventory_keeps_unproven_printings_blocked() {
    let mut candidate: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    candidate["game_setups"] = serde_json::json!([]);
    let manifest = import_game_two_bundle(&serde_json::to_vec(&candidate).expect("JSON"))
        .expect("valid cumulative inventory");
    assert_eq!((manifest.record_count, manifest.card_count), (63, 116));
    assert!(!manifest.playable);
    let counts = |kind| {
        manifest
            .entries
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| u32::from(e.copies))
            .sum::<u32>()
    };
    assert_eq!(counts(EntryKind::HogwartsCard), 44);
    assert_eq!(counts(EntryKind::DarkArts), 15);
    assert_eq!(counts(EntryKind::Villain), 6);
    assert_eq!(counts(EntryKind::Location), 3);
    assert!(!manifest.entries.iter().any(|e| matches!(
        e.catalog_id.as_str(),
        "location:001" | "location:002" | "adventure:001" | "hogwarts-card:012"
    )));
}

#[test]
fn the_location_deck_cannot_reverse_the_official_progression() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    let entities = bundle["game_setups"][0]["entities"]
        .as_array_mut()
        .expect("entities");
    let locations = entities
        .iter()
        .enumerate()
        .filter(|(_, entity)| entity["zone"] == "location_deck")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    entities.swap(locations[0], locations[1]);
    assert!(
        game_content::inspect_game_two_rules(&serde_json::to_vec(&bundle).expect("JSON")).is_err()
    );
}
