use game_content::{EntryKind, import_game_four_bundle};

const BUNDLE: &[u8] = include_bytes!("../../../content/bundles/game-four-en-v1.json");

#[test]
fn game_four_closes_its_cumulative_inventory_and_preserves_the_game_three_heroes() {
    let manifest = import_game_four_bundle(BUNDLE).expect("valid Game 4 bundle");
    assert_eq!((manifest.record_count, manifest.card_count), (98, 169));
    assert_eq!(manifest.manifest_version, 7);
    assert!(
        !manifest.playable,
        "source trust must be granted externally"
    );
    let count = |kind| {
        manifest
            .entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| u32::from(entry.copies))
            .sum::<u32>()
    };
    assert_eq!(count(EntryKind::HogwartsCard), 81);
    assert_eq!(count(EntryKind::DarkArts), 27);
    assert_eq!(count(EntryKind::Villain), 10);
    assert_eq!(count(EntryKind::Location), 3);
    assert_eq!(count(EntryKind::StarterCard), 40);
    assert_eq!(
        manifest
            .entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Hero)
            .map(|entry| entry.catalog_id.as_str())
            .collect::<Vec<_>>(),
        ["hero:002", "hero:005", "hero:008", "hero:011"]
    );
    assert!(
        manifest
            .entries
            .iter()
            .any(|entry| entry.catalog_id.as_str() == "hogwarts-card:061")
    );
    assert!(!manifest.entries.iter().any(|entry| matches!(
        entry.catalog_id.as_str(),
        "location:006" | "location:007" | "location:008" | "adventure:003"
    )));
}

#[test]
fn house_dice_freeze_all_six_faces_and_their_effects_in_the_versioned_bundle() {
    let rules = game_content::inspect_game_four_rules(BUNDLE).expect("valid rules");
    for (house, expected) in [
        (
            "gryffindor",
            [
                "influence",
                "influence",
                "influence",
                "health",
                "draw",
                "attack",
            ],
        ),
        (
            "hufflepuff",
            ["influence", "health", "health", "health", "draw", "attack"],
        ),
        (
            "ravenclaw",
            ["influence", "health", "draw", "draw", "draw", "attack"],
        ),
        (
            "slytherin",
            ["influence", "health", "draw", "attack", "attack", "attack"],
        ),
    ] {
        let id = format!("rule:g4-die-{house}-v1");
        let rule = rules
            .iter()
            .find(|rule| rule.id.as_str() == id)
            .expect("house die rule");
        let effect = serde_json::to_value(&rule.effect).expect("JSON");
        assert_eq!(effect["die"], format!("{house}_v1"));
        let outcomes = effect["outcomes"].as_array().expect("six faces");
        assert_eq!(outcomes.len(), 6);
        for (face, expected) in outcomes.iter().zip(expected) {
            assert_eq!(face["target"]["zone"], "heroes");
            assert_eq!(face["target"]["owner"], "any");
            if expected == "draw" {
                assert_eq!(
                    face["operation"],
                    serde_json::json!({"type":"draw","amount":1})
                );
            } else {
                assert_eq!(
                    face["operation"],
                    serde_json::json!({"type":"modify_resource","resource":expected,"amount":1})
                );
            }
        }
    }
}

#[test]
fn game_four_publication_requires_external_trust_and_closes_every_functional_gap() {
    let rules = game_content::inspect_game_four_rules(BUNDLE).expect("closed AST");
    let sources = ["one", "two", "three", "four"].map(|game| game_content::ProvenanceSource {
        id: format!("game-{game}-adaptation-v1"),
        uri: format!("https://github.com/kauanpolydoro/rust-game-harry-potter/blob/main/content/game-{game}-rules-v1.md"),
        kind: game_content::SourceKind::Adaptation,
    });
    let supported = rules.into_iter().map(|rule| rule.id).collect();
    let manifest =
        game_content::import_game_four_bundle_with_runtime_rules(BUNDLE, &sources, &supported)
            .expect("valid publication");
    assert!(manifest.playable, "gaps: {:?}", manifest.gaps);
    assert_eq!(manifest.game_setups.len(), 1);
    assert_eq!(
        manifest.game_setups[0].adventure_id.as_str(),
        "adventure:004"
    );
}

#[test]
fn extra_revelation_cannot_turn_a_hogwarts_card_into_a_recursive_dark_arts_source() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    let rule = bundle["rules"]
        .as_array_mut()
        .expect("rules")
        .iter_mut()
        .find(|rule| rule["id"] == "rule:g4-hogwarts-card-033-effect")
        .expect("Cedric");
    rule["effect"]["effects"]
        .as_array_mut()
        .expect("sequence")
        .push(serde_json::json!({"type":"reveal_extra_dark_arts"}));
    assert!(import_game_four_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err());
}

#[test]
fn earlier_schemas_reject_house_dice_even_when_the_inventory_is_unchanged() {
    let mut bundle: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../content/bundles/game-three-en-v1.json"
    ))
    .expect("JSON");
    bundle["rules"].as_array_mut().expect("rules").push(serde_json::json!({
        "id":"rule:unexpected-house-die", "order":0, "effect":{"type":"roll","die":"slytherin_v1","outcomes":[
            {"type":"no_op"},{"type":"no_op"},{"type":"no_op"},{"type":"no_op"},{"type":"no_op"},{"type":"no_op"}
        ]}
    }));
    let error = game_content::import_game_three_bundle(&serde_json::to_vec(&bundle).expect("JSON"))
        .expect_err("requires schema 6");
    assert!(error.to_string().contains("Game 4 definitions"));
}

#[test]
fn a_chaining_dark_arts_card_cannot_multiply_revelations_by_target_iteration() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    let rule = bundle["rules"]
        .as_array_mut()
        .expect("rules")
        .iter_mut()
        .find(|rule| rule["id"] == "rule:g4-dark-arts-013-effect")
        .expect("Crucio");
    let effects = rule["effect"]["effects"].as_array_mut().expect("sequence");
    *effects.last_mut().expect("extra revelation") = serde_json::json!({
        "type":"for_each_target", "target":{"zone":"heroes","owner":"any","cardinality":{"min":0,"max":4}},
        "effect":{"type":"reveal_extra_dark_arts"}
    });
    assert!(
        import_game_four_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err(),
        "a single physical card cannot restart the reveal queue once per Hero"
    );
}

#[test]
fn a_chaining_dark_arts_card_cannot_multiply_revelations_by_each_hero_choice() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    let rule = bundle["rules"]
        .as_array_mut()
        .expect("rules")
        .iter_mut()
        .find(|rule| rule["id"] == "rule:g4-dark-arts-013-effect")
        .expect("Crucio");
    let effects = rule["effect"]["effects"].as_array_mut().expect("sequence");
    *effects.last_mut().expect("extra revelation") = serde_json::json!({
        "type":"choice", "audience":"each_hero",
        "options":[{"type":"reveal_extra_dark_arts"},{"type":"no_op"}]
    });
    assert!(
        import_game_four_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err(),
        "independent choices cannot repeat an extra revelation for every Hero"
    );
}

#[test]
fn the_once_per_turn_ally_bonus_requires_its_persisted_card_state() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    let rule = bundle["rules"]
        .as_array_mut()
        .expect("rules")
        .iter_mut()
        .find(|rule| rule["id"] == "rule:g4-hogwarts-card-033-effect")
        .expect("Cedric");
    rule["effect"]["effects"]
        .as_array_mut()
        .expect("sequence")
        .push(serde_json::json!({"type":"other_ally_bonus","health":2}));
    assert!(
        import_game_four_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err(),
        "a playable card must have the state needed to execute its bonus"
    );
}
