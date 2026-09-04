use std::collections::BTreeSet;

use game_content::{
    CardInstanceId, CatalogId, Effect, EffectChoiceAudience, EffectRule, EffectTrigger,
    FunctionalField, GameSetupOwner, ProvenanceSource, RuleId, SourceKind, Zone,
    import_base_bundle, import_base_bundle_with_runtime_rules,
    import_base_bundle_with_trusted_sources,
};
use serde_json::json;

fn complete_bundle() -> Vec<u8> {
    let entries = (0..171)
        .map(|index| {
            json!({
                "id": format!("fixture:entry-{index:03}"),
                "kind": "turn_order",
                "set": "base",
                "copies": if index < 81 { 2 } else { 1 },
                "introduced_in": 1,
                "names": { "en": format!("Fixture {index}") },
                "provenance": {
                    "id": ["fixture-source"],
                    "kind": ["fixture-source"],
                    "set": ["fixture-source"],
                    "copies": ["fixture-source"],
                    "introduced_in": ["fixture-source"],
                    "names.en": ["fixture-source"]
                },
                "required_functional_fields": []
            })
        })
        .collect::<Vec<_>>();

    serde_json::to_vec(&json!({
        "schema_version": 2,
        "content_version": "fixture-v1",
        "ruleset_version": "fixture-rules-v1",
        "locale": "en",
        "sources": [{
            "id": "fixture-source",
            "uri": "https://example.invalid/fixture",
            "kind": "candidate"
        }],
        "rules": [],
        "entries": entries
    }))
    .expect("fixture should serialize")
}

fn bundle_value() -> serde_json::Value {
    serde_json::from_slice(&complete_bundle()).expect("fixture should parse")
}

fn playable_bundle_without_setup() -> serde_json::Value {
    let mut bundle = bundle_value();
    bundle["sources"][0]["kind"] = json!("validated");
    bundle["rules"] = json!([{
        "id": "rule:playable",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "zone": "heroes",
                "owner": "actor",
                "cardinality": { "min": 1, "max": 1 }
            },
            "operation": {
                "type": "modify_resource",
                "resource": "attack",
                "amount": 1
            }
        }
    }]);

    let catalog_shape: [(&str, &[&str]); 12] = [
        ("adventure", &["precedence", "setup"]),
        ("catalog", &[]),
        ("dark_arts", &["effect"]),
        ("hero", &["ability"]),
        ("hogwarts_card", &["cost", "effect"]),
        ("horcrux", &["effect", "precedence", "reward"]),
        ("location", &["control_limit", "dark_arts_count", "effect"]),
        ("proficiency", &["ability"]),
        ("ruleset", &["precedence"]),
        ("starter_card", &["effect"]),
        ("turn_order", &[]),
        ("villain", &["effect", "health", "reward"]),
    ];

    for (index, (kind, fields)) in catalog_shape.into_iter().enumerate() {
        bundle["entries"][index]["kind"] = json!(kind);
        bundle["entries"][index]["required_functional_fields"] = json!(fields);
        let functional = fields
            .iter()
            .map(|field| {
                (
                    (*field).to_owned(),
                    json!({
                        "confidence": "validated",
                        "sources": ["fixture-source"],
                        "rule": "rule:playable"
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        bundle["entries"][index]["functional"] = functional.into();
    }

    bundle
}

fn import_value(bundle: &serde_json::Value) -> Result<game_content::ContentManifest, String> {
    import_base_bundle(&serde_json::to_vec(bundle).expect("fixture should serialize"))
        .map_err(|failure| failure.to_string())
}

fn import_value_with_trusted_sources(
    bundle: &serde_json::Value,
    trusted_sources: &[ProvenanceSource],
) -> Result<game_content::ContentManifest, String> {
    import_base_bundle_with_trusted_sources(
        &serde_json::to_vec(bundle).expect("fixture should serialize"),
        trusted_sources,
    )
    .map_err(|failure| failure.to_string())
}

fn require_effect(bundle: &mut serde_json::Value) {
    bundle["entries"][0]["kind"] = json!("dark_arts");
    bundle["entries"][0]["required_functional_fields"] = json!(["effect"]);
}

fn contains_participant_choice(effect: &Effect) -> bool {
    match effect {
        Effect::Choice { .. } => true,
        Effect::Condition {
            then, otherwise, ..
        } => {
            contains_participant_choice(then)
                || otherwise
                    .as_deref()
                    .is_some_and(contains_participant_choice)
        }
        Effect::Repeat { effect, .. } => contains_participant_choice(effect),
        Effect::Roll { outcomes, .. } => outcomes.iter().any(contains_participant_choice),
        Effect::Sequence { effects } => effects.iter().any(contains_participant_choice),
        Effect::Apply { .. }
        | Effect::NoOp
        | Effect::Reference { .. }
        | Effect::Terminal { .. } => false,
    }
}

fn actor_attack_effect() -> serde_json::Value {
    json!({
        "type": "apply",
        "target": {
            "zone": "heroes",
            "owner": "actor",
            "cardinality": { "min": 1, "max": 1 }
        },
        "operation": {
            "type": "modify_resource",
            "resource": "attack",
            "amount": 1
        }
    })
}

#[test]
fn effect_rule_schema_supports_turn_phase_triggers_and_explicit_order() {
    let cases = [
        ("dark_arts", EffectTrigger::DarkArts, "dark_arts"),
        ("dark_arts_completed", EffectTrigger::DarkArts, "dark_arts"),
        ("villains", EffectTrigger::Villains, "villains"),
        (
            "villain_reward",
            EffectTrigger::VillainReward,
            "villain_reward",
        ),
        ("manual", EffectTrigger::Manual, "manual"),
    ];

    for (json_trigger, expected_trigger, canonical_trigger) in cases {
        let rule = serde_json::from_value::<EffectRule>(json!({
            "id": "rule:phase-trigger",
            "trigger": json_trigger,
            "order": 7,
            "effect": { "type": "no_op" }
        }))
        .expect("the phase trigger and order should deserialize");

        assert_eq!(rule.trigger, expected_trigger);
        assert_eq!(rule.order, 7);
        assert_eq!(
            serde_json::to_value(rule).expect("the rule should serialize")["trigger"],
            canonical_trigger
        );
    }
}

#[test]
fn effect_rule_order_is_required_by_the_json_schema() {
    let failure = serde_json::from_value::<EffectRule>(json!({
        "id": "rule:missing-order",
        "trigger": "manual",
        "effect": { "type": "no_op" }
    }))
    .expect_err("semantic rule order must be explicit");

    assert!(failure.to_string().contains("missing field `order`"));
}

#[test]
fn legacy_bundle_version_is_rejected_before_current_rule_fields_are_decoded() {
    let mut bundle = bundle_value();
    bundle["schema_version"] = json!(1);
    bundle["rules"] = json!([{
        "id": "rule:legacy",
        "trigger": "dark_arts_completed",
        "effect": { "type": "no_op" }
    }]);

    let failure = import_value(&bundle).expect_err("schema v1 must require an explicit upgrade");

    assert_eq!(failure, "unsupported bundle schema version: 1");
}

#[test]
fn manifest_rules_follow_phase_order_independently_from_json_and_rule_id_order() {
    let mut bundle = bundle_value();
    let effect = actor_attack_effect();
    bundle["rules"] = json!([
        {
            "id": "rule:z-villains-first",
            "trigger": "villains",
            "order": 1,
            "effect": effect
        },
        {
            "id": "rule:a-dark-arts-second",
            "trigger": "dark_arts",
            "order": 2,
            "effect": effect
        },
        {
            "id": "rule:a-manual",
            "trigger": "manual",
            "order": 0,
            "effect": effect
        },
        {
            "id": "rule:z-dark-arts-first",
            "trigger": "dark_arts",
            "order": 1,
            "effect": effect
        }
    ]);
    let executable_rules = bundle["rules"]
        .as_array()
        .expect("rules should be an array")
        .iter()
        .map(|rule| {
            RuleId::parse(rule["id"].as_str().expect("rule ID should be a string"))
                .expect("fixture rule ID should be valid")
        })
        .collect::<BTreeSet<_>>();

    let manifest = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &executable_rules,
    )
    .expect("rules with distinct automatic order should import");

    assert_eq!(
        manifest
            .rules
            .iter()
            .map(|rule| (rule.trigger, rule.order, rule.id.as_str()))
            .collect::<Vec<_>>(),
        [
            (EffectTrigger::DarkArts, 1, "rule:z-dark-arts-first"),
            (EffectTrigger::DarkArts, 2, "rule:a-dark-arts-second"),
            (EffectTrigger::Villains, 1, "rule:z-villains-first"),
            (EffectTrigger::Manual, 0, "rule:a-manual"),
        ]
    );
}

#[test]
fn duplicate_order_for_executable_automatic_roots_prevents_manifest_publication() {
    for trigger in ["dark_arts", "villains"] {
        let mut bundle = bundle_value();
        let effect = actor_attack_effect();
        bundle["rules"] = json!([
            {
                "id": "rule:first",
                "trigger": trigger,
                "order": 4,
                "effect": effect
            },
            {
                "id": "rule:second",
                "trigger": trigger,
                "order": 4,
                "effect": effect
            }
        ]);
        let executable_rules = BTreeSet::from([
            RuleId::parse("rule:first").expect("fixture rule ID should be valid"),
            RuleId::parse("rule:second").expect("fixture rule ID should be valid"),
        ]);

        let failure = import_base_bundle_with_runtime_rules(
            &serde_json::to_vec(&bundle).expect("fixture should serialize"),
            &[],
            &executable_rules,
        )
        .expect_err("ambiguous automatic root order must fail closed");

        assert!(failure.to_string().contains("share automatic trigger"));
        assert!(failure.to_string().contains("order 4"));
    }
}

#[test]
fn executable_automatic_roots_cannot_charge_the_active_hero() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:mandatory",
        "trigger": "dark_arts",
        "order": 1,
        "cost": [{ "resource": "influence", "amount": 1 }],
        "effect": actor_attack_effect()
    }]);
    let executable_rules =
        BTreeSet::from([RuleId::parse("rule:mandatory").expect("fixture rule ID should be valid")]);

    let failure = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &executable_rules,
    )
    .expect_err("mandatory phase work cannot depend on a player payment");

    assert!(failure.to_string().contains("automatic root"));
    assert!(failure.to_string().contains("cannot declare a cost"));
}

#[test]
fn duplicate_order_is_allowed_outside_executable_automatic_roots() {
    let effect = actor_attack_effect();

    let mut manual_bundle = bundle_value();
    manual_bundle["rules"] = json!([
        {
            "id": "rule:manual-first",
            "trigger": "manual",
            "order": 4,
            "effect": effect
        },
        {
            "id": "rule:manual-second",
            "trigger": "manual",
            "order": 4,
            "effect": effect
        }
    ]);
    let manual_roots = BTreeSet::from([
        RuleId::parse("rule:manual-first").expect("fixture rule ID should be valid"),
        RuleId::parse("rule:manual-second").expect("fixture rule ID should be valid"),
    ]);

    import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&manual_bundle).expect("fixture should serialize"),
        &[],
        &manual_roots,
    )
    .expect("manual roots do not execute as an automatic phase");

    let mut partially_executable_bundle = manual_bundle;
    for rule in partially_executable_bundle["rules"]
        .as_array_mut()
        .expect("rules should be an array")
    {
        rule["trigger"] = json!("dark_arts");
    }
    let single_root = BTreeSet::from([
        RuleId::parse("rule:manual-first").expect("fixture rule ID should be valid")
    ]);

    import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&partially_executable_bundle).expect("fixture should serialize"),
        &[],
        &single_root,
    )
    .expect("a non-executable rule cannot make executable root order ambiguous");
}

fn bundle_with_setup_entity(
    kind: &str,
    required_functional_fields: &[&str],
    zone: &str,
    owner: &str,
) -> serde_json::Value {
    let mut bundle = bundle_value();
    bundle["entries"][0]["kind"] = json!("adventure");
    bundle["entries"][0]["required_functional_fields"] = json!(["precedence", "setup"]);
    bundle["entries"][1]["kind"] = json!(kind);
    bundle["entries"][1]["required_functional_fields"] = json!(required_functional_fields);
    bundle["entries"][1]["functional"] = match kind {
        "hogwarts_card" => json!({
            "cost": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "value": 1
            },
            "effect": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "rule": "rule:setup-entity"
            }
        }),
        "starter_card" => json!({
            "effect": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "rule": "rule:setup-entity"
            }
        }),
        "villain" => json!({
            "effect": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "rule": "rule:setup-entity"
            },
            "health": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "value": 5
            },
            "reward": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "rule": "rule:setup-reward"
            }
        }),
        "location" => json!({
            "control_limit": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "value": 4
            },
            "dark_arts_count": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "value": 2
            },
            "effect": {
                "confidence": "candidate",
                "sources": ["fixture-source"],
                "rule": "rule:setup-entity"
            }
        }),
        _ => json!({}),
    };
    bundle["rules"] = json!([
        {
            "id": "rule:setup-entity",
            "order": 0,
            "effect": {
                "type": "apply",
                "target": {
                    "zone": "heroes",
                    "owner": "actor",
                    "cardinality": { "min": 1, "max": 1 }
                },
                "operation": {
                    "type": "modify_resource",
                    "resource": "attack",
                    "amount": 1
                }
            }
        },
        {
            "id": "rule:setup-reward",
            "trigger": "villain_reward",
            "order": 0,
            "effect": { "type": "no_op" }
        }
    ]);
    bundle["game_setups"] = json!([{
        "adventure_id": "fixture:entry-000",
        "confidence": "candidate",
        "sources": ["fixture-source"],
        "entities": [{
            "catalog_id": "fixture:entry-001",
            "copies": 1,
            "zone": zone,
            "owner": owner
        }]
    }]);
    bundle
}

fn expand_setup_entity_to_four_copies(bundle: &mut serde_json::Value) {
    bundle["entries"][1]["copies"] = json!(4);
    bundle["entries"][2]["copies"] = json!(1);
    bundle["entries"][3]["copies"] = json!(1);
}

fn bundle_with_ordered_setup_entities() -> serde_json::Value {
    let mut bundle = bundle_with_setup_entity(
        "hogwarts_card",
        &["cost", "effect"],
        "hogwarts_deck",
        "none",
    );
    bundle["entries"][2]["kind"] = json!("villain");
    bundle["entries"][2]["required_functional_fields"] = json!(["effect", "health", "reward"]);
    bundle["entries"][2]["functional"] = json!({
        "effect": {
            "confidence": "candidate",
            "sources": ["fixture-source"],
            "rule": "rule:setup-entity"
        },
        "health": {
            "confidence": "candidate",
            "sources": ["fixture-source"],
            "value": 5
        }
    });
    bundle["game_setups"][0]["entities"] = json!([
        {
            "catalog_id": "fixture:entry-002",
            "copies": 1,
            "zone": "villain_deck",
            "owner": "none"
        },
        {
            "catalog_id": "fixture:entry-001",
            "copies": 2,
            "zone": "hogwarts_deck",
            "owner": "none"
        }
    ]);
    bundle
}

#[test]
fn trusted_game_setup_is_published_in_declared_entity_order() {
    let mut bundle = bundle_with_ordered_setup_entities();
    bundle["sources"]
        .as_array_mut()
        .expect("sources should be an array")
        .push(json!({
            "id": "validated-source",
            "uri": "https://example.invalid/validated",
            "kind": "validated"
        }));
    bundle["game_setups"][0]["confidence"] = json!("validated");
    bundle["game_setups"][0]["sources"] = json!(["validated-source"]);
    let trusted_sources = [ProvenanceSource {
        id: "validated-source".to_owned(),
        uri: "https://example.invalid/validated".to_owned(),
        kind: SourceKind::Validated,
    }];

    let manifest = import_value_with_trusted_sources(&bundle, &trusted_sources)
        .expect("a trusted setup with valid entries should import");

    assert_eq!(manifest.game_setups.len(), 1);
    assert_eq!(
        manifest.game_setups[0].adventure_id.as_str(),
        "fixture:entry-000"
    );
    assert_eq!(
        manifest.game_setups[0]
            .entities
            .iter()
            .map(|entity| (
                entity.catalog_id.as_str(),
                entity.copies,
                entity.zone,
                entity.owner,
            ))
            .collect::<Vec<_>>(),
        [
            (
                "fixture:entry-002",
                1,
                Zone::VillainDeck,
                GameSetupOwner::None,
            ),
            (
                "fixture:entry-001",
                2,
                Zone::HogwartsDeck,
                GameSetupOwner::None,
            ),
        ]
    );
}

#[test]
fn self_declared_trust_does_not_publish_a_game_setup() {
    let mut bundle = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    bundle["sources"][0]["kind"] = json!("validated");
    bundle["game_setups"][0]["confidence"] = json!("validated");

    let manifest = import_value(&bundle)
        .expect("a structurally valid but untrusted setup should remain publishable");

    assert!(manifest.game_setups.is_empty());
}

#[test]
fn game_setup_trust_requires_an_exact_external_source_match() {
    let mut bundle = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    bundle["sources"][0]["kind"] = json!("validated");
    bundle["game_setups"][0]["confidence"] = json!("validated");
    let mismatched_sources = [
        ProvenanceSource {
            id: "fixture-source".to_owned(),
            uri: "https://example.invalid/different".to_owned(),
            kind: SourceKind::Validated,
        },
        ProvenanceSource {
            id: "fixture-source".to_owned(),
            uri: "https://example.invalid/fixture".to_owned(),
            kind: SourceKind::Official,
        },
    ];

    for trusted_source in mismatched_sources {
        let manifest = import_value_with_trusted_sources(&bundle, &[trusted_source])
            .expect("mismatched trust should not prevent structural publication");

        assert!(manifest.game_setups.is_empty());
    }
}

#[test]
fn trusted_cost_and_health_values_are_published_as_proven_functional_data() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["kind"] = json!("hogwarts_card");
    bundle["entries"][0]["required_functional_fields"] = json!(["cost", "effect"]);
    bundle["entries"][0]["functional"] = json!({
        "cost": {
            "confidence": "validated",
            "sources": ["validated-source"],
            "value": 0
        }
    });
    bundle["entries"][1]["kind"] = json!("villain");
    bundle["entries"][1]["required_functional_fields"] = json!(["effect", "health", "reward"]);
    bundle["entries"][1]["functional"] = json!({
        "health": {
            "confidence": "validated",
            "sources": ["validated-source"],
            "value": 5
        }
    });
    bundle["sources"]
        .as_array_mut()
        .expect("sources should be an array")
        .push(json!({
            "id": "validated-source",
            "uri": "https://example.invalid/validated",
            "kind": "validated"
        }));
    let trusted_sources = [ProvenanceSource {
        id: "validated-source".to_owned(),
        uri: "https://example.invalid/validated".to_owned(),
        kind: SourceKind::Validated,
    }];

    let manifest = import_value_with_trusted_sources(&bundle, &trusted_sources)
        .expect("trusted numeric functional data should import");

    assert_eq!(
        manifest.entries[0].functional_provenance[&FunctionalField::Cost].value,
        Some(0)
    );
    assert!(!manifest.entries[0].gaps.contains(&FunctionalField::Cost));
    assert_eq!(
        manifest.entries[1].functional_provenance[&FunctionalField::Health].value,
        Some(5)
    );
    assert!(!manifest.entries[1].gaps.contains(&FunctionalField::Health));
}

#[test]
fn numeric_functional_value_without_external_trust_remains_a_gap() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["kind"] = json!("hogwarts_card");
    bundle["entries"][0]["required_functional_fields"] = json!(["cost", "effect"]);
    bundle["entries"][0]["functional"] = json!({
        "cost": {
            "confidence": "validated",
            "sources": ["fixture-source"],
            "value": 2
        }
    });
    bundle["sources"][0]["kind"] = json!("validated");

    let manifest = import_value(&bundle)
        .expect("untrusted numeric semantics should remain publishable as a gap");

    assert_eq!(
        manifest.entries[0].functional_provenance[&FunctionalField::Cost].value,
        Some(2)
    );
    assert!(manifest.entries[0].gaps.contains(&FunctionalField::Cost));
}

#[test]
fn value_on_a_non_numeric_functional_field_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "candidate",
            "sources": ["fixture-source"],
            "value": 1
        }
    });

    let failure = import_value(&bundle)
        .expect_err("only Cost and Health may carry an explicit numeric value");

    assert!(failure.contains("value is incompatible with functional field Effect"));
}

#[test]
fn legacy_numeric_rule_without_a_game_setup_remains_publishable() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["kind"] = json!("hogwarts_card");
    bundle["entries"][0]["required_functional_fields"] = json!(["cost", "effect"]);
    bundle["entries"][0]["functional"] = json!({
        "cost": {
            "confidence": "candidate",
            "sources": ["fixture-source"],
            "rule": "rule:cost"
        }
    });
    bundle["rules"] = json!([{
        "id": "rule:cost",
        "order": 0,
        "effect": { "type": "no_op" }
    }]);

    let manifest = import_value(&bundle)
        .expect("the pre-existing numeric rule form must remain publishable without a setup");

    let cost = &manifest.entries[0].functional_provenance[&FunctionalField::Cost];
    assert_eq!(cost.rule_id.as_ref().map(RuleId::as_str), Some("rule:cost"));
    assert_eq!(cost.value, None);
    assert!(manifest.entries[0].gaps.contains(&FunctionalField::Cost));
}

#[test]
fn zero_health_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["kind"] = json!("villain");
    bundle["entries"][0]["required_functional_fields"] = json!(["effect", "health", "reward"]);
    bundle["entries"][0]["functional"] = json!({
        "health": {
            "confidence": "candidate",
            "sources": ["fixture-source"],
            "value": 0
        }
    });

    let failure = import_value(&bundle).expect_err("Health must be positive when declared");

    assert!(failure.contains("Health value must be greater than zero"));
}

#[test]
fn game_setup_catalog_references_must_resolve() {
    let mut missing_adventure = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    missing_adventure["game_setups"][0]["adventure_id"] = json!("fixture:missing");
    let mut missing_entity = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    missing_entity["game_setups"][0]["entities"][0]["catalog_id"] = json!("fixture:missing");

    for (bundle, expected_message) in [
        (
            missing_adventure,
            "unknown adventure reference fixture:missing",
        ),
        (missing_entity, "unknown entity reference fixture:missing"),
    ] {
        let failure =
            import_value(&bundle).expect_err("setup references must resolve in the bundle");

        assert!(failure.contains(expected_message), "failure was: {failure}");
    }
}

#[test]
fn game_setup_adventure_reference_must_identify_an_adventure() {
    let mut bundle = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    bundle["entries"][0]["kind"] = json!("turn_order");
    bundle["entries"][0]["required_functional_fields"] = json!([]);

    let failure =
        import_value(&bundle).expect_err("a setup must be attached to an Adventure catalog entry");

    assert!(failure.contains("adventure reference fixture:entry-000 has kind TurnOrder"));
}

#[test]
fn game_setup_references_must_be_unique() {
    let setup_bundle = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    let mut duplicate_adventure = setup_bundle.clone();
    let duplicate_setup = duplicate_adventure["game_setups"][0].clone();
    duplicate_adventure["game_setups"]
        .as_array_mut()
        .expect("setups should be an array")
        .push(duplicate_setup);
    let mut duplicate_entity = setup_bundle.clone();
    let repeated_entity = duplicate_entity["game_setups"][0]["entities"][0].clone();
    duplicate_entity["game_setups"][0]["entities"]
        .as_array_mut()
        .expect("entities should be an array")
        .push(repeated_entity);
    let mut duplicate_source = setup_bundle;
    duplicate_source["game_setups"][0]["sources"] = json!(["fixture-source", "fixture-source"]);

    for (bundle, expected_message) in [
        (duplicate_adventure, "duplicate game setup for adventure"),
        (duplicate_entity, "duplicate entity reference"),
        (duplicate_source, "duplicate provenance source"),
    ] {
        let failure = import_value(&bundle).expect_err("setup references must be unique");

        assert!(failure.contains(expected_message), "failure was: {failure}");
    }
}

#[test]
fn game_setup_provenance_must_be_present_and_resolve() {
    let setup_bundle = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    let mut empty_sources = setup_bundle.clone();
    empty_sources["game_setups"][0]["sources"] = json!([]);
    let mut unknown_source = setup_bundle;
    unknown_source["game_setups"][0]["sources"] = json!(["missing-source"]);

    for (bundle, expected_message) in [
        (empty_sources, "has no provenance sources"),
        (unknown_source, "unknown provenance source missing-source"),
    ] {
        let failure = import_value(&bundle)
            .expect_err("setup provenance must be non-empty and reference declared sources");

        assert!(failure.contains(expected_message), "failure was: {failure}");
    }
}

#[test]
fn empty_game_setup_prevents_manifest_publication() {
    let mut bundle = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    bundle["sources"][0]["kind"] = json!("validated");
    bundle["game_setups"][0]["confidence"] = json!("validated");
    bundle["game_setups"][0]["entities"] = json!([]);
    let trusted_sources = [ProvenanceSource {
        id: "fixture-source".to_owned(),
        uri: "https://example.invalid/fixture".to_owned(),
        kind: SourceKind::Validated,
    }];

    let failure = import_value_with_trusted_sources(&bundle, &trusted_sources)
        .expect_err("a published setup must contain at least one entity");

    assert!(failure.contains("must contain at least one entity"));
}

#[test]
fn game_setup_copy_requirements_must_fit_the_catalog_inventory() {
    let setup_bundle = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    let mut zero_copies = setup_bundle.clone();
    zero_copies["game_setups"][0]["entities"][0]["copies"] = json!(0);
    let mut shared_excess = setup_bundle.clone();
    shared_excess["game_setups"][0]["entities"][0]["copies"] = json!(3);
    let mut participant_excess = setup_bundle;
    participant_excess["game_setups"][0]["entities"][0]["copies"] = json!(1);
    participant_excess["game_setups"][0]["entities"][0]["owner"] = json!("each_participant");
    participant_excess["game_setups"][0]["entities"][0]["zone"] = json!("hero_draw_pile");
    participant_excess["entries"][1]["kind"] = json!("starter_card");
    participant_excess["entries"][1]["required_functional_fields"] = json!(["effect"]);
    participant_excess["entries"][1]["functional"]
        .as_object_mut()
        .expect("functional data should be an object")
        .remove("health");
    participant_excess["entries"][1]["functional"]
        .as_object_mut()
        .expect("functional data should be an object")
        .remove("reward");

    for (bundle, expected_message) in [
        (zero_copies, "must request at least one copy"),
        (
            shared_excess,
            "requires 3 copies but the catalog declares 2",
        ),
        (
            participant_excess,
            "requires 4 copies but the catalog declares 2",
        ),
    ] {
        let failure = import_value(&bundle).expect_err("setup copies must fit physical inventory");

        assert!(failure.contains(expected_message), "failure was: {failure}");
    }
}

#[test]
fn game_setup_entity_kind_zone_and_owner_must_be_compatible() {
    let starter_for_each = bundle_with_setup_entity(
        "starter_card",
        &["effect"],
        "hero_draw_pile",
        "each_participant",
    );
    let mut starter_without_owner = starter_for_each.clone();
    starter_without_owner["game_setups"][0]["entities"][0]["owner"] = json!("none");
    let mut shared_card_with_owner = bundle_with_setup_entity(
        "hogwarts_card",
        &["cost", "effect"],
        "hogwarts_deck",
        "each_participant",
    );
    expand_setup_entity_to_four_copies(&mut shared_card_with_owner);
    let villain_in_card_deck = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "hogwarts_deck",
        "none",
    );
    let starter_in_market = bundle_with_setup_entity("starter_card", &["effect"], "market", "none");
    let villain_in_non_card_zone =
        bundle_with_setup_entity("villain", &["effect", "health", "reward"], "heroes", "none");

    for (bundle, expected_message) in [
        (
            starter_without_owner,
            "kind StarterCard, zone HeroDrawPile, and owner None are incompatible",
        ),
        (
            shared_card_with_owner,
            "kind HogwartsCard, zone HogwartsDeck, and owner EachParticipant are incompatible",
        ),
        (
            villain_in_card_deck,
            "kind Villain, zone HogwartsDeck, and owner None are incompatible",
        ),
        (
            starter_in_market,
            "kind StarterCard, zone Market, and owner None are incompatible",
        ),
        (
            villain_in_non_card_zone,
            "kind Villain, zone Heroes, and owner None are incompatible",
        ),
    ] {
        let failure = import_value(&bundle)
            .expect_err("a setup entity must use a compatible kind, zone, and owner");

        assert!(failure.contains(expected_message), "failure was: {failure}");
    }

    let mut valid_starter = starter_for_each;
    expand_setup_entity_to_four_copies(&mut valid_starter);
    import_value(&valid_starter)
        .expect("a StarterCard may be copied into each participant's ordered draw pile");
}

#[test]
fn every_supported_setup_kind_zone_and_owner_combination_imports() {
    let cases = [
        ("starter_card", "hero_discard_pile", "each_participant"),
        ("starter_card", "hero_draw_pile", "each_participant"),
        ("starter_card", "hero_hand", "each_participant"),
        ("starter_card", "hero_play_area", "each_participant"),
        ("hogwarts_card", "hero_discard_pile", "each_participant"),
        ("hogwarts_card", "hero_draw_pile", "each_participant"),
        ("hogwarts_card", "hero_hand", "each_participant"),
        ("hogwarts_card", "hero_play_area", "each_participant"),
        ("hogwarts_card", "hogwarts_deck", "none"),
        ("hogwarts_card", "market", "none"),
        ("villain", "villain_deck", "none"),
        ("villain", "active_villains", "none"),
        ("villain", "villain_discard", "none"),
        ("location", "active_location", "none"),
        ("location", "location_deck", "none"),
        ("location", "location_discard", "none"),
    ];

    for (kind, zone, owner) in cases {
        let required_fields: &[&str] = match kind {
            "starter_card" => &["effect"],
            "hogwarts_card" => &["cost", "effect"],
            "villain" => &["effect", "health", "reward"],
            "location" => &["control_limit", "dark_arts_count", "effect"],
            _ => unreachable!("the fixture contains only setup-compatible kinds"),
        };
        let mut bundle = bundle_with_setup_entity(kind, required_fields, zone, owner);
        if owner == "each_participant" {
            expand_setup_entity_to_four_copies(&mut bundle);
        }

        import_value(&bundle).unwrap_or_else(|failure| {
            panic!("{kind} in {zone} with {owner} should import: {failure}")
        });
    }
}

#[test]
fn game_setup_entities_require_domain_data_compatible_with_their_kind() {
    let mut starter_without_effect = bundle_with_setup_entity(
        "starter_card",
        &["effect"],
        "hero_draw_pile",
        "each_participant",
    );
    expand_setup_entity_to_four_copies(&mut starter_without_effect);
    starter_without_effect["entries"][1]["functional"] = json!({});
    let mut hogwarts_without_cost = bundle_with_setup_entity(
        "hogwarts_card",
        &["cost", "effect"],
        "hogwarts_deck",
        "none",
    );
    hogwarts_without_cost["entries"][1]["functional"]
        .as_object_mut()
        .expect("functional data should be an object")
        .remove("cost");
    let mut hogwarts_with_ambiguous_cost = bundle_with_setup_entity(
        "hogwarts_card",
        &["cost", "effect"],
        "hogwarts_deck",
        "none",
    );
    hogwarts_with_ambiguous_cost["entries"][1]["functional"]["cost"]["rule"] =
        json!("rule:setup-entity");
    let mut villain_without_health = bundle_with_setup_entity(
        "villain",
        &["effect", "health", "reward"],
        "villain_deck",
        "none",
    );
    villain_without_health["entries"][1]["functional"]
        .as_object_mut()
        .expect("functional data should be an object")
        .remove("health");

    for (bundle, expected_message) in [
        (starter_without_effect, "requires an Effect rule"),
        (hogwarts_without_cost, "requires a Cost value"),
        (hogwarts_with_ambiguous_cost, "requires a Cost value"),
        (villain_without_health, "requires a positive Health value"),
    ] {
        let failure = import_value(&bundle)
            .expect_err("setup entities must provide the data required by the domain");

        assert!(failure.contains(expected_message), "failure was: {failure}");
    }
}

#[test]
fn structurally_complete_but_semantically_empty_bundle_is_not_playable() {
    let manifest = import_base_bundle(&complete_bundle()).expect("bundle should be valid");

    assert_eq!(manifest.manifest_version, 3);
    assert_eq!(manifest.content_version, "fixture-v1");
    assert_eq!(manifest.ruleset_version, "fixture-rules-v1");
    assert!(manifest.digest.starts_with("blake3:"));
    assert_eq!(manifest.record_count, 171);
    assert_eq!(manifest.card_count, 252);
    assert!(!manifest.playable);
    assert!(manifest.gaps.is_empty());
    assert!(serde_json::to_value(manifest).is_ok());
}

#[test]
fn playable_bundle_without_game_setup_preserves_playability() {
    let bundle = playable_bundle_without_setup();
    let trusted_sources = [ProvenanceSource {
        id: "fixture-source".to_owned(),
        uri: "https://example.invalid/fixture".to_owned(),
        kind: SourceKind::Validated,
    }];
    let executable_rules =
        BTreeSet::from([RuleId::parse("rule:playable").expect("fixture rule ID should be valid")]);

    let manifest = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &trusted_sources,
        &executable_rules,
    )
    .expect("a complete bundle without game_setups should remain publishable");

    assert!(manifest.playable);
    assert!(manifest.gaps.is_empty());
    assert!(manifest.game_setups.is_empty());
}

#[test]
fn equivalent_canonical_content_produces_the_same_digest() {
    let original = complete_bundle();
    let mut reordered = bundle_value();
    reordered["entries"]
        .as_array_mut()
        .expect("entries should be an array")
        .reverse();

    let original_manifest = import_base_bundle(&original).expect("original should import");
    let reordered_manifest = import_base_bundle(
        &serde_json::to_vec_pretty(&reordered).expect("reordered fixture should serialize"),
    )
    .expect("reordered bundle should import");

    assert_eq!(original_manifest.digest, reordered_manifest.digest);
}

#[test]
fn omitted_and_explicitly_empty_game_setups_have_the_same_digest() {
    let omitted = complete_bundle();
    let mut explicit = bundle_value();
    explicit["game_setups"] = json!([]);

    let omitted_manifest = import_base_bundle(&omitted).expect("omitted setups should import");
    let explicit_manifest = import_value(&explicit).expect("empty setups should import");

    assert_eq!(omitted_manifest.digest, explicit_manifest.digest);
}

#[test]
fn changing_declared_setup_entity_order_changes_the_digest() {
    let declared = bundle_with_ordered_setup_entities();
    let mut reversed = declared.clone();
    reversed["game_setups"][0]["entities"]
        .as_array_mut()
        .expect("setup entities should be an array")
        .reverse();

    let declared_manifest = import_value(&declared).expect("declared setup should import");
    let reversed_manifest = import_value(&reversed).expect("reversed setup should import");

    assert_ne!(declared_manifest.digest, reversed_manifest.digest);
}

#[test]
fn duplicate_catalog_id_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["entries"][1]["id"] = bundle["entries"][0]["id"].clone();

    let failure =
        import_value(&bundle).expect_err("duplicate catalog identity must prevent publication");

    assert!(failure.contains("duplicate catalog ID"));
}

#[test]
fn duplicate_rule_id_prevents_publication_across_semantic_phase_order() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([
        {
            "id": "rule:duplicate",
            "trigger": "manual",
            "order": 1,
            "effect": { "type": "no_op" }
        },
        {
            "id": "rule:between",
            "trigger": "villains",
            "order": 1,
            "effect": { "type": "no_op" }
        },
        {
            "id": "rule:duplicate",
            "trigger": "dark_arts",
            "order": 1,
            "effect": { "type": "no_op" }
        }
    ]);

    let failure = import_value(&bundle)
        .expect_err("duplicate rule identity must not depend on canonical rule order");

    assert!(failure.contains("duplicate rule ID rule:duplicate"));
}

#[test]
fn unknown_entry_kind_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["kind"] = json!("mystery_card");

    let failure = import_value(&bundle).expect_err("entry kinds must form a closed set");

    assert!(failure.contains("unknown variant `mystery_card`"));
}

#[test]
fn unknown_source_kind_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["sources"][0]["kind"] = json!("self_validated");

    let failure = import_value(&bundle).expect_err("source kinds must form a closed set");

    assert!(failure.contains("unknown variant `self_validated`"));
}

#[test]
fn empty_or_relative_bundle_metadata_prevents_manifest_publication() {
    let cases = [
        ("/content_version", json!(""), "content version"),
        ("/ruleset_version", json!("rules v1"), "ruleset version"),
        ("/locale", json!(""), "locale"),
        ("/sources/0/id", json!(""), "source ID"),
        (
            "/sources/0/uri",
            json!("sources/fixture"),
            "absolute HTTPS URI",
        ),
    ];

    for (pointer, invalid_value, expected_message) in cases {
        let mut bundle = bundle_value();
        *bundle
            .pointer_mut(pointer)
            .expect("metadata path should exist") = invalid_value;

        let failure = import_value(&bundle).expect_err("invalid metadata must fail closed");

        assert!(failure.contains(expected_message), "failure was: {failure}");
    }
}

#[test]
fn bundle_version_identifiers_are_bounded_to_256_bytes() {
    for pointer in ["/content_version", "/ruleset_version"] {
        let mut boundary = bundle_value();
        *boundary
            .pointer_mut(pointer)
            .expect("version path should exist") = json!("v".repeat(256));
        import_value(&boundary).expect("a 256-byte version identifier should remain valid");

        let mut oversized = bundle_value();
        *oversized
            .pointer_mut(pointer)
            .expect("version path should exist") = json!("v".repeat(257));
        let failure =
            import_value(&oversized).expect_err("an oversized version identifier must fail closed");

        assert!(
            failure.contains("at most 256 bytes"),
            "failure was: {failure}"
        );
    }
}

#[test]
fn entry_outside_the_seven_base_games_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["introduced_in"] = json!(8);

    let failure = import_value(&bundle).expect_err("the base progression must stay bounded");

    assert!(failure.contains("base game from 1 through 7"));
}

#[test]
fn catalog_identity_is_independent_from_localized_names_and_instance_identity() {
    let original = complete_bundle();
    let mut localized = bundle_value();
    localized["entries"][0]["names"] = json!({
        "en": "Fixture 0",
        "pt-BR": "Exemplo 0"
    });
    localized["entries"][0]["provenance"]["names.pt-BR"] = json!(["fixture-source"]);

    let original_manifest = import_base_bundle(&original).expect("original should import");
    let localized_manifest = import_base_bundle(
        &serde_json::to_vec(&localized).expect("localized fixture should serialize"),
    )
    .expect("localized bundle should import");

    assert_eq!(
        original_manifest.entries[0].catalog_id,
        localized_manifest.entries[0].catalog_id
    );
    assert_eq!(
        original_manifest.entries[0].catalog_id,
        CatalogId::parse("fixture:entry-000").expect("catalog ID should be valid")
    );
    assert_ne!(
        std::any::TypeId::of::<CatalogId>(),
        std::any::TypeId::of::<CardInstanceId>()
    );
}

#[test]
fn runtime_card_instance_ids_fit_public_choice_values() {
    let maximum = format!("card:{}", "a".repeat(251));
    let too_long = format!("card:{}", "a".repeat(252));

    assert!(CardInstanceId::parse(&maximum).is_ok());
    assert!(CardInstanceId::parse(&too_long).is_err());
}

#[test]
fn unproven_functional_field_keeps_the_entry_and_manifest_unplayable() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);

    let manifest =
        import_value(&bundle).expect("incomplete content should produce a non-playable manifest");

    assert!(!manifest.playable);
    assert!(!manifest.entries[0].playable);
    assert_eq!(manifest.gaps.len(), 1);
    assert_eq!(manifest.gaps[0].entry_id.as_str(), "fixture:entry-000");
    assert_eq!(manifest.gaps[0].field, FunctionalField::Effect);
}

#[test]
fn required_functional_fields_cannot_be_omitted_for_an_entry_kind() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["kind"] = json!("dark_arts");

    let failure = import_value(&bundle)
        .expect_err("an entry kind must determine its required functional fields");

    assert!(failure.contains("must declare functional fields"));
    assert!(failure.contains("Effect"));
}

#[test]
fn candidate_source_cannot_self_validate_a_functional_definition() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["rules"] = json!([{
        "id": "rule:candidate",
        "order": 0,
        "effect": { "type": "no_op" }
    }]);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "validated",
            "sources": ["fixture-source"],
            "rule": "rule:candidate"
        }
    });

    let manifest =
        import_value(&bundle).expect("unproven semantics should produce a non-playable manifest");

    assert!(!manifest.entries[0].playable);
    assert_eq!(manifest.entries[0].gaps, [FunctionalField::Effect]);
    assert_eq!(
        manifest.entries[0].functional_provenance[&FunctionalField::Effect].sources,
        ["fixture-source"]
    );
}

#[test]
fn a_no_op_rule_cannot_satisfy_a_required_functional_effect() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["sources"]
        .as_array_mut()
        .expect("sources should be an array")
        .push(json!({
            "id": "adaptation-source",
            "uri": "https://example.invalid/adaptation",
            "kind": "adaptation"
        }));
    bundle["rules"] = json!([{
        "id": "rule:adapted",
        "order": 0,
        "effect": { "type": "no_op" }
    }]);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "adaptation",
            "sources": ["adaptation-source"],
            "rule": "rule:adapted"
        }
    });

    let manifest = import_value(&bundle).expect("an explicit no-op should remain publishable");

    assert!(!manifest.entries[0].playable);
    assert_eq!(manifest.entries[0].gaps, [FunctionalField::Effect]);
}

#[test]
fn a_self_declared_validated_source_cannot_make_a_functional_field_playable() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["sources"]
        .as_array_mut()
        .expect("sources should be an array")
        .push(json!({
            "id": "validated-source",
            "uri": "https://example.invalid/validated",
            "kind": "validated"
        }));
    bundle["rules"] = json!([{
        "id": "rule:validated",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "zone": "hero_hand",
                "cardinality": { "min": 1, "max": 1 }
            },
            "operation": { "type": "discard" }
        }
    }]);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "validated",
            "sources": ["validated-source"],
            "rule": "rule:validated"
        }
    });

    let manifest = import_value(&bundle).expect("untrusted evidence should remain publishable");

    assert!(!manifest.entries[0].playable);
    assert_eq!(manifest.entries[0].gaps, [FunctionalField::Effect]);
}

#[test]
fn an_external_trust_decision_alone_cannot_publish_a_discarded_rule() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["sources"]
        .as_array_mut()
        .expect("sources should be an array")
        .push(json!({
            "id": "validated-source",
            "uri": "https://example.invalid/validated",
            "kind": "validated"
        }));
    bundle["rules"] = json!([{
        "id": "rule:validated",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "zone": "hero_hand",
                "cardinality": { "min": 1, "max": 1 }
            },
            "operation": { "type": "discard" }
        }
    }]);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "validated",
            "sources": ["validated-source"],
            "rule": "rule:validated"
        }
    });
    let trusted_sources = [ProvenanceSource {
        id: "validated-source".to_owned(),
        uri: "https://example.invalid/validated".to_owned(),
        kind: SourceKind::Validated,
    }];

    let manifest = import_value_with_trusted_sources(&bundle, &trusted_sources)
        .expect("externally trusted evidence should be publishable");

    assert!(!manifest.entries[0].playable);
    assert_eq!(manifest.entries[0].gaps, [FunctionalField::Effect]);
    assert!(
        !manifest.playable,
        "the incomplete catalog shape must remain unplayable"
    );
}

#[test]
fn a_trusted_rule_is_playable_only_when_the_runtime_declares_it_executable() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["sources"]
        .as_array_mut()
        .expect("sources should be an array")
        .push(json!({
            "id": "validated-source",
            "uri": "https://example.invalid/validated",
            "kind": "validated"
        }));
    bundle["rules"] = json!([{
        "id": "rule:validated",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "zone": "hero_hand",
                "cardinality": { "min": 1, "max": 1 }
            },
            "operation": { "type": "discard" }
        }
    }]);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "validated",
            "sources": ["validated-source"],
            "rule": "rule:validated"
        }
    });
    let trusted_sources = [ProvenanceSource {
        id: "validated-source".to_owned(),
        uri: "https://example.invalid/validated".to_owned(),
        kind: SourceKind::Validated,
    }];
    let executable_rules =
        BTreeSet::from([RuleId::parse("rule:validated").expect("fixture rule ID should be valid")]);
    let trust_only_manifest = import_value_with_trusted_sources(&bundle, &trusted_sources)
        .expect("trusted evidence should remain publishable without runtime support");

    let manifest = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &trusted_sources,
        &executable_rules,
    )
    .expect("runtime-supported evidence should be publishable");

    assert!(manifest.entries[0].playable);
    assert!(manifest.entries[0].gaps.is_empty());
    assert_ne!(
        manifest.digest, trust_only_manifest.digest,
        "runtime capability decisions belong to manifest identity"
    );
}

#[test]
fn localized_name_without_field_provenance_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["names"]["pt-BR"] = json!("Exemplo 0");

    let failure = import_value(&bundle).expect_err("every localized field must have provenance");

    assert!(failure.contains("names.pt-BR"));
    assert!(failure.contains("no provenance"));
}

#[test]
fn unknown_rule_reference_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "validated",
            "sources": ["fixture-source"],
            "rule": "rule:missing"
        }
    });

    let failure = import_value(&bundle).expect_err("a missing rule must prevent publication");

    assert!(failure.contains("unknown rule reference"));
    assert!(failure.contains("rule:missing"));
}

#[test]
fn unknown_functional_provenance_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["rules"] = json!([{
        "id": "rule:known",
        "order": 0,
        "effect": { "type": "no_op" }
    }]);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "validated",
            "sources": ["missing-source"],
            "rule": "rule:known"
        }
    });

    let failure =
        import_value(&bundle).expect_err("unknown functional provenance must prevent publication");

    assert!(failure.contains("unknown provenance source"));
    assert!(failure.contains("missing-source"));
}

#[test]
fn cyclic_rules_prevent_manifest_publication() {
    let mut bundle = bundle_value();
    require_effect(&mut bundle);
    bundle["entries"][0]["functional"] = json!({
        "effect": {
            "confidence": "validated",
            "sources": ["fixture-source"],
            "rule": "rule:first"
        }
    });
    bundle["rules"] = json!([
        {
            "id": "rule:first",
            "order": 0,
            "effect": { "type": "reference", "rule": "rule:second" }
        },
        {
            "id": "rule:second",
            "order": 0,
            "effect": { "type": "reference", "rule": "rule:first" }
        }
    ]);

    let failure = import_value(&bundle).expect_err("cyclic rules must prevent publication");

    assert!(failure.contains("rule cycle"));
    assert!(failure.contains("rule:first"));
}

#[test]
fn choice_without_conclusions_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:invalid-choice",
        "order": 0,
        "effect": { "type": "choice", "options": [] }
    }]);

    let failure = import_value(&bundle).expect_err("an open-ended choice must prevent publication");

    assert!(failure.contains("choice must have at least two conclusions"));
}

#[test]
fn participant_choice_audience_survives_the_closed_content_boundary() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:participant-choice",
        "order": 0,
        "effect": {
            "type": "choice",
            "audience": "each_hero",
            "options": [
                { "type": "no_op" },
                { "type": "terminal", "outcome": "won" }
            ]
        }
    }]);
    let rule_id = RuleId::parse("rule:participant-choice").expect("fixture ID should be valid");

    let manifest = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &BTreeSet::from([rule_id]),
    )
    .expect("the closed participant choice should import");

    assert!(matches!(
        &manifest.rules[0].effect,
        Effect::Choice {
            audience: EffectChoiceAudience::EachHero,
            ..
        }
    ));
}

#[test]
fn default_actor_audience_preserves_the_legacy_canonical_choice_shape() {
    let legacy = json!({
        "type": "choice",
        "options": [
            { "type": "no_op" },
            { "type": "terminal", "outcome": "won" }
        ]
    });
    let effect = serde_json::from_value::<Effect>(legacy.clone())
        .expect("the legacy choice shape should remain readable");

    assert_eq!(
        serde_json::to_value(effect).expect("the choice should serialize"),
        legacy
    );
}

#[test]
fn participant_choice_complexity_accounts_for_all_four_heroes() {
    let mut bundle = bundle_value();
    let mut repeated_effects = vec![json!({ "type": "no_op" }); 63];
    repeated_effects.push(json!({
        "type": "apply",
        "target": {
            "zone": "heroes",
            "owner": "actor",
            "cardinality": { "min": 1, "max": 1 }
        },
        "operation": {
            "type": "modify_resource",
            "resource": "attack",
            "amount": 1
        }
    }));
    let repeated_sequence = json!({
        "type": "repeat",
        "times": 16,
        "effect": {
            "type": "sequence",
            "effects": repeated_effects
        }
    });
    bundle["rules"] = json!([{
        "id": "rule:participant-choice-limit",
        "order": 0,
        "effect": {
            "type": "choice",
            "audience": "each_hero",
            "options": [repeated_sequence, { "type": "no_op" }]
        }
    }]);
    let rule_id =
        RuleId::parse("rule:participant-choice-limit").expect("fixture ID should be valid");

    let failure = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &BTreeSet::from([rule_id]),
    )
    .expect_err("four participant resolutions must fit the runtime step limit");

    assert!(
        failure
            .to_string()
            .contains("closed effect execution limit")
    );
}

#[test]
fn executable_rule_ids_leave_room_for_the_deterministic_choice_suffix() {
    let mut bundle = bundle_value();
    let rule_id = format!("rule:{}", "r".repeat(240));
    bundle["rules"] = json!([{
        "id": rule_id,
        "order": 0,
        "effect": {
            "type": "choice",
            "options": [
                { "type": "no_op" },
                { "type": "terminal", "outcome": "won" }
            ]
        }
    }]);
    let rule_id = RuleId::parse(
        bundle["rules"][0]["id"]
            .as_str()
            .expect("fixture ID should be a string"),
    )
    .expect("the structural ID parser should accept the fixture");

    let failure = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &BTreeSet::from([rule_id]),
    )
    .expect_err("runtime rule IDs must fit every public choice value");

    assert!(failure.to_string().contains("runtime rule ID exceeds 244"));
}

#[test]
fn invalid_selector_cardinality_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:invalid-cardinality",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "zone": "hero_hand",
                "cardinality": { "min": 2, "max": 1 }
            },
            "operation": { "type": "discard" }
        }
    }]);

    let failure = import_value(&bundle).expect_err("invalid cardinality must prevent publication");

    assert!(failure.contains("cardinality min 2 exceeds max 1"));
}

#[test]
fn selector_id_is_optional_and_preserved_without_normalization() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:target-id",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "id": "target:villain-01",
                "zone": "active_villains",
                "cardinality": { "min": 1, "max": 1 }
            },
            "operation": {
                "type": "modify_resource",
                "resource": "health",
                "amount": -1
            }
        }
    }]);
    let rule_id = RuleId::parse("rule:target-id").expect("fixture rule ID should be valid");

    let identified = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &BTreeSet::from([rule_id.clone()]),
    )
    .expect("an identified selector should import");
    bundle["rules"][0]["effect"]["target"]
        .as_object_mut()
        .expect("target should be an object")
        .remove("id");
    let anonymous = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &BTreeSet::from([rule_id]),
    )
    .expect("the pre-existing selector shape without an ID should still import");

    let Effect::Apply {
        target: identified_target,
        ..
    } = &identified.rules[0].effect
    else {
        panic!("fixture should preserve the apply effect");
    };
    let Effect::Apply {
        target: anonymous_target,
        ..
    } = &anonymous.rules[0].effect
    else {
        panic!("fixture should preserve the apply effect");
    };
    assert_eq!(identified_target.id.as_deref(), Some("target:villain-01"));
    assert_eq!(anonymous_target.id, None);
    let anonymous_rule =
        serde_json::to_value(&anonymous.rules[0]).expect("manifest rule should serialize");
    assert!(anonymous_rule.pointer("/effect/target/id").is_none());
}

#[test]
fn empty_selector_id_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:empty-target-id",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "id": "",
                "zone": "hero_hand",
                "cardinality": { "min": 1, "max": 1 }
            },
            "operation": { "type": "discard" }
        }
    }]);

    let failure = import_value(&bundle).expect_err("an empty selector ID must fail closed");

    assert!(failure.contains("selector ID must not be empty"));
}

#[test]
fn a_selector_id_cannot_describe_different_targets_within_one_rule() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:conflicting-target-id",
        "order": 0,
        "effect": {
            "type": "sequence",
            "effects": [
                {
                    "type": "apply",
                    "target": {
                        "id": "target:shared",
                        "zone": "hero_hand",
                        "cardinality": { "min": 1, "max": 1 }
                    },
                    "operation": { "type": "discard" }
                },
                {
                    "type": "apply",
                    "target": {
                        "id": "target:shared",
                        "zone": "heroes",
                        "owner": "actor",
                        "cardinality": { "min": 1, "max": 1 }
                    },
                    "operation": {
                        "type": "modify_resource",
                        "resource": "attack",
                        "amount": 1
                    }
                }
            ]
        }
    }]);

    let failure = import_value(&bundle)
        .expect_err("one selector ID must have one structural definition per rule");

    assert!(failure.contains("selector ID target:shared has conflicting definitions"));

    let first_target = bundle["rules"][0]["effect"]["effects"][0]["target"].clone();
    bundle["rules"][0]["effect"]["effects"][1]["target"] = first_target;
    bundle["rules"][0]["effect"]["effects"][1]["operation"] = json!({ "type": "discard" });

    import_value(&bundle).expect("identical definitions may reuse a selector ID within one rule");
}

#[test]
fn operation_incompatible_with_its_zone_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:invalid-zone",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "zone": "market",
                "cardinality": { "min": 1, "max": 1 }
            },
            "operation": { "type": "discard" }
        }
    }]);

    let failure = import_value(&bundle)
        .expect_err("an incompatible operation and zone must prevent publication");

    assert!(failure.contains("operation discard is incompatible with zone market"));
}

#[test]
fn closed_effect_ast_preserves_every_supported_construct_for_the_runtime() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:closed-language",
        "trigger": "manual",
        "order": 0,
        "cost": [{ "resource": "influence", "amount": 1 }],
        "effect": {
            "type": "sequence",
            "effects": [
                {
                    "type": "apply",
                    "target": {
                        "zone": "heroes",
                        "owner": "actor",
                        "cardinality": { "min": 1, "max": 1 },
                        "eligibility": [{
                            "type": "resource_at_least",
                            "resource": "health",
                            "amount": 1
                        }]
                    },
                    "operation": {
                        "type": "modify_resource",
                        "resource": "attack",
                        "amount": 1
                    }
                },
                {
                    "type": "condition",
                    "condition": {
                        "type": "resource_at_least",
                        "target": {
                            "zone": "heroes",
                            "owner": "actor",
                            "cardinality": { "min": 1, "max": 1 }
                        },
                        "resource": "attack",
                        "amount": 1
                    },
                    "then": {
                        "type": "choice",
                        "options": [
                            { "type": "no_op" },
                            {
                                "type": "repeat",
                                "times": 2,
                                "effect": {
                                    "type": "apply",
                                    "target": {
                                        "zone": "hero_hand",
                                        "cardinality": { "min": 1, "max": 1 }
                                    },
                                    "operation": {
                                        "type": "move",
                                        "to": "hero_discard_pile"
                                    }
                                }
                            }
                        ]
                    }
                },
                {
                    "type": "roll",
                    "die": "d4",
                    "outcomes": [
                        { "type": "no_op" },
                        { "type": "no_op" },
                        { "type": "no_op" },
                        { "type": "terminal", "outcome": "won" }
                    ]
                }
            ]
        }
    }]);
    let rule_id = RuleId::parse("rule:closed-language").expect("fixture rule ID should be valid");

    let manifest = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &BTreeSet::from([rule_id.clone()]),
    )
    .expect("the complete closed AST should import");

    assert_eq!(manifest.rules.len(), 1);
    assert_eq!(manifest.rules[0].id, rule_id);
    assert_eq!(manifest.rules[0].trigger, EffectTrigger::Manual);
}

#[test]
fn arbitrary_script_nodes_cannot_enter_the_effect_ast() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:script",
        "order": 0,
        "effect": {
            "type": "script",
            "source": "state.players[0].attack = 999"
        }
    }]);

    let failure = import_value(&bundle).expect_err("arbitrary scripting must fail closed");

    assert!(failure.contains("unknown variant `script`"));
}

#[test]
fn executable_effects_cannot_exceed_the_runtime_step_limit() {
    let mut bundle = bundle_value();
    let repeated = |effect| {
        json!({
            "type": "repeat",
            "times": 16,
            "effect": effect
        })
    };
    let effect = repeated(repeated(repeated(repeated(json!({
        "type": "apply",
        "target": {
            "zone": "hero_hand",
            "cardinality": { "min": 1, "max": 1 }
        },
        "operation": { "type": "discard" }
    })))));
    bundle["rules"] = json!([{
        "id": "rule:unbounded",
        "trigger": "dark_arts_completed",
        "order": 0,
        "effect": effect
    }]);
    let rule_id = RuleId::parse("rule:unbounded").expect("fixture rule ID should be valid");

    let failure = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &BTreeSet::from([rule_id]),
    )
    .expect_err("validated executable effects must always fit the runtime step bound");

    assert!(failure.to_string().contains("effect execution limit"));
}

#[test]
fn executable_effects_cannot_exceed_the_runtime_outcome_limit() {
    let mut bundle = bundle_value();
    let target = json!({
        "zone": "hero_hand",
        "cardinality": { "min": 0, "max": 31 }
    });
    let branch = json!({
        "type": "condition",
        "condition": {
            "type": "has_eligible_target",
            "target": target.clone()
        },
        "then": { "type": "reference", "rule": "rule:outcome-leaf" },
        "otherwise": { "type": "no_op" }
    });
    let choice = json!({
        "type": "choice",
        "options": [branch, { "type": "no_op" }]
    });
    let roll = json!({
        "type": "roll",
        "die": "d4",
        "outcomes": [
            choice,
            { "type": "no_op" },
            { "type": "no_op" },
            { "type": "no_op" }
        ]
    });
    bundle["rules"] = json!([
        {
            "id": "rule:outcome-root",
            "trigger": "manual",
            "order": 0,
            "cost": [{ "resource": "influence", "amount": 1 }],
            "effect": {
                "type": "sequence",
                "effects": [{
                    "type": "repeat",
                    "times": 8,
                    "effect": {
                        "type": "repeat",
                        "times": 16,
                        "effect": roll
                    }
                }]
            }
        },
        {
            "id": "rule:outcome-leaf",
            "trigger": "manual",
            "order": 1,
            "effect": {
                "type": "apply",
                "target": target,
                "operation": { "type": "discard" }
            }
        }
    ]);
    let root = RuleId::parse("rule:outcome-root").expect("fixture rule ID should be valid");

    let failure = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &BTreeSet::from([root]),
    )
    .expect_err("validated effects and their costs must fit the outcome bound");

    assert!(failure.to_string().contains("effect outcome limit"));
}

#[test]
fn automatic_phase_outcomes_share_one_snapshot_limit() {
    let mut bundle = bundle_value();
    let phase_effect = || {
        json!({
            "type": "repeat",
            "times": 4,
            "effect": {
                "type": "repeat",
                "times": 16,
                "effect": {
                    "type": "apply",
                    "target": {
                        "zone": "hero_hand",
                        "cardinality": { "min": 0, "max": 32 }
                    },
                    "operation": { "type": "discard" }
                }
            }
        })
    };
    bundle["rules"] = json!([
        {
            "id": "rule:dark-arts-outcomes",
            "trigger": "dark_arts",
            "order": 0,
            "effect": phase_effect()
        },
        {
            "id": "rule:villain-outcomes",
            "trigger": "villains",
            "order": 0,
            "effect": {
                "type": "sequence",
                "effects": [phase_effect(), { "type": "no_op" }]
            }
        }
    ]);
    let roots = BTreeSet::from([
        RuleId::parse("rule:dark-arts-outcomes").expect("fixture rule ID should be valid"),
        RuleId::parse("rule:villain-outcomes").expect("fixture rule ID should be valid"),
    ]);

    let failure = import_base_bundle_with_runtime_rules(
        &serde_json::to_vec(&bundle).expect("fixture should serialize"),
        &[],
        &roots,
    )
    .expect_err("Dark Arts and Villains outcomes must fit their shared snapshot bound");

    assert!(failure.to_string().contains("effect outcome limit"));
}

#[test]
fn expansion_or_promo_entry_prevents_base_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["entries"][0]["set"] = json!("expansion");

    let failure =
        import_value(&bundle).expect_err("an expansion entry must prevent base publication");

    assert!(failure.contains("base catalog cannot contain expansion or promo entries"));
}

#[test]
fn candidate_base_catalog_closes_the_declared_inventory_without_becoming_playable() {
    let manifest = import_base_bundle(include_bytes!(
        "../../../content/bundles/base-en-candidate-2026-09-02.json"
    ))
    .expect("the checked-in candidate bundle should be structurally valid");

    assert_eq!(manifest.content_version, "base-en-candidate-2026-09-02");
    assert_eq!(manifest.record_count, 171);
    assert_eq!(manifest.card_count, 252);
    assert!(!manifest.playable);
    assert!(!manifest.gaps.is_empty());
    assert!(manifest.sources.len() >= 3);
    assert!(
        manifest
            .rules
            .iter()
            .all(|rule| !contains_participant_choice(&rule.effect)),
        "the phase-one bundle must not activate participant choices during a rolling deployment"
    );
}
