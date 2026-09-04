use std::{fmt::Write as _, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const CURRENT_PASSWORD: &str = "a long uncommon passphrase";
const NEW_PASSWORD: &str = "a newer uncommon recovery phrase";

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
                "recovery_password": CURRENT_PASSWORD
            })
            .to_string(),
        ))
        .expect("the room creation request must be valid")
}

fn join_room_request(room_code: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/rooms/{room_code}/participants"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .body(Body::from(
            json!({
                "display_name": "Luna",
                "hero_id": "hermione"
            })
            .to_string(),
        ))
        .expect("the room join request must be valid")
}

fn rotate_password_request(
    cookie: Option<&str>,
    idempotency_key: &str,
    current_password: &str,
    new_password: &str,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method("PUT")
        .uri("/api/session/recovery-password")
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", idempotency_key);
    if let Some(cookie) = cookie {
        builder = builder.header(header::COOKIE, cookie);
    }
    builder
        .body(Body::from(
            json!({
                "current_recovery_password": current_password,
                "new_recovery_password": new_password
            })
            .to_string(),
        ))
        .expect("the password rotation request must be valid")
}

fn restore_session_request(cookie: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/api/session")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("the session restore request must be valid")
}

fn recover_participation_request(token: &str, password: &str) -> Request<Body> {
    recover_participation_attempt_request(token, password, &uuid::Uuid::new_v4().to_string())
}

fn recover_participation_attempt_request(
    token: &str,
    password: &str,
    recovery_attempt_id: &str,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/session/recover")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "recovery_token": token,
                "recovery_password": password,
                "recovery_attempt_id": recovery_attempt_id
            })
            .to_string(),
        ))
        .expect("the participant recovery request must be valid")
}

fn regenerate_own_credential_request(cookie: &str, idempotency_key: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/session/recovery-credential")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header("idempotency-key", idempotency_key)
        .body(Body::from("{}"))
        .expect("the recovery credential regeneration request must be valid")
}

fn regenerate_assisted_credential_request(
    cookie: &str,
    idempotency_key: &str,
    target_position: i16,
    risk_acknowledged: bool,
) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(format!(
            "/api/rooms/current/participants/{target_position}/recovery-credential"
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header("idempotency-key", idempotency_key)
        .body(Body::from(
            json!({
                "host_assistance_risk_acknowledged": risk_acknowledged
            })
            .to_string(),
        ))
        .expect("the assisted recovery credential request must be valid")
}

fn session_cookie(response: &Response<Body>) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .expect("room creation must set a session cookie")
        .to_owned()
}

async fn response_json(response: Response<Body>) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the response body must be readable");
    serde_json::from_slice(&body).expect("the response body must be JSON")
}

struct CreatedRoomFixture {
    host_cookie: String,
    room_code: String,
    recovery_token: String,
    participant: Value,
}

struct JoinedParticipantFixture {
    guest_cookie: String,
    recovery_token: String,
    participant: Value,
}

async fn create_room_fixture(app: &axum::Router) -> CreatedRoomFixture {
    let response = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let host_cookie = session_cookie(&response);
    let body = response_json(response).await;
    CreatedRoomFixture {
        host_cookie,
        room_code: body["room"]["code"]
            .as_str()
            .expect("room creation must return its code")
            .to_owned(),
        recovery_token: body["recovery_token"]
            .as_str()
            .expect("room creation must return a recovery token")
            .to_owned(),
        participant: body["participant"].clone(),
    }
}

async fn join_room_fixture(app: &axum::Router, room_code: &str) -> JoinedParticipantFixture {
    let response = app
        .clone()
        .oneshot(join_room_request(room_code))
        .await
        .expect("room join must receive a response");
    let guest_cookie = session_cookie(&response);
    let body = response_json(response).await;
    JoinedParticipantFixture {
        guest_cookie,
        recovery_token: body["recovery_token"]
            .as_str()
            .expect("room join must return a recovery token")
            .to_owned(),
        participant: body["participant"].clone(),
    }
}

async fn room_active_session_state(database: &PgPool, room_code: &str) -> (i64, String, String) {
    sqlx::query_as::<_, (i64, String, String)>(
        r"
        SELECT
            COUNT(*),
            MIN(guest_sessions.expires_at)::text,
            MAX(guest_sessions.expires_at)::text
        FROM device_sessions
        JOIN guest_sessions ON guest_sessions.id = device_sessions.guest_session_id
        JOIN participants ON participants.id = device_sessions.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
          AND device_sessions.status = 'active'
        ",
    )
    .bind(room_code)
    .fetch_one(database)
    .await
    .expect("the authenticated sessions must be queryable")
}

async fn rotated_credential_state(database: &PgPool, room_code: &str) -> (i64, i64, i64) {
    sqlx::query_as::<_, (i64, i64, i64)>(
        r"
        SELECT
            COUNT(*) FILTER (WHERE recovery_credentials.status = 'superseded'),
            MIN(recovery_credentials.password_generation),
            MAX(rooms.password_generation)
        FROM recovery_credentials
        JOIN participants ON participants.id = recovery_credentials.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
        ",
    )
    .bind(room_code)
    .fetch_one(database)
    .await
    .expect("the invalidated recovery credentials must be queryable")
}

async fn password_rotation_artifacts(database: &PgPool, room_code: &str) -> (i64, i64, i64) {
    sqlx::query_as::<_, (i64, i64, i64)>(
        r"
        SELECT
            rooms.password_generation,
            COUNT(DISTINCT requests.idempotency_key),
            COUNT(DISTINCT events.sequence)
        FROM rooms
        LEFT JOIN recovery_password_rotation_requests AS requests
          ON requests.room_id = rooms.id
        LEFT JOIN identity_security_events AS events
          ON events.room_id = rooms.id
        WHERE rooms.code = $1
        GROUP BY rooms.id
        ",
    )
    .bind(room_code)
    .fetch_one(database)
    .await
    .expect("the single committed rotation must be queryable")
}

async fn wait_for_waiting_rotation_queries(database: &PgPool) {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let waiting = sqlx::query_scalar::<_, i64>(
                r"
                SELECT COUNT(*)
                FROM pg_stat_activity
                WHERE datname = current_database()
                  AND usename = current_user
                  AND wait_event_type = 'Lock'
                  AND query LIKE '%FOR UPDATE OF rooms%'
                ",
            )
            .fetch_one(database)
            .await
            .expect("the waiting rotation queries must be observable");
            if waiting >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both rotations must reach the held room lock");
}

async fn participant_recovery_credentials(
    database: &PgPool,
    room_code: &str,
) -> Vec<(String, i64, String)> {
    sqlx::query_as::<_, (String, i64, String)>(
        r"
        SELECT
            recovery_credentials.status,
            recovery_credentials.recovery_generation,
            recovery_credentials.token_hmac
        FROM recovery_credentials
        JOIN participants ON participants.id = recovery_credentials.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
        ORDER BY recovery_credentials.recovery_generation
        ",
    )
    .bind(room_code)
    .fetch_all(database)
    .await
    .expect("the credential generations must be queryable")
}

async fn room_active_session_count(database: &PgPool, room_code: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r"
        SELECT COUNT(*)
        FROM device_sessions
        JOIN participants ON participants.id = device_sessions.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
          AND device_sessions.status = 'active'
        ",
    )
    .bind(room_code)
    .fetch_one(database)
    .await
    .expect("the active session count must be queryable")
}

async fn security_event_recipient_positions(
    database: &PgPool,
    room_code: &str,
    delivery: &str,
) -> Vec<i16> {
    sqlx::query_scalar::<_, i16>(
        r"
        SELECT recipient.position
        FROM identity_security_events AS events
        JOIN rooms ON rooms.id = events.room_id
        JOIN identity_security_event_recipients AS event_recipients
          ON event_recipients.room_id = events.room_id
         AND event_recipients.security_event_sequence = events.sequence
        JOIN participants AS recipient ON recipient.id = event_recipients.participant_id
        WHERE rooms.code = $1
          AND events.event_type = 'recovery_credential_regenerated'
          AND events.delivery = $2
        ORDER BY recipient.position
        ",
    )
    .bind(room_code)
    .bind(delivery)
    .fetch_all(database)
    .await
    .expect("the security event recipients must be queryable")
}

async fn regeneration_receipt(
    database: &PgPool,
    idempotency_key: &str,
) -> (String, Option<i64>, Option<i64>, bool) {
    sqlx::query_as::<_, (String, Option<i64>, Option<i64>, bool)>(
        r"
        SELECT
            delivery,
            recovery_generation,
            security_event_sequence,
            completed_at IS NOT NULL
        FROM recovery_credential_regeneration_requests
        WHERE idempotency_key = $1
        ",
    )
    .bind(idempotency_key)
    .fetch_one(database)
    .await
    .expect("the credential regeneration receipt must be queryable")
}

async fn assisted_target_state(database: &PgPool, room_code: &str) -> (i64, i64) {
    sqlx::query_as::<_, (i64, i64)>(
        r"
        SELECT
            participants.recovery_generation,
            (
                SELECT COUNT(*)
                FROM identity_security_events
                WHERE identity_security_events.room_id = rooms.id
            )
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
          AND participants.position = 2
        ",
    )
    .bind(room_code)
    .fetch_one(database)
    .await
    .expect("the rejected assistance state must be queryable")
}

async fn linearized_recovery_state(database: &PgPool, room_code: &str) -> (i64, i64, i64, i64) {
    sqlx::query_as::<_, (i64, i64, i64, i64)>(
        r"
        SELECT
            rooms.password_generation,
            COUNT(credentials.id) FILTER (WHERE credentials.status = 'active'),
            COUNT(credentials.id) FILTER (WHERE credentials.status = 'consumed'),
            (
                SELECT COUNT(*)
                FROM device_sessions
                JOIN participants AS session_participants
                  ON session_participants.id = device_sessions.participant_id
                WHERE session_participants.room_id = rooms.id
                  AND device_sessions.status = 'active'
            )
        FROM rooms
        JOIN participants ON participants.room_id = rooms.id
        JOIN recovery_credentials AS credentials
          ON credentials.participant_id = participants.id
        WHERE rooms.code = $1
        GROUP BY rooms.id
        ",
    )
    .bind(room_code)
    .fetch_one(database)
    .await
    .expect("the linearized recovery state must be queryable")
}

fn assert_direct_regeneration_response(regenerated: &Value, original_token: &str) -> String {
    assert_eq!(regenerated["delivery"], "direct");
    assert_eq!(regenerated["participant"]["display_name"], "Minerva");
    assert_eq!(regenerated["participant"]["position"], 1);
    assert_eq!(regenerated["recovery_generation"], 2);
    assert_eq!(
        regenerated["security_event"]["type"],
        "recovery_credential_regenerated"
    );
    assert_eq!(regenerated["security_event"]["delivery"], "direct");
    let successor_token = regenerated["recovery_token"]
        .as_str()
        .expect("direct regeneration must deliver the successor token");
    assert_ne!(successor_token, original_token);
    assert_eq!(successor_token.len(), 64);
    assert!(successor_token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    successor_token.to_owned()
}

fn assert_assisted_regeneration_response(assisted: &Value) -> String {
    assert_eq!(assisted["delivery"], "host_assisted");
    assert_eq!(assisted["participant"]["display_name"], "Luna");
    assert_eq!(assisted["participant"]["position"], 2);
    assert_eq!(assisted["recovery_generation"], 2);
    assert_eq!(
        assisted["risk_message_key"],
        "participant.recovery.host_assisted_impersonation_risk"
    );
    assert_eq!(
        assisted["security_event"]["type"],
        "recovery_credential_regenerated"
    );
    assert_eq!(assisted["security_event"]["delivery"], "host_assisted");
    assert_eq!(assisted["security_event"]["actor_position"], 1);
    assert_eq!(assisted["security_event"]["target_position"], 2);
    assisted["recovery_token"]
        .as_str()
        .expect("host assistance must deliver the successor token")
        .to_owned()
}

async fn assert_assistance_rejections(app: &axum::Router, host_cookie: &str, guest_cookie: &str) {
    let guest_attempt = app
        .clone()
        .oneshot(regenerate_assisted_credential_request(
            guest_cookie,
            &uuid::Uuid::new_v4().to_string(),
            1,
            true,
        ))
        .await
        .expect("a guest assistance attempt must receive a response");
    assert_eq!(guest_attempt.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        response_json(guest_attempt).await["error"]["code"],
        "NOT_ROOM_HOST"
    );

    let unacknowledged = app
        .clone()
        .oneshot(regenerate_assisted_credential_request(
            host_cookie,
            &uuid::Uuid::new_v4().to_string(),
            2,
            false,
        ))
        .await
        .expect("an unacknowledged assistance attempt must receive a response");
    assert_eq!(unacknowledged.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response_json(unacknowledged).await["error"]["code"],
        "HOST_ASSISTANCE_RISK_NOT_ACKNOWLEDGED"
    );

    let self_assistance = app
        .clone()
        .oneshot(regenerate_assisted_credential_request(
            host_cookie,
            &uuid::Uuid::new_v4().to_string(),
            1,
            true,
        ))
        .await
        .expect("self-assistance must receive a response");
    assert_eq!(self_assistance.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        response_json(self_assistance).await["error"]["code"],
        "RECOVERY_ASSISTANCE_NOT_REQUIRED"
    );
}

async fn assert_sessions_remain_valid(app: &axum::Router, cookies: &[&str]) {
    for cookie in cookies {
        let restored = app
            .clone()
            .oneshot(restore_session_request(cookie))
            .await
            .expect("an existing session must receive a response");
        assert_eq!(restored.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn a_host_rotates_the_recovery_password_after_confirming_the_current_password() {
    let (app, database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    assert_eq!(created.status(), StatusCode::CREATED);
    let host_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let room_code = created["room"]["code"]
        .as_str()
        .expect("room creation must return its code");
    let (initial_hash, initial_generation) = sqlx::query_as::<_, (String, i64)>(
        "SELECT recovery_password_hash, password_generation FROM rooms WHERE code = $1",
    )
    .bind(room_code)
    .fetch_one(&database)
    .await
    .expect("the initial recovery authority must be queryable");

    let response = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &uuid::Uuid::new_v4().to_string(),
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("password rotation must receive a response");

    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    let body = response_json(response).await;
    assert_eq!(body["password_generation"], initial_generation + 1);
    assert_eq!(body["security_event"]["type"], "recovery_password_rotated");
    assert_eq!(body["security_event"]["actor_position"], 1);
    assert_eq!(body["security_event"]["password_generation"], 2);
    assert!(!body.to_string().contains(CURRENT_PASSWORD));
    assert!(!body.to_string().contains(NEW_PASSWORD));

    let (rotated_hash, rotated_generation) = sqlx::query_as::<_, (String, i64)>(
        "SELECT recovery_password_hash, password_generation FROM rooms WHERE code = $1",
    )
    .bind(room_code)
    .fetch_one(&database)
    .await
    .expect("the rotated recovery authority must be queryable");
    assert_ne!(rotated_hash, initial_hash);
    assert!(rotated_hash.starts_with("$argon2id$"));
    assert_eq!(rotated_generation, initial_generation + 1);

    let restored = app
        .oneshot(restore_session_request(&host_cookie))
        .await
        .expect("the existing host session must receive a response");
    assert_eq!(restored.status(), StatusCode::OK);
}

#[tokio::test]
async fn password_rotation_is_idempotent_and_rejects_a_reused_key_with_another_payload() {
    let (app, database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let host_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let room_code = created["room"]["code"]
        .as_str()
        .expect("room creation must return its code");
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let first = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &idempotency_key,
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("the first password rotation must receive a response");
    assert_eq!(first.status(), StatusCode::OK);
    let first = response_json(first).await;

    let replay = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &idempotency_key,
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("the password rotation retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, first);

    let conflict = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &idempotency_key,
            CURRENT_PASSWORD,
            "yet another uncommon recovery phrase",
        ))
        .await
        .expect("the conflicting password rotation must receive a response");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(conflict).await["error"]["code"],
        "IDEMPOTENCY_KEY_REUSED"
    );

    let generation =
        sqlx::query_scalar::<_, i64>("SELECT password_generation FROM rooms WHERE code = $1")
            .bind(room_code)
            .fetch_one(&database)
            .await
            .expect("the password generation must be queryable");
    assert_eq!(generation, 2);
}

#[tokio::test]
async fn concurrent_password_rotation_retries_share_one_committed_result() {
    let (app, database) = test_state().await;
    let room = create_room_fixture(&app).await;
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let mut blocker = database
        .begin()
        .await
        .expect("the lock transaction must begin");
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM rooms WHERE code = $1 FOR UPDATE")
        .bind(&room.room_code)
        .fetch_one(&mut *blocker)
        .await
        .expect("the test must hold the room lock");

    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let first = {
        let app = app.clone();
        let barrier = barrier.clone();
        let host_cookie = room.host_cookie.clone();
        let idempotency_key = idempotency_key.clone();
        async move {
            barrier.wait().await;
            app.oneshot(rotate_password_request(
                Some(&host_cookie),
                &idempotency_key,
                CURRENT_PASSWORD,
                NEW_PASSWORD,
            ))
            .await
            .expect("the first concurrent rotation must receive a response")
        }
    };
    let second = {
        let app = app.clone();
        let barrier = barrier.clone();
        let host_cookie = room.host_cookie.clone();
        let idempotency_key = idempotency_key.clone();
        async move {
            barrier.wait().await;
            app.oneshot(rotate_password_request(
                Some(&host_cookie),
                &idempotency_key,
                CURRENT_PASSWORD,
                NEW_PASSWORD,
            ))
            .await
            .expect("the second concurrent rotation must receive a response")
        }
    };
    let release = async {
        barrier.wait().await;
        wait_for_waiting_rotation_queries(&database).await;
        blocker
            .commit()
            .await
            .expect("the test room lock must be released");
    };

    let (first, second, ()) = tokio::join!(first, second, release);
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(response_json(first).await, response_json(second).await);

    let artifacts = password_rotation_artifacts(&database, &room.room_code).await;
    assert_eq!(artifacts, (2, 1, 1));
}

#[tokio::test]
async fn only_an_authenticated_host_with_the_current_password_can_rotate_recovery() {
    let (app, database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let host_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let room_code = created["room"]["code"]
        .as_str()
        .expect("room creation must return its code");
    let joined = app
        .clone()
        .oneshot(join_room_request(room_code))
        .await
        .expect("room join must receive a response");
    assert_eq!(joined.status(), StatusCode::CREATED);
    let guest_cookie = session_cookie(&joined);

    let unauthenticated = app
        .clone()
        .oneshot(rotate_password_request(
            None,
            &uuid::Uuid::new_v4().to_string(),
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("an unauthenticated rotation must receive a response");
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(unauthenticated).await["error"]["code"],
        "SESSION_INVALID"
    );

    let guest = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&guest_cookie),
            &uuid::Uuid::new_v4().to_string(),
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("a guest rotation must receive a response");
    assert_eq!(guest.status(), StatusCode::FORBIDDEN);
    assert_eq!(response_json(guest).await["error"]["code"], "NOT_ROOM_HOST");

    let stale_confirmation = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &uuid::Uuid::new_v4().to_string(),
            "a mistaken uncommon passphrase",
            NEW_PASSWORD,
        ))
        .await
        .expect("a stale confirmation must receive a response");
    assert_eq!(stale_confirmation.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(stale_confirmation).await["error"]["code"],
        "RECOVERY_CONFIRMATION_FAILED"
    );

    let oversized_confirmation = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &uuid::Uuid::new_v4().to_string(),
            &"x".repeat(129),
            NEW_PASSWORD,
        ))
        .await
        .expect("an oversized confirmation must receive a response");
    assert_eq!(oversized_confirmation.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(oversized_confirmation).await["error"]["code"],
        "RECOVERY_CONFIRMATION_FAILED"
    );

    let (generation, event_count) = sqlx::query_as::<_, (i64, i64)>(
        r"
        SELECT
            rooms.password_generation,
            (
                SELECT COUNT(*)
                FROM identity_security_events
                WHERE identity_security_events.room_id = rooms.id
            )
        FROM rooms
        WHERE rooms.code = $1
        ",
    )
    .bind(room_code)
    .fetch_one(&database)
    .await
    .expect("the rejected rotation state must be queryable");
    assert_eq!(generation, 1);
    assert_eq!(event_count, 0);
}

#[tokio::test]
async fn password_rotation_invalidates_old_recovery_without_revoking_authenticated_sessions() {
    let (app, database) = test_state().await;
    let room = create_room_fixture(&app).await;
    let guest = join_room_fixture(&app, &room.room_code).await;
    let sessions_before = room_active_session_state(&database, &room.room_code).await;

    let rotated = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&room.host_cookie),
            &uuid::Uuid::new_v4().to_string(),
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("password rotation must receive a response");
    assert_eq!(rotated.status(), StatusCode::OK);

    assert_sessions_remain_valid(&app, &[&room.host_cookie, &guest.guest_cookie]).await;
    for (token, password) in [
        (room.recovery_token.as_str(), CURRENT_PASSWORD),
        (room.recovery_token.as_str(), NEW_PASSWORD),
        (guest.recovery_token.as_str(), CURRENT_PASSWORD),
        (guest.recovery_token.as_str(), NEW_PASSWORD),
    ] {
        let recovered = app
            .clone()
            .oneshot(recover_participation_request(token, password))
            .await
            .expect("an obsolete recovery must receive a response");
        assert_eq!(recovered.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response_json(recovered).await["error"]["code"],
            "RECOVERY_FAILED"
        );
    }

    let credential_state = rotated_credential_state(&database, &room.room_code).await;
    assert_eq!(credential_state, (2, 1, 2));

    let sessions_after = room_active_session_state(&database, &room.room_code).await;
    assert_eq!(sessions_after, sessions_before);
}

#[tokio::test]
async fn previous_backend_credential_writes_snapshot_the_current_recovery_authority() {
    let (app, database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let host_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let room_code = created["room"]["code"]
        .as_str()
        .expect("room creation must return its code");

    let rotated = app
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &uuid::Uuid::new_v4().to_string(),
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("password rotation must receive a response");
    assert_eq!(rotated.status(), StatusCode::OK);

    let room_id = sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM rooms WHERE code = $1")
        .bind(room_code)
        .fetch_one(&database)
        .await
        .expect("the rotated room must be queryable");
    let guest_identity_id = uuid::Uuid::new_v4();
    let participant_id = uuid::Uuid::new_v4();
    let token_hmac = format!("hmac-sha256:{0}{0}", uuid::Uuid::new_v4().simple());
    sqlx::query("INSERT INTO guest_identities (id) VALUES ($1)")
        .bind(guest_identity_id)
        .execute(&database)
        .await
        .expect("the previous backend identity write must succeed");
    sqlx::query(
        r"
        INSERT INTO participants (
            id,
            room_id,
            guest_identity_id,
            display_name,
            role,
            position,
            hero_id
        )
        VALUES ($1, $2, $3, 'Neville', 'guest', 2, 'neville')
        ",
    )
    .bind(participant_id)
    .bind(room_id)
    .bind(guest_identity_id)
    .execute(&database)
    .await
    .expect("the previous backend participant write must succeed");
    sqlx::query(
        r"
        INSERT INTO recovery_credentials (id, participant_id, token_hmac)
        VALUES ($1, $2, $3)
        ",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(participant_id)
    .bind(token_hmac)
    .execute(&database)
    .await
    .expect("the previous backend credential write shape must remain accepted");

    let snapshot = sqlx::query_as::<_, (String, i64, i64, i64, String)>(
        r"
        SELECT
            credentials.recovery_password_hash,
            credentials.recovery_epoch,
            credentials.password_generation,
            credentials.recovery_generation,
            rooms.recovery_password_hash
        FROM recovery_credentials AS credentials
        JOIN participants ON participants.id = credentials.participant_id
        JOIN rooms ON rooms.id = participants.room_id
        WHERE credentials.participant_id = $1
        ",
    )
    .bind(participant_id)
    .fetch_one(&database)
    .await
    .expect("the compatibility snapshot must be queryable");
    assert_eq!(snapshot.0, snapshot.4);
    assert_eq!((snapshot.1, snapshot.2, snapshot.3), (1, 2, 1));
}

#[tokio::test]
async fn a_participant_directly_regenerates_one_successor_recovery_credential() {
    let (app, database) = test_state().await;
    let room = create_room_fixture(&app).await;
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let regenerated = app
        .clone()
        .oneshot(regenerate_own_credential_request(
            &room.host_cookie,
            &idempotency_key,
        ))
        .await
        .expect("direct credential regeneration must receive a response");
    assert_eq!(regenerated.status(), StatusCode::OK);
    assert_eq!(
        regenerated.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    assert!(regenerated.headers().get(header::SET_COOKIE).is_none());
    let regenerated = response_json(regenerated).await;
    let successor_token = assert_direct_regeneration_response(&regenerated, &room.recovery_token);

    let replay = app
        .clone()
        .oneshot(regenerate_own_credential_request(
            &room.host_cookie,
            &idempotency_key,
        ))
        .await
        .expect("direct regeneration retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, regenerated);

    let credentials = participant_recovery_credentials(&database, &room.room_code).await;
    assert_eq!(credentials.len(), 2);
    assert_eq!(credentials[0].0, "superseded");
    assert_eq!(credentials[0].1, 1);
    assert_eq!(credentials[1].0, "active");
    assert_eq!(credentials[1].1, 2);
    for (_, _, stored_hmac) in &credentials {
        assert!(stored_hmac.starts_with("hmac-sha256:"));
        assert_ne!(stored_hmac, &room.recovery_token);
        assert_ne!(stored_hmac, &successor_token);
    }
    let active_sessions = room_active_session_count(&database, &room.room_code).await;
    assert_eq!(active_sessions, 1);

    let recipients = security_event_recipient_positions(&database, &room.room_code, "direct").await;
    assert_eq!(recipients, vec![1]);
    let receipt = regeneration_receipt(&database, &idempotency_key).await;
    assert_eq!(
        receipt,
        (
            "direct".to_owned(),
            Some(2),
            regenerated["security_event"]["cursor"].as_i64(),
            true,
        )
    );

    let obsolete = app
        .clone()
        .oneshot(recover_participation_request(
            &room.recovery_token,
            CURRENT_PASSWORD,
        ))
        .await
        .expect("the obsolete credential must receive a response");
    assert_eq!(obsolete.status(), StatusCode::UNAUTHORIZED);

    let recovered = app
        .oneshot(recover_participation_request(
            &successor_token,
            CURRENT_PASSWORD,
        ))
        .await
        .expect("the successor credential must receive a response");
    assert_eq!(recovered.status(), StatusCode::OK);
    assert_eq!(
        response_json(recovered).await["participant"],
        room.participant
    );
}

#[tokio::test]
async fn only_a_host_who_acknowledges_impersonation_risk_can_assist_a_participant() {
    let (app, database) = test_state().await;
    let room = create_room_fixture(&app).await;
    let guest = join_room_fixture(&app, &room.room_code).await;
    assert_assistance_rejections(&app, &room.host_cookie, &guest.guest_cookie).await;

    let state_before = assisted_target_state(&database, &room.room_code).await;
    assert_eq!(state_before, (1, 0));

    let idempotency_key = uuid::Uuid::new_v4().to_string();
    let assisted = app
        .clone()
        .oneshot(regenerate_assisted_credential_request(
            &room.host_cookie,
            &idempotency_key,
            2,
            true,
        ))
        .await
        .expect("host assistance must receive a response");
    assert_eq!(assisted.status(), StatusCode::OK);
    assert_eq!(
        assisted.headers().get(header::CACHE_CONTROL),
        Some(&header::HeaderValue::from_static("no-store"))
    );
    assert!(assisted.headers().get(header::SET_COOKIE).is_none());
    let assisted = response_json(assisted).await;
    let successor_token = assert_assisted_regeneration_response(&assisted);

    let replay = app
        .clone()
        .oneshot(regenerate_assisted_credential_request(
            &room.host_cookie,
            &idempotency_key,
            2,
            true,
        ))
        .await
        .expect("host assistance retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, assisted);

    let recipients =
        security_event_recipient_positions(&database, &room.room_code, "host_assisted").await;
    assert_eq!(recipients, vec![1, 2]);
    let receipt = regeneration_receipt(&database, &idempotency_key).await;
    assert_eq!(
        receipt,
        (
            "host_assisted".to_owned(),
            Some(2),
            assisted["security_event"]["cursor"].as_i64(),
            true,
        )
    );

    let obsolete = app
        .clone()
        .oneshot(recover_participation_request(
            &guest.recovery_token,
            CURRENT_PASSWORD,
        ))
        .await
        .expect("the superseded assisted credential must receive a response");
    assert_eq!(obsolete.status(), StatusCode::UNAUTHORIZED);

    let recovered = app
        .clone()
        .oneshot(recover_participation_request(
            &successor_token,
            CURRENT_PASSWORD,
        ))
        .await
        .expect("the assisted successor credential must receive a response");
    assert_eq!(recovered.status(), StatusCode::OK);
    assert_eq!(
        response_json(recovered).await["participant"],
        guest.participant
    );

    assert_sessions_remain_valid(&app, &[&room.host_cookie, &guest.guest_cookie]).await;
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
    cookie: &str,
) -> (u16, String, RawWebSocket) {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("the WebSocket client must connect");
    let mut request = format!(
        "GET /api/session/events HTTP/1.1\r\nHost: {address}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    write!(request, "Cookie: {cookie}\r\n").expect("writing to a String cannot fail");
    request.push_str("Origin: http://127.0.0.1:5173\r\n");
    request.push_str("Sec-WebSocket-Protocol: hogwarts.session.v1\r\n\r\n");
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
async fn authenticated_sessions_receive_a_committed_secretless_password_rotation_notice() {
    let (app, database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let host_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let room_code = created["room"]["code"]
        .as_str()
        .expect("room creation must return its code")
        .to_owned();
    let joined = app
        .clone()
        .oneshot(join_room_request(&room_code))
        .await
        .expect("room join must receive a response");
    let guest_cookie = session_cookie(&joined);
    let (address, server) = start_network_server(app.clone()).await;

    let (host_status, host_headers, mut host_socket) =
        websocket_handshake(address, &host_cookie).await;
    assert_eq!(host_status, 101);
    assert!(
        host_headers
            .to_ascii_lowercase()
            .contains("sec-websocket-protocol: hogwarts.session.v1")
    );
    let (guest_status, _, mut guest_socket) = websocket_handshake(address, &guest_cookie).await;
    assert_eq!(guest_status, 101);

    for socket in [&mut host_socket, &mut guest_socket] {
        let initial: Value = serde_json::from_str(
            &tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_text())
                .await
                .expect("the initial security Snapshot must arrive"),
        )
        .expect("the initial security Snapshot must be JSON");
        assert_eq!(initial["protocol_version"], 1);
        assert_eq!(initial["type"], "security_snapshot");
        assert_eq!(initial["cursor"], 0);
        assert_eq!(initial["events"], json!([]));
    }

    let rotated = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &uuid::Uuid::new_v4().to_string(),
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("password rotation must receive a response");
    assert_eq!(rotated.status(), StatusCode::OK);

    for socket in [&mut host_socket, &mut guest_socket] {
        let serialized =
            tokio::time::timeout(std::time::Duration::from_secs(2), socket.read_text())
                .await
                .expect("the committed security notice must arrive");
        let notice: Value =
            serde_json::from_str(&serialized).expect("the security notice must be JSON");
        assert_eq!(notice["protocol_version"], 1);
        assert_eq!(notice["type"], "security_events");
        assert_eq!(notice["from_cursor"], 0);
        assert_eq!(notice["cursor"], 1);
        assert_eq!(notice["events"][0]["type"], "recovery_password_rotated");
        assert_eq!(notice["events"][0]["actor_position"], 1);
        assert_eq!(notice["events"][0]["password_generation"], 2);
        for forbidden in [
            CURRENT_PASSWORD,
            NEW_PASSWORD,
            "recovery_token",
            "token_hmac",
            "participant_id",
            "session_id",
            "idempotency_key",
            "__Host-session",
        ] {
            assert!(!serialized.contains(forbidden));
        }
    }

    let committed = sqlx::query_as::<_, (i64, i64, i64)>(
        r"
        SELECT
            rooms.password_generation,
            rooms.security_event_sequence,
            COUNT(recipients.participant_id)
        FROM rooms
        JOIN identity_security_events AS events ON events.room_id = rooms.id
        JOIN identity_security_event_recipients AS recipients
          ON recipients.room_id = events.room_id
         AND recipients.security_event_sequence = events.sequence
        WHERE rooms.code = $1
        GROUP BY rooms.id
        ",
    )
    .bind(&room_code)
    .fetch_one(&database)
    .await
    .expect("the committed security notice must be queryable");
    assert_eq!(committed, (2, 1, 2));
    server.abort();
}

#[tokio::test]
async fn a_committed_recovery_remains_idempotent_after_rotation_but_a_new_attempt_fails() {
    let (app, _database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let host_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let original_token = created["recovery_token"]
        .as_str()
        .expect("room creation must return a recovery token");
    let attempt_id = uuid::Uuid::new_v4().to_string();

    let recovered = app
        .clone()
        .oneshot(recover_participation_attempt_request(
            original_token,
            CURRENT_PASSWORD,
            &attempt_id,
        ))
        .await
        .expect("the first recovery must receive a response");
    assert_eq!(recovered.status(), StatusCode::OK);
    let recovered_cookie = session_cookie(&recovered);
    let recovered = response_json(recovered).await;

    let rotated = app
        .clone()
        .oneshot(rotate_password_request(
            Some(&host_cookie),
            &uuid::Uuid::new_v4().to_string(),
            CURRENT_PASSWORD,
            NEW_PASSWORD,
        ))
        .await
        .expect("password rotation must receive a response");
    assert_eq!(rotated.status(), StatusCode::OK);

    let replay = app
        .clone()
        .oneshot(recover_participation_attempt_request(
            original_token,
            CURRENT_PASSWORD,
            &attempt_id,
        ))
        .await
        .expect("the committed recovery retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(session_cookie(&replay), recovered_cookie);
    assert_eq!(response_json(replay).await, recovered);

    for password in [CURRENT_PASSWORD, NEW_PASSWORD] {
        let rejected = app
            .clone()
            .oneshot(recover_participation_request(original_token, password))
            .await
            .expect("a new obsolete recovery attempt must receive a response");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response_json(rejected).await["error"]["code"],
            "RECOVERY_FAILED"
        );
    }
}

#[tokio::test]
async fn recovery_and_password_rotation_linearize_without_partial_state() {
    let (app, database) = test_state().await;
    let room = create_room_fixture(&app).await;
    let recovery_attempt_id = uuid::Uuid::new_v4().to_string();
    let rotation_key = uuid::Uuid::new_v4().to_string();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));

    let recovery = {
        let app = app.clone();
        let barrier = barrier.clone();
        let recovery_token = room.recovery_token.clone();
        let recovery_attempt_id = recovery_attempt_id.clone();
        async move {
            barrier.wait().await;
            app.oneshot(recover_participation_attempt_request(
                &recovery_token,
                CURRENT_PASSWORD,
                &recovery_attempt_id,
            ))
            .await
            .expect("the concurrent recovery must receive a response")
        }
    };
    let rotation = {
        let app = app.clone();
        let barrier = barrier.clone();
        let host_cookie = room.host_cookie.clone();
        async move {
            barrier.wait().await;
            app.oneshot(rotate_password_request(
                Some(&host_cookie),
                &rotation_key,
                CURRENT_PASSWORD,
                NEW_PASSWORD,
            ))
            .await
            .expect("the concurrent rotation must receive a response")
        }
    };
    let release = async move {
        barrier.wait().await;
    };
    let (recovery, rotation, ()) = tokio::join!(recovery, rotation, release);

    assert_eq!(rotation.status(), StatusCode::OK);
    let recovery_committed = recovery.status() == StatusCode::OK;
    assert!(
        recovery_committed || recovery.status() == StatusCode::UNAUTHORIZED,
        "recovery must linearize before or after rotation, not fail internally"
    );
    if recovery_committed {
        let recovered_cookie = session_cookie(&recovery);
        let restored = app
            .clone()
            .oneshot(restore_session_request(&recovered_cookie))
            .await
            .expect("a recovery committed before rotation must remain authenticated");
        assert_eq!(restored.status(), StatusCode::OK);
    } else {
        assert_eq!(
            response_json(recovery).await["error"]["code"],
            "RECOVERY_FAILED"
        );
    }

    for password in [CURRENT_PASSWORD, NEW_PASSWORD] {
        let rejected = app
            .clone()
            .oneshot(recover_participation_request(
                &room.recovery_token,
                password,
            ))
            .await
            .expect("a post-rotation recovery must receive a response");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    }

    let state = linearized_recovery_state(&database, &room.room_code).await;
    assert_eq!(state.0, 2);
    assert_eq!(state.1, 0);
    assert_eq!(state.2, i64::from(recovery_committed));
    assert_eq!(state.3, if recovery_committed { 2 } else { 1 });
}
