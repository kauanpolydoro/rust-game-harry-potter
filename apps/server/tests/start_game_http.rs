use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use game_content::{
    ContentManifest, ProvenanceSource, RuleId, SourceKind, import_base_bundle,
    import_base_bundle_with_runtime_rules,
};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    fmt::Write as _,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Barrier,
    task::JoinSet,
};
use tower::ServiceExt;

struct ReadyRoom {
    app: axum::Router,
    state: AppState,
    database: PgPool,
    room_code: String,
    host_cookie: String,
    host_recovery_token: String,
    guest_cookie: String,
    guest_recovery_token: String,
    manifest: ContentManifest,
}

fn playable_manifest() -> ContentManifest {
    playable_manifest_variant(None)
}

fn terminal_manifest() -> ContentManifest {
    playable_manifest_variant(Some(json!({ "type": "terminal", "outcome": "won" })))
}

fn stunned_locations_manifest() -> ContentManifest {
    let mut candidate = playable_candidate(Some(json!({
        "type": "apply",
        "target": { "zone": "heroes", "owner": "actor", "cardinality": { "min": 1, "max": 1 } },
        "operation": { "type": "modify_resource", "resource": "health", "amount": -10 }
    })));
    candidate["entries"][6]["functional"]["control_limit"] = proven_value(1);
    candidate["entries"][6]["functional"]["dark_arts_count"] = proven_value(1);
    candidate["entries"][9]["copies"] = json!(8);
    for index in 12..18 {
        candidate["entries"][index]["copies"] = json!(1);
    }
    candidate["entries"][18]["kind"] = json!("location");
    candidate["entries"][18]["required_functional_fields"] =
        json!(["control_limit", "dark_arts_count", "effect"]);
    candidate["entries"][18]["functional"] = candidate["entries"][6]["functional"].clone();
    candidate["game_setups"] = json!([{
        "adventure_id": "adventure:001",
        "confidence": "adaptation",
        "sources": ["fixture-source"],
        "entities": [
            { "catalog_id": "fixture:location", "copies": 1, "zone": "active_location", "owner": "none" },
            { "catalog_id": "fixture:entry-018", "copies": 1, "zone": "location_deck", "owner": "none" },
            { "catalog_id": "fixture:starter-card", "copies": 2, "zone": "hero_hand", "owner": "each_participant" }
        ]
    }]);
    import_playable_candidate(&candidate, &["rule:functional"])
}

async fn accepted_request(room: &ReadyRoom, request: Request<Body>) -> Value {
    let response = room
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("the command must receive a response");
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::OK, "response body: {body}");
    body["projection"].clone()
}

#[tokio::test]
async fn stunned_players_resume_after_reload_and_locations_end_the_game_without_elimination() {
    let room = ready_room_with_manifest(stunned_locations_manifest()).await;
    let initial = start_ready_game(&room, "stunned-locations").await;
    assert_eq!(initial["participant"]["resources"]["health"], 0);
    assert_eq!(initial["participant"]["stunned"], true);
    assert_eq!(initial["choice"]["kind"], "stun_discard");
    assert_eq!(initial["choice"]["min"], 1);
    assert_eq!(initial["table"]["current_location"]["control"], 0);

    let resolved = accepted_request(
        &room,
        resolve_choice_request(
            &room.host_cookie,
            uuid::Uuid::new_v4(),
            1,
            initial["choice"]["id"].as_str().expect("stun choice id"),
            &[initial["choice"]["options"][0]
                .as_str()
                .expect("stun discard option")],
        ),
    )
    .await;
    assert_eq!(resolved["participant"]["hand_count"], 1);
    assert_eq!(resolved["table"]["current_location"]["control"], 1);
    assert_eq!(resolved["game"]["status"], "in_progress");
    let next = accepted_request(
        &room,
        command_request(&room.host_cookie, uuid::Uuid::new_v4(), 2),
    )
    .await;
    assert_eq!(next["participant"]["resources"]["health"], 10);
    assert_eq!(
        next["table"]["current_location"]["catalog_id"],
        "fixture:entry-018"
    );
    assert_eq!(next["table"]["location_discard_count"], 1);
    assert_eq!(
        next["participants"].as_array().expect("participants").len(),
        2
    );
    assert_eq!(next["choice"]["responsible_position"], 2);

    let guest = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, &room.guest_cookie)
                .body(Body::empty())
                .expect("guest session request"),
        )
        .await
        .expect("guest session response");
    assert_eq!(guest.status(), StatusCode::OK);
    let guest = response_json(guest).await;
    let resolved = accepted_request(
        &room,
        resolve_choice_request(
            &room.guest_cookie,
            uuid::Uuid::new_v4(),
            3,
            guest["choice"]["id"]
                .as_str()
                .expect("guest stun choice id"),
            &[guest["choice"]["options"][0]
                .as_str()
                .expect("guest stun discard option")],
        ),
    )
    .await;
    assert_eq!(resolved["table"]["current_location"]["control"], 1);
    let lost = accepted_request(
        &room,
        command_request(&room.guest_cookie, uuid::Uuid::new_v4(), 4),
    )
    .await;
    assert_eq!(lost["game"]["status"], "lost");
    assert_eq!(lost["choice"]["status"], "none");
    assert_eq!(lost["legal_actions"], json!([]));
    assert_eq!(lost["table"]["location_discard_count"], 2);
    let rejected = room
        .app
        .clone()
        .oneshot(command_request(&room.guest_cookie, uuid::Uuid::new_v4(), 5))
        .await
        .expect("terminal command response");
    assert_eq!(rejected.status(), StatusCode::CONFLICT);
}

fn final_reward_manifest() -> ContentManifest {
    let mut candidate = playable_candidate(Some(json!({
        "type": "apply",
        "target": { "zone": "heroes", "owner": "actor", "cardinality": { "min": 1, "max": 1 } },
        "operation": { "type": "modify_resource", "resource": "attack", "amount": 2 }
    })));
    candidate["entries"][6]["functional"]["control_limit"] = proven_value(1);
    candidate["entries"][6]["functional"]["dark_arts_count"] = proven_value(1);
    candidate["entries"][11]["functional"]["health"] = proven_value(2);
    candidate["entries"][11]["functional"]["reward"] = proven_rule("rule:final-reward");
    candidate["entries"][9]["functional"]["effect"] = proven_rule("rule:card-damage");
    candidate["entries"][9]["copies"] = json!(4);
    candidate["entries"][12]["copies"] = json!(1);
    candidate["entries"][13]["copies"] = json!(1);
    candidate["rules"].as_array_mut().expect("fixture rules").push(json!({
        "id": "rule:card-damage", "trigger": "manual", "order": 0,
        "effect": {"type": "apply", "target": {"zone": "active_villains", "owner": "any", "cardinality": {"min": 1, "max": 1}},
                   "operation": {"type": "modify_resource", "resource": "health", "amount": -2}}
    }));
    candidate["rules"].as_array_mut().expect("fixture rules").push(json!({
        "id": "rule:final-reward", "trigger": "villain_reward", "order": 0,
        "effect": { "type": "choice", "audience": "actor", "options": [
            { "type": "apply",
              "target": { "zone": "heroes", "owner": "actor", "cardinality": { "min": 1, "max": 1 } },
              "operation": { "type": "modify_resource", "resource": "influence", "amount": 1 } },
            { "type": "apply",
              "target": { "zone": "active_location", "owner": "any", "cardinality": { "min": 1, "max": 1 } },
              "operation": { "type": "modify_resource", "resource": "control", "amount": 1 } }
        ] }
    }));
    candidate["game_setups"] = json!([{
        "adventure_id": "adventure:001", "confidence": "adaptation", "sources": ["fixture-source"],
        "entities": [
            { "catalog_id": "fixture:location", "copies": 1, "zone": "active_location", "owner": "none" },
            { "catalog_id": "fixture:villain", "copies": 1, "zone": "active_villains", "owner": "none" },
            { "catalog_id": "fixture:starter-card", "copies": 1, "zone": "hero_hand", "owner": "each_participant" }
        ]
    }]);
    import_playable_candidate(
        &candidate,
        &["rule:functional", "rule:final-reward", "rule:card-damage"],
    )
}

#[tokio::test]
async fn final_villain_reward_resumes_from_storage_before_victory_or_tie_defeat() {
    for (card_damage, option, expected_status) in [
        (false, "option:1", "won"),
        (false, "option:2", "lost"),
        (true, "option:1", "won"),
        (true, "option:2", "lost"),
    ] {
        let room = ready_room_with_manifest(final_reward_manifest()).await;
        let initial = start_ready_game(&room, "final-reward").await;
        let mut command = json!({
            "command_id": uuid::Uuid::new_v4().to_string(), "expected_state_version": 1, "type": "assign_attack",
            "villain_id": initial["table"]["active_villains"][0]["instance_id"], "amount": 2
        });
        if card_damage {
            command = json!({"command_id": uuid::Uuid::new_v4().to_string(), "expected_state_version": 1, "type": "play_card",
                "card_id": initial["table"]["hand"][0]["instance_id"], "targets": []});
        }
        let reward = accepted_request(
            &room,
            json_request(
                "POST",
                "/api/games/current/commands",
                &command,
                Some(&room.host_cookie),
                None,
            ),
        )
        .await;
        assert_eq!(reward["game"]["status"], "in_progress");
        assert_eq!(reward["table"]["active_villains"], json!([]));
        assert_eq!(reward["table"]["villain_discard_count"], 1);
        assert_eq!(reward["choice"]["status"], "pending");
        let terminal = accepted_request(
            &room,
            resolve_choice_request(
                &room.host_cookie,
                uuid::Uuid::new_v4(),
                2,
                reward["choice"]["id"].as_str().expect("reward choice"),
                &[option],
            ),
        )
        .await;
        assert_eq!(terminal["game"]["status"], expected_status);
        assert_eq!(terminal["choice"]["status"], "none");
        assert_eq!(terminal["legal_actions"], json!([]));
        let rejected = room
            .app
            .clone()
            .oneshot(command_request(&room.host_cookie, uuid::Uuid::new_v4(), 3))
            .await
            .expect("terminal response");
        assert_eq!(rejected.status(), StatusCode::CONFLICT);
    }
}

#[tokio::test]
async fn persisted_locations_and_villain_capacity_must_remain_restorable() {
    let room = ready_room_with_manifest(final_reward_manifest()).await;
    start_ready_game(&room, "terminal-layout").await;
    let snapshot: Value = sqlx::query_scalar("SELECT games.snapshot FROM games JOIN rooms ON rooms.id = games.room_id WHERE rooms.code = $1")
        .bind(&room.room_code).fetch_one(&room.database).await.expect("committed snapshot");
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT valid_game_snapshot_v4($1)")
            .bind(&snapshot)
            .fetch_one(&room.database)
            .await
            .expect("valid layout")
    );
    let mut no_capacity = snapshot.clone();
    no_capacity["active_villain_limit"] = json!(0);
    let mut two_locations = snapshot.clone();
    let mut extra = two_locations["effects"]["entities"]
        .as_array()
        .expect("entities")
        .iter()
        .find(|entity| entity["zone"] == "active_location")
        .expect("location")
        .clone();
    extra["id"] = json!("instance:duplicate-location");
    extra["zone_index"] = json!(1);
    two_locations["effects"]["entities"]
        .as_array_mut()
        .expect("entities")
        .push(extra);
    for invalid in [no_capacity, two_locations] {
        assert!(
            !sqlx::query_scalar::<_, bool>("SELECT valid_game_snapshot_v4($1)")
                .bind(&invalid)
                .fetch_one(&room.database)
                .await
                .expect("invalid layout validation")
        );
    }
}

fn each_hero_choice_manifest() -> ContentManifest {
    playable_manifest_variant(Some(json!({
        "type": "choice",
        "audience": "each_hero",
        "options": [
            {
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
            },
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
                    "amount": 1
                }
            }
        ]
    })))
}

fn functional_effect_rule() -> Value {
    json!({
        "id": "rule:functional",
        "trigger": "dark_arts",
        "order": 1,
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
                        "resource": "health",
                        "amount": -1
                    }
                },
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
    })
}

fn playable_manifest_variant(effect_override: Option<Value>) -> ContentManifest {
    import_playable_candidate(&playable_candidate(effect_override), &["rule:functional"])
}

fn playable_candidate(effect_override: Option<Value>) -> Value {
    let entries = (0..171).map(playable_fixture_entry).collect::<Vec<_>>();
    let mut candidate = json!({
        "schema_version": 2,
        "content_version": "fixture-v1",
        "ruleset_version": "fixture-rules-v1",
        "locale": "en",
        "sources": [{
            "id": "fixture-source",
            "uri": "https://example.invalid/fixture",
            "kind": "adaptation"
        }],
        "rules": [functional_effect_rule()],
        "entries": entries
    });
    if let Some(effect) = effect_override {
        candidate["rules"][0]["effect"] = effect;
    }
    candidate
}

fn gameplay_manifest() -> ContentManifest {
    let mut candidate = playable_candidate(None);
    candidate["rules"] = gameplay_rules();
    configure_gameplay_entries(&mut candidate);
    candidate["game_setups"] = gameplay_setups();

    import_playable_candidate(
        &candidate,
        &[
            "rule:functional",
            "rule:starter-resources",
            "rule:card-effect",
            "rule:villain-reward",
        ],
    )
}

fn optional_target_manifest() -> ContentManifest {
    let mut candidate = playable_candidate(None);
    candidate["rules"] = gameplay_rules();
    candidate["rules"][1] = json!({
        "id": "rule:optional-card",
        "trigger": "manual",
        "order": 0,
        "effect": {
            "type": "apply",
            "target": {
                "id": "target:optional-hero",
                "zone": "heroes",
                "owner": "actor",
                "cardinality": { "min": 0, "max": 1 }
            },
            "operation": {
                "type": "modify_resource",
                "resource": "health",
                "amount": 1
            }
        }
    });
    configure_gameplay_entries(&mut candidate);
    candidate["entries"][9]["functional"]["effect"] = proven_rule("rule:optional-card");
    candidate["game_setups"] = gameplay_setups();

    import_playable_candidate(
        &candidate,
        &[
            "rule:functional",
            "rule:optional-card",
            "rule:card-effect",
            "rule:villain-reward",
        ],
    )
}

fn proven_rule(rule: &str) -> Value {
    json!({
        "confidence": "adaptation",
        "sources": ["fixture-source"],
        "rule": rule
    })
}

fn proven_value(value: u16) -> Value {
    json!({
        "confidence": "adaptation",
        "sources": ["fixture-source"],
        "value": value
    })
}

fn gameplay_rules() -> Value {
    json!([
        {
            "id": "rule:functional",
            "trigger": "dark_arts_completed",
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
                    "resource": "health",
                    "amount": -1
                }
            }
        },
        {
            "id": "rule:starter-resources",
            "trigger": "manual",
            "order": 0,
            "effect": {
                "type": "sequence",
                "effects": [
                    {
                        "type": "apply",
                        "target": {
                            "id": "target:hero",
                            "zone": "heroes",
                            "owner": "any",
                            "cardinality": { "min": 1, "max": 1 }
                        },
                        "operation": {
                            "type": "modify_resource",
                            "resource": "attack",
                            "amount": 2
                        }
                    },
                    {
                        "type": "apply",
                        "target": {
                            "id": "target:hero",
                            "zone": "heroes",
                            "owner": "any",
                            "cardinality": { "min": 1, "max": 1 }
                        },
                        "operation": {
                            "type": "modify_resource",
                            "resource": "influence",
                            "amount": 3
                        }
                    }
                ]
            }
        },
        {
            "id": "rule:card-effect",
            "trigger": "manual",
            "order": 1,
            "effect": {
                "type": "apply",
                "target": {
                    "zone": "heroes",
                    "owner": "actor",
                    "cardinality": { "min": 1, "max": 1 }
                },
                "operation": {
                    "type": "modify_resource",
                    "resource": "health",
                    "amount": 1
                }
            }
        }
        ,{
            "id": "rule:villain-reward",
            "trigger": "villain_reward",
            "order": 0,
            "effect": {
                "type": "apply",
                "target": { "zone": "heroes", "owner": "actor", "cardinality": { "min": 1, "max": 1 } },
                "operation": { "type": "modify_resource", "resource": "health", "amount": 1 }
            }
        }
    ])
}

fn configure_gameplay_entries(candidate: &mut Value) {
    candidate["entries"][4]["functional"]["cost"] = proven_value(2);
    candidate["entries"][4]["functional"]["effect"] = proven_rule("rule:card-effect");
    candidate["entries"][9]["copies"] = json!(4);
    candidate["entries"][9]["functional"]["effect"] = proven_rule("rule:starter-resources");
    candidate["entries"][11]["functional"]["effect"] = proven_rule("rule:card-effect");
    candidate["entries"][11]["functional"]["health"] = proven_value(2);
    candidate["entries"][11]["functional"]["reward"] = proven_rule("rule:villain-reward");
    candidate["entries"][14]["kind"] = json!("villain");
    candidate["entries"][14]["required_functional_fields"] =
        candidate["entries"][11]["required_functional_fields"].clone();
    candidate["entries"][14]["functional"] = candidate["entries"][11]["functional"].clone();
    for (index, id, name, cost) in [
        (12, "fixture:market-second", "Market Second", 1),
        (13, "fixture:deck-first", "Deck First", 4),
    ] {
        candidate["entries"][index]["id"] = json!(id);
        candidate["entries"][index]["kind"] = json!("hogwarts_card");
        candidate["entries"][index]["copies"] = json!(1);
        candidate["entries"][index]["names"] = json!({ "en": name });
        candidate["entries"][index]["required_functional_fields"] = json!(["cost", "effect"]);
        candidate["entries"][index]["functional"] = json!({
            "cost": proven_value(cost),
            "effect": proven_rule("rule:card-effect")
        });
    }
}

fn gameplay_setups() -> Value {
    json!([{
        "adventure_id": "adventure:001",
        "confidence": "adaptation",
        "sources": ["fixture-source"],
        "entities": [
            {
                "catalog_id": "fixture:starter-card",
                "copies": 1,
                "zone": "hero_hand",
                "owner": "each_participant"
            },
            {
                "catalog_id": "fixture:hogwarts-card",
                "copies": 1,
                "zone": "market",
                "owner": "none"
            },
            {
                "catalog_id": "fixture:market-second",
                "copies": 1,
                "zone": "market",
                "owner": "none"
            },
            {
                "catalog_id": "fixture:deck-first",
                "copies": 1,
                "zone": "hogwarts_deck",
                "owner": "none"
            },
            {
                "catalog_id": "fixture:villain",
                "copies": 1,
                "zone": "active_villains",
                "owner": "none"
            },
            {
                "catalog_id": "fixture:entry-014",
                "copies": 1,
                "zone": "villain_deck",
                "owner": "none"
            }
        ]
    }])
}

fn import_playable_candidate(candidate: &Value, executable_rule_ids: &[&str]) -> ContentManifest {
    let bundle = serde_json::to_vec(candidate).expect("the playable fixture must serialize");

    import_base_bundle_with_runtime_rules(
        &bundle,
        &[ProvenanceSource {
            id: "fixture-source".to_owned(),
            uri: "https://example.invalid/fixture".to_owned(),
            kind: SourceKind::Adaptation,
        }],
        &executable_rule_ids
            .iter()
            .map(|rule_id| RuleId::parse(rule_id).expect("fixture rule ID should be valid"))
            .collect(),
    )
    .expect("the playable fixture must import")
}

fn playable_fixture_entry(index: usize) -> Value {
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

async fn database() -> PgPool {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available")
}

async fn test_app(manifest: ContentManifest) -> (axum::Router, PgPool, AppState) {
    let database = database().await;
    let state = AppState::with_content_manifests(database.clone(), vec![manifest]);
    initialize(&state)
        .await
        .expect("database initialization must succeed");
    (build_router(state.clone()), database, state)
}

fn unique_key(_prefix: &str) -> String {
    uuid::Uuid::new_v4().to_string()
}

fn assert_database_error_code(error: &sqlx::Error, expected: &str) {
    let actual = error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned);
    assert_eq!(actual.as_deref(), Some(expected));
}

async fn assert_history_prevents_game_deletion(room: &ReadyRoom) {
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the protected game deletion transaction must start");
    sqlx::query(
        r"
        DELETE FROM game_start_requests
        WHERE game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        ",
    )
    .bind(&room.room_code)
    .execute(&mut *transaction)
    .await
    .expect("the older start-request reference must be removed inside the test transaction");
    let error = sqlx::query(
        r"
        DELETE FROM games
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(&room.room_code)
    .execute(&mut *transaction)
    .await
    .expect_err("the append-only event or receipt must prevent deleting its game");
    assert_database_error_code(&error, "23503");
    transaction
        .rollback()
        .await
        .expect("the protected game deletion transaction must roll back");
}

async fn insert_test_event(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    game_id: uuid::Uuid,
    room_id: uuid::Uuid,
    command_id: uuid::Uuid,
    actor_id: uuid::Uuid,
) {
    let (actor_position, prng_counter) = sqlx::query_as::<_, (i16, i64)>(
        r"
        SELECT participants.position, games.prng_counter
        FROM participants
        JOIN games ON games.room_id = participants.room_id
        WHERE participants.id = $1
          AND games.id = $2
        ",
    )
    .bind(actor_id)
    .bind(game_id)
    .fetch_one(&mut **transaction)
    .await
    .expect("the integrity event actor and random counter must be queryable");
    let payload = test_turn_completed_payload(
        1,
        2,
        u8::try_from(actor_position).expect("the actor position must fit in u8"),
        u64::try_from(prng_counter).expect("the random counter must be non-negative"),
    );
    sqlx::query(
        r"
        UPDATE games
        SET state_version = ($2 ->> 'state_version')::BIGINT,
            sequence = ($2 ->> 'sequence')::BIGINT,
            snapshot = snapshot || jsonb_build_object(
                'snapshot_version', 4,
                'state_version', $2 -> 'state_version',
                'sequence', $2 -> 'sequence',
                'turn', jsonb_build_object(
                    'number', $2 -> 'control' -> 'turn',
                    'phase', $2 -> 'control' -> 'phase',
                    'active_position', $2 -> 'control' -> 'active_position'
                ),
                'queued_phases', $2 -> 'control' -> 'queued_phases',
                'queued_effects', $2 -> 'control' -> 'queued_effects',
                'decision_point', $2 -> 'control' -> 'decision_point',
                'last_turn_steps', $2 -> 'steps',
                'effects', (snapshot -> 'effects') || jsonb_build_object(
                    'outcomes', '[]'::jsonb,
                    'choice', 'null'::jsonb
                )
            )
        WHERE id = $1
        ",
    )
    .bind(game_id)
    .bind(&payload)
    .execute(&mut **transaction)
    .await
    .expect("the integrity fixture snapshot must match the event control envelope");
    sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_version,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES (
            $1,
            $2,
            1,
            5,
            'turn_completed',
            $3,
            $4,
            2,
            $5
        )
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(command_id)
    .bind(actor_id)
    .bind(payload)
    .execute(&mut **transaction)
    .await
    .expect("the official event must be inserted for the integrity test");
}

async fn assert_v3_choice_payload_rejected(room: &ReadyRoom, payload: Value, description: &str) {
    let accepted = sqlx::query_scalar::<_, bool>(
        r"
        SELECT valid_legacy_game_event_for_replay(
            3::SMALLINT,
            'dark_arts_completed',
            $1,
            1::BIGINT,
            2::BIGINT,
            1::SMALLINT
        )
        ",
    )
    .bind(payload)
    .fetch_one(&room.database)
    .await
    .unwrap_or_else(|error| panic!("the v3 replay validator must evaluate {description}: {error}"));
    assert!(!accepted, "the v3 replay validator accepted {description}");
}

fn test_turn_completed_payload(
    sequence: u64,
    state_version: u64,
    actor_position: u8,
    prng_counter: u64,
) -> Value {
    json!({
        "event_version": 5,
        "type": "turn_completed",
        "sequence": sequence,
        "state_version": state_version,
        "turn": 1,
        "actor_position": actor_position,
        "end_turn": [
            { "type": "resource_reset", "resource": "attack", "before": 0 },
            { "type": "resource_reset", "resource": "influence", "before": 0 }
        ],
        "steps": [
            { "phase": "end_turn", "effects": [] },
            { "phase": "dark_arts", "effects": [] },
            { "phase": "villains", "effects": [] }
        ],
        "control": {
            "status": "in_progress",
            "turn": 2,
            "phase": "hero_actions",
            "active_position": 2,
            "queued_phases": ["end_turn"],
            "queued_effects": [],
            "decision_point": {
                "type": "player_intent",
                "responsible_position": 2
            }
        },
        "prng_counter": prng_counter
    })
}

async fn insert_cloned_game_snapshot(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source_room_code: &str,
    target_room_code: &str,
    snapshot: &Value,
) -> Result<(), sqlx::Error> {
    let serialized = serde_json::to_string(snapshot).expect("the cloned snapshot must serialize");
    let state_digest = format!("blake3:{}", blake3::hash(serialized.as_bytes()).to_hex());
    sqlx::query(
        r"
        INSERT INTO games (
            id,
            room_id,
            started_by_participant_id,
            status,
            adventure_id,
            adventure_name,
            manifest_digest,
            manifest_version,
            content_version,
            ruleset_version,
            snapshot_version,
            state_version,
            sequence,
            state_digest,
            snapshot,
            prng_algorithm,
            prng_seed,
            prng_counter,
            shuffle_algorithm,
            sampling_algorithm
        )
        SELECT
            $3,
            target_room.id,
            target_host.id,
            $5 ->> 'status',
            source.adventure_id,
            source.adventure_name,
            source.manifest_digest,
            source.manifest_version,
            source.content_version,
            source.ruleset_version,
            ($5 ->> 'snapshot_version')::SMALLINT,
            ($5 ->> 'state_version')::BIGINT,
            ($5 ->> 'sequence')::BIGINT,
            $4,
            $5,
            source.prng_algorithm,
            source.prng_seed,
            ($5 -> 'prng' ->> 'counter')::BIGINT,
            source.shuffle_algorithm,
            source.sampling_algorithm
        FROM games AS source
        JOIN rooms AS source_room ON source_room.id = source.room_id
        JOIN rooms AS target_room ON target_room.code = $2
        JOIN participants AS target_host
          ON target_host.room_id = target_room.id
         AND target_host.role = 'host'
        WHERE source_room.code = $1
        ",
    )
    .bind(source_room_code)
    .bind(target_room_code)
    .bind(uuid::Uuid::new_v4())
    .bind(state_digest)
    .bind(snapshot)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
}

async fn advance_cloned_game_snapshot(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    target_room_code: &str,
    snapshot: &Value,
) {
    let serialized = serde_json::to_string(snapshot).expect("the cloned snapshot must serialize");
    let state_digest = format!("blake3:{}", blake3::hash(serialized.as_bytes()).to_hex());
    sqlx::query(
        r"
        UPDATE games
        SET status = $2 ->> 'status',
            snapshot_version = ($2 ->> 'snapshot_version')::SMALLINT,
            state_version = ($2 ->> 'state_version')::BIGINT,
            sequence = ($2 ->> 'sequence')::BIGINT,
            state_digest = $3,
            snapshot = $2,
            prng_counter = ($2 -> 'prng' ->> 'counter')::BIGINT
        FROM rooms
        WHERE games.room_id = rooms.id
          AND rooms.code = $1
        ",
    )
    .bind(target_room_code)
    .bind(snapshot)
    .bind(state_digest)
    .execute(&mut **transaction)
    .await
    .expect("the cloned game must advance to the committed source snapshot");
}

struct V4EventFixture {
    initial_snapshot: Value,
    snapshot: Value,
    event_version: i16,
    event_type: String,
    sequence: i64,
    state_version: i64,
    payload: Value,
}

struct ClonedEventContext {
    game: uuid::Uuid,
    room: uuid::Uuid,
    actor: uuid::Uuid,
}

async fn v4_event_fixture_for_target(
    source: &ReadyRoom,
    target: &ReadyRoom,
    source_initial_snapshot: &Value,
) -> V4EventFixture {
    let (mut snapshot, event_version, event_type, sequence, state_version, payload) =
        sqlx::query_as::<_, (Value, i16, String, i64, i64, Value)>(
            r"
            SELECT
                games.snapshot,
                events.event_version,
                events.event_type,
                events.sequence,
                events.state_version,
                events.payload
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            JOIN game_events AS events ON events.game_id = games.id
            WHERE rooms.code = $1
            ORDER BY events.sequence DESC
            LIMIT 1
            ",
        )
        .bind(&source.room_code)
        .fetch_one(&source.database)
        .await
        .expect("the source v4 event and committed snapshot must be queryable");
    let target_participants = sqlx::query_scalar::<_, Value>(
        r"
        SELECT jsonb_agg(
            jsonb_build_object(
                'participant_id', participants.id::TEXT,
                'position', participants.position,
                'hero_id', participants.hero_id
            )
            ORDER BY participants.position
        )
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&target.room_code)
    .fetch_one(&target.database)
    .await
    .expect("the target participants must be queryable");
    let mut initial_snapshot = source_initial_snapshot.clone();
    initial_snapshot["participants"] = target_participants.clone();
    snapshot["participants"] = target_participants;

    V4EventFixture {
        initial_snapshot,
        snapshot,
        event_version,
        event_type,
        sequence,
        state_version,
        payload,
    }
}

async fn insert_cloned_state_anchor(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    target_room_code: &str,
) {
    sqlx::query(
        r"
        INSERT INTO game_state_anchors (game_id, sequence, snapshot_version, state_digest)
        SELECT games.id, games.sequence, games.snapshot_version, games.state_digest
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(target_room_code)
    .execute(&mut **transaction)
    .await
    .expect("the cloned committed state must have a matching replay anchor");
}

async fn cloned_event_context(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    target_room_code: &str,
    payload: &Value,
) -> ClonedEventContext {
    let (game_id, room_id, actor_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
            SELECT games.id, games.room_id, participants.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            JOIN participants
              ON participants.room_id = rooms.id
             AND participants.position = ($2 ->> 'actor_position')::SMALLINT
            WHERE rooms.code = $1
            ",
    )
    .bind(target_room_code)
    .bind(payload)
    .fetch_one(&mut **transaction)
    .await
    .expect("the cloned event actor must be queryable");
    ClonedEventContext {
        game: game_id,
        room: room_id,
        actor: actor_id,
    }
}

async fn insert_v4_event_for_pairing(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    context: &ClonedEventContext,
    fixture: &V4EventFixture,
    command_id: uuid::Uuid,
) {
    sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_version,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        ",
    )
    .bind(context.game)
    .bind(context.room)
    .bind(fixture.sequence)
    .bind(fixture.event_version)
    .bind(&fixture.event_type)
    .bind(command_id)
    .bind(context.actor)
    .bind(fixture.state_version)
    .bind(&fixture.payload)
    .execute(&mut **transaction)
    .await
    .expect("the individually valid v4 event must reach the deferred pairing guard");
}

async fn insert_wrong_command_receipt(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    context: &ClonedEventContext,
    fixture: &V4EventFixture,
    command_id: uuid::Uuid,
    wrong_command_type: &str,
) {
    sqlx::query(
        r"
        INSERT INTO game_command_receipts (
            game_id,
            room_id,
            command_id,
            actor_participant_id,
            command_type,
            expected_state_version,
            payload_digest,
            accepted_state_version,
            accepted_sequence,
            expires_at
        )
        SELECT
            $1,
            $2,
            $3,
            $4,
            $5,
            $6 - 1,
            'blake3:0000000000000000000000000000000000000000000000000000000000000000',
            $6,
            $7,
            games.expires_at
        FROM games
        WHERE games.id = $1
        ",
    )
    .bind(context.game)
    .bind(context.room)
    .bind(command_id)
    .bind(context.actor)
    .bind(wrong_command_type)
    .bind(fixture.state_version)
    .bind(fixture.sequence)
    .execute(&mut **transaction)
    .await
    .expect("the individually valid receipt must reach the deferred pairing guard");
}

async fn current_game_snapshot(room: &ReadyRoom) -> Value {
    sqlx::query_scalar::<_, Value>(
        r"
        SELECT games.snapshot
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the current game snapshot must be queryable")
}

async fn assert_v4_event_rejects_wrong_command_type(
    source: &ReadyRoom,
    target: &ReadyRoom,
    source_initial_snapshot: &Value,
    wrong_command_type: &str,
) {
    let fixture = v4_event_fixture_for_target(source, target, source_initial_snapshot).await;
    let mut transaction = target
        .database
        .begin()
        .await
        .expect("the event-command pairing transaction must start");
    insert_cloned_game_snapshot(
        &mut transaction,
        &source.room_code,
        &target.room_code,
        &fixture.initial_snapshot,
    )
    .await
    .expect("the source initial snapshot must be cloneable into the target room");
    insert_cloned_state_anchor(&mut transaction, &target.room_code).await;
    advance_cloned_game_snapshot(&mut transaction, &target.room_code, &fixture.snapshot).await;
    insert_cloned_state_anchor(&mut transaction, &target.room_code).await;
    let context = cloned_event_context(&mut transaction, &target.room_code, &fixture.payload).await;
    let command_id = uuid::Uuid::new_v4();
    insert_v4_event_for_pairing(&mut transaction, &context, &fixture, command_id).await;
    insert_wrong_command_receipt(
        &mut transaction,
        &context,
        &fixture,
        command_id,
        wrong_command_type,
    )
    .await;

    let error = transaction
        .commit()
        .await
        .expect_err("a v4 event cannot commit with the other command type");
    assert_database_error_code(&error, "23514");
    assert!(
        error.to_string().contains("receipt"),
        "unexpected event-command pairing error: {error}"
    );
}

fn json_request(
    method: &str,
    uri: &str,
    body: &Value,
    cookie: Option<&str>,
    idempotency_key: Option<&str>,
) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    if let Some(key) = idempotency_key {
        request = request.header("idempotency-key", key);
    }
    request
        .body(Body::from(body.to_string()))
        .expect("the request must be valid")
}

async fn response_json(response: Response<Body>) -> Value {
    let bytes = to_bytes(response.into_body(), 128 * 1024)
        .await
        .expect("the response body must be readable");
    serde_json::from_slice(&bytes).expect("the response body must be JSON")
}

fn session_cookie(response: &Response<Body>) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .expect("the response must establish a session")
        .to_str()
        .expect("the cookie must be ASCII")
        .split(';')
        .next()
        .expect("the cookie must contain a value")
        .to_owned()
}

async fn create_room(app: &axum::Router) -> (String, String, String) {
    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/rooms",
            &json!({
                "display_name": "Minerva",
                "recovery_password": "a long uncommon passphrase"
            }),
            None,
            Some(&unique_key("start-create")),
        ))
        .await
        .expect("room creation must receive a response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let cookie = session_cookie(&response);
    let body = response_json(response).await;
    (
        body["room"]["code"]
            .as_str()
            .expect("the room code must be present")
            .to_owned(),
        cookie,
        body["recovery_token"]
            .as_str()
            .expect("the host recovery token must be present")
            .to_owned(),
    )
}

async fn join_room(app: &axum::Router, room_code: &str) -> (String, String) {
    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/api/rooms/{room_code}/participants"),
            &json!({ "display_name": "Luna", "hero_id": "hermione" }),
            None,
            Some(&unique_key("start-join")),
        ))
        .await
        .expect("room join must receive a response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let cookie = session_cookie(&response);
    let body = response_json(response).await;
    (
        cookie,
        body["recovery_token"]
            .as_str()
            .expect("the guest recovery token must be present")
            .to_owned(),
    )
}

async fn select_hero(app: &axum::Router, cookie: &str, hero_id: &str) -> Response<Body> {
    app.clone()
        .oneshot(json_request(
            "PUT",
            "/api/session/hero",
            &json!({ "hero_id": hero_id }),
            Some(cookie),
            None,
        ))
        .await
        .expect("hero selection must receive a response")
}

async fn set_ready(app: &axum::Router, cookie: &str, ready: bool) -> Response<Body> {
    app.clone()
        .oneshot(json_request(
            "PUT",
            "/api/session/readiness",
            &json!({ "ready": ready }),
            Some(cookie),
            None,
        ))
        .await
        .expect("readiness must receive a response")
}

fn start_request(
    cookie: &str,
    key: &str,
    manifest: &ContentManifest,
    adventure_id: &str,
) -> Request<Body> {
    json_request(
        "POST",
        "/api/games",
        &json!({
            "adventure_id": adventure_id,
            "manifest_digest": manifest.digest,
            "ruleset_version": manifest.ruleset_version
        }),
        Some(cookie),
        Some(key),
    )
}

fn command_request(
    cookie: &str,
    command_id: uuid::Uuid,
    expected_state_version: u64,
) -> Request<Body> {
    json_request(
        "POST",
        "/api/games/current/commands",
        &json!({
            "command_id": command_id.to_string(),
            "expected_state_version": expected_state_version,
            "type": "end_hero_actions"
        }),
        Some(cookie),
        None,
    )
}

fn list_device_sessions_request(cookie: &str) -> Request<Body> {
    Request::builder()
        .uri("/api/session/device-sessions")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("the device session list request must be valid")
}

fn revoke_device_session_request(
    cookie: &str,
    session_id: &str,
    idempotency_key: &str,
) -> Request<Body> {
    json_request(
        "PUT",
        &format!("/api/session/device-sessions/{session_id}/revocation"),
        &json!({}),
        Some(cookie),
        Some(idempotency_key),
    )
}

fn resolve_choice_request(
    cookie: &str,
    command_id: uuid::Uuid,
    expected_state_version: u64,
    choice_id: &str,
    selected_options: &[&str],
) -> Request<Body> {
    json_request(
        "POST",
        "/api/games/current/commands",
        &json!({
            "command_id": command_id.to_string(),
            "expected_state_version": expected_state_version,
            "type": "resolve_choice",
            "choice_id": choice_id,
            "selected_options": selected_options
        }),
        Some(cookie),
        None,
    )
}

async fn command_result(
    app: &axum::Router,
    cookie: &str,
    command_id: uuid::Uuid,
) -> Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/games/current/commands/{command_id}"))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("the command result request must be valid"),
        )
        .await
        .expect("the command result lookup must receive a response")
}

async fn start_ready_game(room: &ReadyRoom, key_prefix: &str) -> Value {
    let response = room
        .app
        .clone()
        .oneshot(start_request(
            &room.host_cookie,
            &unique_key(key_prefix),
            &room.manifest,
            "adventure:001",
        ))
        .await
        .expect("game start must receive a response");
    let status = response.status();
    let body = response_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "response body: {body}");
    body
}

async fn authoritative_command_state(room: &ReadyRoom) -> (i64, i64, i64, i64, String) {
    sqlx::query_as::<_, (i64, i64, i64, i64, String)>(
        r"
        SELECT
            games.state_version,
            games.sequence,
            (SELECT count(*) FROM game_events WHERE game_events.game_id = games.id),
            (SELECT count(*) FROM game_command_receipts WHERE game_command_receipts.game_id = games.id),
            games.expires_at::text
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the authoritative command state must be queryable")
}

async fn assert_choice_codec_versions(room: &ReadyRoom) {
    let versions = sqlx::query_as::<_, (i16, i16, i16)>(
        r"
        SELECT
            games.snapshot_version,
            first_event.event_version,
            second_event.event_version
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        JOIN game_events AS first_event
          ON first_event.game_id = games.id
         AND first_event.sequence = 1
        JOIN game_events AS second_event
          ON second_event.game_id = games.id
         AND second_event.sequence = 2
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the v3 Snapshot and v4 choice events must be queryable");
    assert_eq!(versions, (4, 5, 5));
}

async fn assert_winning_choice_artifacts(room: &ReadyRoom, winning_command_id: uuid::Uuid) {
    let stored = sqlx::query_as::<_, (i64, i64, i64, i64, uuid::Uuid, String, String)>(
        r"
        SELECT
            games.state_version,
            games.sequence,
            (SELECT count(*) FROM game_events WHERE game_id = games.id),
            (SELECT count(*) FROM game_command_receipts WHERE game_id = games.id),
            events.command_id,
            events.event_type,
            receipts.command_type
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        JOIN game_events AS events
          ON events.game_id = games.id
         AND events.sequence = 1
        JOIN game_command_receipts AS receipts
          ON receipts.game_id = events.game_id
         AND receipts.command_id = events.command_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the winning choice artifacts must remain paired");
    assert_eq!((stored.0, stored.1, stored.2, stored.3), (2, 1, 1, 1));
    assert_eq!(stored.4, winning_command_id);
    assert_eq!(stored.5, "choice_resolved");
    assert_eq!(stored.6, "resolve_choice");
}

fn realtime_path(projection: &Value) -> String {
    format!(
        "/api/games/current/events?cursor={}&snapshot_version={}&digest={}",
        projection["snapshot"]["cursor"]
            .as_u64()
            .expect("the projection cursor must be present"),
        projection["snapshot"]["snapshot_version"]
            .as_u64()
            .expect("the Snapshot version must be present"),
        projection["snapshot"]["digest"]
            .as_str()
            .expect("the projection digest must be present")
    )
}

async fn ready_room() -> ReadyRoom {
    let manifest = playable_manifest();
    ready_room_with_manifest(manifest).await
}

async fn ready_room_with_manifest(manifest: ContentManifest) -> ReadyRoom {
    let (app, database, state) = test_app(manifest.clone()).await;
    let (room_code, host_cookie, host_recovery_token) = create_room(&app).await;
    assert_eq!(
        select_hero(&app, &host_cookie, "harry").await.status(),
        StatusCode::OK
    );
    let (guest_cookie, guest_recovery_token) = join_room(&app, &room_code).await;
    assert_eq!(
        set_ready(&app, &host_cookie, true).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        set_ready(&app, &guest_cookie, true).await.status(),
        StatusCode::OK
    );

    ReadyRoom {
        app,
        state,
        database,
        room_code,
        host_cookie,
        host_recovery_token,
        guest_cookie,
        guest_recovery_token,
        manifest,
    }
}

#[tokio::test]
async fn routine_recovery_rotation_and_regeneration_do_not_renew_game_retention() {
    let room = ready_room().await;
    start_ready_game(&room, "recovery-management-retention").await;
    let before = sqlx::query_as::<_, (i64, i64, String, String, i64)>(
        r"
        SELECT
            games.state_version,
            games.sequence,
            games.last_game_action_at::text,
            games.expires_at::text,
            (SELECT COUNT(*) FROM game_events WHERE game_events.game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the initial retention state must be queryable");

    let rotated = room
        .app
        .clone()
        .oneshot(json_request(
            "PUT",
            "/api/session/recovery-password",
            &json!({
                "current_recovery_password": "a long uncommon passphrase",
                "new_recovery_password": "a newer uncommon recovery phrase"
            }),
            Some(&room.host_cookie),
            Some(&unique_key("retention-rotation")),
        ))
        .await
        .expect("password rotation must receive a response");
    assert_eq!(rotated.status(), StatusCode::OK);

    let regenerated = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recovery-credential",
            &json!({}),
            Some(&room.guest_cookie),
            Some(&unique_key("retention-regeneration")),
        ))
        .await
        .expect("recovery credential regeneration must receive a response");
    assert_eq!(regenerated.status(), StatusCode::OK);

    let after = sqlx::query_as::<_, (i64, i64, String, String, i64)>(
        r"
        SELECT
            games.state_version,
            games.sequence,
            games.last_game_action_at::text,
            games.expires_at::text,
            (SELECT COUNT(*) FROM game_events WHERE game_events.game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the final retention state must be queryable");
    assert_eq!(after, before);
}

fn assert_initial_synchronization_projection(projection: &Value) {
    assert_eq!(projection["snapshot"]["snapshot_version"], 4);
    assert_eq!(projection["snapshot"]["state_version"], 1);
    assert_eq!(projection["snapshot"]["sequence"], 0);
    assert_eq!(projection["snapshot"]["cursor"], 0);
    assert_eq!(projection["queued_phases"], json!(["end_turn"]));
    assert_eq!(projection["queued_effect_count"], 0);
    assert!(projection.get("queued_effects").is_none());
    assert!(projection["turn"].get("queued_phases").is_none());
    assert!(projection["turn"].get("queued_effect_count").is_none());
    assert_eq!(projection["legal_actions"], json!(["end_hero_actions"]));
    assert_eq!(projection["choice"], json!({ "status": "none" }));
    assert_eq!(projection["effects"]["status"], "resolved");
    assert_eq!(
        projection["effects"]["outcomes"].as_array().map(Vec::len),
        Some(7)
    );
    assert_eq!(
        projection["participant"]["resources"],
        json!({ "health": 9, "attack": 2, "influence": 2 })
    );
}

async fn expire_game(database: &PgPool, room_code: &str) {
    sqlx::query(
        r"
        UPDATE games
        SET
            last_game_action_at = clock_timestamp() - INTERVAL '2 days',
            expires_at = clock_timestamp() - INTERVAL '1 day'
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(room_code)
    .execute(database)
    .await
    .expect("the fixture game must be expired");
}

#[tokio::test]
async fn host_seals_a_ready_room_and_every_participant_gets_a_redacted_initial_projection() {
    let room = ready_room().await;
    let response = room
        .app
        .clone()
        .oneshot(start_request(
            &room.host_cookie,
            &unique_key("start-game"),
            &room.manifest,
            "adventure:001",
        ))
        .await
        .expect("game start must receive a response");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    let host_projection = response_json(response).await;
    assert_eq!(host_projection["game"]["status"], "in_progress");
    assert_eq!(host_projection["game"]["adventure"]["id"], "adventure:001");
    assert_initial_synchronization_projection(&host_projection);
    assert_eq!(
        host_projection["snapshot"]["versions"]["prng"],
        "chacha20-v1"
    );
    assert_eq!(
        host_projection["snapshot"]["versions"]["manifest_digest"],
        room.manifest.digest
    );
    assert_eq!(host_projection["turn"]["number"], 1);
    assert_eq!(host_projection["turn"]["phase"], "hero_actions");
    assert_eq!(host_projection["participant"]["display_name"], "Minerva");
    assert_eq!(
        host_projection["participants"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(!host_projection.to_string().contains("seed"));

    let guest_response = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, &room.guest_cookie)
                .body(Body::empty())
                .expect("session restoration request must be valid"),
        )
        .await
        .expect("guest projection must receive a response");
    assert_eq!(guest_response.status(), StatusCode::OK);
    let guest_projection = response_json(guest_response).await;
    assert_eq!(
        guest_projection["game"]["id"],
        host_projection["game"]["id"]
    );
    assert_eq!(
        guest_projection["snapshot"]["digest"],
        host_projection["snapshot"]["digest"]
    );
    assert_eq!(guest_projection["participant"]["display_name"], "Luna");
    assert_eq!(guest_projection["legal_actions"], json!([]));
    assert!(!guest_projection.to_string().contains("seed"));

    let stored = sqlx::query_as::<_, (String, String, i32, i64, i64, i64, String, String)>(
        r"
        SELECT
            rooms.status,
            games.prng_algorithm,
            octet_length(games.prng_seed),
            games.state_version,
            games.sequence,
            games.prng_counter,
            games.state_digest,
            games.snapshot::text
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the committed game must be queryable");
    assert_eq!(stored.0, "sealed");
    assert_eq!(stored.1, "chacha20-v1");
    assert_eq!(stored.2, 32);
    assert_eq!(stored.3, 1);
    assert_eq!(stored.4, 0);
    assert_eq!(stored.5, 1);
    assert!(stored.6.starts_with("blake3:"));
    let snapshot: Value =
        serde_json::from_str(&stored.7).expect("the persisted Snapshot must be JSON");
    assert_eq!(snapshot["snapshot_version"], 4);
    assert_eq!(
        snapshot["versions"]["manifest_digest"],
        room.manifest.digest
    );
    assert!(!stored.7.contains("seed"));
}

#[tokio::test]
async fn recovery_restores_the_same_game_snapshot_without_renewing_retention() {
    let room = ready_room().await;
    let initial = start_ready_game(&room, "recover-started-game").await;
    let expires_before = sqlx::query_scalar::<_, String>(
        r"
        SELECT games.expires_at::text
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the initial game expiration must be queryable");

    let recovered_response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &json!({
                "recovery_token": room.host_recovery_token,
                "recovery_password": "a long uncommon passphrase",
                "recovery_attempt_id": uuid::Uuid::new_v4().to_string()
            }),
            None,
            None,
        ))
        .await
        .expect("participant recovery must receive a response");
    assert_eq!(recovered_response.status(), StatusCode::OK);
    assert!(
        recovered_response
            .headers()
            .get(header::SET_COOKIE)
            .is_some()
    );
    let recovered = response_json(recovered_response).await;

    assert_eq!(recovered["kind"], "game");
    assert_eq!(recovered["game"]["participant"], initial["participant"]);
    assert_eq!(recovered["game"]["participants"], initial["participants"]);
    assert_eq!(recovered["game"]["snapshot"], initial["snapshot"]);
    assert_eq!(recovered["game"]["game"], initial["game"]);
    assert!(recovered["recovery_token"].as_str().is_some_and(|token| {
        token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
    }));

    let (expires_after, active_sessions, consumed_credentials, active_credentials) =
        recovered_participation_state(&room).await;
    assert_eq!(expires_after, expires_before);
    assert_eq!(active_sessions, 2);
    assert_eq!(consumed_credentials, 1);
    assert_eq!(active_credentials, 1);

    expire_game(&room.database, &room.room_code).await;

    let expired_recovery = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &json!({
                "recovery_token": room.guest_recovery_token,
                "recovery_password": "a long uncommon passphrase",
                "recovery_attempt_id": uuid::Uuid::new_v4().to_string()
            }),
            None,
            None,
        ))
        .await
        .expect("expired participant recovery must receive a response");
    assert_eq!(expired_recovery.status(), StatusCode::UNAUTHORIZED);
    assert!(expired_recovery.headers().get(header::SET_COOKIE).is_none());
    assert_eq!(
        response_json(expired_recovery).await["error"]["code"],
        "RECOVERY_FAILED"
    );
}

async fn recovered_participation_state(room: &ReadyRoom) -> (String, i64, i64, i64) {
    sqlx::query_as(
        r"
        SELECT
            games.expires_at::text,
            (
                SELECT COUNT(*)
                FROM device_sessions
                JOIN guest_sessions
                  ON guest_sessions.id = device_sessions.guest_session_id
                WHERE device_sessions.participant_id = rooms.host_participant_id
                  AND device_sessions.status = 'active'
                  AND guest_sessions.expires_at > clock_timestamp()
            ),
            COUNT(*) FILTER (WHERE recovery_credentials.status = 'consumed'),
            COUNT(*) FILTER (WHERE recovery_credentials.status = 'active')
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        JOIN recovery_credentials
          ON recovery_credentials.participant_id = rooms.host_participant_id
        WHERE rooms.code = $1
        GROUP BY games.expires_at, rooms.host_participant_id
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the recovered session and retention must be queryable")
}

#[tokio::test]
async fn concurrent_identical_start_retries_create_exactly_one_game() {
    let room = ready_room().await;
    let key = unique_key("concurrent-start");
    let (first, second) = tokio::join!(
        room.app.clone().oneshot(start_request(
            &room.host_cookie,
            &key,
            &room.manifest,
            "adventure:001",
        )),
        room.app.clone().oneshot(start_request(
            &room.host_cookie,
            &key,
            &room.manifest,
            "adventure:001",
        )),
    );
    let first = first.expect("the first start must receive a response");
    let second = second.expect("the retry must receive a response");

    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(response_json(first).await, response_json(second).await);

    let game_count = sqlx::query_scalar::<_, i64>(
        r"
        SELECT COUNT(*)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the game count must be queryable");
    assert_eq!(game_count, 1);

    let conflicting = room
        .app
        .oneshot(start_request(
            &room.host_cookie,
            &key,
            &room.manifest,
            "adventure:002",
        ))
        .await
        .expect("the conflicting retry must receive a response");
    assert_eq!(conflicting.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(conflicting).await["error"]["code"],
        "IDEMPOTENCY_KEY_REUSED"
    );
}

#[tokio::test]
async fn start_validates_host_count_heroes_readiness_and_authorization() {
    let manifest = playable_manifest();
    let (app, _, _) = test_app(manifest.clone()).await;

    let (_, lone_host, _) = create_room(&app).await;
    assert_eq!(
        select_hero(&app, &lone_host, "harry").await.status(),
        StatusCode::OK
    );
    assert_eq!(
        set_ready(&app, &lone_host, true).await.status(),
        StatusCode::OK
    );
    let invalid_count = app
        .clone()
        .oneshot(start_request(
            &lone_host,
            &unique_key("invalid-count"),
            &manifest,
            "adventure:001",
        ))
        .await
        .expect("invalid participant count must receive a response");
    assert_eq!(invalid_count.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(invalid_count).await["error"]["code"],
        "ROOM_PARTICIPANT_COUNT_INVALID"
    );

    let (missing_hero_code, missing_hero_host, _) = create_room(&app).await;
    let _ = join_room(&app, &missing_hero_code).await;
    let missing_hero = app
        .clone()
        .oneshot(start_request(
            &missing_hero_host,
            &unique_key("missing-hero"),
            &manifest,
            "adventure:001",
        ))
        .await
        .expect("missing hero must receive a response");
    assert_eq!(missing_hero.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(missing_hero).await["error"]["code"],
        "PARTICIPANT_HEROES_INVALID"
    );

    let (not_ready_code, not_ready_host, _) = create_room(&app).await;
    assert_eq!(
        select_hero(&app, &not_ready_host, "harry").await.status(),
        StatusCode::OK
    );
    let (not_ready_guest, _) = join_room(&app, &not_ready_code).await;
    let not_ready = app
        .clone()
        .oneshot(start_request(
            &not_ready_host,
            &unique_key("not-ready"),
            &manifest,
            "adventure:001",
        ))
        .await
        .expect("unready participants must receive a response");
    assert_eq!(not_ready.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(not_ready).await["error"]["code"],
        "PARTICIPANTS_NOT_READY"
    );

    let unauthorized = app
        .clone()
        .oneshot(start_request(
            &not_ready_guest,
            &unique_key("guest-start"),
            &manifest,
            "adventure:001",
        ))
        .await
        .expect("guest start must receive a response");
    assert_eq!(unauthorized.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(unauthorized).await["error"]["code"],
        "NOT_ROOM_HOST"
    );
}

#[tokio::test]
async fn candidate_content_with_functional_gaps_cannot_start_a_game() {
    let candidate = import_base_bundle(include_bytes!(
        "../../../content/bundles/base-en-candidate-2026-09-02.json"
    ))
    .expect("the candidate bundle must import");
    let (candidate_app, _, _) = test_app(candidate.clone()).await;
    let (candidate_code, candidate_host, _) = create_room(&candidate_app).await;
    assert_eq!(
        select_hero(&candidate_app, &candidate_host, "harry")
            .await
            .status(),
        StatusCode::OK
    );
    let (candidate_guest, _) = join_room(&candidate_app, &candidate_code).await;
    assert_eq!(
        set_ready(&candidate_app, &candidate_host, true)
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        set_ready(&candidate_app, &candidate_guest, true)
            .await
            .status(),
        StatusCode::OK
    );
    let unplayable = candidate_app
        .oneshot(start_request(
            &candidate_host,
            &unique_key("unplayable"),
            &candidate,
            "adventure:001",
        ))
        .await
        .expect("unplayable content must receive a response");
    assert_eq!(unplayable.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response_json(unplayable).await["error"]["code"],
        "CONTENT_NOT_PLAYABLE"
    );
}

#[tokio::test]
async fn sealed_room_rejects_entry_hero_readiness_and_position_changes() {
    let room = ready_room().await;
    let started = room
        .app
        .clone()
        .oneshot(start_request(
            &room.host_cookie,
            &unique_key("seal-room"),
            &room.manifest,
            "adventure:001",
        ))
        .await
        .expect("game start must receive a response");
    assert_eq!(started.status(), StatusCode::CREATED);

    let late_join = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            &format!("/api/rooms/{}/participants", room.room_code),
            &json!({ "display_name": "Cedric", "hero_id": "neville" }),
            None,
            Some(&unique_key("late-join")),
        ))
        .await
        .expect("late join must receive a response");
    assert_eq!(late_join.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        response_json(late_join).await["error"]["code"],
        "ROOM_UNAVAILABLE"
    );

    for response in [
        select_hero(&room.app, &room.host_cookie, "neville").await,
        set_ready(&room.app, &room.host_cookie, false).await,
    ] {
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "ROOM_SEALED"
        );
    }

    let position_change = sqlx::query(
        r"
        UPDATE participants
        SET position = 3
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
          AND role = 'host'
        ",
    )
    .bind(&room.room_code)
    .execute(&room.database)
    .await
    .expect_err("a sealed position must be immutable");
    assert!(
        position_change
            .to_string()
            .contains("sealed room participants cannot change")
    );

    let participant_deletion = sqlx::query(
        r"
        DELETE FROM participants
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
          AND role = 'guest'
        ",
    )
    .bind(&room.room_code)
    .execute(&room.database)
    .await
    .expect_err("a sealed participant must not be removable");
    assert!(
        participant_deletion
            .to_string()
            .contains("sealed room participants cannot change")
    );

    let participant_count = sqlx::query_scalar::<_, i64>(
        r"
        SELECT COUNT(*)
        FROM participants
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the sealed room participant count must remain queryable");
    assert_eq!(participant_count, 2);
}

fn assert_committed_turn_event(snapshot: &Value, event: &Value) {
    assert_eq!(snapshot["state_version"], 2);
    assert_eq!(snapshot["sequence"], 1);
    assert_eq!(snapshot["turn"]["number"], 2);
    assert_eq!(snapshot["turn"]["phase"], "hero_actions");
    assert_eq!(snapshot["turn"]["active_position"], 2);
    assert_eq!(snapshot["prng"]["counter"], 2);
    assert_eq!(snapshot["last_turn_steps"], event["steps"]);
    assert_eq!(
        snapshot["effects"]["outcomes"],
        event["steps"][1]["effects"]
    );
    assert_eq!(event["sequence"], 1);
    assert_eq!(event["state_version"], 2);
    assert_eq!(event["event_version"], 5);
    assert_eq!(event["type"], "turn_completed");
    assert_eq!(event["turn"], 1);
    assert_eq!(event["actor_position"], 1);
    assert_eq!(event["prng_counter"], 2);
    assert_eq!(
        event["end_turn"],
        json!([
            { "type": "resource_reset", "resource": "attack", "before": 2 },
            { "type": "resource_reset", "resource": "influence", "before": 2 }
        ])
    );
    assert_eq!(
        event["steps"]
            .as_array()
            .expect("turn steps must be an array")
            .iter()
            .map(|step| step["phase"].as_str())
            .collect::<Vec<_>>(),
        vec![Some("end_turn"), Some("dark_arts"), Some("villains")]
    );
    assert!(
        event["steps"][1]["effects"]
            .as_array()
            .is_some_and(|effects| effects
                .iter()
                .all(|effect| effect["rule_id"] == "rule:functional"))
    );
    assert_eq!(event["steps"][2]["effects"], json!([]));
    assert_eq!(event["control"]["turn"], 2);
    assert_eq!(event["control"]["phase"], "hero_actions");
    assert_eq!(event["control"]["active_position"], 2);
    assert_eq!(event["control"]["queued_phases"], json!(["end_turn"]));
    assert_eq!(
        event["control"]["decision_point"],
        json!({ "type": "player_intent", "responsible_position": 2 })
    );
}

async fn assert_committed_command_artifacts(room: &ReadyRoom, initial_expiration: &str) {
    let stored = sqlx::query_as::<
        _,
        (
            i64,
            i64,
            i64,
            String,
            String,
            String,
            i64,
            i64,
            bool,
            bool,
            i16,
            i16,
        ),
    >(
        r"
            SELECT
                games.state_version,
                games.sequence,
                games.prng_counter,
                games.snapshot::text,
                events.event_type,
                events.payload::text,
                receipts.accepted_state_version,
                receipts.accepted_sequence,
                games.expires_at = receipts.expires_at,
                games.expires_at > $2::timestamptz,
                games.snapshot_version,
                events.event_version
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            JOIN game_events AS events ON events.game_id = games.id
            JOIN game_command_receipts AS receipts
              ON receipts.game_id = games.id
             AND receipts.command_id = events.command_id
            WHERE rooms.code = $1
            ",
    )
    .bind(&room.room_code)
    .bind(initial_expiration)
    .fetch_one(&room.database)
    .await
    .expect("every committed command artifact must be queryable together");
    assert_eq!(stored.0, 2);
    assert_eq!(stored.1, 1);
    assert_eq!(stored.2, 2);
    assert_eq!(stored.4, "turn_completed");
    assert_eq!(stored.6, 2);
    assert_eq!(stored.7, 1);
    assert!(stored.8, "the receipt and game must share one expiration");
    assert!(stored.9, "an accepted action must renew retention");
    assert_eq!(stored.10, 4);
    assert_eq!(stored.11, 5);
    let snapshot: Value = serde_json::from_str(&stored.3).expect("snapshot must be JSON");
    let event: Value = serde_json::from_str(&stored.5).expect("event must be JSON");
    assert_committed_turn_event(&snapshot, &event);

    let anchors = sqlx::query_as::<_, (i64, bool)>(
        r"
        SELECT
            COUNT(*),
            bool_and(
                anchors.snapshot_version = games.snapshot_version
                AND (
                    anchors.sequence < games.sequence
                    OR anchors.state_digest = games.state_digest
                )
            )
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        JOIN game_state_anchors AS anchors ON anchors.game_id = games.id
        WHERE rooms.code = $1
        GROUP BY games.id
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the immutable replay anchors must be queryable");
    assert_eq!(anchors, (2, true));
}

fn assert_host_projection_after_turn_handoff(accepted: &Value) {
    assert_eq!(accepted["projection"]["turn"]["number"], 2);
    assert_eq!(accepted["projection"]["turn"]["phase"], "hero_actions");
    assert_eq!(accepted["projection"]["turn"]["active_position"], 2);
    assert_eq!(accepted["projection"]["queued_phases"], json!(["end_turn"]));
    assert_eq!(accepted["projection"]["queued_effect_count"], 0);
    assert!(accepted["projection"].get("queued_effects").is_none());
    assert!(
        accepted["projection"]["turn"]
            .get("queued_phases")
            .is_none()
    );
    assert!(
        accepted["projection"]["turn"]
            .get("queued_effect_count")
            .is_none()
    );
    assert_eq!(accepted["projection"]["snapshot"]["state_version"], 2);
    assert_eq!(accepted["projection"]["snapshot"]["sequence"], 1);
    assert_eq!(accepted["projection"]["legal_actions"], json!([]));
    assert_eq!(
        accepted["projection"]["participant"]["resources"],
        json!({ "health": 9, "attack": 0, "influence": 0 })
    );
    assert_eq!(accepted["projection"]["effects"]["status"], "resolved");
    let outcomes = accepted["projection"]["effects"]["outcomes"]
        .as_array()
        .expect("effect outcomes must be an array");
    assert_eq!(outcomes.len(), 7);
    assert!(outcomes.iter().any(|outcome| {
        outcome["type"] == "die_rolled"
            && outcome["die"] == "d4"
            && (1..=4).contains(&outcome["result"].as_u64().unwrap_or_default())
    }));
    assert!(outcomes.iter().any(|outcome| {
        outcome["type"] == "no_op" && outcome["reason"] == "no_eligible_target"
    }));
}

#[tokio::test]
async fn active_player_ends_actions_and_commits_the_next_turn_after_automatic_phases() {
    let room = ready_room().await;
    let start_response = room
        .app
        .clone()
        .oneshot(start_request(
            &room.host_cookie,
            &unique_key("authoritative-start"),
            &room.manifest,
            "adventure:001",
        ))
        .await
        .expect("game start must receive a response");
    assert_eq!(start_response.status(), StatusCode::CREATED);
    let initial = response_json(start_response).await;
    assert_eq!(initial["turn"]["number"], 1);
    assert_eq!(initial["turn"]["phase"], "hero_actions");
    assert_eq!(initial["turn"]["active_position"], 1);
    assert_eq!(initial["legal_actions"], json!(["end_hero_actions"]));
    let initial_expiration = initial["game"]["expires_at"]
        .as_str()
        .expect("the initial expiration must be present")
        .to_owned();
    let command_id = uuid::Uuid::new_v4();

    let response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": command_id.to_string(),
                "expected_state_version": 1,
                "type": "end_hero_actions"
            }),
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the command must receive a response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    let accepted = response_json(response).await;
    assert_eq!(accepted["receipt"]["command_id"], command_id.to_string());
    assert_eq!(accepted["receipt"]["type"], "end_hero_actions");
    assert_eq!(accepted["receipt"]["status"], "accepted");
    assert_eq!(accepted["receipt"]["accepted_state_version"], 2);
    assert_eq!(accepted["receipt"]["accepted_sequence"], 1);
    assert_host_projection_after_turn_handoff(&accepted);

    let guest_response = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, &room.guest_cookie)
                .body(Body::empty())
                .expect("the guest projection request must be valid"),
        )
        .await
        .expect("the next active player must receive a projection");
    assert_eq!(guest_response.status(), StatusCode::OK);
    let guest = response_json(guest_response).await;
    assert_eq!(guest["turn"]["number"], 2);
    assert_eq!(guest["turn"]["phase"], "hero_actions");
    assert_eq!(guest["turn"]["active_position"], 2);
    assert_eq!(guest["legal_actions"], json!(["end_hero_actions"]));
    assert_eq!(
        guest["participant"]["resources"],
        json!({ "health": 9, "attack": 2, "influence": 2 })
    );

    assert_committed_command_artifacts(&room, &initial_expiration).await;

    let recovered = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/games/current/commands/{command_id}"))
                .header(header::COOKIE, &room.host_cookie)
                .body(Body::empty())
                .expect("the receipt lookup request must be valid"),
        )
        .await
        .expect("the committed receipt must remain recoverable");
    assert_eq!(recovered.status(), StatusCode::OK);
    let recovered = response_json(recovered).await;
    assert_eq!(recovered["receipt"], accepted["receipt"]);
    assert_eq!(
        recovered["projection"]["snapshot"],
        accepted["projection"]["snapshot"]
    );
}

fn assert_initial_each_hero_choice(started: &Value) -> String {
    assert_eq!(started["snapshot"]["snapshot_version"], 4);
    assert_eq!(started["snapshot"]["state_version"], 1);
    assert_eq!(started["snapshot"]["sequence"], 0);
    assert_eq!(started["turn"]["phase"], "dark_arts");
    assert_eq!(started["turn"]["active_position"], 1);
    assert_eq!(started["choice"]["status"], "pending");
    assert_eq!(started["choice"]["cause"], "rule:functional");
    assert_eq!(started["choice"]["responsible_position"], 1);
    assert_eq!(
        started["choice"]["options"],
        json!(["option:1", "option:2"])
    );
    assert_eq!(started["legal_actions"], json!(["resolve_choice"]));
    started["choice"]["id"]
        .as_str()
        .expect("the global choice ID must be present")
        .to_owned()
}

struct GameplayInstances {
    host_card: String,
    guest_card: String,
    market_card: String,
    second_market_card: String,
    villain: String,
}

fn projected_instance_id(projection: &Value, pointer: &str, expectation: &str) -> String {
    projection
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{expectation}"))
        .to_owned()
}

async fn inspect_initial_gameplay(room: &ReadyRoom, initial: &Value) -> GameplayInstances {
    let host_card_id = projected_instance_id(
        initial,
        "/table/hand/0/instance_id",
        "the host starter instance must be projected",
    );
    let market_card_id = projected_instance_id(
        initial,
        "/table/market/0/instance_id",
        "the first market instance must be projected",
    );
    let second_market_card_id = projected_instance_id(
        initial,
        "/table/market/1/instance_id",
        "the second market instance must be projected",
    );
    let villain_id = projected_instance_id(
        initial,
        "/table/active_villains/0/instance_id",
        "the active villain instance must be projected",
    );
    assert_eq!(initial["participant"]["hand_count"], 1);
    assert_eq!(initial["table"]["hogwarts_deck_count"], 1);
    assert_eq!(initial["table"]["market"][0]["affordable"], false);
    assert_eq!(initial["table"]["active_villains"][0]["attackable"], false);

    let guest_response = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, &room.guest_cookie)
                .body(Body::empty())
                .expect("the guest session request must be valid"),
        )
        .await
        .expect("the guest projection must receive a response");
    assert_eq!(guest_response.status(), StatusCode::OK);
    let guest = response_json(guest_response).await;
    let guest_card_id = projected_instance_id(
        &guest,
        "/table/hand/0/instance_id",
        "the guest starter instance must be projected",
    );
    assert_ne!(guest_card_id, host_card_id);
    assert!(!guest.to_string().contains(&host_card_id));

    GameplayInstances {
        host_card: host_card_id,
        guest_card: guest_card_id,
        market_card: market_card_id,
        second_market_card: second_market_card_id,
        villain: villain_id,
    }
}

fn assert_gameplay_hero_actions(initial: &Value, instances: &GameplayInstances) {
    assert_eq!(initial["turn"]["phase"], "hero_actions");
    assert_eq!(
        initial["participant"]["resources"],
        json!({ "health": 9, "attack": 0, "influence": 0 })
    );
    assert_eq!(
        initial["legal_intentions"]["play_cards"][0]["card_id"],
        instances.host_card
    );
}

async fn assert_foreign_card_is_rejected(room: &ReadyRoom, instances: &GameplayInstances) {
    let response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": uuid::Uuid::new_v4().to_string(),
                "expected_state_version": 1,
                "type": "play_card",
                "card_id": instances.guest_card,
                "targets": [{
                    "selector_id": "target:hero",
                    "target_ids": ["hero:1"]
                }]
            }),
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the foreign-card command must receive a response");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(response).await["error"]["code"],
        "GAME_ACTION_NOT_ALLOWED"
    );
    assert_eq!(current_official_state(room).await.0, 1);
}

async fn play_gameplay_starter_card(room: &ReadyRoom, instances: &GameplayInstances) {
    let response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": uuid::Uuid::new_v4().to_string(),
                "expected_state_version": 1,
                "type": "play_card",
                "card_id": instances.host_card,
                "targets": [{
                    "selector_id": "target:hero",
                    "target_ids": ["hero:1"]
                }]
            }),
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the play-card command must receive a response");
    assert_eq!(response.status(), StatusCode::OK);
    let played = response_json(response).await;
    assert_eq!(
        played["projection"]["participant"]["resources"],
        json!({ "health": 9, "attack": 2, "influence": 3 })
    );
    assert_eq!(played["projection"]["table"]["hand"], json!([]));
    assert_eq!(
        played["projection"]["table"]["play_area"][0]["instance_id"],
        instances.host_card
    );
    assert_eq!(
        played["projection"]["legal_intentions"]["assign_attack"][0],
        json!({ "villain_id": instances.villain, "max_amount": 2 })
    );
}

async fn assign_all_gameplay_attack(room: &ReadyRoom, instances: &GameplayInstances) {
    let response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": uuid::Uuid::new_v4().to_string(),
                "expected_state_version": 2,
                "type": "assign_attack",
                "villain_id": instances.villain,
                "amount": 2
            }),
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the attack command must receive a response");
    assert_eq!(response.status(), StatusCode::OK);
    let attacked = response_json(response).await;
    assert_eq!(
        attacked["projection"]["participant"]["resources"]["attack"],
        0
    );
    assert_eq!(
        attacked["projection"]["table"]["active_villains"],
        json!([])
    );
    assert_eq!(attacked["projection"]["table"]["villain_discard_count"], 1);
}

async fn acquire_card_and_assert_replay(room: &ReadyRoom, instances: &GameplayInstances) -> String {
    let acquire_body = json!({
        "command_id": uuid::Uuid::new_v4().to_string(),
        "expected_state_version": 3,
        "type": "acquire_card",
        "card_id": instances.market_card
    });
    let response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &acquire_body,
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the acquisition command must receive a response");
    assert_eq!(response.status(), StatusCode::OK);
    let acquired = response_json(response).await;
    assert_eq!(acquired["projection"]["snapshot"]["state_version"], 4);
    assert_eq!(
        acquired["projection"]["participant"]["resources"]["influence"],
        1
    );
    assert_eq!(acquired["projection"]["table"]["discard_pile_count"], 1);
    assert_eq!(acquired["projection"]["table"]["hogwarts_deck_count"], 0);
    assert_eq!(
        acquired["projection"]["table"]["market"][0]["instance_id"],
        instances.second_market_card
    );
    let refill_card_id = projected_instance_id(
        &acquired["projection"],
        "/table/market/1/instance_id",
        "the refill instance must be projected",
    );

    let replay_response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &acquire_body,
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the idempotent acquisition replay must receive a response");
    assert_eq!(replay_response.status(), StatusCode::OK);
    let replayed = response_json(replay_response).await;
    assert_eq!(replayed["receipt"], acquired["receipt"]);
    assert_eq!(
        replayed["projection"]["snapshot"],
        acquired["projection"]["snapshot"]
    );

    refill_card_id
}

fn instance_ids_in_zone(entities: &[Value], zone: &str) -> Vec<String> {
    entities
        .iter()
        .filter(|entity| entity["zone"] == zone)
        .map(|entity| {
            entity["id"]
                .as_str()
                .expect("every persisted entity must have an ID")
                .to_owned()
        })
        .collect()
}

async fn assert_persisted_gameplay_state(
    room: &ReadyRoom,
    instances: &GameplayInstances,
    refill_card_id: &str,
) {
    let stored = sqlx::query_as::<_, (i64, i64, String)>(
        r"
        SELECT games.state_version, games.sequence, games.snapshot::text
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the final game state must be persisted");
    assert_eq!((stored.0, stored.1), (4, 3));
    let snapshot: Value = serde_json::from_str(&stored.2).expect("the Snapshot must be JSON");
    let entities = snapshot["effects"]["entities"]
        .as_array()
        .expect("the persisted entities must be an array");
    assert_eq!(
        instance_ids_in_zone(entities, "hero_hand"),
        vec![instances.guest_card.clone()]
    );
    assert_eq!(
        instance_ids_in_zone(entities, "hero_play_area"),
        vec![instances.host_card.clone()]
    );
    assert_eq!(
        instance_ids_in_zone(entities, "hero_discard_pile"),
        vec![instances.market_card.clone()]
    );
    assert_eq!(
        instance_ids_in_zone(entities, "market"),
        vec![
            instances.second_market_card.clone(),
            refill_card_id.to_owned()
        ]
    );
    assert!(instance_ids_in_zone(entities, "hogwarts_deck").is_empty());
    assert_eq!(
        instance_ids_in_zone(entities, "active_villains"),
        Vec::<String>::new()
    );
    assert_eq!(
        instance_ids_in_zone(entities, "villain_discard"),
        vec![instances.villain.clone()]
    );
}

async fn assert_gameplay_command_history(
    room: &ReadyRoom,
    instances: &GameplayInstances,
    refill_card_id: &str,
) {
    let events = sqlx::query_as::<_, (String, String)>(
        r"
        SELECT event_type, payload::text
        FROM game_events
        WHERE game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        ORDER BY sequence
        ",
    )
    .bind(&room.room_code)
    .fetch_all(&room.database)
    .await
    .expect("the hero-action events must be persisted in order");
    assert_eq!(
        events
            .iter()
            .map(|(event_type, _)| event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["card_played", "attack_assigned", "card_acquired"]
    );
    let acquisition_event: Value =
        serde_json::from_str(&events[2].1).expect("the acquisition event must be JSON");
    assert_eq!(acquisition_event["event_version"], 5);
    assert_eq!(acquisition_event["card_id"], instances.market_card);
    assert_eq!(acquisition_event["cost"], 2);
    assert_eq!(acquisition_event["refill_card_id"], refill_card_id);

    let command_types = sqlx::query_scalar::<_, String>(
        r"
        SELECT receipts.command_type
        FROM game_events AS events
        JOIN game_command_receipts AS receipts
          ON receipts.game_id = events.game_id
         AND receipts.command_id = events.command_id
        WHERE events.game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        ORDER BY events.sequence
        ",
    )
    .bind(&room.room_code)
    .fetch_all(&room.database)
    .await
    .expect("each accepted command must have one receipt");
    assert_eq!(
        command_types,
        vec!["play_card", "assign_attack", "acquire_card"]
    );
}

#[tokio::test]
async fn hero_actions_move_exact_instances_spend_resources_refill_and_replay_idempotently() {
    let room = ready_room_with_manifest(gameplay_manifest()).await;
    let initial = start_ready_game(&room, "hero-actions-start").await;
    let instances = inspect_initial_gameplay(&room, &initial).await;

    assert_gameplay_hero_actions(&initial, &instances);
    assert_foreign_card_is_rejected(&room, &instances).await;
    play_gameplay_starter_card(&room, &instances).await;
    assign_all_gameplay_attack(&room, &instances).await;
    let refill_card_id = acquire_card_and_assert_replay(&room, &instances).await;
    assert_persisted_gameplay_state(&room, &instances, &refill_card_id).await;
    assert_gameplay_command_history(&room, &instances, &refill_card_id).await;
}

#[tokio::test]
async fn optional_named_card_target_can_be_empty_and_survive_receipt_recovery() {
    let room = ready_room_with_manifest(optional_target_manifest()).await;
    let initial = start_ready_game(&room, "optional-target-start").await;
    let instances = inspect_initial_gameplay(&room, &initial).await;
    assert_gameplay_hero_actions(&initial, &instances);

    let command_id = uuid::Uuid::new_v4();
    let response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": command_id.to_string(),
                "expected_state_version": 1,
                "type": "play_card",
                "card_id": instances.host_card,
                "targets": [{
                    "selector_id": "target:optional-hero",
                    "target_ids": []
                }]
            }),
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the optional-target card command must receive a response");

    assert_eq!(response.status(), StatusCode::OK);
    let accepted = response_json(response).await;
    assert_eq!(accepted["receipt"]["status"], "accepted");
    assert_eq!(accepted["projection"]["snapshot"]["state_version"], 2);
    assert_eq!(accepted["projection"]["table"]["hand"], json!([]));
    assert_eq!(
        accepted["projection"]["table"]["play_area"][0]["instance_id"],
        instances.host_card
    );

    let recovered = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/games/current/commands/{command_id}"))
                .header(header::COOKIE, &room.host_cookie)
                .body(Body::empty())
                .expect("the optional-target receipt lookup request must be valid"),
        )
        .await
        .expect("the optional-target receipt must remain recoverable");
    assert_eq!(recovered.status(), StatusCode::OK);
    let recovered = response_json(recovered).await;
    assert_eq!(recovered["receipt"], accepted["receipt"]);
    assert_eq!(
        recovered["projection"]["snapshot"],
        accepted["projection"]["snapshot"]
    );
}

async fn assert_unassigned_choice_rejected_without_artifacts(room: &ReadyRoom, choice_id: &str) {
    let before_rejection = authoritative_command_state(room).await;
    assert_eq!(
        (
            before_rejection.0,
            before_rejection.1,
            before_rejection.2,
            before_rejection.3,
        ),
        (1, 0, 0, 0)
    );

    let rejected_command_id = uuid::Uuid::new_v4();
    let rejected_response = room
        .app
        .clone()
        .oneshot(resolve_choice_request(
            &room.guest_cookie,
            rejected_command_id,
            1,
            choice_id,
            &["option:1"],
        ))
        .await
        .expect("the unassigned choice command must receive a response");
    assert_eq!(rejected_response.status(), StatusCode::FORBIDDEN);
    let rejected = response_json(rejected_response).await;
    assert_eq!(rejected["error"]["code"], "CHOICE_NOT_ASSIGNED");
    assert_eq!(rejected["error"]["category"], "authorization");
    assert_eq!(rejected["error"]["message_key"], "game.choice.not_assigned");
    assert_eq!(authoritative_command_state(room).await, before_rejection);

    let missing_receipt = command_result(&room.app, &room.guest_cookie, rejected_command_id).await;
    assert_eq!(missing_receipt.status(), StatusCode::NOT_FOUND);
}

async fn resolve_first_each_hero_choice(room: &ReadyRoom, choice_id: &str) -> String {
    let resolve_response = room
        .app
        .clone()
        .oneshot(resolve_choice_request(
            &room.host_cookie,
            uuid::Uuid::new_v4(),
            1,
            choice_id,
            &["option:1"],
        ))
        .await
        .expect("the assigned choice command must receive a response");
    assert_eq!(resolve_response.status(), StatusCode::OK);
    let resolved = response_json(resolve_response).await;
    assert_eq!(resolved["receipt"]["type"], "resolve_choice");
    assert_eq!(resolved["receipt"]["accepted_state_version"], 2);
    assert_eq!(resolved["receipt"]["accepted_sequence"], 1);
    assert_eq!(resolved["projection"]["snapshot"]["snapshot_version"], 4);
    assert_eq!(resolved["projection"]["snapshot"]["state_version"], 2);
    assert_eq!(resolved["projection"]["snapshot"]["sequence"], 1);
    assert_eq!(resolved["projection"]["turn"]["active_position"], 1);
    assert_eq!(resolved["projection"]["turn"]["phase"], "dark_arts");
    assert_eq!(resolved["projection"]["choice"]["status"], "pending");
    assert_eq!(resolved["projection"]["choice"]["cause"], "rule:functional");
    assert_eq!(resolved["projection"]["choice"]["responsible_position"], 2);
    assert_eq!(
        resolved["projection"]["choice"]["options"],
        json!(["option:1", "option:2"])
    );
    assert_eq!(resolved["projection"]["legal_actions"], json!([]));

    resolved["projection"]["choice"]["id"]
        .as_str()
        .expect("the second participant choice must be present")
        .to_owned()
}

async fn complete_second_each_hero_choice(room: &ReadyRoom, second_choice_id: &str) {
    let completed = room
        .app
        .clone()
        .oneshot(resolve_choice_request(
            &room.guest_cookie,
            uuid::Uuid::new_v4(),
            2,
            second_choice_id,
            &["option:2"],
        ))
        .await
        .expect("the second participant must complete the automatic phase");
    if completed.status() != StatusCode::OK {
        let status = completed.status();
        let body = response_json(completed).await;
        panic!("the second participant choice returned {status}: {body}");
    }
    let completed = response_json(completed).await;
    assert_eq!(completed["receipt"]["accepted_state_version"], 3);
    assert_eq!(completed["receipt"]["accepted_sequence"], 2);
    assert_eq!(completed["projection"]["snapshot"]["snapshot_version"], 4);
    assert_eq!(completed["projection"]["snapshot"]["state_version"], 3);
    assert_eq!(completed["projection"]["snapshot"]["sequence"], 2);
    assert_eq!(completed["projection"]["turn"]["number"], 1);
    assert_eq!(completed["projection"]["turn"]["phase"], "hero_actions");
    assert_eq!(completed["projection"]["turn"]["active_position"], 1);
    assert_eq!(
        completed["projection"]["choice"],
        json!({ "status": "none" })
    );
    assert_eq!(completed["projection"]["legal_actions"], json!([]));
}

async fn assert_active_host_can_end_hero_actions(room: &ReadyRoom) {
    let host_session = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, &room.host_cookie)
                .body(Body::empty())
                .expect("the active participant projection request must be valid"),
        )
        .await
        .expect("the active participant projection must receive a response");
    assert_eq!(host_session.status(), StatusCode::OK);
    assert_eq!(
        response_json(host_session).await["legal_actions"],
        json!(["end_hero_actions"])
    );
}

#[tokio::test]
async fn each_hero_choice_is_assigned_in_position_order_without_rejected_command_artifacts() {
    let room = ready_room_with_manifest(each_hero_choice_manifest()).await;
    let started = start_ready_game(&room, "each-hero-choice-start").await;
    let choice_id = assert_initial_each_hero_choice(&started);

    assert_unassigned_choice_rejected_without_artifacts(&room, &choice_id).await;
    let second_choice_id = resolve_first_each_hero_choice(&room, &choice_id).await;
    complete_second_each_hero_choice(&room, &second_choice_id).await;
    assert_active_host_can_end_hero_actions(&room).await;

    assert_choice_codec_versions(&room).await;
}

#[tokio::test]
async fn two_sessions_for_the_responsible_participant_accept_one_choice_resolution() {
    let room = ready_room_with_manifest(each_hero_choice_manifest()).await;
    let started = start_ready_game(&room, "choice-session-race-start").await;
    let choice_id = started["choice"]["id"]
        .as_str()
        .expect("the first participant choice must be present")
        .to_owned();
    let second_host_cookie = additional_session_for_participant(&room, "host").await;
    let first_command_id = uuid::Uuid::new_v4();
    let second_command_id = uuid::Uuid::new_v4();
    let barrier = Arc::new(Barrier::new(2));

    let first = {
        let app = room.app.clone();
        let cookie = room.host_cookie.clone();
        let barrier = Arc::clone(&barrier);
        let choice_id = choice_id.clone();
        async move {
            barrier.wait().await;
            app.oneshot(resolve_choice_request(
                &cookie,
                first_command_id,
                1,
                &choice_id,
                &["option:1"],
            ))
            .await
            .expect("the first session must receive a response")
        }
    };
    let second = {
        let app = room.app.clone();
        let cookie = second_host_cookie.clone();
        let barrier = Arc::clone(&barrier);
        async move {
            barrier.wait().await;
            app.oneshot(resolve_choice_request(
                &cookie,
                second_command_id,
                1,
                &choice_id,
                &["option:1"],
            ))
            .await
            .expect("the second session must receive a response")
        }
    };
    let (first_response, second_response) = tokio::join!(first, second);
    let (winning_command_id, losing_command_id, accepted_response, stale_response) =
        match (first_response.status(), second_response.status()) {
            (StatusCode::OK, StatusCode::CONFLICT) => (
                first_command_id,
                second_command_id,
                first_response,
                second_response,
            ),
            (StatusCode::CONFLICT, StatusCode::OK) => (
                second_command_id,
                first_command_id,
                second_response,
                first_response,
            ),
            statuses => panic!("expected one accepted and one stale response, got {statuses:?}"),
        };
    let accepted = response_json(accepted_response).await;
    assert_eq!(
        response_json(stale_response).await["error"]["code"],
        "STALE_STATE_VERSION"
    );

    let recovered = command_result(&room.app, &second_host_cookie, winning_command_id).await;
    assert_eq!(recovered.status(), StatusCode::OK);
    let recovered = response_json(recovered).await;
    assert_eq!(recovered["receipt"], accepted["receipt"]);
    assert_eq!(recovered["projection"]["snapshot"]["state_version"], 2);
    assert_eq!(
        command_result(&room.app, &second_host_cookie, losing_command_id)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    assert_winning_choice_artifacts(&room, winning_command_id).await;
}

#[tokio::test]
async fn a_terminal_effect_commits_the_same_status_to_snapshot_row_and_projection() {
    let room = ready_room_with_manifest(terminal_manifest()).await;
    let started = start_ready_game(&room, "terminal-effect-start").await;

    assert_eq!(started["game"]["status"], "won");
    assert_eq!(started["effects"]["status"], "terminal");
    assert_eq!(started["legal_actions"], json!([]));
    assert_eq!(started["choice"], json!({ "status": "none" }));
    assert!(
        started["effects"]["outcomes"]
            .as_array()
            .is_some_and(|outcomes| outcomes
                .iter()
                .any(|outcome| { outcome["type"] == "terminal" && outcome["outcome"] == "won" }))
    );

    let (status, snapshot_status) = sqlx::query_as::<_, (String, String)>(
        r"
        SELECT games.status, games.snapshot ->> 'status'
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the terminal game state must be queryable");
    assert_eq!((status.as_str(), snapshot_status.as_str()), ("won", "won"));
}

#[tokio::test]
async fn committed_events_receipts_and_replay_anchors_are_append_only() {
    let room = ready_room().await;
    start_ready_game(&room, "append-only-start").await;
    let response = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, uuid::Uuid::new_v4(), 1))
        .await
        .expect("the command must receive a response");
    assert_eq!(response.status(), StatusCode::OK);

    for statement in [
        r#"
        UPDATE game_events
        SET payload = payload || '{"tampered": true}'::jsonb
        WHERE game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        "#,
        r"
        DELETE FROM game_events
        WHERE game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        ",
        r"
        UPDATE game_command_receipts
        SET command_type = command_type || '_tampered'
        WHERE game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        ",
        r"
        DELETE FROM game_command_receipts
        WHERE game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        ",
        r"
        UPDATE game_state_anchors
        SET state_digest = 'blake3:0000000000000000000000000000000000000000000000000000000000000000'
        WHERE game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        ",
        r"
        DELETE FROM game_state_anchors
        WHERE game_id = (
            SELECT games.id
            FROM games
            JOIN rooms ON rooms.id = games.room_id
            WHERE rooms.code = $1
        )
        ",
    ] {
        let error = sqlx::query(statement)
            .bind(&room.room_code)
            .execute(&room.database)
            .await
            .expect_err("official history must reject direct mutation");
        assert_database_error_code(&error, "55000");
    }

    assert_history_prevents_game_deletion(&room).await;

    let artifact_counts = sqlx::query_as::<_, (i64, i64, i64)>(
        r"
        SELECT
            (SELECT COUNT(*) FROM game_events WHERE game_id = games.id),
            (SELECT COUNT(*) FROM game_command_receipts WHERE game_id = games.id),
            (SELECT COUNT(*) FROM game_state_anchors WHERE game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the protected history must remain queryable");
    assert_eq!(artifact_counts, (1, 1, 2));
}

#[tokio::test]
async fn an_event_actor_must_belong_to_the_games_room() {
    let room = ready_room().await;
    let other_room = ready_room().await;
    start_ready_game(&room, "event-actor-room-start").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the actor integrity transaction must start");
    let (game_id, room_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid)>(
        r"
        UPDATE games
        SET sequence = 1,
            state_version = 2
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        RETURNING id, room_id
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the test game cursor must advance inside the transaction");
    let other_actor = sqlx::query_scalar::<_, uuid::Uuid>(
        r"
        SELECT participants.id
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
          AND participants.role = 'host'
        ",
    )
    .bind(&other_room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the unrelated actor must exist");
    let payload = test_turn_completed_payload(1, 2, 1, 1);

    let error = sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_version,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, 1, 5, 'turn_completed', $3, $4, 2, $5)
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(uuid::Uuid::new_v4())
    .bind(other_actor)
    .bind(payload)
    .execute(&mut *transaction)
    .await
    .expect_err("an event actor from another room must be rejected");
    assert_database_error_code(&error, "23514");
    assert!(error.to_string().contains("payload metadata must match"));
    transaction
        .rollback()
        .await
        .expect("the actor integrity transaction must roll back");
}

#[tokio::test]
async fn an_event_envelope_and_payload_must_match_the_committed_snapshot() {
    let room = ready_room().await;
    start_ready_game(&room, "event-envelope-start").await;
    let (game_id, room_id, actor_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
            SELECT id, room_id, started_by_participant_id
            FROM games
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the test game must exist");

    for (row_state_version, payload_state_version, expected_message) in [
        (3_i64, 3_i64, "state version must match"),
        (2_i64, 3_i64, "payload metadata must match"),
    ] {
        let mut transaction = room
            .database
            .begin()
            .await
            .expect("the event coherence transaction must start");
        sqlx::query("UPDATE games SET sequence = 1, state_version = 2 WHERE id = $1")
            .bind(game_id)
            .execute(&mut *transaction)
            .await
            .expect("the test snapshot must advance inside the transaction");
        let payload = test_turn_completed_payload(
            1,
            u64::try_from(payload_state_version).expect("the state version must be positive"),
            1,
            1,
        );
        let error = sqlx::query(
            r"
            INSERT INTO game_events (
                game_id,
                room_id,
                sequence,
                event_version,
                event_type,
                command_id,
                actor_participant_id,
                state_version,
                payload
            )
            VALUES (
                $1,
                $2,
                1,
                5,
                'turn_completed',
                $3,
                $4,
                $5,
                $6
            )
            ",
        )
        .bind(game_id)
        .bind(room_id)
        .bind(uuid::Uuid::new_v4())
        .bind(actor_id)
        .bind(row_state_version)
        .bind(payload)
        .execute(&mut *transaction)
        .await
        .expect_err("incoherent event metadata must be rejected");
        assert_database_error_code(&error, "23514");
        assert!(error.to_string().contains(expected_message));
        transaction
            .rollback()
            .await
            .expect("the rejected event transaction must roll back");
    }
}

#[tokio::test]
async fn an_event_payload_must_match_the_supported_codec_exactly() {
    let room = ready_room().await;
    start_ready_game(&room, "event-codec-start").await;
    let (game_id, room_id, actor_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
        SELECT id, room_id, started_by_participant_id
        FROM games
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the test game must exist");

    for (event_type, extra_payload) in [
        ("future_event", "{}"),
        ("turn_completed", r#"{"unexpected":true}"#),
        ("turn_completed", r#"{"sequence":1.0}"#),
    ] {
        let mut transaction = room
            .database
            .begin()
            .await
            .expect("the event codec transaction must start");
        sqlx::query("UPDATE games SET sequence = 1, state_version = 2 WHERE id = $1")
            .bind(game_id)
            .execute(&mut *transaction)
            .await
            .expect("the test snapshot must advance inside the transaction");
        let mut payload = test_turn_completed_payload(1, 2, 1, 1);
        payload["type"] = json!(event_type);
        let error = sqlx::query(
            r"
            INSERT INTO game_events (
                game_id,
                room_id,
                sequence,
                event_version,
                event_type,
                command_id,
                actor_participant_id,
                state_version,
                payload
            )
            VALUES (
                $1,
                $2,
                1,
                5,
                $3,
                $4,
                $5,
                2,
                $6 || $7::jsonb
            )
            ",
        )
        .bind(game_id)
        .bind(room_id)
        .bind(event_type)
        .bind(uuid::Uuid::new_v4())
        .bind(actor_id)
        .bind(payload)
        .bind(extra_payload)
        .execute(&mut *transaction)
        .await
        .expect_err("an event outside the current codec must be rejected");
        assert_database_error_code(&error, "23514");
        assert!(
            error
                .to_string()
                .contains("payload must match the current codec shape")
        );
        transaction
            .rollback()
            .await
            .expect("the rejected event transaction must roll back");
    }
}

#[tokio::test]
async fn legacy_event_codecs_are_rejected_for_new_appends() {
    let room = ready_room().await;
    start_ready_game(&room, "legacy-event-write-start").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the legacy event transaction must start");
    let (game_id, room_id, actor_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
            UPDATE games
            SET sequence = 1,
                state_version = 2
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            RETURNING id, room_id, started_by_participant_id
            ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the test snapshot must advance inside the transaction");
    let legacy = json!({
        "event_version": 2,
        "type": "dark_arts_completed",
        "sequence": 1,
        "state_version": 2,
        "turn": 1,
        "actor_position": 1,
        "effects": [],
        "effect_stop": "stable",
        "prng_counter": 1
    });
    let error = sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_version,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, 1, 2, 'dark_arts_completed', $3, $4, 2, $5)
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(uuid::Uuid::new_v4())
    .bind(actor_id)
    .bind(legacy)
    .execute(&mut *transaction)
    .await
    .expect_err("legacy event codecs must be read-only");

    assert_database_error_code(&error, "23514");
    assert!(
        error
            .to_string()
            .contains("legacy game event codecs are read-only")
    );
    transaction
        .rollback()
        .await
        .expect("the legacy event transaction must roll back");
}

#[tokio::test]
async fn a_v4_event_rejects_an_incomplete_automatic_effect_outcome() {
    let room = ready_room().await;
    start_ready_game(&room, "event-v3-effect-start").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the v4 effect transaction must start");
    let (game_id, room_id, actor_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
            UPDATE games
            SET sequence = 1,
                state_version = 2
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            RETURNING id, room_id, started_by_participant_id
            ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the test snapshot must advance inside the transaction");
    let mut malformed = test_turn_completed_payload(1, 2, 1, 1);
    malformed["steps"][1]["effects"] = json!([{
        "type": "die_rolled",
        "rule_id": "rule:functional"
    }]);

    let error = sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_version,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, 1, 5, 'turn_completed', $3, $4, 2, $5)
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(uuid::Uuid::new_v4())
    .bind(actor_id)
    .bind(malformed)
    .execute(&mut *transaction)
    .await
    .expect_err("an incomplete closed effect outcome must be rejected");

    assert_database_error_code(&error, "23514");
    assert!(
        error
            .to_string()
            .contains("payload must match the current codec shape")
    );
    transaction
        .rollback()
        .await
        .expect("the rejected v4 effect transaction must roll back");
}

#[tokio::test]
async fn v4_event_validation_rejects_nulls_and_cross_field_inconsistencies() {
    let room = ready_room().await;
    let mut valid = test_turn_completed_payload(1, 2, 1, 1);
    valid["event_version"] = json!(4);
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT valid_turn_completed_payload_v4($1)")
            .bind(&valid)
            .fetch_one(&room.database)
            .await
            .expect("the v4 validator must accept the canonical fixture")
    );

    let mut null_choice_kind = valid.clone();
    null_choice_kind["control"] = json!({
        "status": "in_progress",
        "turn": 2,
        "phase": "dark_arts",
        "active_position": 2,
        "queued_phases": ["villains", "hero_actions", "end_turn"],
        "queued_effects": [],
        "decision_point": {
            "type": "effect_choice",
            "choice": {
                "id": "rule:test:effect:0",
                "rule_id": "rule:test",
                "path": [],
                "responsible_position": 2,
                "kind": null,
                "options": ["option:1", "option:2"],
                "min": 1,
                "max": 1
            }
        }
    });
    null_choice_kind["steps"] = json!([
        { "phase": "end_turn", "effects": [] },
        { "phase": "dark_arts", "effects": [] }
    ]);

    let mut null_shuffle_zone = valid.clone();
    null_shuffle_zone["end_turn"] = json!([{
        "type": "pile_shuffled",
        "owner_position": 1,
        "zone": null,
        "bottom_to_top": ["card:a"]
    }]);

    let mut null_step_phase = valid.clone();
    null_step_phase["steps"][2]["phase"] = Value::Null;

    let mut null_control_status = valid.clone();
    null_control_status["control"]["status"] = Value::Null;

    let mut same_zone_move = valid.clone();
    same_zone_move["steps"][1]["effects"] = json!([{
        "type": "moved",
        "rule_id": "rule:test",
        "target_id": "card:a",
        "from": "hero_hand",
        "to": "hero_hand"
    }]);

    let mut stable_without_villains = valid.clone();
    stable_without_villains["steps"] = json!([
        { "phase": "end_turn", "effects": [] },
        { "phase": "dark_arts", "effects": [] }
    ]);

    for (case, payload) in [
        ("null choice kind", null_choice_kind),
        ("null shuffle zone", null_shuffle_zone),
        ("null step phase", null_step_phase),
        ("null control status", null_control_status),
        ("same-zone move", same_zone_move),
        ("stable event without Villains", stable_without_villains),
    ] {
        let accepted = sqlx::query_scalar::<_, bool>("SELECT valid_turn_completed_payload_v4($1)")
            .bind(payload)
            .fetch_one(&room.database)
            .await
            .unwrap_or_else(|error| panic!("the v4 validator must evaluate {case}: {error}"));
        assert!(!accepted, "the v4 validator accepted {case}");
    }
}

fn v3_terminal_event_fixture() -> Value {
    json!({
        "event_version": 3,
        "type": "dark_arts_completed",
        "sequence": 1,
        "state_version": 2,
        "turn": 1,
        "actor_position": 1,
        "effects": [
            { "type": "terminal", "rule_id": "rule:first", "outcome": "lost" }
        ],
        "effect_stop": "terminal",
        "choice": null,
        "prng_counter": 0
    })
}

async fn legacy_event_passes_replay_preflight(
    room: &ReadyRoom,
    event_version: i16,
    payload: &Value,
    description: &str,
) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT valid_legacy_game_event_for_replay(\
            $1, 'dark_arts_completed', $2, 1::BIGINT, 2::BIGINT, 1::SMALLINT\
        )",
    )
    .bind(event_version)
    .bind(payload)
    .fetch_one(&room.database)
    .await
    .unwrap_or_else(|error| panic!("the legacy preflight must evaluate {description}: {error}"))
}

#[tokio::test]
async fn legacy_replay_preflight_rejects_rows_the_rust_codec_cannot_decode() {
    let room = ready_room().await;
    let v1 = json!({
        "event_version": 1,
        "type": "dark_arts_completed",
        "sequence": 1,
        "state_version": 2,
        "turn": 1,
        "actor_position": 1
    });
    let v2 = json!({
        "event_version": 2,
        "type": "dark_arts_completed",
        "sequence": 1,
        "state_version": 2,
        "turn": 1,
        "actor_position": 1,
        "effects": [],
        "effect_stop": "stable",
        "choice": null,
        "prng_counter": 0
    });
    let v3_terminal = v3_terminal_event_fixture();

    for (version, payload) in [(1_i16, &v1), (2_i16, &v2)] {
        let accepted =
            legacy_event_passes_replay_preflight(&room, version, payload, "canonical fixture")
                .await;
        assert!(accepted, "the canonical v{version} fixture was rejected");
    }
    let accepted = legacy_event_passes_replay_preflight(
        &room,
        3,
        &v3_terminal,
        "canonical v3 terminal fixture",
    )
    .await;
    assert!(accepted, "the canonical v3 terminal fixture was rejected");

    let mut null_stop = v2.clone();
    null_stop["effect_stop"] = Value::Null;

    let mut non_card_move = v2.clone();
    non_card_move["effects"] = json!([{
        "type": "moved",
        "rule_id": "rule:test",
        "target_id": "hero:1",
        "target_position": 1,
        "from": "heroes",
        "to": "heroes"
    }]);

    let mut malformed_choice_id = v2.clone();
    malformed_choice_id["effect_stop"] = json!("choice");
    malformed_choice_id["choice"] = json!({
        "id": "missing-address",
        "responsible_position": 1,
        "kind": "target",
        "options": ["card:a", "card:b"],
        "min": 1,
        "max": 1
    });

    let mut duplicate_choice_options = malformed_choice_id.clone();
    duplicate_choice_options["choice"]["id"] = json!("rule:test:target:0");
    duplicate_choice_options["choice"]["options"] = json!(["card:a", "card:a"]);

    let mut duplicate_terminal = v2.clone();
    duplicate_terminal["effect_stop"] = json!("terminal");
    duplicate_terminal["effects"] = json!([
        { "type": "terminal", "rule_id": "rule:first", "outcome": "lost" },
        { "type": "terminal", "rule_id": "rule:second", "outcome": "lost" }
    ]);

    let mut oversized = v2;
    oversized["padding"] = json!("x".repeat(4 * 1_024 * 1_024));

    for (case, payload) in [
        ("null effect stop", null_stop),
        ("non-card same-zone move", non_card_move),
        ("choice without an address", malformed_choice_id),
        ("duplicate choice options", duplicate_choice_options),
        ("multiple terminal outcomes", duplicate_terminal),
        ("oversized payload", oversized),
    ] {
        let accepted = legacy_event_passes_replay_preflight(&room, 2, &payload, case).await;
        assert!(!accepted, "the legacy preflight accepted {case}");
    }

    let mut duplicate_v3_terminal = v3_terminal;
    duplicate_v3_terminal["effects"] = json!([
        { "type": "terminal", "rule_id": "rule:first", "outcome": "lost" },
        { "type": "terminal", "rule_id": "rule:second", "outcome": "lost" }
    ]);
    let accepted = legacy_event_passes_replay_preflight(
        &room,
        3,
        &duplicate_v3_terminal,
        "multiple v3 terminal outcomes",
    )
    .await;
    assert!(
        !accepted,
        "the legacy preflight accepted multiple v3 terminal outcomes"
    );
}

async fn is_snapshot_valid_for_v15_upgrade(
    database: &PgPool,
    candidate: &Value,
    expected_participants: &Value,
    failure_context: &str,
) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT valid_game_snapshot_for_v15_upgrade($1::JSONB, $2::JSONB)",
    )
    .bind(candidate)
    .bind(expected_participants)
    .fetch_one(database)
    .await
    .unwrap_or_else(|error| panic!("{failure_context}: {error}"))
}

fn legacy_v3_snapshot_fixture(mut current: Value) -> Value {
    current["snapshot_version"] = json!(3);
    current
        .as_object_mut()
        .expect("snapshot object")
        .remove("active_villain_limit");
    for entity in current["effects"]["entities"]
        .as_array_mut()
        .expect("effect entities")
    {
        let entity = entity.as_object_mut().expect("entity object");
        entity.remove("resource_limits");
        entity.remove("reward_rule_id");
        entity.remove("dark_arts_count");
    }
    current
}

fn legacy_v1_snapshot_fixture(current: &Value) -> Value {
    let mut legacy_v1 = current.clone();
    legacy_v1["snapshot_version"] = json!(1);
    let legacy_v1_object = legacy_v1
        .as_object_mut()
        .expect("the current snapshot must be an object");
    for field in [
        "queued_phases",
        "queued_effects",
        "decision_point",
        "last_turn_steps",
        "effects",
    ] {
        legacy_v1_object.remove(field);
    }
    legacy_v1["turn"]["phase"] = json!("hero_action");
    legacy_v1
}

fn legacy_v2_snapshot_fixture(current: &Value, legacy_v1: &Value) -> Value {
    let mut legacy_v2 = legacy_v1.clone();
    legacy_v2["snapshot_version"] = json!(2);
    let mut entities = current["effects"]["entities"]
        .as_array()
        .expect("the current effect entities must be an array")
        .clone();
    for entity in &mut entities {
        entity
            .as_object_mut()
            .expect("each effect entity must be an object")
            .remove("zone_index");
    }
    entities.push(json!({
        "id": "legacy-card:extra",
        "owner_position": 1,
        "zone": "hero_draw_pile"
    }));
    legacy_v2["effects"] = json!({
        "entities": entities,
        "outcomes": [{
            "type": "no_op",
            "rule_id": "legacy-rule:test",
            "reason": "explicit"
        }]
    });
    legacy_v2
}

async fn legacy_resumable_choice_snapshot(
    database: &PgPool,
    choice_room_code: &str,
    legacy_v2: &Value,
) -> Value {
    let choice_snapshot = sqlx::query_scalar::<_, Value>(
        r"
        SELECT games.snapshot
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(choice_room_code)
    .fetch_one(database)
    .await
    .expect("the resumable choice fixture must be queryable");
    let mut legacy_choice = legacy_v2.clone();
    legacy_choice["turn"]["phase"] = json!("dark_arts");
    legacy_choice["effects"]["choice"] = choice_snapshot["effects"]["choice"].clone();
    legacy_choice
}

fn invalid_v15_upgrade_snapshot_fixtures(
    legacy_v1: &Value,
    legacy_v2: Value,
    legacy_choice: Value,
    current: Value,
) -> [(&'static str, Value); 9] {
    let mut null_phase = legacy_v1.clone();
    null_phase["turn"]["phase"] = Value::Null;

    let mut partial_control = legacy_v1.clone();
    partial_control["queued_phases"] = json!(["end_turn"]);

    let mut reordered_participants = legacy_v1.clone();
    reordered_participants["participants"]
        .as_array_mut()
        .expect("legacy participants must be an array")
        .swap(0, 1);

    let mut wrong_participant_uuid = legacy_v1.clone();
    wrong_participant_uuid["participants"][0]["participant_id"] =
        json!(uuid::Uuid::new_v4().to_string());

    let mut swapped_heroes = legacy_v1.clone();
    let first_hero = swapped_heroes["participants"][0]["hero_id"].clone();
    swapped_heroes["participants"][0]["hero_id"] =
        swapped_heroes["participants"][1]["hero_id"].clone();
    swapped_heroes["participants"][1]["hero_id"] = first_hero;

    let mut invalid_effects = legacy_v2;
    invalid_effects["effects"]["entities"][0]["resources"] = json!({ "mana": 1 });

    let mut malformed_effects = legacy_v1.clone();
    malformed_effects["effects"] = json!("not-an-effect-object");

    let mut wrong_choice_actor = legacy_choice;
    wrong_choice_actor["effects"]["choice"]["responsible_position"] = json!(4);

    let mut divergent_structured_history = current;
    let structured_outcomes = divergent_structured_history["effects"]["outcomes"]
        .as_array_mut()
        .expect("the structured outcomes must be an array");
    assert!(
        structured_outcomes.len() > 1,
        "the fixture must retain a non-empty outcome history after mutation"
    );
    structured_outcomes.pop();

    [
        ("null turn phase", null_phase),
        ("partial structured control", partial_control),
        ("reordered participants", reordered_participants),
        ("another participant UUID", wrong_participant_uuid),
        ("heroes assigned to other positions", swapped_heroes),
        (
            "an effect resource unsupported by its zone",
            invalid_effects,
        ),
        ("a malformed effects value", malformed_effects),
        (
            "a pending choice assigned outside the participants",
            wrong_choice_actor,
        ),
        (
            "structured outcomes divergent from turn steps",
            divergent_structured_history,
        ),
    ]
}

async fn committed_snapshot(room: &ReadyRoom) -> Value {
    sqlx::query_scalar("SELECT games.snapshot FROM games JOIN rooms ON rooms.id = games.room_id WHERE rooms.code = $1")
        .bind(&room.room_code).fetch_one(&room.database).await.expect("the committed snapshot must be queryable")
}

#[tokio::test]
async fn v15_upgrade_preflight_rejects_legacy_snapshots_that_cannot_be_restored() {
    let room = ready_room().await;
    start_ready_game(&room, "legacy-snapshot-preflight-start").await;
    let current = committed_snapshot(&room).await;
    let current = legacy_v3_snapshot_fixture(current);
    let expected_participants = current["participants"].clone();

    assert!(
        is_snapshot_valid_for_v15_upgrade(
            &room.database,
            &current,
            &expected_participants,
            "the V3 snapshot must pass its historical preflight"
        )
        .await
    );

    let legacy_v1 = legacy_v1_snapshot_fixture(&current);
    let accepted = is_snapshot_valid_for_v15_upgrade(
        &room.database,
        &legacy_v1,
        &expected_participants,
        "the V1 snapshot preflight must evaluate the canonical fixture",
    )
    .await;
    assert!(accepted, "a legitimate V1 snapshot must remain upgradeable");

    let mut legacy_with_null_control = legacy_v1.clone();
    for field in [
        "queued_phases",
        "queued_effects",
        "decision_point",
        "last_turn_steps",
    ] {
        legacy_with_null_control[field] = Value::Null;
    }
    let accepted = is_snapshot_valid_for_v15_upgrade(
        &room.database,
        &legacy_with_null_control,
        &expected_participants,
        "the null legacy control preflight must evaluate the fixture",
    )
    .await;
    assert!(
        accepted,
        "explicit null Option fields must remain equivalent to absent legacy control"
    );

    let legacy_v2 = legacy_v2_snapshot_fixture(&current, &legacy_v1);
    let accepted = is_snapshot_valid_for_v15_upgrade(
        &room.database,
        &legacy_v2,
        &expected_participants,
        "the V2 snapshot preflight must evaluate the canonical fixture",
    )
    .await;
    assert!(
        accepted,
        "a legitimate V2 snapshot without zone indexes must remain upgradeable"
    );

    let choice_room = ready_room_with_manifest(each_hero_choice_manifest()).await;
    start_ready_game(&choice_room, "legacy-choice-preflight-start").await;
    let legacy_choice =
        legacy_resumable_choice_snapshot(&room.database, &choice_room.room_code, &legacy_v2).await;
    let accepted = is_snapshot_valid_for_v15_upgrade(
        &room.database,
        &legacy_choice,
        &expected_participants,
        "the V2 choice preflight must evaluate the resumable fixture",
    )
    .await;
    assert!(accepted, "a legitimate resumable V2 choice must upgrade");

    let mut delegated_choice = legacy_choice.clone();
    delegated_choice["effects"]["choice"]["responsible_position"] = json!(2);
    let accepted = is_snapshot_valid_for_v15_upgrade(
        &room.database,
        &delegated_choice,
        &expected_participants,
        "the delegated V2 choice preflight must evaluate the fixture",
    )
    .await;
    assert!(
        accepted,
        "a pending choice may be assigned to a non-active participant"
    );

    for (case, candidate) in
        invalid_v15_upgrade_snapshot_fixtures(&legacy_v1, legacy_v2, legacy_choice, current)
    {
        let failure_context = format!("the upgrade preflight must evaluate {case}");
        let accepted = is_snapshot_valid_for_v15_upgrade(
            &room.database,
            &candidate,
            &expected_participants,
            &failure_context,
        )
        .await;
        assert!(!accepted, "the upgrade preflight accepted {case}");
    }
}

#[tokio::test]
async fn v4_snapshot_validation_rejects_codec_incompatible_effects_and_identifiers() {
    let room = ready_room().await;
    start_ready_game(&room, "snapshot-validator-start").await;
    let snapshot = sqlx::query_scalar::<_, Value>(
        r"
        SELECT games.snapshot
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the current snapshot must exist");
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT valid_game_snapshot_v4($1)")
            .bind(&snapshot)
            .fetch_one(&room.database)
            .await
            .expect("the snapshot validator must accept the committed snapshot")
    );

    let mut same_zone_move = snapshot.clone();
    same_zone_move["effects"]["outcomes"] = json!([{
        "type": "moved",
        "rule_id": "rule:test",
        "target_id": "card:a",
        "from": "hero_hand",
        "to": "hero_hand"
    }]);

    let mut non_card_move = snapshot.clone();
    non_card_move["effects"]["outcomes"] = json!([{
        "type": "moved",
        "rule_id": "rule:test",
        "target_id": "hero:1",
        "target_position": 1,
        "from": "heroes",
        "to": "active_location"
    }]);

    let mut null_prng_algorithm = snapshot.clone();
    null_prng_algorithm["prng"]["algorithm"] = Value::Null;

    let mut oversized_identifier = snapshot.clone();
    oversized_identifier["adventure_id"] = json!(format!("adventure:{}", "a".repeat(247)));
    assert_eq!(
        oversized_identifier["adventure_id"]
            .as_str()
            .expect("the fixture ID must be a string")
            .len(),
        257
    );

    let mut non_contiguous_positions = snapshot.clone();
    non_contiguous_positions["participants"][1]["position"] = json!(3);

    let mut divergent_effect_history = snapshot.clone();
    let outcomes = divergent_effect_history["effects"]["outcomes"]
        .as_array_mut()
        .expect("the snapshot outcomes must be an array");
    assert!(
        outcomes.len() > 1,
        "the fixture must retain a non-empty outcome history after mutation"
    );
    outcomes.pop();

    let mut absent_active_player = snapshot;
    absent_active_player["turn"]["active_position"] = json!(4);
    absent_active_player["decision_point"]["responsible_position"] = json!(4);

    for (case, candidate) in [
        ("same-zone move", same_zone_move),
        ("non-card move", non_card_move),
        ("null PRNG algorithm", null_prng_algorithm),
        ("257-byte identifier", oversized_identifier),
        (
            "non-contiguous participant positions",
            non_contiguous_positions,
        ),
        (
            "effects divergent from turn steps",
            divergent_effect_history,
        ),
        ("active player outside participants", absent_active_player),
    ] {
        let accepted = sqlx::query_scalar::<_, bool>("SELECT valid_game_snapshot_v4($1)")
            .bind(candidate)
            .fetch_one(&room.database)
            .await
            .unwrap_or_else(|error| panic!("the snapshot validator must evaluate {case}: {error}"));
        assert!(!accepted, "the snapshot validator accepted {case}");
    }
}

#[tokio::test]
async fn a_new_game_snapshot_must_exactly_match_the_room_participants() {
    let source = ready_room().await;
    let target = ready_room().await;
    start_ready_game(&source, "participant-bound-source-start").await;
    let mut snapshot = sqlx::query_scalar::<_, Value>(
        r"
        SELECT games.snapshot
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&source.room_code)
    .fetch_one(&source.database)
    .await
    .expect("the source snapshot must exist");
    snapshot["participants"] = sqlx::query_scalar::<_, Value>(
        r"
        SELECT jsonb_agg(
            jsonb_build_object(
                'participant_id', participants.id::TEXT,
                'position', participants.position,
                'hero_id', participants.hero_id
            )
            ORDER BY participants.position
        )
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&target.room_code)
    .fetch_one(&target.database)
    .await
    .expect("the target participants must be queryable");

    let mut valid_transaction = target
        .database
        .begin()
        .await
        .expect("the valid cloned game transaction must start");
    insert_cloned_game_snapshot(
        &mut valid_transaction,
        &source.room_code,
        &target.room_code,
        &snapshot,
    )
    .await
    .expect("a snapshot with the exact room participants must pass the INSERT trigger");
    valid_transaction
        .rollback()
        .await
        .expect("the valid cloned game transaction must roll back");

    let mut reordered = snapshot.clone();
    reordered["participants"]
        .as_array_mut()
        .expect("snapshot participants must be an array")
        .swap(0, 1);

    let mut wrong_uuid = snapshot.clone();
    wrong_uuid["participants"][0]["participant_id"] = json!(uuid::Uuid::new_v4().to_string());

    let mut swapped_heroes = snapshot.clone();
    let first_hero = swapped_heroes["participants"][0]["hero_id"].clone();
    swapped_heroes["participants"][0]["hero_id"] =
        swapped_heroes["participants"][1]["hero_id"].clone();
    swapped_heroes["participants"][1]["hero_id"] = first_hero;

    for (case, candidate) in [
        ("reordered participants", reordered),
        ("another participant UUID", wrong_uuid),
        ("heroes assigned to other positions", swapped_heroes),
    ] {
        let mut transaction = target
            .database
            .begin()
            .await
            .unwrap_or_else(|error| panic!("the {case} transaction must start: {error}"));
        let error = insert_cloned_game_snapshot(
            &mut transaction,
            &source.room_code,
            &target.room_code,
            &candidate,
        )
        .await
        .expect_err(
            "the INSERT trigger must reject snapshot participants that differ from the room",
        );
        assert_database_error_code(&error, "23514");
        assert!(
            error
                .to_string()
                .contains("snapshot must match the current codec and relational metadata"),
            "unexpected error for {case}: {error}"
        );
        transaction
            .rollback()
            .await
            .unwrap_or_else(|error| panic!("the {case} transaction must roll back: {error}"));
    }
}

#[tokio::test]
async fn v3_choice_payload_limits_match_the_domain_decoder() {
    let room = ready_room_with_manifest(each_hero_choice_manifest()).await;
    start_ready_game(&room, "event-v3-cursor-limit-start").await;
    let snapshot = sqlx::query_scalar::<_, Value>(
        r"
        SELECT games.snapshot
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the persisted pending choice must be queryable");
    let effects = snapshot["effects"]
        .get("outcomes")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let base = json!({
        "event_version": 3,
        "type": "dark_arts_completed",
        "sequence": 1,
        "state_version": 2,
        "turn": 1,
        "actor_position": 1,
        "effects": effects,
        "effect_stop": "choice",
        "choice": snapshot["effects"]["choice"],
        "prng_counter": snapshot["prng"]["counter"]
    });
    let accepted = sqlx::query_scalar::<_, bool>(
        "SELECT valid_legacy_game_event_for_replay(\
            3::SMALLINT, 'dark_arts_completed', $1, \
            1::BIGINT, 2::BIGINT, 1::SMALLINT\
        )",
    )
    .bind(&base)
    .fetch_one(&room.database)
    .await
    .expect("the v3 replay validator must accept the canonical choice fixture");
    assert!(accepted);

    let mut oversized_path = base.clone();
    oversized_path["choice"]["continuation"]["choice_cursor"]["path"] =
        Value::Array(vec![json!({ "type": "condition_then" }); 4097]);
    let mut oversized_option = base.clone();
    oversized_option["choice"]["options"][0] = json!("x".repeat(257));
    let mut invalid_effect = base.clone();
    invalid_effect["choice"]["min"] = json!(0);
    let mut target_selects_all = base.clone();
    target_selects_all["choice"]["kind"] = json!("target");
    target_selects_all["choice"]["min"] = json!(0);
    target_selects_all["choice"]["max"] = json!(2);
    let mut target_selects_none = base;
    target_selects_none["choice"]["kind"] = json!("target");
    target_selects_none["choice"]["min"] = json!(0);
    target_selects_none["choice"]["max"] = json!(0);

    for (description, payload) in [
        ("oversized cursor path", oversized_path),
        ("oversized choice option", oversized_option),
        ("effect choice cardinality", invalid_effect),
        ("target choice selecting every option", target_selects_all),
        ("target choice with a zero maximum", target_selects_none),
    ] {
        assert_v3_choice_payload_rejected(&room, payload, description).await;
    }
}

#[tokio::test]
async fn a_game_snapshot_cannot_advance_without_official_history() {
    let room = ready_room().await;
    start_ready_game(&room, "orphan-snapshot-start").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the orphan snapshot transaction must start");
    sqlx::query(
        r"
        UPDATE games
        SET sequence = sequence + 1,
            state_version = state_version + 1
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(&room.room_code)
    .execute(&mut *transaction)
    .await
    .expect("the deferred history constraint should allow the statement");

    let error = transaction
        .commit()
        .await
        .expect_err("the orphan snapshot must be rejected at commit");
    assert_database_error_code(&error, "23514");
    assert!(
        error
            .to_string()
            .contains("matching official event and receipt")
    );
}

#[tokio::test]
async fn authoritative_game_state_cannot_change_without_advancing_its_cursor() {
    let room = ready_room().await;
    start_ready_game(&room, "same-cursor-rewrite-start").await;
    let stored_snapshot = sqlx::query_scalar::<_, String>(
        r"
        SELECT games.snapshot::text
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the initial snapshot must be queryable");
    let mut rewritten_snapshot: Value =
        serde_json::from_str(&stored_snapshot).expect("the initial snapshot must be JSON");
    rewritten_snapshot["turn"]["number"] = json!(999);
    let rewritten_snapshot =
        serde_json::to_string(&rewritten_snapshot).expect("the rewritten snapshot must serialize");
    let rewritten_digest = format!(
        "blake3:{}",
        blake3::hash(rewritten_snapshot.as_bytes()).to_hex()
    );
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the same-cursor rewrite transaction must start");
    sqlx::query(
        r"
        UPDATE games
        SET snapshot = $2::jsonb,
            state_digest = $3
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(&room.room_code)
    .bind(rewritten_snapshot)
    .bind(rewritten_digest)
    .execute(&mut *transaction)
    .await
    .expect("the deferred history constraint should allow the statement");

    let error = transaction
        .commit()
        .await
        .expect_err("a same-cursor state rewrite must be rejected at commit");
    assert_database_error_code(&error, "23514");
    assert!(error.to_string().contains("without advancing its cursor"));
}

#[tokio::test]
async fn snapshot_numeric_representation_cannot_change_without_advancing_its_cursor() {
    let room = ready_room().await;
    start_ready_game(&room, "decimal-snapshot-rewrite-start").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the decimal snapshot rewrite transaction must start");
    sqlx::query(
        r"
        UPDATE games
        SET snapshot = jsonb_set(snapshot, '{turn,number}', '1.0'::jsonb)
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(&room.room_code)
    .execute(&mut *transaction)
    .await
    .expect("the deferred history constraint should allow the statement");

    let error = transaction
        .commit()
        .await
        .expect_err("a decimal snapshot rewrite must be rejected at commit");
    assert_database_error_code(&error, "23514");
    assert!(error.to_string().contains("without advancing its cursor"));
}

#[tokio::test]
async fn an_official_receipt_must_share_the_committed_game_expiration() {
    let room = ready_room().await;
    start_ready_game(&room, "receipt-expiration-start").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the receipt expiration transaction must start");
    let (game_id, actor_id, room_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
            SELECT id, started_by_participant_id, room_id
            FROM games
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the integrity test game must be queryable");
    let command_id = uuid::Uuid::new_v4();
    insert_test_event(&mut transaction, game_id, room_id, command_id, actor_id).await;
    sqlx::query(
        r"
        INSERT INTO game_command_receipts (
            game_id,
            room_id,
            command_id,
            actor_participant_id,
            command_type,
            expected_state_version,
            payload_digest,
            accepted_state_version,
            accepted_sequence,
            expires_at
        )
        VALUES (
            $1,
            $2,
            $3,
            $4,
            'end_hero_actions',
            1,
            'blake3:0000000000000000000000000000000000000000000000000000000000000000',
            2,
            1,
            (SELECT expires_at + INTERVAL '1 second' FROM games WHERE id = $1)
        )
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(command_id)
    .bind(actor_id)
    .execute(&mut *transaction)
    .await
    .expect("the mismatched receipt should reach the deferred constraint");

    let error = transaction
        .commit()
        .await
        .expect_err("a receipt with another expiration must be rejected at commit");
    assert_database_error_code(&error, "23514");
    assert!(
        error
            .to_string()
            .contains("matching official event and receipt")
    );
}

#[tokio::test]
async fn an_official_event_cannot_commit_without_its_receipt() {
    let room = ready_room().await;
    start_ready_game(&room, "orphan-event-start").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the orphan event transaction must start");
    let (game_id, room_id, actor_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
            SELECT id, room_id, started_by_participant_id
            FROM games
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the integrity test game must be queryable");
    insert_test_event(
        &mut transaction,
        game_id,
        room_id,
        uuid::Uuid::new_v4(),
        actor_id,
    )
    .await;

    let error = transaction
        .commit()
        .await
        .expect_err("the orphan event must be rejected at commit");
    assert_database_error_code(&error, "23514");
    assert!(error.to_string().contains("receipt"));
}

#[tokio::test]
async fn a_receipt_must_identify_the_actor_and_version_of_its_event() {
    let room = ready_room().await;
    let other_room = ready_room().await;
    start_ready_game(&room, "receipt-event-start").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the receipt integrity transaction must start");
    let (game_id, actor_id, room_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
            SELECT id, started_by_participant_id, room_id
            FROM games
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the integrity test game must be queryable");
    let other_actor = sqlx::query_scalar::<_, uuid::Uuid>(
        r"
        SELECT participants.id
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
          AND participants.role = 'host'
        ",
    )
    .bind(&other_room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the unrelated actor must exist");
    let command_id = uuid::Uuid::new_v4();
    insert_test_event(&mut transaction, game_id, room_id, command_id, actor_id).await;

    let error = sqlx::query(
        r"
        INSERT INTO game_command_receipts (
            game_id,
            room_id,
            command_id,
            actor_participant_id,
            command_type,
            expected_state_version,
            payload_digest,
            accepted_state_version,
            accepted_sequence,
            expires_at
        )
        VALUES (
            $1,
            $2,
            $3,
            $4,
            'end_hero_actions',
            1,
            'blake3:0000000000000000000000000000000000000000000000000000000000000000',
            2,
            1,
            clock_timestamp() + INTERVAL '7 days'
        )
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(command_id)
    .bind(other_actor)
    .execute(&mut *transaction)
    .await
    .expect_err("a receipt must not claim a different event actor");
    assert_database_error_code(&error, "23503");
    transaction
        .rollback()
        .await
        .expect("the receipt integrity transaction must roll back");
}

#[tokio::test]
async fn v4_events_require_the_matching_command_type_at_commit() {
    let turn_source = ready_room().await;
    start_ready_game(&turn_source, "turn-event-command-pair-source").await;
    let turn_initial_snapshot = current_game_snapshot(&turn_source).await;
    let turn = turn_source
        .app
        .clone()
        .oneshot(command_request(
            &turn_source.host_cookie,
            uuid::Uuid::new_v4(),
            1,
        ))
        .await
        .expect("the turn command must receive a response");
    assert_eq!(turn.status(), StatusCode::OK);
    let turn_target = ready_room().await;
    assert_v4_event_rejects_wrong_command_type(
        &turn_source,
        &turn_target,
        &turn_initial_snapshot,
        "resolve_choice",
    )
    .await;

    let choice_source = ready_room_with_manifest(each_hero_choice_manifest()).await;
    let started = start_ready_game(&choice_source, "choice-event-command-pair-source").await;
    let choice_id = started["choice"]["id"]
        .as_str()
        .expect("the initial participant choice must be present");
    let choice_initial_snapshot = current_game_snapshot(&choice_source).await;
    let choice = choice_source
        .app
        .clone()
        .oneshot(resolve_choice_request(
            &choice_source.host_cookie,
            uuid::Uuid::new_v4(),
            1,
            choice_id,
            &["option:1"],
        ))
        .await
        .expect("the choice command must receive a response");
    assert_eq!(choice.status(), StatusCode::OK);
    let choice_target = ready_room_with_manifest(each_hero_choice_manifest()).await;
    assert_v4_event_rejects_wrong_command_type(
        &choice_source,
        &choice_target,
        &choice_initial_snapshot,
        "end_hero_actions",
    )
    .await;
}

#[tokio::test]
async fn unsupported_persisted_versions_are_rejected_at_the_database_boundary() {
    let room = ready_room().await;
    start_ready_game(&room, "persisted-version-start").await;

    let mut snapshot_transaction = room
        .database
        .begin()
        .await
        .expect("the Snapshot version transaction must start");
    let error = sqlx::query(
        r"
        UPDATE games
        SET snapshot_version = 999
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(&room.room_code)
    .execute(&mut *snapshot_transaction)
    .await
    .expect_err("an unsupported persisted Snapshot version must be rejected");
    assert_database_error_code(&error, "23514");
    snapshot_transaction
        .rollback()
        .await
        .expect("the Snapshot version transaction must roll back");

    let mut event_transaction = room
        .database
        .begin()
        .await
        .expect("the event version transaction must start");
    let (game_id, room_id, actor_id) = sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
        r"
            UPDATE games
            SET sequence = 1,
                state_version = 2
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            RETURNING id, room_id, started_by_participant_id
            ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *event_transaction)
    .await
    .expect("the test game cursor must advance inside the transaction");
    let error = sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_version,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, 1, 999, 'turn_completed', $3, $4, 2, '{}'::jsonb)
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(uuid::Uuid::new_v4())
    .bind(actor_id)
    .execute(&mut *event_transaction)
    .await
    .expect_err("an unsupported persisted event version must be rejected");
    assert_database_error_code(&error, "23514");
    event_transaction
        .rollback()
        .await
        .expect("the event version transaction must roll back");
}

#[tokio::test]
async fn identical_command_retries_return_the_original_receipt_without_duplicate_effects() {
    let room = ready_room().await;
    start_ready_game(&room, "idempotent-command-start").await;
    let command_id = uuid::Uuid::new_v4();

    let first_response = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, command_id, 1))
        .await
        .expect("the original command must receive a response");
    assert_eq!(first_response.status(), StatusCode::OK);
    let first = response_json(first_response).await;

    let retry_response = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, command_id, 1))
        .await
        .expect("the identical retry must receive a response");
    assert_eq!(retry_response.status(), StatusCode::OK);
    let retry = response_json(retry_response).await;

    assert_eq!(retry["receipt"], first["receipt"]);
    assert_eq!(retry["projection"]["snapshot"]["state_version"], 2);
    assert_eq!(retry["projection"]["snapshot"]["sequence"], 1);

    let artifacts = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        r"
        SELECT
            games.state_version,
            games.sequence,
            (SELECT count(*) FROM game_events WHERE game_id = games.id),
            (SELECT count(*) FROM game_command_receipts WHERE game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the idempotent command artifacts must remain queryable");
    assert_eq!(artifacts, (2, 1, 1, 1));
}

#[tokio::test]
async fn concurrent_identical_commands_share_one_receipt_and_one_effect() {
    let room = ready_room().await;
    start_ready_game(&room, "concurrent-idempotent-command-start").await;
    let command_id = uuid::Uuid::new_v4();
    let barrier = Arc::new(Barrier::new(2));

    let first_barrier = Arc::clone(&barrier);
    let first_app = room.app.clone();
    let first_cookie = room.host_cookie.clone();
    let first = async move {
        first_barrier.wait().await;
        first_app
            .oneshot(command_request(&first_cookie, command_id, 1))
            .await
            .expect("the first concurrent retry must receive a response")
    };
    let second_barrier = Arc::clone(&barrier);
    let second_app = room.app.clone();
    let second_cookie = room.host_cookie.clone();
    let second = async move {
        second_barrier.wait().await;
        second_app
            .oneshot(command_request(&second_cookie, command_id, 1))
            .await
            .expect("the second concurrent retry must receive a response")
    };
    let (first_response, second_response) = tokio::join!(first, second);

    assert_eq!(first_response.status(), StatusCode::OK);
    assert_eq!(second_response.status(), StatusCode::OK);
    let first = response_json(first_response).await;
    let second = response_json(second_response).await;
    assert_eq!(first["receipt"], second["receipt"]);

    let artifacts = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        r"
        SELECT
            games.state_version,
            games.sequence,
            (SELECT count(*) FROM game_events WHERE game_id = games.id),
            (SELECT count(*) FROM game_command_receipts WHERE game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the concurrent idempotent artifacts must remain queryable");
    assert_eq!(artifacts, (2, 1, 1, 1));
}

#[tokio::test]
async fn reusing_a_command_id_with_another_payload_is_rejected() {
    let room = ready_room().await;
    start_ready_game(&room, "command-payload-conflict-start").await;
    let command_id = uuid::Uuid::new_v4();

    let accepted = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, command_id, 1))
        .await
        .expect("the original command must receive a response");
    assert_eq!(accepted.status(), StatusCode::OK);

    let conflict = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, command_id, 2))
        .await
        .expect("the conflicting retry must receive a response");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(conflict).await["error"]["code"],
        "IDEMPOTENCY_KEY_REUSED"
    );

    let artifacts = sqlx::query_as::<_, (i64, i64)>(
        r"
        SELECT
            (SELECT count(*) FROM game_events WHERE game_id = games.id),
            (SELECT count(*) FROM game_command_receipts WHERE game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the original command artifacts must remain queryable");
    assert_eq!(artifacts, (1, 1));
}

#[tokio::test]
async fn commands_for_the_same_state_version_have_one_acceptance_and_one_stale_result() {
    let room = ready_room().await;
    start_ready_game(&room, "same-version-race-start").await;
    let first_command_id = uuid::Uuid::new_v4();
    let second_command_id = uuid::Uuid::new_v4();
    let barrier = Arc::new(Barrier::new(2));

    let first_barrier = Arc::clone(&barrier);
    let first_app = room.app.clone();
    let first_cookie = room.host_cookie.clone();
    let first = async move {
        first_barrier.wait().await;
        first_app
            .oneshot(command_request(&first_cookie, first_command_id, 1))
            .await
            .expect("the first competing command must receive a response")
    };
    let second_barrier = Arc::clone(&barrier);
    let second_app = room.app.clone();
    let second_cookie = room.host_cookie.clone();
    let second = async move {
        second_barrier.wait().await;
        second_app
            .oneshot(command_request(&second_cookie, second_command_id, 1))
            .await
            .expect("the second competing command must receive a response")
    };
    let (first_response, second_response) = tokio::join!(first, second);

    let responses = [first_response, second_response];
    let mut accepted = 0;
    let mut stale = 0;
    for response in responses {
        match response.status() {
            StatusCode::OK => accepted += 1,
            StatusCode::CONFLICT => {
                let body = response_json(response).await;
                assert_eq!(body["error"]["code"], "STALE_STATE_VERSION");
                stale += 1;
            }
            status => panic!("unexpected competing command status: {status}"),
        }
    }
    assert_eq!((accepted, stale), (1, 1));

    let artifacts = sqlx::query_as::<_, (i64, i64)>(
        r"
        SELECT
            (SELECT count(*) FROM game_events WHERE game_id = games.id),
            (SELECT count(*) FROM game_command_receipts WHERE game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the winning command artifacts must remain queryable");
    assert_eq!(artifacts, (1, 1));
}

async fn non_current_device_session_id(room: &ReadyRoom) -> String {
    let sessions = room
        .app
        .clone()
        .oneshot(list_device_sessions_request(&room.host_cookie))
        .await
        .expect("the device session list must receive a response");
    assert_eq!(sessions.status(), StatusCode::OK);
    response_json(sessions).await["sessions"]
        .as_array()
        .expect("device sessions must be an array")
        .iter()
        .find(|session| session["current"] == false)
        .and_then(|session| session["id"].as_str())
        .expect("the second device session must be listed")
        .to_owned()
}

async fn wait_for_race_to_reach_game_lock(database: &PgPool, revocation_key: &str) {
    let observation_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let completed_revocation = sqlx::query_scalar::<_, bool>(
            r"
            SELECT EXISTS(
                SELECT 1
                FROM device_session_revocation_requests
                WHERE idempotency_key = $1
                  AND completed_at IS NOT NULL
            )
            ",
        )
        .bind(revocation_key)
        .fetch_one(database)
        .await
        .expect("the revocation receipt must be observable");
        let waiting_requests = sqlx::query_scalar::<_, i64>(
            r"
            SELECT COUNT(*)
            FROM pg_stat_activity
            WHERE datname = current_database()
              AND pid <> pg_backend_pid()
              AND wait_event_type = 'Lock'
            ",
        )
        .fetch_one(database)
        .await
        .expect("waiting requests must be observable");
        if completed_revocation || waiting_requests >= 2 {
            return;
        }
        assert!(
            Instant::now() < observation_deadline,
            "the racing requests must reach the database fence"
        );
        tokio::task::yield_now().await;
    }
}

async fn assert_revocation_command_order(
    room: &ReadyRoom,
    command_id: uuid::Uuid,
    command_status: StatusCode,
) {
    let ordering = sqlx::query_as::<_, (i64, bool)>(
        r"
        SELECT
            COUNT(receipts.command_id),
            COALESCE(bool_and(receipts.created_at <= events.created_at), TRUE)
        FROM identity_security_events AS events
        JOIN rooms ON rooms.id = events.room_id
        LEFT JOIN game_command_receipts AS receipts
          ON receipts.room_id = rooms.id
         AND receipts.command_id = $2
        WHERE rooms.code = $1
          AND events.event_type = 'session_revoked'
        ",
    )
    .bind(&room.room_code)
    .bind(command_id)
    .fetch_one(&room.database)
    .await
    .expect("the committed command and revocation order must be queryable");
    assert!(
        ordering.1,
        "an accepted command must commit before the revocation event"
    );
    assert_eq!(ordering.0, i64::from(command_status == StatusCode::OK));
}

async fn wait_for_requests_blocked_by(database: &PgPool, blocker_pid: i32, minimum: i64) {
    let observation_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let waiting_requests = sqlx::query_scalar::<_, i64>(
            r"
            WITH RECURSIVE blocked(pid) AS (
                SELECT $1::INTEGER
                UNION
                SELECT activity.pid
                FROM pg_stat_activity AS activity
                JOIN blocked ON blocked.pid = ANY(pg_blocking_pids(activity.pid))
            )
            SELECT COUNT(*) - 1 FROM blocked
            ",
        )
        .bind(blocker_pid)
        .fetch_one(database)
        .await
        .expect("requests blocked by the database fence must be observable");
        if waiting_requests >= minimum {
            return;
        }
        assert!(
            Instant::now() < observation_deadline,
            "expected {minimum} requests blocked by the database fence, observed {waiting_requests}"
        );
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn device_session_revocation_and_a_command_linearize_at_the_game_lock() {
    let room = ready_room().await;
    start_ready_game(&room, "revocation-command-race-start").await;
    let second_host_cookie = additional_session_for_participant(&room, "host").await;
    let second_session_id = non_current_device_session_id(&room).await;
    let command_id = uuid::Uuid::new_v4();
    let revocation_key = unique_key("revocation-command-race");

    let mut fence = room
        .database
        .begin()
        .await
        .expect("the linearization fence must begin");
    sqlx::query(
        r"
        SELECT games.id
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        FOR UPDATE OF games
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *fence)
    .await
    .expect("the game root must be lockable");

    let barrier = Arc::new(Barrier::new(3));
    let command = {
        let app = room.app.clone();
        let barrier = barrier.clone();
        let cookie = second_host_cookie.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            app.oneshot(command_request(&cookie, command_id, 1))
                .await
                .expect("the racing command must receive a response")
        })
    };
    let revocation = {
        let app = room.app.clone();
        let barrier = barrier.clone();
        let cookie = room.host_cookie.clone();
        let session_id = second_session_id.clone();
        let key = revocation_key.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            app.oneshot(revoke_device_session_request(&cookie, &session_id, &key))
                .await
                .expect("the racing revocation must receive a response")
        })
    };
    barrier.wait().await;
    wait_for_race_to_reach_game_lock(&room.database, &revocation_key).await;

    fence
        .commit()
        .await
        .expect("the linearization fence must release");
    let command = command.await.expect("the command task must finish");
    let revocation = revocation.await.expect("the revocation task must finish");
    assert_eq!(revocation.status(), StatusCode::OK);
    assert!(
        command.status() == StatusCode::OK || command.status() == StatusCode::UNAUTHORIZED,
        "the command must commit before revocation or lose authorization after it"
    );

    assert_revocation_command_order(&room, command_id, command.status()).await;

    let post_commit_command = room
        .app
        .clone()
        .oneshot(command_request(
            &second_host_cookie,
            uuid::Uuid::new_v4(),
            if command.status() == StatusCode::OK {
                2
            } else {
                1
            },
        ))
        .await
        .expect("the post-revocation command must receive a response");
    assert_eq!(post_commit_command.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(post_commit_command).await["error"]["code"],
        "SESSION_INVALID"
    );
}

async fn lock_game_fence(room: &ReadyRoom) -> (sqlx::Transaction<'_, sqlx::Postgres>, i32) {
    let mut fence = room
        .database
        .begin()
        .await
        .expect("the replacement linearization fence must begin");
    let fence_pid = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
        .fetch_one(&mut *fence)
        .await
        .expect("the replacement fence backend must be identifiable");
    sqlx::query(
        r"
        SELECT games.id
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        FOR UPDATE OF games
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *fence)
    .await
    .expect("the replacement game root must be lockable");
    (fence, fence_pid)
}

async fn assert_replacement_command_order(
    room: &ReadyRoom,
    recovery_attempt_id: uuid::Uuid,
    command_id: uuid::Uuid,
    command_status: StatusCode,
) {
    let ordering = sqlx::query_as::<_, (i64, bool)>(
        r"
        SELECT
            COUNT(receipts.command_id),
            COALESCE(bool_and(receipts.created_at <= credentials.consumed_at), TRUE)
        FROM recovery_credentials AS credentials
        JOIN participants ON participants.id = credentials.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        LEFT JOIN game_command_receipts AS receipts
          ON receipts.room_id = rooms.id
         AND receipts.command_id = $3
        WHERE rooms.code = $1
          AND credentials.recovery_attempt_id = $2
        ",
    )
    .bind(&room.room_code)
    .bind(recovery_attempt_id)
    .bind(command_id)
    .fetch_one(&room.database)
    .await
    .expect("the committed command and replacement order must be queryable");
    assert!(
        ordering.1,
        "an accepted command must commit before the device replacement"
    );
    assert_eq!(ordering.0, i64::from(command_status == StatusCode::OK));
}

async fn pause_revocation_across_game_start<'a>(
    room: &'a ReadyRoom,
    session_id: &str,
) -> (
    tokio::task::JoinHandle<Response<Body>>,
    sqlx::Transaction<'a, sqlx::Postgres>,
    i32,
) {
    let mut room_fence = room
        .database
        .begin()
        .await
        .expect("the room fence must begin");
    let room_pid = sqlx::query_scalar::<_, i32>(
        "SELECT pg_backend_pid() FROM rooms WHERE code = $1 FOR UPDATE",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *room_fence)
    .await
    .expect("the lobby root must be locked before the game exists");
    let mut device_fence = room
        .database
        .begin()
        .await
        .expect("the device fence must begin");
    let device_pid = sqlx::query_scalar::<_, i32>(
        "SELECT pg_backend_pid() FROM device_sessions WHERE id = $1 FOR UPDATE",
    )
    .bind(uuid::Uuid::parse_str(session_id).expect("the device ID must be a UUID"))
    .fetch_one(&mut *device_fence)
    .await
    .expect("the target device must be locked before revocation");

    let start = {
        let app = room.app.clone();
        let request = start_request(
            &room.host_cookie,
            &unique_key("access-start-race"),
            &room.manifest,
            "adventure:001",
        );
        tokio::spawn(async move { app.oneshot(request).await.expect("game start must respond") })
    };
    wait_for_requests_blocked_by(&room.database, room_pid, 1).await;
    let revocation = {
        let app = room.app.clone();
        let request = revoke_device_session_request(
            &room.host_cookie,
            session_id,
            &unique_key("access-start-race-revoke"),
        );
        tokio::spawn(async move { app.oneshot(request).await.expect("revocation must respond") })
    };
    wait_for_requests_blocked_by(&room.database, room_pid, 2).await;
    room_fence
        .commit()
        .await
        .expect("the lobby fence must release");
    assert_eq!(
        start.await.expect("game start must finish").status(),
        StatusCode::CREATED
    );
    wait_for_requests_blocked_by(&room.database, device_pid, 1).await;
    (revocation, device_fence, device_pid)
}

#[tokio::test]
async fn revocation_waiting_for_game_start_locks_the_new_game_before_invalidating_access() {
    let room = ready_room().await;
    let second_cookie = additional_session_for_participant(&room, "host").await;
    let second_session_id = non_current_device_session_id(&room).await;
    let (revocation, device_fence, device_pid) =
        pause_revocation_across_game_start(&room, &second_session_id).await;

    let command = {
        let app = room.app.clone();
        tokio::spawn(async move {
            app.oneshot(command_request(&second_cookie, uuid::Uuid::new_v4(), 1))
                .await
                .expect("the command during revocation must respond")
        })
    };
    wait_for_requests_blocked_by(&room.database, device_pid, 2).await;
    device_fence
        .commit()
        .await
        .expect("the device fence must release");
    assert_eq!(
        revocation.await.expect("revocation must finish").status(),
        StatusCode::OK
    );
    let command = command.await.expect("the command must finish");
    assert_eq!(command.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(command).await["error"]["code"],
        "SESSION_INVALID"
    );
}

#[tokio::test]
async fn device_replacement_and_a_command_linearize_at_the_game_lock() {
    let room = ready_room().await;
    start_ready_game(&room, "replacement-command-race-start").await;
    let second_host_cookie = additional_session_for_participant(&room, "host").await;
    let second_session_id = non_current_device_session_id(&room).await;
    let recovery_attempt_id = uuid::Uuid::new_v4();
    let candidates = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &json!({
                "recovery_token": room.host_recovery_token,
                "recovery_password": "a long uncommon passphrase",
                "recovery_attempt_id": recovery_attempt_id.to_string()
            }),
            None,
            None,
        ))
        .await
        .expect("replacement discovery must receive a response");
    assert_eq!(candidates.status(), StatusCode::CONFLICT);

    let command_id = uuid::Uuid::new_v4();
    let (fence, fence_pid) = lock_game_fence(&room).await;

    let barrier = Arc::new(Barrier::new(3));
    let command = {
        let app = room.app.clone();
        let barrier = barrier.clone();
        let cookie = second_host_cookie.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            app.oneshot(command_request(&cookie, command_id, 1))
                .await
                .expect("the command racing replacement must receive a response")
        })
    };
    let replacement = {
        let app = room.app.clone();
        let barrier = barrier.clone();
        let recovery_token = room.host_recovery_token.clone();
        let replacement_session_id = second_session_id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            app.oneshot(json_request(
                "POST",
                "/api/session/recover",
                &json!({
                    "recovery_token": recovery_token,
                    "recovery_password": "a long uncommon passphrase",
                    "recovery_attempt_id": recovery_attempt_id.to_string(),
                    "replace_session_id": replacement_session_id
                }),
                None,
                None,
            ))
            .await
            .expect("the racing replacement must receive a response")
        })
    };
    barrier.wait().await;
    wait_for_requests_blocked_by(&room.database, fence_pid, 2).await;

    fence
        .commit()
        .await
        .expect("the replacement linearization fence must release");
    let command = command.await.expect("the command task must finish");
    let replacement = replacement.await.expect("the replacement task must finish");
    assert_eq!(replacement.status(), StatusCode::OK);
    assert!(
        command.status() == StatusCode::OK || command.status() == StatusCode::UNAUTHORIZED,
        "the command must commit before replacement or lose authorization after it"
    );

    assert_replacement_command_order(&room, recovery_attempt_id, command_id, command.status())
        .await;

    let post_replacement_command = room
        .app
        .clone()
        .oneshot(command_request(
            &second_host_cookie,
            uuid::Uuid::new_v4(),
            if command.status() == StatusCode::OK {
                2
            } else {
                1
            },
        ))
        .await
        .expect("the post-replacement command must receive a response");
    assert_eq!(post_replacement_command.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(post_replacement_command).await["error"]["code"],
        "SESSION_INVALID"
    );
}

#[tokio::test]
async fn a_locked_game_does_not_block_a_command_for_another_game() {
    let first_room = ready_room().await;
    let second_room = ready_room().await;
    let first_game = start_ready_game(&first_room, "locked-first-game-start").await;
    start_ready_game(&second_room, "parallel-second-game-start").await;
    let first_game_id = uuid::Uuid::parse_str(
        first_game["game"]["id"]
            .as_str()
            .expect("the first game id must be present"),
    )
    .expect("the first game id must be valid");

    let mut blocker = first_room
        .database
        .begin()
        .await
        .expect("the blocking transaction must begin");
    let blocker_process_id = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .expect("the blocking database process must be identifiable");
    sqlx::query("SELECT id FROM games WHERE id = $1 FOR UPDATE")
        .bind(first_game_id)
        .execute(&mut *blocker)
        .await
        .expect("the first game row must be locked");

    let blocked_app = first_room.app.clone();
    let blocked_cookie = first_room.host_cookie.clone();
    let blocked_command = tokio::spawn(async move {
        blocked_app
            .oneshot(command_request(&blocked_cookie, uuid::Uuid::new_v4(), 1))
            .await
            .expect("the blocked command must receive a response after release")
    });

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting = sqlx::query_scalar::<_, bool>(
                r"
                SELECT EXISTS (
                    SELECT 1
                    FROM pg_stat_activity
                    WHERE datname = current_database()
                      AND $1 = ANY(pg_blocking_pids(pid))
                      AND wait_event_type = 'Lock'
                      AND query LIKE '%FOR UPDATE OF games%'
                )
                ",
            )
            .bind(blocker_process_id)
            .fetch_one(&first_room.database)
            .await
            .expect("the lock wait must be observable");
            if waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the first command must wait on the locked game");

    let parallel_response = tokio::time::timeout(
        Duration::from_secs(5),
        second_room.app.clone().oneshot(command_request(
            &second_room.host_cookie,
            uuid::Uuid::new_v4(),
            1,
        )),
    )
    .await
    .expect("another game must continue while the first game is locked")
    .expect("the parallel command must receive a response");
    assert_eq!(parallel_response.status(), StatusCode::OK);

    blocker
        .rollback()
        .await
        .expect("the first game lock must be released");
    let blocked_response = tokio::time::timeout(Duration::from_secs(5), blocked_command)
        .await
        .expect("the first command must finish after its lock is released")
        .expect("the blocked command task must not panic");
    assert_eq!(blocked_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn rejected_or_stale_intentions_leave_no_official_artifacts() {
    let room = ready_room().await;
    let start_response = room
        .app
        .clone()
        .oneshot(start_request(
            &room.host_cookie,
            &unique_key("rejected-command-start"),
            &room.manifest,
            "adventure:001",
        ))
        .await
        .expect("game start must receive a response");
    assert_eq!(start_response.status(), StatusCode::CREATED);

    let guest_response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": uuid::Uuid::new_v4().to_string(),
                "expected_state_version": 1,
                "type": "end_hero_actions"
            }),
            Some(&room.guest_cookie),
            None,
        ))
        .await
        .expect("the unauthorized command must receive a response");
    assert_eq!(guest_response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(guest_response).await["error"]["code"],
        "GAME_ACTION_NOT_ALLOWED"
    );

    let stale_command_id = uuid::Uuid::new_v4();
    let stale_response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": stale_command_id.to_string(),
                "expected_state_version": 2,
                "type": "end_hero_actions"
            }),
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the stale command must receive a response");
    assert_eq!(stale_response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(stale_response).await["error"]["code"],
        "STALE_STATE_VERSION"
    );

    let artifacts = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        r"
        SELECT
            games.state_version,
            games.sequence,
            (SELECT count(*) FROM game_events WHERE game_id = games.id),
            (SELECT count(*) FROM game_command_receipts WHERE game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the unchanged game must remain queryable");
    assert_eq!(artifacts, (1, 0, 0, 0));

    let missing_receipt = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/games/current/commands/{stale_command_id}"))
                .header(header::COOKIE, &room.host_cookie)
                .body(Body::empty())
                .expect("the receipt lookup request must be valid"),
        )
        .await
        .expect("the missing receipt lookup must receive a response");
    assert_eq!(missing_receipt.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_expired_game_rejects_the_command_at_the_database_clock_boundary() {
    let room = ready_room().await;
    let start_response = room
        .app
        .clone()
        .oneshot(start_request(
            &room.host_cookie,
            &unique_key("expired-command-start"),
            &room.manifest,
            "adventure:001",
        ))
        .await
        .expect("game start must receive a response");
    assert_eq!(start_response.status(), StatusCode::CREATED);
    sqlx::query(
        r"
        UPDATE games
        SET
            last_game_action_at = clock_timestamp() - INTERVAL '8 days',
            expires_at = clock_timestamp() - INTERVAL '1 day'
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        ",
    )
    .bind(&room.room_code)
    .execute(&room.database)
    .await
    .expect("the test game must be expired");

    let projection_response = room
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, &room.host_cookie)
                .body(Body::empty())
                .expect("the session request must be valid"),
        )
        .await
        .expect("the expired game projection must receive a response");
    assert_eq!(projection_response.status(), StatusCode::OK);
    assert_eq!(
        response_json(projection_response).await["legal_actions"],
        json!([]),
        "an expired game must not advertise a command that the server rejects"
    );

    let response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": uuid::Uuid::new_v4().to_string(),
                "expected_state_version": 1,
                "type": "end_hero_actions"
            }),
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the expired command must receive a response");

    assert_eq!(response.status(), StatusCode::GONE);
    assert_eq!(
        response_json(response).await["error"]["code"],
        "GAME_EXPIRED"
    );
}

struct RawWebSocket {
    stream: tokio::net::TcpStream,
    buffered: Vec<u8>,
}

impl RawWebSocket {
    async fn read_text(&mut self) -> String {
        loop {
            let (opcode, payload) = self.read_frame().await;
            match opcode {
                1 => {
                    let serialized =
                        String::from_utf8(payload).expect("text frames must contain UTF-8");
                    let is_presence = serde_json::from_str::<Value>(&serialized)
                        .is_ok_and(|message| message["type"] == "presence");
                    if !is_presence {
                        return serialized;
                    }
                }
                8 => panic!("the WebSocket closed before a text message arrived"),
                9 | 10 => {}
                other => panic!("unexpected WebSocket opcode {other}"),
            }
        }
    }

    async fn read_presence(&mut self) -> Value {
        loop {
            let (opcode, payload) = self.read_frame().await;
            match opcode {
                1 => {
                    let message: Value = serde_json::from_slice(&payload)
                        .expect("realtime text messages must contain JSON");
                    if message["type"] == "presence" {
                        return message;
                    }
                }
                8 => panic!("the WebSocket closed before presence arrived"),
                9 | 10 => {}
                other => panic!("unexpected WebSocket opcode {other}"),
            }
        }
    }

    async fn read_close_code(&mut self) -> u16 {
        loop {
            let (opcode, payload) = self.read_frame().await;
            match opcode {
                8 => {
                    assert!(
                        payload.len() >= 2,
                        "the close frame must contain a status code"
                    );
                    return u16::from_be_bytes([payload[0], payload[1]]);
                }
                1 | 9 | 10 => {}
                other => panic!("expected a close frame, received opcode {other}"),
            }
        }
    }

    async fn read_frame(&mut self) -> (u8, Vec<u8>) {
        let header = self.read_exact(2).await;
        let opcode = header[0] & 0x0f;
        assert_eq!(header[1] & 0x80, 0, "server frames must not be masked");
        let mut length = u64::from(header[1] & 0x7f);
        if length == 126 {
            let extended = self.read_exact(2).await;
            length = u64::from(u16::from_be_bytes([extended[0], extended[1]]));
        } else if length == 127 {
            let extended = self.read_exact(8).await;
            length = u64::from_be_bytes(
                extended
                    .try_into()
                    .expect("a 64-bit WebSocket length must contain eight bytes"),
            );
        }
        let payload = self
            .read_exact(usize::try_from(length).expect("test frames must fit in memory"))
            .await;
        (opcode, payload)
    }

    async fn read_exact(&mut self, length: usize) -> Vec<u8> {
        while self.buffered.len() < length {
            let mut chunk = [0_u8; 4096];
            let read = self
                .stream
                .read(&mut chunk)
                .await
                .expect("the WebSocket frame must be readable");
            assert!(read > 0, "the socket closed while a frame was being read");
            self.buffered.extend_from_slice(&chunk[..read]);
        }
        self.buffered.drain(..length).collect()
    }
}

async fn start_network_server(
    app: axum::Router,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the test server must bind");
    let address = listener
        .local_addr()
        .expect("the test server must have an address");
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("the test server must run");
    });
    (address, task)
}

async fn websocket_handshake(
    address: std::net::SocketAddr,
    path: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
    protocol: Option<&str>,
) -> (u16, String, RawWebSocket) {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("the WebSocket client must connect");
    let mut request = format!(
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    if let Some(cookie) = cookie {
        write!(request, "Cookie: {cookie}\r\n").expect("writing to a String cannot fail");
    }
    if let Some(origin) = origin {
        write!(request, "Origin: {origin}\r\n").expect("writing to a String cannot fail");
    }
    if let Some(protocol) = protocol {
        write!(request, "Sec-WebSocket-Protocol: {protocol}\r\n")
            .expect("writing to a String cannot fail");
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("the WebSocket handshake must be writable");

    let mut response = Vec::new();
    let header_end = loop {
        if let Some(index) = response.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .await
            .expect("the WebSocket handshake must be readable");
        assert!(read > 0, "the handshake response ended before its headers");
        response.extend_from_slice(&chunk[..read]);
    };
    let headers = String::from_utf8(response[..header_end].to_vec())
        .expect("the handshake headers must be ASCII");
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .expect("the handshake must contain a status code");
    let buffered = response[header_end..].to_vec();

    (status, headers, RawWebSocket { stream, buffered })
}

async fn additional_session_for_participant(room: &ReadyRoom, participant_role: &str) -> String {
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let guest_session_id = uuid::Uuid::new_v4();
    sqlx::query(
        r"
        INSERT INTO guest_sessions (id, guest_identity_id, token_digest)
        SELECT
            $1,
            participants.guest_identity_id,
            'sha256:' || encode(sha256(convert_to($2, 'UTF8')), 'hex')
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $3
          AND participants.role = $4
        ",
    )
    .bind(guest_session_id)
    .bind(&token)
    .bind(&room.room_code)
    .bind(participant_role)
    .execute(&room.database)
    .await
    .expect("the additional guest session must be inserted");
    sqlx::query(
        r"
        INSERT INTO device_sessions (id, guest_session_id, participant_id, slot)
        SELECT $1, $2, participants.id, 2
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $3
          AND participants.role = $4
        ",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(guest_session_id)
    .bind(&room.room_code)
    .bind(participant_role)
    .execute(&room.database)
    .await
    .expect("the additional device session must be inserted");

    format!("__Host-session={token}")
}

fn participant_presence(message: &Value, position: usize) -> &str {
    message["participants"]
        .as_array()
        .and_then(|participants| {
            participants
                .iter()
                .find(|participant| participant["position"] == position)
        })
        .and_then(|participant| participant["status"].as_str())
        .expect("the participant presence must be present")
}

fn assert_public_choice_resolution_event(
    serialized: &str,
    first_choice_id: &str,
    expected_choice: &Value,
) {
    let event_batch: Value =
        serde_json::from_str(serialized).expect("the choice event batch must be JSON");
    let event = &event_batch["events"][0];
    assert_eq!(event["event_version"], 5);
    assert_eq!(event["type"], "choice_resolved");
    assert_eq!(event["choice_id"], first_choice_id);
    assert_eq!(event["choice_cause"], "rule:functional");
    assert_eq!(event["selected_options"], json!(["option:1"]));
    assert_eq!(event["steps"].as_array().map(Vec::len), Some(1));
    assert_eq!(event["steps"][0]["phase"], "dark_arts");
    assert_eq!(
        event["control"]["decision_point"]["choice"],
        *expected_choice
    );
    assert!(event.get("command_id").is_none());
    assert_no_private_choice_continuation(serialized);
}

fn assert_reconnected_choice_snapshot(serialized: &str, expected_choice: &Value) {
    let snapshot: Value =
        serde_json::from_str(serialized).expect("the reconnect Snapshot must be JSON");
    assert_eq!(snapshot["type"], "snapshot");
    assert_eq!(snapshot["projection"]["choice"], *expected_choice);
    assert_eq!(
        snapshot["projection"]["legal_actions"],
        json!(["resolve_choice"])
    );
    assert_eq!(snapshot["projection"]["turn"]["active_position"], 1);
    assert_no_private_choice_continuation(serialized);
}

fn assert_no_private_choice_continuation(serialized: &str) {
    for private_field in [
        "continuation",
        "choice_cursor",
        "queued_effects",
        "steps_completed",
    ] {
        assert!(!serialized.contains(private_field));
    }
}

fn assert_realtime_turn_completed(event: &Value) {
    assert_eq!(event["event_version"], 5);
    assert_eq!(event["type"], "turn_completed");
    assert_eq!(
        event["steps"]
            .as_array()
            .expect("the realtime turn steps must be present")
            .iter()
            .map(|step| step["phase"].as_str())
            .collect::<Vec<_>>(),
        vec![Some("end_turn"), Some("dark_arts"), Some("villains")]
    );
}

async fn current_official_state(room: &ReadyRoom) -> (i64, i64, String, i64) {
    sqlx::query_as::<_, (i64, i64, String, i64)>(
        r"
        SELECT
            games.state_version,
            games.sequence,
            games.expires_at::text,
            (SELECT count(*) FROM game_events WHERE game_events.game_id = games.id)
        FROM games
        JOIN rooms ON rooms.id = games.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(&room.room_code)
    .fetch_one(&room.database)
    .await
    .expect("the official state must be queryable")
}

async fn connect_current_game(address: std::net::SocketAddr, cookie: &str) -> RawWebSocket {
    let (status, _, mut socket) = websocket_handshake(
        address,
        "/api/games/current/events",
        Some(cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v2"),
    )
    .await;
    assert_eq!(status, 101);
    let _ = socket.read_text().await;
    socket
}

#[tokio::test]
async fn websocket_handshake_requires_the_session_exact_origin_and_current_protocol() {
    let room = ready_room().await;
    let projection = start_ready_game(&room, "realtime-handshake").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let path = realtime_path(&projection);

    let (status, _, _) = websocket_handshake(
        address,
        &path,
        Some(&room.host_cookie),
        Some("https://attacker.invalid"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 403);

    let (status, _, _) = websocket_handshake(
        address,
        &path,
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v0"),
    )
    .await;
    assert_eq!(status, 426);

    let (status, _, _) = websocket_handshake(
        address,
        &path,
        None,
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 401);

    let (status, headers, _) = websocket_handshake(
        address,
        &path,
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("sec-websocket-protocol: hogwarts.realtime.v1")
    );

    let (status, headers, _) = websocket_handshake(
        address,
        &path,
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v2"),
    )
    .await;
    assert_eq!(status, 101);
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("sec-websocket-protocol: hogwarts.realtime.v2")
    );
    server.abort();
}

#[tokio::test]
async fn a_websocket_opened_during_shutdown_closes_without_waiting_for_another_signal() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-shutdown").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    room.state.begin_shutdown();

    let (status, _, mut socket) = websocket_handshake(
        address,
        "/api/games/current/events?snapshot_version=1",
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;

    assert_eq!(status, 101);
    let close_code =
        tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_close_code())
            .await
            .expect("shutdown must close the WebSocket promptly");
    assert_eq!(close_code, 1012);
    server.abort();
}

#[tokio::test]
async fn shutdown_closes_an_active_websocket_with_a_restart_code() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-active-shutdown").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let (status, _, mut socket) = websocket_handshake(
        address,
        "/api/games/current/events?snapshot_version=1",
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_text())
        .await
        .expect("the initial snapshot must arrive");

    room.state.begin_shutdown();

    let close_code =
        tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_close_code())
            .await
            .expect("shutdown must close an active WebSocket promptly");
    assert_eq!(close_code, 1012);
    server.abort();
}

#[tokio::test]
async fn an_idle_websocket_receives_the_configured_heartbeat() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-heartbeat").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let (status, _, mut socket) = websocket_handshake(
        address,
        "/api/games/current/events?snapshot_version=1",
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v2"),
    )
    .await;
    assert_eq!(status, 101);
    tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_text())
        .await
        .expect("the initial snapshot must arrive");
    tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_presence())
        .await
        .expect("the initial presence must arrive");

    let (opcode, payload) =
        tokio::time::timeout(std::time::Duration::from_secs(22), socket.read_frame())
            .await
            .expect("an idle connection must receive a timely heartbeat");
    assert_eq!(opcode, 9);
    assert!(payload.is_empty());
    server.abort();
}

#[tokio::test]
async fn websocket_snapshots_are_authorized_versioned_and_redacted_by_participant() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-snapshot").await;
    let (address, server) = start_network_server(room.app.clone()).await;

    let (status, _, mut host_socket) = websocket_handshake(
        address,
        "/api/games/current/events?snapshot_version=1",
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let host_serialized =
        tokio::time::timeout(std::time::Duration::from_secs(2), host_socket.read_text())
            .await
            .expect("the initial host Snapshot must arrive");
    let host: Value = serde_json::from_str(&host_serialized).expect("Snapshot must be JSON");
    assert_eq!(host["protocol_version"], 1);
    assert_eq!(host["type"], "snapshot");
    assert_eq!(host["cursor"], 0);
    assert_eq!(host["projection"]["snapshot"]["snapshot_version"], 4);
    assert_eq!(host["projection"]["snapshot"]["cursor"], 0);
    assert_eq!(
        host["projection"]["legal_actions"],
        json!(["end_hero_actions"])
    );
    assert_eq!(host["projection"]["choice"], json!({ "status": "none" }));

    let (status, _, mut guest_socket) = websocket_handshake(
        address,
        "/api/games/current/events",
        Some(&room.guest_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let guest_serialized =
        tokio::time::timeout(std::time::Duration::from_secs(2), guest_socket.read_text())
            .await
            .expect("the initial guest Snapshot must arrive");
    let guest: Value = serde_json::from_str(&guest_serialized).expect("Snapshot must be JSON");
    assert_eq!(guest["projection"]["participant"]["display_name"], "Luna");
    assert_eq!(guest["projection"]["legal_actions"], json!([]));

    for serialized in [host_serialized, guest_serialized] {
        assert!(!serialized.contains("participant_id"));
        assert!(!serialized.contains("prng_seed"));
        assert!(!serialized.contains("__Host-session"));
    }
    server.abort();
}

#[tokio::test]
async fn offline_presence_does_not_block_http_intent_or_automatic_phase_resolution() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-presence").await;
    let initial_state = current_official_state(&room).await;
    let (address, server) = start_network_server(room.app.clone()).await;

    let mut host_socket = connect_current_game(address, &room.host_cookie).await;
    let host_presence = host_socket.read_presence().await;
    assert_eq!(participant_presence(&host_presence, 1), "online");
    assert_eq!(participant_presence(&host_presence, 2), "offline");
    assert_eq!(host_presence["required_participant_position"], 1);
    assert_eq!(host_presence["blocked"], false);

    let mut guest_socket = connect_current_game(address, &room.guest_cookie).await;
    let guest_presence = guest_socket.read_presence().await;
    assert_eq!(participant_presence(&guest_presence, 1), "online");
    assert_eq!(participant_presence(&guest_presence, 2), "online");
    assert_eq!(guest_presence["blocked"], false);

    drop(host_socket);
    let reconnecting = tokio::time::timeout(Duration::from_secs(2), guest_socket.read_presence())
        .await
        .expect("disconnect must publish reconnecting presence");
    assert_eq!(participant_presence(&reconnecting, 1), "reconnecting");
    assert_eq!(reconnecting["required_participant_position"], 1);
    assert_eq!(reconnecting["blocked"], true);

    sqlx::query(
        r"
        UPDATE game_realtime_connections AS connections
        SET connected_at = clock_timestamp() - INTERVAL '62 seconds',
            last_heartbeat_at = clock_timestamp() - INTERVAL '61 seconds'
        FROM participants, rooms
        WHERE connections.participant_id = participants.id
          AND participants.room_id = rooms.id
          AND rooms.code = $1
          AND participants.position = 1
        ",
    )
    .bind(&room.room_code)
    .execute(&room.database)
    .await
    .expect("the test heartbeat must be ageable");
    let offline = tokio::time::timeout(Duration::from_secs(7), guest_socket.read_presence())
        .await
        .expect("the presence reconciliation must derive offline from the heartbeat");
    assert_eq!(participant_presence(&offline, 1), "offline");
    assert_eq!(offline["blocked"], true);

    let after_presence = current_official_state(&room).await;
    assert_eq!(after_presence, initial_state);

    let command = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, uuid::Uuid::new_v4(), 1))
        .await
        .expect("presence must not deny an otherwise authorized command");
    assert_eq!(command.status(), StatusCode::OK);
    let accepted = response_json(command).await;
    assert_eq!(accepted["projection"]["turn"]["number"], 2);
    assert_eq!(accepted["projection"]["turn"]["phase"], "hero_actions");
    assert_eq!(accepted["projection"]["turn"]["active_position"], 2);
    let event: Value = serde_json::from_str(
        &tokio::time::timeout(Duration::from_secs(2), guest_socket.read_text())
            .await
            .expect("the official event must still be published"),
    )
    .expect("the official event batch must be JSON");
    assert_eq!(event["type"], "events");
    assert_realtime_turn_completed(&event["events"][0]);
    assert_eq!(event["events"][0]["control"]["phase"], "hero_actions");
    assert_eq!(event["events"][0]["control"]["active_position"], 2);
    let automatic = tokio::time::timeout(Duration::from_secs(2), guest_socket.read_presence())
        .await
        .expect("the resolved decision must publish unblocked presence");
    assert_eq!(automatic["required_participant_position"], 2);
    assert_eq!(participant_presence(&automatic, 1), "offline");
    assert_eq!(participant_presence(&automatic, 2), "online");
    assert_eq!(automatic["blocked"], false);
    server.abort();
}

#[tokio::test]
async fn websocket_disconnect_while_an_intent_is_in_flight_does_not_interrupt_automatic_phases() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-concurrent-disconnect").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let mut host_socket = connect_current_game(address, &room.host_cookie).await;
    let initial_presence = host_socket.read_presence().await;
    assert_eq!(participant_presence(&initial_presence, 1), "online");

    let barrier = Arc::new(Barrier::new(2));
    let command_barrier = Arc::clone(&barrier);
    let command_app = room.app.clone();
    let host_cookie = room.host_cookie.clone();
    let command = async move {
        command_barrier.wait().await;
        command_app
            .oneshot(command_request(&host_cookie, uuid::Uuid::new_v4(), 1))
            .await
            .expect("the in-flight command must receive a response")
    };
    let disconnect = async move {
        barrier.wait().await;
        tokio::task::yield_now().await;
        drop(host_socket);
    };

    let (response, ()) = tokio::join!(command, disconnect);
    assert_eq!(response.status(), StatusCode::OK);
    let accepted = response_json(response).await;
    assert_eq!(accepted["projection"]["turn"]["number"], 2);
    assert_eq!(accepted["projection"]["turn"]["phase"], "hero_actions");
    assert_eq!(accepted["projection"]["turn"]["active_position"], 2);

    let (state_version, sequence, _, event_count) = current_official_state(&room).await;
    assert_eq!((state_version, sequence, event_count), (2, 1, 1));
    server.abort();
}

#[tokio::test]
async fn responsible_choice_survives_offline_and_reconnect_without_mutating_the_game() {
    let room = ready_room_with_manifest(each_hero_choice_manifest()).await;
    let started = start_ready_game(&room, "choice-presence-start").await;
    let first_choice_id = started["choice"]["id"]
        .as_str()
        .expect("the first participant choice must be present")
        .to_owned();
    let (address, server) = start_network_server(room.app.clone()).await;

    let mut host_socket = connect_current_game(address, &room.host_cookie).await;
    let host_initial = host_socket.read_presence().await;
    assert_eq!(host_initial["required_participant_position"], 1);
    assert_eq!(host_initial["blocked"], false);
    assert_eq!(participant_presence(&host_initial, 2), "offline");

    let mut guest_socket = connect_current_game(address, &room.guest_cookie).await;
    let guest_initial = guest_socket.read_presence().await;
    assert_eq!(guest_initial["required_participant_position"], 1);
    assert_eq!(guest_initial["blocked"], false);
    let host_sees_guest = host_socket.read_presence().await;
    assert_eq!(participant_presence(&host_sees_guest, 2), "online");

    let resolution_command_id = uuid::Uuid::new_v4();
    let resolved = room
        .app
        .clone()
        .oneshot(resolve_choice_request(
            &room.host_cookie,
            resolution_command_id,
            1,
            &first_choice_id,
            &["option:1"],
        ))
        .await
        .expect("the first participant must resolve their choice");
    assert_eq!(resolved.status(), StatusCode::OK);
    let resolved = response_json(resolved).await;
    let expected_choice = &resolved["projection"]["choice"];
    assert_eq!(expected_choice["responsible_position"], 2);

    let serialized_event = tokio::time::timeout(Duration::from_secs(2), guest_socket.read_text())
        .await
        .expect("the choice resolution event must reach the other participant");
    assert_public_choice_resolution_event(&serialized_event, &first_choice_id, expected_choice);

    let required_online = host_socket.read_presence().await;
    assert_eq!(required_online["required_participant_position"], 2);
    assert_eq!(participant_presence(&required_online, 2), "online");
    assert_eq!(required_online["blocked"], false);
    let before_disconnect = authoritative_command_state(&room).await;

    drop(guest_socket);
    let reconnecting = tokio::time::timeout(Duration::from_secs(2), host_socket.read_presence())
        .await
        .expect("disconnect must publish reconnecting presence");
    assert_eq!(reconnecting["required_participant_position"], 2);
    assert_eq!(participant_presence(&reconnecting, 2), "reconnecting");
    assert_eq!(reconnecting["blocked"], true);

    sqlx::query(
        r"
        UPDATE game_realtime_connections AS connections
        SET connected_at = clock_timestamp() - INTERVAL '62 seconds',
            last_heartbeat_at = clock_timestamp() - INTERVAL '61 seconds'
        FROM participants, rooms
        WHERE connections.participant_id = participants.id
          AND participants.room_id = rooms.id
          AND rooms.code = $1
          AND participants.position = 2
        ",
    )
    .bind(&room.room_code)
    .execute(&room.database)
    .await
    .expect("the responsible participant heartbeat must be ageable");
    let offline = tokio::time::timeout(Duration::from_secs(7), host_socket.read_presence())
        .await
        .expect("presence reconciliation must publish the responsible participant offline");
    assert_eq!(offline["required_participant_position"], 2);
    assert_eq!(participant_presence(&offline, 2), "offline");
    assert_eq!(offline["blocked"], true);
    assert_eq!(authoritative_command_state(&room).await, before_disconnect);

    let (status, _, mut reconnected_socket) = websocket_handshake(
        address,
        "/api/games/current/events",
        Some(&room.guest_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v2"),
    )
    .await;
    assert_eq!(status, 101);
    let reconnected_snapshot = reconnected_socket.read_text().await;
    assert_reconnected_choice_snapshot(&reconnected_snapshot, expected_choice);
    let online_again = reconnected_socket.read_presence().await;
    assert_eq!(online_again["required_participant_position"], 2);
    assert_eq!(participant_presence(&online_again, 2), "online");
    assert_eq!(online_again["blocked"], false);
    assert_eq!(authoritative_command_state(&room).await, before_disconnect);
    server.abort();
}

#[tokio::test]
async fn either_of_two_valid_sessions_keeps_the_participant_online() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-two-sessions").await;
    let second_host_cookie = additional_session_for_participant(&room, "host").await;
    let (address, server) = start_network_server(room.app.clone()).await;

    let mut first_host_socket = connect_current_game(address, &room.host_cookie).await;
    let _ = first_host_socket.read_presence().await;
    let mut second_host_socket = connect_current_game(address, &second_host_cookie).await;
    let _ = second_host_socket.read_presence().await;

    drop(first_host_socket);
    let mut guest_socket = connect_current_game(address, &room.guest_cookie).await;
    let one_session_online = guest_socket.read_presence().await;
    assert_eq!(participant_presence(&one_session_online, 1), "online");
    assert_eq!(one_session_online["blocked"], false);

    drop(second_host_socket);
    let no_session_online =
        tokio::time::timeout(Duration::from_secs(2), guest_socket.read_presence())
            .await
            .expect("losing the final session must publish reconnecting presence");
    assert_eq!(participant_presence(&no_session_online, 1), "reconnecting");
    assert_eq!(no_session_online["blocked"], true);
    server.abort();
}

#[tokio::test]
async fn a_revoked_session_closes_its_existing_websocket_before_delivering_an_event() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-revocation").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let (status, _, mut guest_socket) = websocket_handshake(
        address,
        "/api/games/current/events",
        Some(&room.guest_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let _ = guest_socket.read_text().await;

    sqlx::query(
        r"
        UPDATE device_sessions
        SET status = 'revoked'
        FROM participants, rooms
        WHERE device_sessions.participant_id = participants.id
          AND participants.room_id = rooms.id
          AND rooms.code = $1
          AND participants.role = 'guest'
        ",
    )
    .bind(&room.room_code)
    .execute(&room.database)
    .await
    .expect("the guest session must be revocable");

    let command = room
        .app
        .clone()
        .oneshot(command_request(&room.host_cookie, uuid::Uuid::new_v4(), 1))
        .await
        .expect("the host command must receive a response");
    assert_eq!(command.status(), StatusCode::OK);
    let close_code = tokio::time::timeout(Duration::from_secs(2), guest_socket.read_close_code())
        .await
        .expect("the revoked socket must be closed promptly");
    assert_eq!(close_code, 1008);
    server.abort();
}

#[tokio::test]
async fn device_session_revocation_closes_the_target_websocket_within_two_seconds() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-device-session-revocation").await;
    let second_host_cookie = additional_session_for_participant(&room, "host").await;
    let sessions = room
        .app
        .clone()
        .oneshot(list_device_sessions_request(&room.host_cookie))
        .await
        .expect("the device session list must receive a response");
    let sessions = response_json(sessions).await;
    let second_session_id = sessions["sessions"]
        .as_array()
        .expect("device sessions must be an array")
        .iter()
        .find(|session| session["current"] == false)
        .and_then(|session| session["id"].as_str())
        .expect("the second device session must be listed")
        .to_owned();
    let (address, server) = start_network_server(room.app.clone()).await;
    let (status, _, mut target_socket) = websocket_handshake(
        address,
        "/api/games/current/events",
        Some(&second_host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let _ = target_socket.read_text().await;

    let committed_at = Instant::now();
    let revoked = room
        .app
        .clone()
        .oneshot(revoke_device_session_request(
            &room.host_cookie,
            &second_session_id,
            &unique_key("close-revoked-session"),
        ))
        .await
        .expect("device session revocation must receive a response");
    assert_eq!(revoked.status(), StatusCode::OK);

    let close_code = tokio::time::timeout(Duration::from_secs(2), target_socket.read_close_code())
        .await
        .expect("the revoked device connection must close within the p95 target");
    assert_eq!(close_code, 1008);
    assert!(committed_at.elapsed() <= Duration::from_secs(2));
    server.abort();
}

#[tokio::test]
async fn device_session_revocation_closes_connections_within_p95_and_p99_targets() {
    const CONNECTION_COUNT: usize = 100;

    let room = ready_room().await;
    start_ready_game(&room, "realtime-device-session-revocation-percentiles").await;
    let second_host_cookie = additional_session_for_participant(&room, "host").await;
    let sessions = room
        .app
        .clone()
        .oneshot(list_device_sessions_request(&room.host_cookie))
        .await
        .expect("the device session list must receive a response");
    let sessions = response_json(sessions).await;
    let second_session_id = sessions["sessions"]
        .as_array()
        .expect("device sessions must be an array")
        .iter()
        .find(|session| session["current"] == false)
        .and_then(|session| session["id"].as_str())
        .expect("the second device session must be listed")
        .to_owned();
    let second_state =
        AppState::with_content_manifests(room.database.clone(), vec![room.manifest.clone()])
            .with_session_token_key(*b"test-session-token-key-000000000");
    initialize(&second_state)
        .await
        .expect("the remote connection owner must initialize");
    let (address, server) = start_network_server(build_router(second_state)).await;

    let mut connection_tasks = JoinSet::new();
    for index in 0..CONNECTION_COUNT {
        let cookie = second_host_cookie.clone();
        connection_tasks.spawn(async move {
            let (path, protocol) = match index % 3 {
                0 => ("/api/games/current/events", "hogwarts.realtime.v1"),
                1 => ("/api/games/current/events", "hogwarts.realtime.v2"),
                _ => ("/api/session/events", "hogwarts.session.v1"),
            };
            let (status, _, mut socket) = websocket_handshake(
                address,
                path,
                Some(&cookie),
                Some("http://127.0.0.1:5173"),
                Some(protocol),
            )
            .await;
            assert_eq!(status, 101);
            let _ = socket.read_text().await;
            socket
        });
    }
    let mut sockets = Vec::with_capacity(CONNECTION_COUNT);
    while let Some(result) = connection_tasks.join_next().await {
        sockets.push(result.expect("each target WebSocket must connect"));
    }
    assert_eq!(sockets.len(), CONNECTION_COUNT);

    let started_at = Instant::now();
    let revoked = room
        .app
        .clone()
        .oneshot(revoke_device_session_request(
            &room.host_cookie,
            &second_session_id,
            &unique_key("close-revoked-session-percentiles"),
        ))
        .await
        .expect("device session revocation must receive a response");
    assert_eq!(revoked.status(), StatusCode::OK);

    let mut close_tasks = JoinSet::new();
    for mut socket in sockets {
        close_tasks.spawn(async move {
            let code = tokio::time::timeout(Duration::from_secs(5), socket.read_close_code())
                .await
                .expect("every revoked connection must close within the p99 target");
            (started_at.elapsed(), code)
        });
    }
    let mut latencies = Vec::with_capacity(CONNECTION_COUNT);
    while let Some(result) = close_tasks.join_next().await {
        let (latency, code) = result.expect("each close observation must complete");
        assert_eq!(code, 1008);
        latencies.push(latency);
    }
    latencies.sort_unstable();
    let p95 = latencies[94];
    let p99 = latencies[98];
    eprintln!("cross-instance mixed-channel revocation: p95={p95:?}, p99={p99:?}");
    assert!(
        p95 <= Duration::from_secs(2),
        "revocation close p95 was {p95:?}"
    );
    assert!(
        p99 <= Duration::from_secs(5),
        "revocation close p99 was {p99:?}"
    );
    server.abort();
}

#[tokio::test]
async fn a_replaced_session_loses_cross_instance_v1_websocket_authorization_at_the_recovery_commit()
{
    let room = ready_room().await;
    start_ready_game(&room, "realtime-device-replacement").await;

    let second_device = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &json!({
                "recovery_token": room.host_recovery_token,
                "recovery_password": "a long uncommon passphrase",
                "recovery_attempt_id": uuid::Uuid::new_v4().to_string()
            }),
            None,
            None,
        ))
        .await
        .expect("second-device recovery must receive a response");
    assert_eq!(second_device.status(), StatusCode::OK);
    let second_device = response_json(second_device).await;
    let successor_token = second_device["recovery_token"]
        .as_str()
        .expect("recovery must rotate the individual credential");

    let second_state =
        AppState::with_content_manifests(room.database.clone(), vec![room.manifest.clone()])
            .with_session_token_key(*b"test-session-token-key-000000000");
    initialize(&second_state)
        .await
        .expect("the second application instance must initialize");
    let (address, server) = start_network_server(build_router(second_state)).await;
    let (status, _, mut replaced_socket) = websocket_handshake(
        address,
        "/api/games/current/events",
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let _ = replaced_socket.read_text().await;

    let replacement_attempt = uuid::Uuid::new_v4().to_string();
    let candidates = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &json!({
                "recovery_token": successor_token,
                "recovery_password": "a long uncommon passphrase",
                "recovery_attempt_id": replacement_attempt
            }),
            None,
            None,
        ))
        .await
        .expect("replacement discovery must receive a response");
    assert_eq!(candidates.status(), StatusCode::CONFLICT);
    let candidates = response_json(candidates).await;
    let first_session_id = candidates["sessions"][0]["id"]
        .as_str()
        .expect("the first session must be replaceable");

    let replacement = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/session/recover",
            &json!({
                "recovery_token": successor_token,
                "recovery_password": "a long uncommon passphrase",
                "recovery_attempt_id": replacement_attempt,
                "replace_session_id": first_session_id
            }),
            None,
            None,
        ))
        .await
        .expect("replacement confirmation must receive a response");
    assert_eq!(replacement.status(), StatusCode::OK);
    let close_code =
        tokio::time::timeout(Duration::from_secs(2), replaced_socket.read_close_code())
            .await
            .expect("the recovery commit must promptly close the replaced socket");
    assert_eq!(close_code, 1008);
    server.abort();
}

#[tokio::test]
async fn an_incompatible_snapshot_version_cursor_or_digest_receives_a_full_snapshot() {
    let room = ready_room().await;
    let projection = start_ready_game(&room, "realtime-resync").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let digest = projection["snapshot"]["digest"]
        .as_str()
        .expect("the initial projection digest must be present");

    for path in [
        format!("/api/games/current/events?cursor=8&snapshot_version=1&digest={digest}"),
        format!("/api/games/current/events?cursor=0&snapshot_version=999&digest={digest}"),
        format!(
            "/api/games/current/events?cursor=0&snapshot_version=1&digest=blake3:{}",
            "0".repeat(64)
        ),
    ] {
        let (status, _, mut socket) = websocket_handshake(
            address,
            &path,
            Some(&room.host_cookie),
            Some("http://127.0.0.1:5173"),
            Some("hogwarts.realtime.v1"),
        )
        .await;
        assert_eq!(status, 101);
        let serialized =
            tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_text())
                .await
                .expect("an incompatible cursor must receive a Snapshot");
        let snapshot: Value = serde_json::from_str(&serialized).expect("Snapshot must be JSON");
        assert_eq!(snapshot["type"], "snapshot");
        assert_eq!(snapshot["cursor"], 0);
        assert_eq!(snapshot["projection"]["snapshot"]["cursor"], 0);
    }
    server.abort();
}

#[tokio::test]
async fn database_rejects_a_gap_in_the_official_event_sequence() {
    let room = ready_room().await;
    start_ready_game(&room, "realtime-sequence").await;
    let mut transaction = room
        .database
        .begin()
        .await
        .expect("the gap test transaction must start");
    let (game_id, room_id, participant_id) =
        sqlx::query_as::<_, (uuid::Uuid, uuid::Uuid, uuid::Uuid)>(
            r"
        UPDATE games
        SET sequence = 2,
            state_version = 3
        WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
        RETURNING id, room_id, started_by_participant_id
        ",
        )
        .bind(&room.room_code)
        .fetch_one(&mut *transaction)
        .await
        .expect("the test Snapshot cursor must advance inside the transaction");
    let error = sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_version,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, 2, 3, 'turn_completed', $3, $4, 2, '{}'::jsonb)
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(uuid::Uuid::new_v4())
    .bind(participant_id)
    .execute(&mut *transaction)
    .await
    .expect_err("the database must reject sequence two when sequence one is absent");
    assert!(
        error
            .to_string()
            .contains("game event sequence must be contiguous")
    );
    transaction
        .rollback()
        .await
        .expect("the rejected gap transaction must roll back");
}

#[tokio::test]
async fn committed_log_replays_contiguous_events_and_redacts_another_participants_command() {
    let room = ready_room().await;
    let projection = start_ready_game(&room, "realtime-events").await;
    let (address, server) = start_network_server(room.app.clone()).await;
    let path = realtime_path(&projection);
    let (status, _, mut host_socket) = websocket_handshake(
        address,
        &path,
        Some(&room.host_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let synchronized = tokio::time::timeout(Duration::from_secs(2), host_socket.read_text())
        .await
        .expect("the matching cursor must receive a synchronization acknowledgement");
    let synchronized: Value =
        serde_json::from_str(&synchronized).expect("synchronization must be JSON");
    assert_eq!(synchronized["type"], "synchronized");
    assert_eq!(synchronized["cursor"], 0);
    assert_eq!(synchronized["snapshot_version"], 4);
    assert_eq!(synchronized["digest"], projection["snapshot"]["digest"]);

    let command_id = uuid::Uuid::new_v4();
    let response = room
        .app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/games/current/commands",
            &json!({
                "command_id": command_id.to_string(),
                "expected_state_version": 1,
                "type": "end_hero_actions"
            }),
            Some(&room.host_cookie),
            None,
        ))
        .await
        .expect("the command must receive a post-commit response");
    assert_eq!(response.status(), StatusCode::OK);

    let host_serialized =
        tokio::time::timeout(std::time::Duration::from_secs(2), host_socket.read_text())
            .await
            .expect("the committed event must reach the connected host");
    let host: Value = serde_json::from_str(&host_serialized).expect("event batch must be JSON");
    assert_eq!(host["type"], "events");
    assert_eq!(host["from_cursor"], 0);
    assert_eq!(host["cursor"], 1);
    assert_eq!(host["events"][0]["sequence"], 1);
    assert_realtime_turn_completed(&host["events"][0]);
    assert_eq!(host["events"][0]["command_id"], command_id.to_string());
    assert_eq!(host["projection"]["snapshot"]["cursor"], 1);
    assert_eq!(host["projection"]["turn"]["active_position"], 2);
    assert_eq!(host["projection"]["turn"]["phase"], "hero_actions");

    let (status, _, mut guest_socket) = websocket_handshake(
        address,
        &path,
        Some(&room.guest_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let guest_serialized =
        tokio::time::timeout(std::time::Duration::from_secs(2), guest_socket.read_text())
            .await
            .expect("the durable log must replay without the original signal");
    let guest: Value = serde_json::from_str(&guest_serialized).expect("event batch must be JSON");
    assert_eq!(guest["events"][0]["sequence"], 1);
    assert_realtime_turn_completed(&guest["events"][0]);
    assert!(guest["events"][0].get("command_id").is_none());
    assert_eq!(
        guest["projection"]["legal_actions"],
        json!(["end_hero_actions"])
    );
    assert!(!guest_serialized.contains("actor_participant_id"));
    assert!(!guest_serialized.contains(&command_id.to_string()));

    let (status, _, mut redelivery_socket) = websocket_handshake(
        address,
        &path,
        Some(&room.guest_cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let redelivery = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        redelivery_socket.read_text(),
    )
    .await
    .expect("at-least-once delivery must allow the same cursor to be replayed");
    assert_eq!(
        serde_json::from_str::<Value>(&redelivery).expect("redelivery must be JSON")["events"][0]["sequence"],
        1
    );
    server.abort();
}

struct ReferenceRecoveryGame {
    host_cookie: String,
    guest_cookie: String,
    replay_path: String,
    snapshot_path: String,
}

async fn create_reference_recovery_game(
    app: axum::Router,
    manifest: ContentManifest,
) -> ReferenceRecoveryGame {
    let (room_code, host_cookie, _) = create_room(&app).await;
    assert_eq!(
        select_hero(&app, &host_cookie, "harry").await.status(),
        StatusCode::OK
    );
    let (guest_cookie, _) = join_room(&app, &room_code).await;
    assert_eq!(
        set_ready(&app, &host_cookie, true).await.status(),
        StatusCode::OK
    );
    assert_eq!(
        set_ready(&app, &guest_cookie, true).await.status(),
        StatusCode::OK
    );
    let response = app
        .clone()
        .oneshot(start_request(
            &host_cookie,
            &unique_key("reference-recovery-start"),
            &manifest,
            "adventure:001",
        ))
        .await
        .expect("the reference game must receive its initial Snapshot");
    assert_eq!(response.status(), StatusCode::CREATED);
    let projection = response_json(response).await;
    let replay_path = realtime_path(&projection);
    let snapshot_path = format!(
        "/api/games/current/events?cursor=0&snapshot_version=1&digest=blake3:{}",
        "0".repeat(64)
    );
    ReferenceRecoveryGame {
        host_cookie,
        guest_cookie,
        replay_path,
        snapshot_path,
    }
}

#[derive(Clone, Copy)]
enum ReferenceRecoveryMode {
    Replay,
    Snapshot,
}

fn percentile_95(durations: &mut [Duration]) -> Duration {
    durations.sort_unstable();
    durations[(durations.len() * 95).div_ceil(100) - 1]
}

async fn recover_reference_connection(
    address: std::net::SocketAddr,
    path: String,
    cookie: String,
    mode: ReferenceRecoveryMode,
    lose_first_attempt: bool,
    reference_rtt: Duration,
    start_barrier: Arc<Barrier>,
) -> (ReferenceRecoveryMode, Duration) {
    start_barrier.wait().await;
    let started = Instant::now();
    if lose_first_attempt {
        let (_, _, lost_socket) = websocket_handshake(
            address,
            &path,
            Some(&cookie),
            Some("http://127.0.0.1:5173"),
            Some("hogwarts.realtime.v1"),
        )
        .await;
        drop(lost_socket);
        tokio::time::sleep(reference_rtt).await;
    }
    tokio::time::sleep(reference_rtt).await;
    let (status, _, mut socket) = websocket_handshake(
        address,
        &path,
        Some(&cookie),
        Some("http://127.0.0.1:5173"),
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    let serialized = tokio::time::timeout(Duration::from_secs(10), socket.read_text())
        .await
        .expect("reference recovery must produce an official message");
    let message: Value = serde_json::from_str(&serialized).expect("recovery message must be JSON");
    let expected_type = match mode {
        ReferenceRecoveryMode::Replay => "events",
        ReferenceRecoveryMode::Snapshot => "snapshot",
    };
    assert_eq!(message["type"], expected_type);
    (mode, started.elapsed())
}

#[tokio::test]
#[ignore = "reference load profile; run make check-reconnect-profile"]
async fn reference_reconnect_profile_meets_replay_and_snapshot_slos() {
    const GAME_COUNT: usize = 100;
    const CONNECTIONS_PER_GAME: usize = 4;
    const REFERENCE_RTT: Duration = Duration::from_millis(150);
    const REPLAY_SLO: Duration = Duration::from_secs(3);
    const SNAPSHOT_SLO: Duration = Duration::from_secs(5);

    let manifest = playable_manifest();
    let (app, _, _) = test_app(manifest.clone()).await;
    let mut setups = JoinSet::new();
    for _ in 0..GAME_COUNT {
        let app = app.clone();
        let manifest = manifest.clone();
        setups.spawn(create_reference_recovery_game(app, manifest));
    }
    let mut games = Vec::with_capacity(GAME_COUNT);
    while let Some(result) = setups.join_next().await {
        games.push(result.expect("reference game setup must finish"));
    }

    let (address, server) = start_network_server(app.clone()).await;
    let mut command_pacer = tokio::time::interval(Duration::from_millis(50));
    command_pacer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    for game in &games {
        command_pacer.tick().await;
        let response = app
            .clone()
            .oneshot(command_request(&game.host_cookie, uuid::Uuid::new_v4(), 1))
            .await
            .expect("the reference command must receive a response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    let connection_count = GAME_COUNT * CONNECTIONS_PER_GAME;
    let start_barrier = Arc::new(Barrier::new(connection_count + 1));
    let mut recoveries = JoinSet::new();
    for (game_index, game) in games.into_iter().enumerate() {
        for connection_index in 0..CONNECTIONS_PER_GAME {
            let mode = if connection_index < 2 {
                ReferenceRecoveryMode::Replay
            } else {
                ReferenceRecoveryMode::Snapshot
            };
            let path = match mode {
                ReferenceRecoveryMode::Replay => game.replay_path.clone(),
                ReferenceRecoveryMode::Snapshot => game.snapshot_path.clone(),
            };
            let cookie = if connection_index % 2 == 0 {
                game.host_cookie.clone()
            } else {
                game.guest_cookie.clone()
            };
            let ordinal = game_index * CONNECTIONS_PER_GAME + connection_index;
            let lose_first_attempt = ordinal.is_multiple_of(101);
            let start_barrier = Arc::clone(&start_barrier);
            recoveries.spawn(recover_reference_connection(
                address,
                path,
                cookie,
                mode,
                lose_first_attempt,
                REFERENCE_RTT,
                start_barrier,
            ));
        }
    }
    start_barrier.wait().await;

    let mut replay_durations = Vec::with_capacity(connection_count / 2);
    let mut snapshot_durations = Vec::with_capacity(connection_count / 2);
    while let Some(result) = recoveries.join_next().await {
        let (mode, duration) = result.expect("reference recovery must finish");
        match mode {
            ReferenceRecoveryMode::Replay => replay_durations.push(duration),
            ReferenceRecoveryMode::Snapshot => snapshot_durations.push(duration),
        }
    }
    server.abort();

    let replay_p95 = percentile_95(&mut replay_durations);
    let snapshot_p95 = percentile_95(&mut snapshot_durations);
    println!(
        "reference recovery: games={GAME_COUNT}, sockets={connection_count}, rtt_ms={}, loss_percent=1, replay_p95_ms={}, snapshot_p95_ms={}",
        REFERENCE_RTT.as_millis(),
        replay_p95.as_millis(),
        snapshot_p95.as_millis()
    );
    assert!(
        replay_p95 <= REPLAY_SLO,
        "replay p95 {replay_p95:?} exceeded {REPLAY_SLO:?}"
    );
    assert!(
        snapshot_p95 <= SNAPSHOT_SLO,
        "Snapshot p95 {snapshot_p95:?} exceeded {SNAPSHOT_SLO:?}"
    );
}
