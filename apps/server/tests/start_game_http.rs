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
    collections::BTreeSet,
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
    guest_cookie: String,
    manifest: ContentManifest,
}

fn playable_manifest() -> ContentManifest {
    let entries = (0..171).map(playable_fixture_entry).collect::<Vec<_>>();
    let bundle = serde_json::to_vec(&json!({
        "schema_version": 1,
        "content_version": "fixture-v1",
        "ruleset_version": "fixture-rules-v1",
        "locale": "en",
        "sources": [{
            "id": "fixture-source",
            "uri": "https://example.invalid/fixture",
            "kind": "adaptation"
        }],
        "rules": [{
            "id": "rule:functional",
            "effect": {
                "type": "apply",
                "target": {
                    "zone": "hero_hand",
                    "cardinality": { "min": 1, "max": 1 }
                },
                "operation": { "type": "discard" }
            }
        }],
        "entries": entries
    }))
    .expect("the playable fixture must serialize");

    import_base_bundle_with_runtime_rules(
        &bundle,
        &[ProvenanceSource {
            id: "fixture-source".to_owned(),
            uri: "https://example.invalid/fixture".to_owned(),
            kind: SourceKind::Adaptation,
        }],
        &BTreeSet::from([
            RuleId::parse("rule:functional").expect("fixture rule ID should be valid")
        ]),
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
    sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
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
            'dark_arts_completed',
            $3,
            $4,
            2,
            jsonb_build_object(
                'event_version', 1,
                'type', 'dark_arts_completed',
                'sequence', 1,
                'state_version', 2,
                'turn', 1,
                'actor_position', 1
            )
        )
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(command_id)
    .bind(actor_id)
    .execute(&mut **transaction)
    .await
    .expect("the official event must be inserted for the integrity test");
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

async fn create_room(app: &axum::Router) -> (String, String) {
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
    )
}

async fn join_room(app: &axum::Router, room_code: &str) -> String {
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
    session_cookie(&response)
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
            "type": "complete_dark_arts"
        }),
        Some(cookie),
        None,
    )
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
    assert_eq!(response.status(), StatusCode::CREATED);
    response_json(response).await
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
    let (app, database, state) = test_app(manifest.clone()).await;
    let (room_code, host_cookie) = create_room(&app).await;
    assert_eq!(
        select_hero(&app, &host_cookie, "harry").await.status(),
        StatusCode::OK
    );
    let guest_cookie = join_room(&app, &room_code).await;
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
        guest_cookie,
        manifest,
    }
}

fn assert_initial_synchronization_projection(projection: &Value) {
    assert_eq!(projection["snapshot"]["snapshot_version"], 1);
    assert_eq!(projection["snapshot"]["state_version"], 1);
    assert_eq!(projection["snapshot"]["sequence"], 0);
    assert_eq!(projection["snapshot"]["cursor"], 0);
    assert_eq!(projection["legal_actions"], json!(["complete_dark_arts"]));
    assert_eq!(projection["choice"], json!({ "status": "none" }));
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
    assert_eq!(host_projection["turn"]["phase"], "dark_arts");
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
    assert!(!guest_projection.to_string().contains("seed"));

    let stored = sqlx::query_as::<_, (String, String, i32, i64, i64, String, String)>(
        r"
        SELECT
            rooms.status,
            games.prng_algorithm,
            octet_length(games.prng_seed),
            games.state_version,
            games.sequence,
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
    assert!(stored.5.starts_with("blake3:"));
    let snapshot: Value =
        serde_json::from_str(&stored.6).expect("the persisted Snapshot must be JSON");
    assert_eq!(snapshot["snapshot_version"], 1);
    assert_eq!(
        snapshot["versions"]["manifest_digest"],
        room.manifest.digest
    );
    assert!(!stored.6.contains("seed"));
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

    let (_, lone_host) = create_room(&app).await;
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

    let (missing_hero_code, missing_hero_host) = create_room(&app).await;
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

    let (not_ready_code, not_ready_host) = create_room(&app).await;
    assert_eq!(
        select_hero(&app, &not_ready_host, "harry").await.status(),
        StatusCode::OK
    );
    let not_ready_guest = join_room(&app, &not_ready_code).await;
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
    let (candidate_code, candidate_host) = create_room(&candidate_app).await;
    assert_eq!(
        select_hero(&candidate_app, &candidate_host, "harry")
            .await
            .status(),
        StatusCode::OK
    );
    let candidate_guest = join_room(&candidate_app, &candidate_code).await;
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
}

async fn assert_committed_command_artifacts(room: &ReadyRoom, initial_expiration: &str) {
    let stored =
        sqlx::query_as::<_, (i64, i64, i64, String, String, String, i64, i64, bool, bool)>(
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
                games.expires_at > $2::timestamptz
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
    assert_eq!(stored.2, 0);
    assert_eq!(stored.4, "dark_arts_completed");
    assert_eq!(stored.6, 2);
    assert_eq!(stored.7, 1);
    assert!(stored.8, "the receipt and game must share one expiration");
    assert!(stored.9, "an accepted action must renew retention");
    let snapshot: Value = serde_json::from_str(&stored.3).expect("snapshot must be JSON");
    let event: Value = serde_json::from_str(&stored.5).expect("event must be JSON");
    assert_eq!(snapshot["state_version"], 2);
    assert_eq!(snapshot["sequence"], 1);
    assert_eq!(snapshot["turn"]["phase"], "hero_action");
    assert_eq!(snapshot["prng"]["counter"], 0);
    assert_eq!(event["sequence"], 1);
    assert_eq!(event["state_version"], 2);

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

#[tokio::test]
async fn active_command_commits_snapshot_prng_receipt_event_sequence_and_expiration() {
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
                "type": "complete_dark_arts"
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
    assert_eq!(accepted["receipt"]["status"], "accepted");
    assert_eq!(accepted["receipt"]["accepted_state_version"], 2);
    assert_eq!(accepted["receipt"]["accepted_sequence"], 1);
    assert_eq!(accepted["projection"]["turn"]["phase"], "hero_action");
    assert_eq!(accepted["projection"]["snapshot"]["state_version"], 2);
    assert_eq!(accepted["projection"]["snapshot"]["sequence"], 1);
    assert_eq!(accepted["projection"]["legal_actions"], json!([]));

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

    let error = sqlx::query(
        r"
        INSERT INTO game_events (
            game_id,
            room_id,
            sequence,
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, 1, 'dark_arts_completed', $3, $4, 2, '{}'::jsonb)
        ",
    )
    .bind(game_id)
    .bind(room_id)
    .bind(uuid::Uuid::new_v4())
    .bind(other_actor)
    .execute(&mut *transaction)
    .await
    .expect_err("an event actor from another room must be rejected");
    assert_database_error_code(&error, "23503");
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
        let error = sqlx::query(
            r"
            INSERT INTO game_events (
                game_id,
                room_id,
                sequence,
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
                'dark_arts_completed',
                $3,
                $4,
                $5,
                jsonb_build_object(
                    'event_version', 1,
                    'type', 'dark_arts_completed',
                    'sequence', 1,
                    'state_version', $6,
                    'turn', 1,
                    'actor_position', 1
                )
            )
            ",
        )
        .bind(game_id)
        .bind(room_id)
        .bind(uuid::Uuid::new_v4())
        .bind(actor_id)
        .bind(row_state_version)
        .bind(payload_state_version)
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

    for (event_type, extra_payload, expected_message) in [
        (
            "future_event",
            "{}",
            "event type is not supported by the current codec",
        ),
        (
            "dark_arts_completed",
            r#"{"unexpected":true}"#,
            "payload metadata must match",
        ),
        (
            "dark_arts_completed",
            r#"{"sequence":1.0}"#,
            "payload must match the current codec shape",
        ),
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
        let error = sqlx::query(
            r"
            INSERT INTO game_events (
                game_id,
                room_id,
                sequence,
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
                $3,
                $4,
                $5,
                2,
                jsonb_build_object(
                    'event_version', 1,
                    'type', $3,
                    'sequence', 1,
                    'state_version', 2,
                    'turn', 1,
                    'actor_position', 1
                ) || $6::jsonb
            )
            ",
        )
        .bind(game_id)
        .bind(room_id)
        .bind(event_type)
        .bind(uuid::Uuid::new_v4())
        .bind(actor_id)
        .bind(extra_payload)
        .execute(&mut *transaction)
        .await
        .expect_err("an event outside the current codec must be rejected");
        assert_database_error_code(&error, "23514");
        assert!(error.to_string().contains(expected_message));
        transaction
            .rollback()
            .await
            .expect("the rejected event transaction must roll back");
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
            UPDATE games
            SET sequence = 1,
                state_version = 2
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            RETURNING id, started_by_participant_id, room_id
            ",
    )
    .bind(&room.room_code)
    .fetch_one(&mut *transaction)
    .await
    .expect("the test game cursor must advance inside the transaction");
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
            'complete_dark_arts',
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
            UPDATE games
            SET sequence = 1,
                state_version = 2
            WHERE room_id = (SELECT id FROM rooms WHERE code = $1)
            RETURNING id, started_by_participant_id, room_id
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
            'complete_dark_arts',
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
        VALUES ($1, $2, 1, 999, 'dark_arts_completed', $3, $4, 2, '{}'::jsonb)
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
                "type": "complete_dark_arts"
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
                "type": "complete_dark_arts"
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
                "type": "complete_dark_arts"
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
                1 => return String::from_utf8(payload).expect("text frames must contain UTF-8"),
                8 => panic!("the WebSocket closed before a text message arrived"),
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
                9 | 10 => {}
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
        Some("hogwarts.realtime.v1"),
    )
    .await;
    assert_eq!(status, 101);
    tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_text())
        .await
        .expect("the initial snapshot must arrive");

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
    assert_eq!(host["projection"]["snapshot"]["snapshot_version"], 1);
    assert_eq!(host["projection"]["snapshot"]["cursor"], 0);
    assert_eq!(
        host["projection"]["legal_actions"],
        json!(["complete_dark_arts"])
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
            event_type,
            command_id,
            actor_participant_id,
            state_version,
            payload
        )
        VALUES ($1, $2, 2, 'dark_arts_completed', $3, $4, 2, '{}'::jsonb)
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
    assert_eq!(synchronized["snapshot_version"], 1);
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
                "type": "complete_dark_arts"
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
    assert_eq!(host["events"][0]["command_id"], command_id.to_string());
    assert_eq!(host["projection"]["snapshot"]["cursor"], 1);

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
    assert!(guest["events"][0].get("command_id").is_none());
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
    let (room_code, host_cookie) = create_room(&app).await;
    assert_eq!(
        select_hero(&app, &host_cookie, "harry").await.status(),
        StatusCode::OK
    );
    let guest_cookie = join_room(&app, &room_code).await;
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
