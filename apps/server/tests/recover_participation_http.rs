use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tower::ServiceExt;

const RECOVERY_PASSWORD: &str = "a long uncommon passphrase";

async fn test_state() -> (axum::Router, PgPool) {
    let database_url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must point to the integration PostgreSQL database");
    let database = PgPoolOptions::new()
        .connect(&database_url)
        .await
        .expect("the integration PostgreSQL database must be available");
    let state = AppState::new(database.clone())
        .with_session_token_key(*b"test-session-token-key-000000000");
    initialize(&state)
        .await
        .expect("database initialization must succeed");
    (build_router(state), database)
}

fn create_room_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/rooms")
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .body(Body::from(
            json!({
                "display_name": "Minerva",
                "recovery_password": RECOVERY_PASSWORD
            })
            .to_string(),
        ))
        .expect("the room creation request must be valid")
}

fn recovery_request(token: &str, password: &str, attempt_id: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/session/recover")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "recovery_token": token,
                "recovery_password": password,
                "recovery_attempt_id": attempt_id
            })
            .to_string(),
        ))
        .expect("the participant recovery request must be valid")
}

fn session_cookie(response: &Response<Body>) -> &str {
    response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .expect("participant recovery must set a session cookie")
}

async fn assert_concurrent_idempotent_redelivery(
    app: &axum::Router,
    token: &str,
    attempt_id: &str,
    expected_cookie: &str,
) {
    let (first, second) = tokio::join!(
        app.clone()
            .oneshot(recovery_request(token, RECOVERY_PASSWORD, attempt_id)),
        app.clone()
            .oneshot(recovery_request(token, RECOVERY_PASSWORD, attempt_id)),
    );
    for response in [first, second] {
        let response = response.expect("the recovery retry must receive a response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(session_cookie(&response), expected_cookie);
    }
}

async fn response_json(response: Response<Body>) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the response body must be readable");
    serde_json::from_slice(&body).expect("the response body must be JSON")
}

async fn safe_error_body(response: Response<Body>) -> Value {
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let mut body = response_json(response).await;
    body["error"]
        .as_object_mut()
        .expect("the error envelope must contain an object")
        .remove("correlation_id");
    body
}

async fn persisted_recovery_credential(
    database: &PgPool,
    room_code: &str,
) -> (String, String, i64) {
    sqlx::query_as(
        r"
        SELECT
            recovery_credentials.token_hmac,
            recovery_credentials.status,
            (
                SELECT COUNT(*)
                FROM device_sessions
                JOIN guest_sessions
                  ON guest_sessions.id = device_sessions.guest_session_id
                WHERE device_sessions.participant_id = participants.id
                  AND device_sessions.status = 'active'
                  AND guest_sessions.expires_at > clock_timestamp()
            )
        FROM recovery_credentials
        JOIN participants ON participants.id = recovery_credentials.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(room_code)
    .fetch_one(database)
    .await
    .expect("the recovery credential must be persisted")
}

fn assert_hmac_only(stored_hmac: &str, recovery_token: &str) {
    assert_ne!(stored_hmac, recovery_token);
    assert!(stored_hmac.starts_with("hmac-sha256:"));
    assert_eq!(stored_hmac.len(), "hmac-sha256:".len() + 64);
}

async fn consumed_recovery_state(database: &PgPool, stored_hmac: &str) -> (String, i64, bool) {
    sqlx::query_as(
        r"
        SELECT
            recovery_credentials.status,
            (
                SELECT COUNT(*)
                FROM device_sessions
                JOIN guest_sessions
                  ON guest_sessions.id = device_sessions.guest_session_id
                WHERE device_sessions.participant_id = recovery_credentials.participant_id
                  AND device_sessions.status = 'active'
                  AND guest_sessions.expires_at > clock_timestamp()
            ),
            EXISTS(
                SELECT 1
                FROM device_sessions
                WHERE device_sessions.guest_session_id =
                      recovery_credentials.consumed_by_guest_session_id
                  AND device_sessions.participant_id = recovery_credentials.participant_id
                  AND device_sessions.status = 'active'
            )
        FROM recovery_credentials
        WHERE token_hmac = $1
        ",
    )
    .bind(stored_hmac)
    .fetch_one(database)
    .await
    .expect("the consumed recovery credential must be queryable")
}

#[tokio::test]
async fn a_recovery_token_is_single_use_hmac_only_and_creates_one_second_session_atomically() {
    let (app, database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = response_json(created).await;
    let recovery_token = created["recovery_token"]
        .as_str()
        .expect("room creation must return the individual recovery token")
        .to_owned();
    let room_code = created["room"]["code"]
        .as_str()
        .expect("room creation must return its code")
        .to_owned();
    assert_eq!(recovery_token.len(), 64);
    assert!(recovery_token.bytes().all(|byte| byte.is_ascii_hexdigit()));

    let (stored_hmac, initial_status, initial_sessions) =
        persisted_recovery_credential(&database, &room_code).await;
    assert_hmac_only(&stored_hmac, &recovery_token);
    assert_eq!(initial_status, "active");
    assert_eq!(initial_sessions, 1);

    let wrong_password = app
        .clone()
        .oneshot(recovery_request(
            &recovery_token,
            "an incorrect uncommon passphrase",
            &uuid::Uuid::new_v4().to_string(),
        ))
        .await
        .expect("an isolated link must receive a safe response");
    let wrong_token = app
        .clone()
        .oneshot(recovery_request(
            &"0".repeat(64),
            RECOVERY_PASSWORD,
            &uuid::Uuid::new_v4().to_string(),
        ))
        .await
        .expect("an isolated password must receive a safe response");
    assert_eq!(
        safe_error_body(wrong_password).await,
        safe_error_body(wrong_token).await
    );

    let first_attempt = uuid::Uuid::new_v4().to_string();
    let second_attempt = uuid::Uuid::new_v4().to_string();
    let (first, second) = tokio::join!(
        app.clone().oneshot(recovery_request(
            &recovery_token,
            RECOVERY_PASSWORD,
            &first_attempt
        )),
        app.clone().oneshot(recovery_request(
            &recovery_token,
            RECOVERY_PASSWORD,
            &second_attempt
        )),
    );
    let first = first.expect("the first recovery must receive a response");
    let second = second.expect("the concurrent recovery must receive a response");
    let statuses = [first.status(), second.status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::UNAUTHORIZED)
            .count(),
        1
    );
    let (success, winning_attempt) = if first.status() == StatusCode::OK {
        (first, first_attempt)
    } else {
        (second, second_attempt)
    };
    let recovered_cookie = session_cookie(&success).to_owned();
    let recovered = response_json(success).await;
    assert_eq!(recovered["participant"], created["participant"]);
    assert_eq!(recovered["participant"]["position"], 1);
    assert_concurrent_idempotent_redelivery(
        &app,
        &recovery_token,
        &winning_attempt,
        &recovered_cookie,
    )
    .await;

    let (status, active_sessions, consumed_session_matches) =
        consumed_recovery_state(&database, &stored_hmac).await;
    assert_eq!(status, "consumed");
    assert_eq!(active_sessions, 2);
    assert!(consumed_session_matches);
}
