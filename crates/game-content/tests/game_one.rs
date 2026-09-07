use game_content::{EntryKind, import_game_one_bundle};

const BUNDLE: &[u8] = include_bytes!("../../../content/bundles/game-one-en-v1.json");

fn trusted_game_one(
    bundle: &serde_json::Value,
) -> Result<game_content::ContentManifest, game_content::ImportFailure> {
    use game_content::{
        ProvenanceSource, RuleId, SourceKind, import_game_one_bundle_with_runtime_rules,
    };
    let runtime_rules = bundle["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .map(|rule| RuleId::parse(rule["id"].as_str().expect("rule ID")).expect("valid ID"))
        .collect();
    import_game_one_bundle_with_runtime_rules(
        &serde_json::to_vec(bundle).expect("JSON"),
        &[ProvenanceSource {
            id: "game-one-adaptation-v1".to_owned(),
            uri: "https://github.com/kauanpolydoro/rust-game-harry-potter/blob/main/content/game-one-rules-v1.md".to_owned(),
            kind: SourceKind::Adaptation,
        }],
        &runtime_rules,
    )
}

#[test]
fn trusted_game_one_manifest_closes_the_playable_inventory_and_preparation() {
    let bundle = serde_json::from_slice(BUNDLE).expect("JSON");
    let manifest = trusted_game_one(&bundle).expect("valid Game1");
    assert!(manifest.playable, "gaps: {:?}", manifest.gaps);
    assert!(manifest.entries.iter().all(|entry| entry.playable));
    assert!(manifest.gaps.is_empty());
    assert_eq!(manifest.game_setups.len(), 1);
    assert_eq!(
        manifest.game_setups[0].adventure_id.as_str(),
        "adventure:001"
    );
    assert_eq!((manifest.record_count, manifest.card_count), (45, 93));
    assert_eq!(manifest.manifest_version, 4);
}

#[test]
fn game_one_rules_are_inspected_without_granting_source_or_runtime_trust() {
    let rules = game_content::inspect_game_one_rules(BUNDLE).expect("validated closed AST");
    assert!(
        rules
            .iter()
            .any(|rule| rule.id.as_str() == "rule:g1-dark-phase")
    );
    assert!(
        !import_game_one_bundle(BUNDLE)
            .expect("untrusted candidate")
            .playable
    );
    let mut legacy: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    legacy["schema_version"] = serde_json::json!(2);
    assert!(
        game_content::inspect_game_one_rules(&serde_json::to_vec(&legacy).expect("JSON")).is_err()
    );
}

#[test]
fn every_game_one_rule_requires_its_own_externally_trusted_provenance() {
    let original: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    for index in 0..original["rules"].as_array().expect("rules").len() {
        let mut bundle = original.clone();
        bundle["rules"][index]
            .as_object_mut()
            .expect("rule")
            .remove("provenance");
        let manifest = trusted_game_one(&bundle).expect("unproven but structurally valid rule");
        assert!(
            !manifest.playable,
            "rule {} must not inherit trust from its name or phase",
            original["rules"][index]["id"]
        );
    }
}

#[test]
fn game_one_preparation_requires_one_dark_arts_revelation_root() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    bundle["rules"]
        .as_array_mut()
        .expect("rules")
        .retain(|rule| rule["id"] != "rule:g1-dark-phase");
    assert!(
        trusted_game_one(&bundle).is_err(),
        "a playable game must reveal Dark Arts"
    );
}

#[test]
fn revealing_dark_arts_cannot_recurse_from_the_revealed_card_or_a_nested_root() {
    for (id, effect) in [
        (
            "rule:g1-dark-001",
            serde_json::json!({"type":"reference","rule":"rule:g1-dark-phase"}),
        ),
        (
            "rule:g1-dark-phase",
            serde_json::json!({"type":"repeat","times":2,"effect":{"type":"reveal_dark_arts"}}),
        ),
    ] {
        let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
        let rule = bundle["rules"]
            .as_array_mut()
            .expect("rules")
            .iter_mut()
            .find(|rule| rule["id"] == id)
            .expect("rule");
        rule["effect"] = effect;
        assert!(
            trusted_game_one(&bundle).is_err(),
            "recursive or nested revelation must fail closed"
        );
    }
}

#[test]
fn card_categories_cannot_be_ambiguous_or_attached_to_villain_rules() {
    for id in ["rule:g1-hogwarts-001", "rule:g1-villain-003"] {
        let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
        let rule = bundle["rules"]
            .as_array_mut()
            .expect("rules")
            .iter_mut()
            .find(|rule| rule["id"] == id)
            .expect("rule");
        rule["effect"] = serde_json::json!({"type":"sequence","effects":[
            {"type":"card_type","card_type":"spell"}, rule["effect"].clone()
        ]});
        assert!(
            trusted_game_one(&bundle).is_err(),
            "one category belongs only to a playable hero card"
        );
    }
}

#[test]
fn drawing_limits_account_for_discard_reshuffles_and_every_hero() {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    let rule = bundle["rules"]
        .as_array_mut()
        .expect("rules")
        .iter_mut()
        .find(|rule| rule["id"] == "rule:g1-hogwarts-006")
        .expect("Lumos");
    rule["effect"] = serde_json::json!({"type":"sequence","effects":[
        {"type":"card_type","card_type":"spell"},
        {"type":"repeat","times":16,"effect":{
            "type":"apply","target":{"zone":"heroes","owner":"any","cardinality":{"min":1,"max":4}},
            "operation":{"type":"draw","amount":16}
        }}
    ]});
    assert!(
        trusted_game_one(&bundle).is_err(),
        "the publication budget includes draws, moves and random samples"
    );
}

#[test]
fn game_one_preparation_rejects_missing_cards_wrong_owners_and_reversed_locations() {
    for (id, field, value) in [
        ("hogwarts-card:002", "copies", serde_json::json!(1)),
        ("starter:001", "copies", serde_json::json!(6)),
        ("starter:002", "owner", serde_json::json!("hermione")),
        ("location:001", "zone", serde_json::json!("location_deck")),
        ("location:002", "zone", serde_json::json!("active_location")),
    ] {
        let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
        let entity = bundle["game_setups"][0]["entities"]
            .as_array_mut()
            .expect("setup")
            .iter_mut()
            .find(|entity| entity["catalog_id"] == id)
            .expect("entity");
        entity[field] = value;
        assert!(
            trusted_game_one(&bundle).is_err(),
            "invalid {field} for {id}"
        );
    }
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("JSON");
    bundle["game_setups"][0]["entities"]
        .as_array_mut()
        .expect("setup")
        .remove(0);
    assert!(
        trusted_game_one(&bundle).is_err(),
        "every playable physical card must have a destination"
    );
}

#[test]
fn reactive_definitions_are_closed_and_cannot_hide_inside_choices_or_references() {
    use serde_json::json;
    let reaction = json!({"type": "reaction", "trigger": "control_added", "effect": {
        "type": "apply", "target": {"zone": "heroes", "owner": "actor", "cardinality": {"min": 1, "max": 1}},
        "operation": {"type": "modify_resource", "resource": "health", "amount": -2}
    }});
    let mut bundle = unproven_bundle();
    bundle["rules"] =
        json!([{"id": "rule:reactive", "trigger": "villains", "order": 0, "effect": reaction}]);
    assert!(import_game_one_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_ok());
    for invalid in [
        json!({"type": "choice", "options": [reaction, {"type": "noop"}]}),
        json!({"type": "reaction", "trigger": "control_added", "effect": reaction}),
        json!({"type": "reaction", "trigger": "control_added", "effect": {"type": "reveal_dark_arts"}}),
    ] {
        bundle["rules"][0]["effect"] = invalid;
        assert!(import_game_one_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err());
    }
    bundle["rules"] = json!([
        {"id": "rule:reactive", "trigger": "villains", "order": 0, "effect": reaction},
        {"id": "rule:indirect", "trigger": "manual", "order": 0, "effect": {"type": "reference", "rule": "rule:reactive"}}
    ]);
    assert!(import_game_one_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err());
}

fn unproven_bundle() -> serde_json::Value {
    let mut bundle: serde_json::Value = serde_json::from_slice(BUNDLE).expect("bundle JSON");
    for entry in bundle["entries"].as_array_mut().expect("entries") {
        entry.as_object_mut().expect("entry").remove("functional");
    }
    bundle["rules"] = serde_json::json!([]);
    bundle["game_setups"] = serde_json::json!([]);
    bundle
}

#[test]
fn structural_definitions_cannot_replace_a_cards_effect() {
    use serde_json::json;
    let mut bundle = unproven_bundle();
    bundle["rules"] = json!([{
        "id": "rule:no-ability", "order": 0,
        "effect": { "type": "structural", "rule": "no_hero_ability" }
    }]);
    let card = bundle["entries"]
        .as_array_mut()
        .expect("entries")
        .iter_mut()
        .find(|entry| entry["id"] == "starter:001")
        .expect("Alohomora");
    card["functional"] = json!({ "effect": {
        "confidence": "adaptation", "sources": ["project-spec"], "rule": "rule:no-ability"
    }});
    assert!(import_game_one_bundle(&serde_json::to_vec(&bundle).expect("JSON")).is_err());
}

#[test]
fn game_one_can_prove_the_absence_of_a_hero_ability_without_a_fake_effect() {
    use game_content::{
        ProvenanceSource, RuleId, SourceKind, import_game_one_bundle_with_runtime_rules,
    };
    use serde_json::json;
    use std::collections::BTreeSet;

    let mut bundle = unproven_bundle();
    bundle["rules"] = json!([{
        "id": "rule:no-ability", "trigger": "manual", "order": 0,
        "provenance": { "confidence": "adaptation", "sources": ["project-spec"] },
        "effect": { "type": "structural", "rule": "no_hero_ability" }
    }]);
    let hero = bundle["entries"]
        .as_array_mut()
        .expect("entries")
        .iter_mut()
        .find(|entry| entry["id"] == "hero:001")
        .expect("Harry");
    hero["functional"] = json!({ "ability": {
        "confidence": "adaptation", "sources": ["project-spec"], "rule": "rule:no-ability"
    }});
    let manifest = import_game_one_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("serialize bundle"),
        &[ProvenanceSource {
            id: "project-spec".to_owned(),
            uri: "https://github.com/kauanpolydoro/rust-game-harry-potter/issues/1".to_owned(),
            kind: SourceKind::Adaptation,
        }],
        &BTreeSet::from([RuleId::parse("rule:no-ability").expect("rule ID")]),
    )
    .expect("an explicit structural definition should import");
    assert!(
        manifest
            .entries
            .iter()
            .find(|entry| entry.catalog_id.as_str() == "hero:001")
            .expect("hero")
            .playable
    );
    assert!(!manifest.playable, "other definitions remain unproven");
}

#[test]
fn game_one_inventory_closes_without_promoting_candidate_semantics() {
    let manifest = import_game_one_bundle(BUNDLE).expect("Game 1 inventory should import");
    assert_eq!(manifest.record_count, 45);
    assert_eq!(manifest.card_count, 93);
    for (kind, expected) in [
        (EntryKind::HogwartsCard, 30),
        (EntryKind::StarterCard, 40),
        (EntryKind::DarkArts, 10),
        (EntryKind::Villain, 3),
        (EntryKind::Location, 2),
        (EntryKind::Hero, 4),
        (EntryKind::TurnOrder, 4),
    ] {
        assert_eq!(
            manifest
                .entries
                .iter()
                .filter(|entry| entry.kind == kind)
                .map(|entry| u32::from(entry.copies))
                .sum::<u32>(),
            expected,
            "{kind:?}"
        );
    }
    assert!(
        !manifest.playable,
        "untrusted rules must remain unavailable"
    );
    assert!(!manifest.gaps.is_empty());
}
