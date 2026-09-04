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
    let state =
        AppState::new(database).with_session_token_key(*b"test-session-token-key-000000000");

    initialize(&state)
        .await
        .expect("database initialization must succeed");

    state
}

fn unique_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn room_creation_request(
    idempotency_key: &str,
    display_name: &str,
    password: &str,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/rooms")
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", idempotency_key)
        .body(Body::from(
            json!({
                "display_name": display_name,
                "recovery_password": password
            })
            .to_string(),
        ))
        .expect("the request must be valid")
}

fn session_cookie(response: &axum::response::Response) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .expect("the response must grant a session")
        .to_str()
        .expect("the session cookie must be ASCII")
        .split(';')
        .next()
        .expect("the session cookie must contain a value")
        .to_owned()
}

async fn persisted_room_creation_counts(
    idempotency_key: &str,
) -> (i64, i64, i64, i64, i64, String) {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let database = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    sqlx::query_as(
        r"
        SELECT
            COUNT(DISTINCT guest_identities.id),
            COUNT(DISTINCT rooms.id),
            COUNT(DISTINCT participants.id),
            COUNT(DISTINCT guest_sessions.id),
            COUNT(DISTINCT device_sessions.id),
            MIN(rooms.recovery_password_hash)
        FROM room_creation_requests
        JOIN rooms ON rooms.id = room_creation_requests.room_id
        JOIN participants ON participants.id = room_creation_requests.participant_id
        JOIN guest_identities ON guest_identities.id = participants.guest_identity_id
        JOIN guest_sessions
          ON guest_sessions.guest_identity_id = guest_identities.id
        JOIN device_sessions
          ON device_sessions.guest_session_id = guest_sessions.id
         AND device_sessions.participant_id = participants.id
        WHERE room_creation_requests.idempotency_key = $1
        ",
    )
    .bind(idempotency_key)
    .fetch_one(&database)
    .await
    .expect("the committed room creation must be queryable")
}

#[tokio::test]
async fn room_creation_rejects_a_weak_recovery_password_without_starting_a_session() {
    let response = build_router(test_state().await)
        .oneshot(room_creation_request(&unique_key(), "Minerva", "password"))
        .await
        .expect("the room router must respond");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let correlation_id = response
        .headers()
        .get("x-correlation-id")
        .expect("every response must expose its request correlation ID")
        .to_str()
        .expect("the correlation ID must be ASCII")
        .to_owned();
    assert!(
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .next()
            .is_none()
    );

    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the error body must be readable");
    let body: Value = serde_json::from_slice(&body).expect("the error body must be JSON");

    assert_eq!(body["error"]["code"], "WEAK_RECOVERY_PASSWORD");
    assert_eq!(body["error"]["retry"], "after_correction");
    assert_eq!(body["error"]["correlation_id"], correlation_id);
}

#[tokio::test]
async fn room_creation_requires_an_unpredictable_session_grant_key() {
    let response = build_router(test_state().await)
        .oneshot(room_creation_request(
            "predictable-idempotency-key",
            "Minerva",
            "a long uncommon passphrase",
        ))
        .await
        .expect("the room router must respond");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the error body must be readable");
    let body: Value = serde_json::from_slice(&body).expect("the error body must be JSON");
    assert_eq!(body["error"]["code"], "INVALID_IDEMPOTENCY_KEY");
}

#[tokio::test]
async fn room_creation_rejects_a_long_but_repetitive_recovery_password() {
    let response = build_router(test_state().await)
        .oneshot(room_creation_request(
            &unique_key(),
            "Minerva",
            "minervaminerva",
        ))
        .await
        .expect("the room router must respond");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn room_creation_rejects_a_password_based_recovery_phrase() {
    let response = build_router(test_state().await)
        .oneshot(room_creation_request(
            &unique_key(),
            "Minerva",
            "passwordpassword",
        ))
        .await
        .expect("the room router must respond");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn room_creation_rejects_a_sequential_recovery_password() {
    let response = build_router(test_state().await)
        .oneshot(room_creation_request(
            &unique_key(),
            "Minerva",
            "abcdefghijkl",
        ))
        .await
        .expect("the room router must respond");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn guest_creates_an_open_room_and_receives_a_secure_server_side_session() {
    let app = build_router(test_state().await);
    let idempotency_key = unique_key();
    let response = app
        .clone()
        .oneshot(room_creation_request(
            &idempotency_key,
            "Minerva",
            "a long uncommon passphrase",
        ))
        .await
        .expect("the room router must respond");

    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        response.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );

    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("room creation must start a session")
        .to_str()
        .expect("the session cookie must be valid ASCII");
    assert!(cookie.starts_with("__Host-session="));
    assert!(cookie.contains("; Path=/"));
    assert!(cookie.contains("; Secure"));
    assert!(cookie.contains("; HttpOnly"));
    assert!(cookie.contains("; SameSite=Strict"));

    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the response body must be readable");
    let body: Value = serde_json::from_slice(&body).expect("the response body must be JSON");
    let room_code = body["room"]["code"]
        .as_str()
        .expect("the room code must be a string");

    assert_eq!(room_code.len(), 8);
    assert_eq!(body["room"]["status"], "open");
    assert_eq!(body["participant"]["display_name"], "Minerva");
    assert_eq!(body["participant"]["role"], "host");
    assert!(!body.to_string().contains("a long uncommon passphrase"));

    let lookup = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/rooms/{room_code}"))
                .body(Body::empty())
                .expect("the lookup request must be valid"),
        )
        .await
        .expect("the room router must respond");

    assert_eq!(lookup.status(), StatusCode::OK);
    assert!(lookup.headers().get(header::SET_COOKIE).is_none());

    let lookup_body = to_bytes(lookup.into_body(), 64 * 1024)
        .await
        .expect("the lookup body must be readable");
    let lookup_body: Value =
        serde_json::from_slice(&lookup_body).expect("the lookup body must be JSON");

    assert_eq!(lookup_body["room"]["code"], room_code);
    assert_eq!(lookup_body["room"]["status"], "open");
}

#[tokio::test]
async fn a_session_bearer_token_is_stored_only_as_a_digest_and_still_authenticates() {
    let app = build_router(test_state().await);
    let idempotency_key = unique_key();
    let response = app
        .clone()
        .oneshot(room_creation_request(
            &idempotency_key,
            "Minerva",
            "a long uncommon passphrase",
        ))
        .await
        .expect("room creation must receive a response");
    assert_eq!(response.status(), StatusCode::CREATED);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("room creation must start a session")
        .to_str()
        .expect("the session cookie must be ASCII")
        .split(';')
        .next()
        .expect("the cookie must contain a value")
        .to_owned();
    let bearer = cookie
        .strip_prefix("__Host-session=")
        .expect("the cookie must use the session name");

    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let database = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    let stored = sqlx::query_scalar::<_, String>(
        r"
        SELECT guest_sessions.token_digest
        FROM guest_sessions
        JOIN room_creation_requests
          ON room_creation_requests.guest_session_id = guest_sessions.id
        WHERE room_creation_requests.idempotency_key = $1
        ",
    )
    .bind(&idempotency_key)
    .fetch_one(&database)
    .await
    .expect("the stored session credential must be queryable");
    assert_ne!(stored, bearer);
    assert!(stored.starts_with("sha256:"));
    assert_eq!(stored.len(), "sha256:".len() + 64);

    let restored = app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("the restore request must be valid"),
        )
        .await
        .expect("the original bearer token must still authenticate");
    assert_eq!(restored.status(), StatusCode::OK);
}

#[tokio::test]
async fn room_creation_retry_survives_application_state_reconstruction() {
    let idempotency_key = unique_key();
    let first_app = build_router(test_state().await);
    let first = first_app
        .oneshot(room_creation_request(
            &idempotency_key,
            "Minerva",
            "a long uncommon passphrase",
        ))
        .await
        .expect("the initial room creation must receive a response");
    assert_eq!(first.status(), StatusCode::CREATED);
    let first_cookie = session_cookie(&first);

    let restarted_app = build_router(test_state().await);
    let retried = restarted_app
        .clone()
        .oneshot(room_creation_request(
            &idempotency_key,
            "Minerva",
            "a long uncommon passphrase",
        ))
        .await
        .expect("the retried room creation must receive a response");
    assert_eq!(retried.status(), StatusCode::CREATED);
    let retried_cookie = session_cookie(&retried);
    assert_eq!(retried_cookie, first_cookie);

    let restored = restarted_app
        .oneshot(
            Request::builder()
                .uri("/api/session")
                .header(header::COOKIE, retried_cookie)
                .body(Body::empty())
                .expect("the restore request must be valid"),
        )
        .await
        .expect("the replayed session must authenticate after restart");
    assert_eq!(restored.status(), StatusCode::OK);
}

#[tokio::test]
async fn concurrent_identical_retries_return_one_room_and_one_session_grant() {
    let state = test_state().await;
    let app = build_router(state);
    let unique_suffix = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("the system clock must be after the Unix epoch")
            .as_nanos()
    );
    let idempotency_key = unique_key();
    let display_name = format!("Host {unique_suffix}");
    let password = "another long uncommon passphrase";

    let (first, second) = tokio::join!(
        app.clone().oneshot(room_creation_request(
            &idempotency_key,
            &display_name,
            password
        )),
        app.clone().oneshot(room_creation_request(
            &idempotency_key,
            &display_name,
            password
        ))
    );
    let first = first.expect("the first retry must receive a response");
    let second = second.expect("the concurrent retry must receive a response");

    assert_eq!(first.status(), StatusCode::CREATED);
    assert_eq!(second.status(), StatusCode::CREATED);
    let first_cookie = session_cookie(&first);
    let second_cookie = session_cookie(&second);
    assert_eq!(first_cookie, second_cookie);

    let first_body = to_bytes(first.into_body(), 64 * 1024)
        .await
        .expect("the first response body must be readable");
    let second_body = to_bytes(second.into_body(), 64 * 1024)
        .await
        .expect("the retry response body must be readable");
    assert_eq!(first_body, second_body);

    let persisted = persisted_room_creation_counts(&idempotency_key).await;

    assert_eq!(persisted.0, 1);
    assert_eq!(persisted.1, 1);
    assert_eq!(persisted.2, 1);
    assert_eq!(persisted.3, 1);
    assert_eq!(persisted.4, 1);
    assert!(persisted.5.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    assert!(!persisted.5.contains(password));

    for cookie in [first_cookie, second_cookie] {
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
async fn reusing_an_idempotency_key_for_another_payload_is_rejected() {
    let app = build_router(test_state().await);
    let idempotency_key = unique_key();
    let first = app
        .clone()
        .oneshot(room_creation_request(
            &idempotency_key,
            "Minerva",
            "a long uncommon passphrase",
        ))
        .await
        .expect("the initial room creation must receive a response");
    assert_eq!(first.status(), StatusCode::CREATED);

    let conflicting = app
        .oneshot(room_creation_request(
            &idempotency_key,
            "Pomona",
            "a different uncommon passphrase",
        ))
        .await
        .expect("the conflicting retry must receive a response");

    assert_eq!(conflicting.status(), StatusCode::CONFLICT);
    assert!(conflicting.headers().get(header::SET_COOKIE).is_none());
    let body = to_bytes(conflicting.into_body(), 64 * 1024)
        .await
        .expect("the conflict body must be readable");
    let body: Value = serde_json::from_slice(&body).expect("the conflict body must be JSON");
    assert_eq!(body["error"]["code"], "IDEMPOTENCY_KEY_REUSED");
}
