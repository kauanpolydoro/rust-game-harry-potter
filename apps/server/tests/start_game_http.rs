use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use game_content::{ContentManifest, import_base_bundle};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
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
    assert_eq!(host_projection["legal_actions"], json!([]));
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
