use std::collections::BTreeSet;

use game_content::{
    CardInstanceId, CatalogId, EffectTrigger, FunctionalField, ProvenanceSource, RuleId,
    SourceKind, import_base_bundle, import_base_bundle_with_runtime_rules,
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
        "schema_version": 1,
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

#[test]
fn structurally_complete_but_semantically_empty_bundle_is_not_playable() {
    let manifest = import_base_bundle(&complete_bundle()).expect("bundle should be valid");

    assert_eq!(manifest.manifest_version, 2);
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
fn duplicate_catalog_id_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["entries"][1]["id"] = bundle["entries"][0]["id"].clone();

    let failure =
        import_value(&bundle).expect_err("duplicate catalog identity must prevent publication");

    assert!(failure.contains("duplicate catalog ID"));
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
            "effect": { "type": "reference", "rule": "rule:second" }
        },
        {
            "id": "rule:second",
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
        "effect": { "type": "choice", "options": [] }
    }]);

    let failure = import_value(&bundle).expect_err("an open-ended choice must prevent publication");

    assert!(failure.contains("choice must have at least two conclusions"));
}

#[test]
fn invalid_selector_cardinality_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:invalid-cardinality",
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
fn operation_incompatible_with_its_zone_prevents_manifest_publication() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:invalid-zone",
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
        "trigger": "dark_arts_completed",
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
    assert_eq!(manifest.rules[0].trigger, EffectTrigger::DarkArtsCompleted);
}

#[test]
fn arbitrary_script_nodes_cannot_enter_the_effect_ast() {
    let mut bundle = bundle_value();
    bundle["rules"] = json!([{
        "id": "rule:script",
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
}
