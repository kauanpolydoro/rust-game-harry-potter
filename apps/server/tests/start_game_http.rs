use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use game_content::{ContentManifest, import_base_bundle};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{sync::Arc, time::Duration};
use tokio::sync::Barrier;
use tower::ServiceExt;

struct ReadyRoom {
    app: axum::Router,
    database: PgPool,
    room_code: String,
    host_cookie: String,
    guest_cookie: String,
    manifest: ContentManifest,
}

fn playable_manifest() -> ContentManifest {
    let entries = (0..171)
        .map(|index| {
            if index == 0 {
                return json!({
                    "id": "adventure:001",
                    "kind": "adventure",
                    "set": "base",
                    "copies": 2,
                    "introduced_in": 1,
                    "names": { "en": "Game 1" },
                    "provenance": {
                        "id": ["fixture-source"],
                        "kind": ["fixture-source"],
                        "set": ["fixture-source"],
                        "copies": ["fixture-source"],
                        "introduced_in": ["fixture-source"],
                        "names.en": ["fixture-source"]
                    },
                    "required_functional_fields": ["setup", "precedence"],
                    "functional": {
                        "setup": {
                            "confidence": "adaptation",
                            "sources": ["fixture-source"],
                            "rule": "rule:setup"
                        },
                        "precedence": {
                            "confidence": "adaptation",
                            "sources": ["fixture-source"],
                            "rule": "rule:precedence"
                        }
                    }
                });
            }

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
        "rules": [
            { "id": "rule:setup", "effect": { "type": "no_op" } },
            { "id": "rule:precedence", "effect": { "type": "no_op" } }
        ],
        "entries": entries
    }))
    .expect("the playable fixture must serialize");

    import_base_bundle(&bundle).expect("the playable fixture must import")
}

async fn database() -> PgPool {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available")
}

async fn test_app(manifest: ContentManifest) -> (axum::Router, PgPool) {
    let database = database().await;
    let state = AppState::with_content_manifests(database.clone(), vec![manifest]);
    initialize(&state)
        .await
        .expect("database initialization must succeed");
    (build_router(state), database)
}

fn unique_key(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock must be after the Unix epoch")
            .as_nanos()
    )
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

async fn ready_room() -> ReadyRoom {
    let manifest = playable_manifest();
    let (app, database) = test_app(manifest.clone()).await;
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
        database,
        room_code,
        host_cookie,
        guest_cookie,
        manifest,
    }
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
    assert_eq!(host_projection["snapshot"]["snapshot_version"], 1);
    assert_eq!(host_projection["snapshot"]["state_version"], 1);
    assert_eq!(host_projection["snapshot"]["sequence"], 0);
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
    assert_eq!(
        host_projection["legal_actions"],
        json!(["complete_dark_arts"])
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
    let (app, _) = test_app(manifest.clone()).await;

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
    let (candidate_app, _) = test_app(candidate.clone()).await;
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
