use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

async fn test_state() -> AppState {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let database = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    let state = AppState::new(database);

    initialize(&state)
        .await
        .expect("database initialization must succeed");

    state
}

fn unique_key(_prefix: &str) -> String {
    uuid::Uuid::new_v4().to_string()
}

fn create_room_request(idempotency_key: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/rooms")
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", idempotency_key)
        .body(Body::from(
            json!({
                "display_name": "Minerva",
                "recovery_password": "a long uncommon passphrase"
            })
            .to_string(),
        ))
        .expect("the room creation request must be valid")
}

fn join_room_request(
    room_code: &str,
    idempotency_key: &str,
    display_name: &str,
    hero_id: &str,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/rooms/{room_code}/participants"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", idempotency_key)
        .body(Body::from(
            json!({
                "display_name": display_name,
                "hero_id": hero_id
            })
            .to_string(),
        ))
        .expect("the room join request must be valid")
}

fn select_hero_request(session_cookie: &str, hero_id: &str) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri("/api/session/hero")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, session_cookie)
        .body(Body::from(json!({ "hero_id": hero_id }).to_string()))
        .expect("the hero selection request must be valid")
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the response body must be readable");
    serde_json::from_slice(&body).expect("the response body must be JSON")
}

async fn create_room(app: &axum::Router) -> (String, String) {
    let response = app
        .clone()
        .oneshot(create_room_request(&unique_key("create-room")))
        .await
        .expect("room creation must receive a response");
    assert_eq!(response.status(), StatusCode::CREATED);

    let session_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("room creation must start the host session")
        .to_str()
        .expect("the session cookie must be ASCII")
        .split(';')
        .next()
        .expect("the session cookie must contain a value")
        .to_owned();
    let body = response_json(response).await;
    let room_code = body["room"]["code"]
        .as_str()
        .expect("room creation must return its code")
        .to_owned();

    (room_code, session_cookie)
}

#[tokio::test]
async fn guest_joins_an_open_room_and_restores_the_same_durable_position() {
    let app = build_router(test_state().await);
    let (room_code, _) = create_room(&app).await;
    let response = app
        .clone()
        .oneshot(join_room_request(
            &room_code,
            &unique_key("join-room"),
            "Luna",
            "hermione",
        ))
        .await
        .expect("room join must receive a response");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    let session_cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("room join must start a guest session")
        .to_str()
        .expect("the session cookie must be ASCII")
        .split(';')
        .next()
        .expect("the session cookie must contain a value")
        .to_owned();
    let body = response_json(response).await;

    assert_eq!(body["room"]["code"], room_code);
    assert_eq!(body["participant"]["display_name"], "Luna");
    assert_eq!(body["participant"]["role"], "guest");
    assert_eq!(body["participant"]["position"], 2);
    assert_eq!(body["participant"]["hero"]["id"], "hermione");
    assert_eq!(body["participant"]["hero"]["name"], "Hermione");
    assert_eq!(body["participants"].as_array().map(Vec::len), Some(2));

    let restored = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, session_cookie)
                .body(Body::empty())
                .expect("the session request must be valid"),
        )
        .await
        .expect("session restoration must receive a response");

    assert_eq!(restored.status(), StatusCode::OK);
    let restored = response_json(restored).await;
    assert_eq!(restored["room"]["code"], room_code);
    assert_eq!(restored["participant"], body["participant"]);
}

#[tokio::test]
async fn concurrent_joins_for_the_same_hero_have_exactly_one_winner() {
    let app = build_router(test_state().await);
    let (room_code, _) = create_room(&app).await;

    let (first, second) = tokio::join!(
        app.clone().oneshot(join_room_request(
            &room_code,
            &unique_key("join-luna"),
            "Luna",
            "harry",
        )),
        app.oneshot(join_room_request(
            &room_code,
            &unique_key("join-ginny"),
            "Ginny",
            "harry",
        )),
    );
    let first = first.expect("the first join must receive a response");
    let second = second.expect("the second join must receive a response");
    let statuses = [first.status(), second.status()];

    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CREATED)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::CONFLICT)
            .count(),
        1
    );

    let losing_response = if first.status() == StatusCode::CONFLICT {
        first
    } else {
        second
    };
    assert!(losing_response.headers().get(header::SET_COOKIE).is_none());
    let body = response_json(losing_response).await;
    assert_eq!(body["error"]["code"], "HERO_UNAVAILABLE");
    assert_eq!(body["error"]["retry"], "after_correction");
}

#[tokio::test]
async fn an_open_room_accepts_only_three_guests_after_its_host() {
    let app = build_router(test_state().await);
    let (room_code, _) = create_room(&app).await;

    for (display_name, hero_id, expected_position) in [
        ("Luna", "harry", 2),
        ("Ginny", "hermione", 3),
        ("Cedric", "neville", 4),
    ] {
        let response = app
            .clone()
            .oneshot(join_room_request(
                &room_code,
                &unique_key("fill-room"),
                display_name,
                hero_id,
            ))
            .await
            .expect("an available room position must be joinable");
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_json(response).await;
        assert_eq!(body["participant"]["position"], expected_position);
    }

    let full = app
        .oneshot(join_room_request(
            &room_code,
            &unique_key("overfill-room"),
            "Cho",
            "ron",
        ))
        .await
        .expect("the full room must reject another guest");

    assert_eq!(full.status(), StatusCode::CONFLICT);
    assert!(full.headers().get(header::SET_COOKIE).is_none());
    let body = response_json(full).await;
    assert_eq!(body["error"]["code"], "ROOM_FULL");
    assert_eq!(body["error"]["message_key"], "room.full");
}

#[tokio::test]
async fn hero_selection_updates_only_the_participant_derived_from_the_session() {
    let app = build_router(test_state().await);
    let (room_code, host_cookie) = create_room(&app).await;
    let host_selection = app
        .clone()
        .oneshot(select_hero_request(&host_cookie, "ron"))
        .await
        .expect("the host hero selection must receive a response");
    assert_eq!(host_selection.status(), StatusCode::OK);

    let joined = app
        .clone()
        .oneshot(join_room_request(
            &room_code,
            &unique_key("join-for-selection"),
            "Luna",
            "hermione",
        ))
        .await
        .expect("the guest join must receive a response");
    assert_eq!(joined.status(), StatusCode::CREATED);
    let guest_cookie = joined
        .headers()
        .get(header::SET_COOKIE)
        .expect("the guest must receive a session cookie")
        .to_str()
        .expect("the session cookie must be ASCII")
        .split(';')
        .next()
        .expect("the session cookie must contain a value")
        .to_owned();

    let changed = app
        .oneshot(select_hero_request(&guest_cookie, "neville"))
        .await
        .expect("the guest hero selection must receive a response");

    assert_eq!(changed.status(), StatusCode::OK);
    let body = response_json(changed).await;
    assert_eq!(body["participant"]["display_name"], "Luna");
    assert_eq!(body["participant"]["hero"]["id"], "neville");
    assert_eq!(body["participants"][0]["display_name"], "Minerva");
    assert_eq!(body["participants"][0]["hero"]["id"], "ron");
}

#[tokio::test]
async fn room_join_does_not_distinguish_an_unknown_code_from_a_closed_room() {
    let app = build_router(test_state().await);
    let (room_code, _) = create_room(&app).await;
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let database = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    sqlx::query("UPDATE rooms SET status = 'sealed' WHERE code = $1")
        .bind(&room_code)
        .execute(&database)
        .await
        .expect("the room must be sealable for this boundary test");

    let sealed = app
        .clone()
        .oneshot(join_room_request(
            &room_code,
            &unique_key("join-sealed"),
            "Luna",
            "harry",
        ))
        .await
        .expect("the sealed room must receive a safe response");
    let unknown = app
        .oneshot(join_room_request(
            "ZZZZZZZZ",
            &unique_key("join-unknown"),
            "Luna",
            "harry",
        ))
        .await
        .expect("the unknown room must receive a safe response");

    assert_eq!(sealed.status(), StatusCode::NOT_FOUND);
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let sealed = response_json(sealed).await;
    let unknown = response_json(unknown).await;
    for field in ["code", "category", "retry", "message_key", "details"] {
        assert_eq!(sealed["error"][field], unknown["error"][field]);
    }
    assert_eq!(sealed["error"]["code"], "ROOM_UNAVAILABLE");
}

#[tokio::test]
async fn room_lookup_reports_which_heroes_are_still_available() {
    let app = build_router(test_state().await);
    let (room_code, _) = create_room(&app).await;
    let joined = app
        .clone()
        .oneshot(join_room_request(
            &room_code,
            &unique_key("join-before-lookup"),
            "Luna",
            "harry",
        ))
        .await
        .expect("the guest join must receive a response");
    assert_eq!(joined.status(), StatusCode::CREATED);

    let lookup = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/rooms/{room_code}"))
                .body(Body::empty())
                .expect("the room lookup request must be valid"),
        )
        .await
        .expect("the room lookup must receive a response");

    assert_eq!(lookup.status(), StatusCode::OK);
    let body = response_json(lookup).await;
    assert_eq!(body["heroes"].as_array().map(Vec::len), Some(4));
    assert_eq!(body["heroes"][0]["id"], "harry");
    assert_eq!(body["heroes"][0]["available"], false);
    assert_eq!(body["heroes"][1]["id"], "hermione");
    assert_eq!(body["heroes"][1]["available"], true);
}

#[tokio::test]
async fn retrying_the_same_join_returns_the_same_participant_and_session_grant() {
    let app = build_router(test_state().await);
    let (room_code, _) = create_room(&app).await;
    let idempotency_key = unique_key("retry-join");

    let first = app
        .clone()
        .oneshot(join_room_request(
            &room_code,
            &idempotency_key,
            "Luna",
            "hermione",
        ))
        .await
        .expect("the first join must receive a response");
    let retried = app
        .clone()
        .oneshot(join_room_request(
            &room_code,
            &idempotency_key,
            "Luna",
            "hermione",
        ))
        .await
        .expect("the retried join must receive a response");

    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(retried.status(), StatusCode::CREATED);
    let first_cookie = first
        .headers()
        .get(header::SET_COOKIE)
        .expect("the first join must grant a session")
        .to_str()
        .expect("the first cookie must be ASCII")
        .split(';')
        .next()
        .expect("the first cookie must contain a value")
        .to_owned();
    let retried_cookie = retried
        .headers()
        .get(header::SET_COOKIE)
        .expect("the retried join must grant a session")
        .to_str()
        .expect("the retried cookie must be ASCII")
        .split(';')
        .next()
        .expect("the retried cookie must contain a value")
        .to_owned();
    assert_eq!(first_cookie, retried_cookie);
    assert_eq!(response_json(first).await, response_json(retried).await);

    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let database = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    let session_count = sqlx::query_scalar::<_, i64>(
        r"
        SELECT COUNT(*)
        FROM guest_sessions
        JOIN room_join_requests
          ON room_join_requests.guest_session_id = guest_sessions.id
        WHERE room_join_requests.idempotency_key = $1
        ",
    )
    .bind(&idempotency_key)
    .fetch_one(&database)
    .await
    .expect("the idempotent session grant must remain queryable");
    assert_eq!(session_count, 1);

    for cookie in [first_cookie, retried_cookie] {
        let restored = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/session")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("the restore request must be valid"),
            )
            .await
            .expect("each granted session must receive a response");
        assert_eq!(restored.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn a_legacy_join_retry_requires_its_existing_session() {
    let app = build_router(test_state().await);
    let (room_code, _) = create_room(&app).await;
    let original_key = unique_key("legacy-join");
    let first = app
        .clone()
        .oneshot(join_room_request(
            &room_code,
            &original_key,
            "Luna",
            "hermione",
        ))
        .await
        .expect("the initial join must receive a response");
    assert_eq!(first.status(), StatusCode::CREATED);
    let original_cookie = first
        .headers()
        .get(header::SET_COOKIE)
        .expect("the initial join must grant a session")
        .to_str()
        .expect("the session cookie must be ASCII")
        .split(';')
        .next()
        .expect("the session cookie must contain a value")
        .to_owned();
    let legacy_key = format!("legacy-{}", uuid::Uuid::new_v4());
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let database = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    sqlx::query("UPDATE room_join_requests SET idempotency_key = $2 WHERE idempotency_key = $1")
        .bind(original_key)
        .bind(&legacy_key)
        .execute(&database)
        .await
        .expect("the fixture must represent a persisted legacy request");

    let unauthenticated = app
        .clone()
        .oneshot(join_room_request(
            &room_code,
            &legacy_key,
            "Luna",
            "hermione",
        ))
        .await
        .expect("the unauthenticated retry must receive a response");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let mut authenticated_request = join_room_request(&room_code, &legacy_key, "Luna", "hermione");
    authenticated_request.headers_mut().insert(
        header::COOKIE,
        original_cookie
            .parse()
            .expect("the fixture cookie must be a valid header"),
    );
    let authenticated = app
        .clone()
        .oneshot(authenticated_request)
        .await
        .expect("the authenticated legacy retry must receive a response");
    assert_eq!(authenticated.status(), StatusCode::CREATED);
    let replacement_cookie = authenticated
        .headers()
        .get(header::SET_COOKIE)
        .expect("the authorized retry must refresh its grant")
        .to_str()
        .expect("the replacement cookie must be ASCII")
        .split(';')
        .next()
        .expect("the replacement cookie must contain a value")
        .to_owned();
    let restored = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, replacement_cookie)
                .body(Body::empty())
                .expect("the restore request must be valid"),
        )
        .await
        .expect("the replacement session must receive a response");
    assert_eq!(restored.status(), StatusCode::OK);
}
