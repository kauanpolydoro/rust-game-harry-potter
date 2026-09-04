use std::{collections::BTreeSet, env, net::SocketAddr};

use game_content::{
    ContentManifest, ProvenanceSource, RuleId, SourceKind, import_base_bundle_with_runtime_rules,
};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;

fn executable_fixture_manifest() -> ContentManifest {
    let entries = (0..171).map(fixture_entry).collect::<Vec<_>>();
    let bundle = serde_json::to_vec(&json!({
        "schema_version": 1,
        "content_version": "e2e-fixture-v1",
        "ruleset_version": "e2e-fixture-rules-v1",
        "locale": "en",
        "sources": [{
            "id": "fixture-source",
            "uri": "https://example.invalid/e2e-fixture",
            "kind": "adaptation"
        }],
        "rules": [{
            "id": "rule:functional",
            "trigger": "dark_arts_completed",
            "cost": [{ "resource": "health", "amount": 1 }],
            "effect": {
                "type": "sequence",
                "effects": [
                    {
                        "type": "apply",
                        "target": {
                            "zone": "heroes",
                            "owner": "actor",
                            "cardinality": { "min": 1, "max": 1 }
                        },
                        "operation": {
                            "type": "modify_resource",
                            "resource": "influence",
                            "amount": 2
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
                            "resource": "influence",
                            "amount": 2
                        },
                        "then": {
                            "type": "repeat",
                            "times": 2,
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
                        }
                    },
                    {
                        "type": "roll",
                        "die": "d4",
                        "outcomes": [
                            { "type": "no_op" },
                            { "type": "no_op" },
                            { "type": "no_op" },
                            { "type": "no_op" }
                        ]
                    },
                    {
                        "type": "apply",
                        "target": {
                            "zone": "hero_hand",
                            "owner": "actor",
                            "cardinality": { "min": 1, "max": 1 }
                        },
                        "operation": { "type": "discard" }
                    }
                ]
            }
        }],
        "entries": entries
    }))
    .expect("the E2E fixture must serialize");

    import_base_bundle_with_runtime_rules(
        &bundle,
        &[ProvenanceSource {
            id: "fixture-source".to_owned(),
            uri: "https://example.invalid/e2e-fixture".to_owned(),
            kind: SourceKind::Adaptation,
        }],
        &BTreeSet::from([
            RuleId::parse("rule:functional").expect("fixture rule ID should be valid")
        ]),
    )
    .expect("the E2E fixture must import")
}

fn fixture_entry(index: usize) -> serde_json::Value {
    let (id, kind, required_fields): (String, &str, &[&str]) = match index {
        0 => (
            "adventure:001".to_owned(),
            "adventure",
            &["precedence", "setup"],
        ),
        1 => ("fixture:catalog".to_owned(), "catalog", &[]),
        2 => ("fixture:dark-arts".to_owned(), "dark_arts", &["effect"]),
        3 => ("fixture:hero".to_owned(), "hero", &["ability"]),
        4 => (
            "fixture:hogwarts-card".to_owned(),
            "hogwarts_card",
            &["cost", "effect"],
        ),
        5 => (
            "fixture:horcrux".to_owned(),
            "horcrux",
            &["effect", "precedence", "reward"],
        ),
        6 => (
            "fixture:location".to_owned(),
            "location",
            &["control_limit", "dark_arts_count", "effect"],
        ),
        7 => (
            "fixture:proficiency".to_owned(),
            "proficiency",
            &["ability"],
        ),
        8 => ("fixture:ruleset".to_owned(), "ruleset", &["precedence"]),
        9 => (
            "fixture:starter-card".to_owned(),
            "starter_card",
            &["effect"],
        ),
        10 => ("fixture:turn-order".to_owned(), "turn_order", &[]),
        11 => (
            "fixture:villain".to_owned(),
            "villain",
            &["effect", "health", "reward"],
        ),
        _ => (format!("fixture:entry-{index:03}"), "turn_order", &[]),
    };
    let functional = required_fields
        .iter()
        .map(|field| {
            (
                (*field).to_owned(),
                json!({
                    "confidence": "adaptation",
                    "sources": ["fixture-source"],
                    "rule": "rule:functional"
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();

    json!({
        "id": id,
        "kind": kind,
        "set": "base",
        "copies": if index < 81 { 2 } else { 1 },
        "introduced_in": 1,
        "names": { "en": if index == 0 { "Game 1".to_owned() } else { format!("Fixture {index}") } },
        "provenance": {
            "id": ["fixture-source"],
            "kind": ["fixture-source"],
            "set": ["fixture-source"],
            "copies": ["fixture-source"],
            "introduced_in": ["fixture-source"],
            "names.en": ["fixture-source"]
        },
        "required_functional_fields": required_fields,
        "functional": functional
    })
}

#[tokio::main]
async fn main() {
    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be configured");
    let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "127.0.0.1:18080".to_owned());
    let application_origin =
        env::var("APPLICATION_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:4173".to_owned());
    let database = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the E2E database must be reachable");
    let state = AppState::with_content_manifests(database, vec![executable_fixture_manifest()])
        .with_application_origin(application_origin);
    initialize(&state)
        .await
        .expect("the E2E database must initialize");
    state.mark_started();
    let listener = TcpListener::bind(
        bind_address
            .parse::<SocketAddr>()
            .expect("BIND_ADDRESS must be a socket address"),
    )
    .await
    .expect("the E2E listener must bind");

    axum::serve(listener, build_router(state))
        .await
        .expect("the E2E server must run");
}
