use std::{
    fmt::Write as _,
    time::{Duration, Instant},
};

use axum::{
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const RECOVERY_PASSWORD: &str = "a long uncommon passphrase";
const NEW_RECOVERY_PASSWORD: &str = "a newer uncommon recovery phrase";

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

fn recover_participation_request(token: &str) -> Request<Body> {
    recover_participation_with_password_request(token, RECOVERY_PASSWORD)
}

fn recover_participation_with_password_request(token: &str, password: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/session/recover")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "recovery_token": token,
                "recovery_password": password,
                "recovery_attempt_id": uuid::Uuid::new_v4().to_string()
            })
            .to_string(),
        ))
        .expect("the participant recovery request must be valid")
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

fn list_device_sessions_request(cookie: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
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
    Request::builder()
        .method("PUT")
        .uri(format!(
            "/api/session/device-sessions/{session_id}/revocation"
        ))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header("idempotency-key", idempotency_key)
        .body(Body::from("{}"))
        .expect("the device session revocation request must be valid")
}

fn protect_participant_request(cookie: &str, idempotency_key: &str) -> Request<Body> {
    protect_participant_confirmation_request(cookie, idempotency_key, true)
}

fn protect_participant_confirmation_request(
    cookie: &str,
    idempotency_key: &str,
    protection_confirmed: bool,
) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri("/api/session/protection")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header("idempotency-key", idempotency_key)
        .body(Body::from(
            json!({ "protection_confirmed": protection_confirmed }).to_string(),
        ))
        .expect("the participant protection request must be valid")
}

fn protect_room_request(
    cookie: &str,
    idempotency_key: &str,
    preserve_current_session: bool,
) -> Request<Body> {
    protect_room_with_values_request(
        cookie,
        idempotency_key,
        RECOVERY_PASSWORD,
        NEW_RECOVERY_PASSWORD,
        preserve_current_session,
        true,
    )
}

fn protect_room_with_values_request(
    cookie: &str,
    idempotency_key: &str,
    current_recovery_password: &str,
    new_recovery_password: &str,
    preserve_current_session: bool,
    protection_confirmed: bool,
) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri("/api/rooms/current/protection")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header("idempotency-key", idempotency_key)
        .body(Body::from(
            json!({
                "current_recovery_password": current_recovery_password,
                "new_recovery_password": new_recovery_password,
                "preserve_current_session": preserve_current_session,
                "protection_confirmed": protection_confirmed
            })
            .to_string(),
        ))
        .expect("the room protection request must be valid")
}

fn regenerate_own_credential_request(cookie: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/session/recovery-credential")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, cookie)
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .body(Body::from("{}"))
        .expect("the recovery credential regeneration request must be valid")
}

fn restore_session_request(cookie: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri("/api/session")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .expect("the session restore request must be valid")
}

fn session_cookie(response: &Response<Body>) -> String {
    response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .expect("the response must set a session cookie")
        .to_owned()
}

async fn response_json(response: Response<Body>) -> Value {
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("the response body must be readable");
    serde_json::from_slice(&body).expect("the response body must be JSON")
}

async fn assert_error_response(
    app: &axum::Router,
    request: Request<Body>,
    expected_status: StatusCode,
    expected_code: &str,
) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the rejected protection request must receive a response");
    assert_eq!(response.status(), expected_status);
    assert_eq!(
        response_json(response).await["error"]["code"],
        expected_code
    );
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

fn assert_preserved_room_protection(response: &Value) {
    assert_eq!(response["status"], "protected");
    assert_eq!(response["password_generation"], 2);
    assert_eq!(response["recovery_epoch"], 2);
    assert_eq!(response["revoked_sessions"], 1);
    assert_eq!(response["current_session_preserved"], true);
    assert_eq!(response["security_event"]["type"], "room_protected");
    assert_eq!(response["security_event"]["actor_position"], 1);
    assert_eq!(response["security_event"]["password_generation"], 2);
    assert_eq!(response["security_event"]["recovery_epoch"], 2);
    assert_eq!(response["security_event"]["revoked_sessions"], 1);
    assert_eq!(
        response["security_event"]["current_session_preserved"],
        true
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

async fn session_events_websocket(address: std::net::SocketAddr, cookie: &str) -> RawWebSocket {
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
    assert!(
        headers.starts_with("HTTP/1.1 101"),
        "the security WebSocket handshake must succeed"
    );
    RawWebSocket {
        stream,
        buffered: response[header_end..].to_vec(),
    }
}

#[tokio::test]
async fn a_participant_lists_and_revokes_only_the_chosen_device_session() {
    let (app, _database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let first_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let recovery_token = created["recovery_token"]
        .as_str()
        .expect("room creation must return a recovery token");
    let recovered = app
        .clone()
        .oneshot(recover_participation_request(recovery_token))
        .await
        .expect("participant recovery must receive a response");
    assert_eq!(recovered.status(), StatusCode::OK);
    let second_cookie = session_cookie(&recovered);

    let sessions = app
        .clone()
        .oneshot(list_device_sessions_request(&first_cookie))
        .await
        .expect("device session listing must receive a response");
    assert_eq!(sessions.status(), StatusCode::OK);
    let sessions = response_json(sessions).await;
    assert_eq!(sessions["sessions"].as_array().map(Vec::len), Some(2));
    assert_eq!(sessions["sessions"][0]["label"], "Sessão 1");
    assert_eq!(sessions["sessions"][0]["current"], true);
    assert_eq!(sessions["sessions"][1]["label"], "Sessão 2");
    assert_eq!(sessions["sessions"][1]["current"], false);
    let second_session_id = sessions["sessions"][1]["id"]
        .as_str()
        .expect("the second session must expose an opaque ID");
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let revoked = app
        .clone()
        .oneshot(revoke_device_session_request(
            &first_cookie,
            second_session_id,
            &idempotency_key,
        ))
        .await
        .expect("device session revocation must receive a response");
    assert_eq!(revoked.status(), StatusCode::OK);
    let revoked = response_json(revoked).await;
    assert_eq!(revoked["status"], "revoked");
    assert_eq!(revoked["revoked_session"]["id"], second_session_id);
    assert_eq!(revoked["revoked_session"]["label"], "Sessão 2");
    assert_eq!(revoked["security_event"]["type"], "session_revoked");
    assert_eq!(revoked["security_event"]["actor_position"], 1);
    assert_eq!(revoked["security_event"]["target_position"], 1);
    assert_eq!(revoked["security_event"]["session_label"], "Sessão 2");

    let replay = app
        .clone()
        .oneshot(revoke_device_session_request(
            &first_cookie,
            second_session_id,
            &idempotency_key,
        ))
        .await
        .expect("the revocation retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, revoked);

    let first_session = app
        .clone()
        .oneshot(restore_session_request(&first_cookie))
        .await
        .expect("the retained session must receive a response");
    assert_eq!(first_session.status(), StatusCode::OK);
    let second_session = app
        .clone()
        .oneshot(restore_session_request(&second_cookie))
        .await
        .expect("the revoked session must receive a response");
    assert_eq!(second_session.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(second_session).await["error"]["code"],
        "SESSION_INVALID"
    );

    let sessions = app
        .clone()
        .oneshot(list_device_sessions_request(&first_cookie))
        .await
        .expect("the remaining device session list must receive a response");
    let sessions = response_json(sessions).await;
    assert_eq!(sessions["sessions"].as_array().map(Vec::len), Some(1));
    assert_eq!(sessions["sessions"][0]["label"], "Sessão 1");
    assert_eq!(sessions["sessions"][0]["current"], true);
}

#[tokio::test]
async fn revoking_the_current_device_clears_its_cookie_and_remains_idempotent() {
    let (app, _database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let cookie = session_cookie(&created);
    let sessions = app
        .clone()
        .oneshot(list_device_sessions_request(&cookie))
        .await
        .expect("device session listing must receive a response");
    let sessions = response_json(sessions).await;
    let current_session_id = sessions["sessions"][0]["id"]
        .as_str()
        .expect("the current session must expose an opaque ID")
        .to_owned();
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let revoked = app
        .clone()
        .oneshot(revoke_device_session_request(
            &cookie,
            &current_session_id,
            &idempotency_key,
        ))
        .await
        .expect("current device revocation must receive a response");
    assert_eq!(revoked.status(), StatusCode::OK);
    assert!(
        revoked
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.contains("__Host-session=") && value.contains("Max-Age=0")),
        "current device revocation must clear the browser cookie"
    );
    let revoked = response_json(revoked).await;

    let restored = app
        .clone()
        .oneshot(restore_session_request(&cookie))
        .await
        .expect("the revoked current session must receive a response");
    assert_eq!(restored.status(), StatusCode::UNAUTHORIZED);

    let replay = app
        .clone()
        .oneshot(revoke_device_session_request(
            &cookie,
            &current_session_id,
            &idempotency_key,
        ))
        .await
        .expect("the current device revocation retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, revoked);
}

#[tokio::test]
async fn revocation_notifies_the_retained_session_and_closes_the_target_security_channel() {
    let (app, _database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let first_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let recovery_token = created["recovery_token"]
        .as_str()
        .expect("room creation must return a recovery token");
    let recovered = app
        .clone()
        .oneshot(recover_participation_request(recovery_token))
        .await
        .expect("participant recovery must receive a response");
    let second_cookie = session_cookie(&recovered);
    let sessions = app
        .clone()
        .oneshot(list_device_sessions_request(&first_cookie))
        .await
        .expect("device session listing must receive a response");
    let sessions = response_json(sessions).await;
    let second_session_id = sessions["sessions"][1]["id"]
        .as_str()
        .expect("the second session must expose an opaque ID")
        .to_owned();
    let (address, server) = start_network_server(app.clone()).await;
    let mut retained_socket = session_events_websocket(address, &first_cookie).await;
    let mut target_socket = session_events_websocket(address, &second_cookie).await;
    for socket in [&mut retained_socket, &mut target_socket] {
        let initial: Value = serde_json::from_str(&socket.read_text().await)
            .expect("the initial security Snapshot must be JSON");
        assert_eq!(initial["type"], "security_snapshot");
    }

    let revoked = app
        .clone()
        .oneshot(revoke_device_session_request(
            &first_cookie,
            &second_session_id,
            &uuid::Uuid::new_v4().to_string(),
        ))
        .await
        .expect("device session revocation must receive a response");
    assert_eq!(revoked.status(), StatusCode::OK);

    let notice = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        retained_socket.read_text(),
    )
    .await
    .expect("the retained session must receive the committed security event");
    let event: Value = serde_json::from_str(&notice).expect("the security event must be JSON");
    assert_eq!(event["type"], "security_events");
    assert_eq!(event["events"][0]["type"], "session_revoked");
    assert_eq!(event["events"][0]["session_label"], "Sessão 2");
    for forbidden in [
        second_session_id.as_str(),
        recovery_token,
        "token_hmac",
        "participant_id",
        "idempotency_key",
        "__Host-session",
    ] {
        assert!(!notice.contains(forbidden));
    }

    let close_code = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        target_socket.read_close_code(),
    )
    .await
    .expect("the revoked security channel must close within the p95 target");
    assert_eq!(close_code, 1008);
    server.abort();
}

#[tokio::test]
async fn revocation_closes_a_security_channel_served_by_another_application_instance() {
    let (first_app, database) = test_state().await;
    let second_state =
        AppState::new(database).with_session_token_key(*b"test-session-token-key-000000000");
    initialize(&second_state)
        .await
        .expect("the second application instance must initialize");
    let second_app = build_router(second_state);
    let created = first_app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let first_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let recovery_token = created["recovery_token"]
        .as_str()
        .expect("room creation must return a recovery token");
    let recovered = first_app
        .clone()
        .oneshot(recover_participation_request(recovery_token))
        .await
        .expect("participant recovery must receive a response");
    let second_cookie = session_cookie(&recovered);
    let sessions = first_app
        .clone()
        .oneshot(list_device_sessions_request(&first_cookie))
        .await
        .expect("device session listing must receive a response");
    let sessions = response_json(sessions).await;
    let second_session_id = sessions["sessions"][1]["id"]
        .as_str()
        .expect("the second session must expose an opaque ID")
        .to_owned();
    let (address, server) = start_network_server(second_app).await;
    let mut target_socket = session_events_websocket(address, &second_cookie).await;
    let initial: Value = serde_json::from_str(&target_socket.read_text().await)
        .expect("the initial security Snapshot must be JSON");
    assert_eq!(initial["type"], "security_snapshot");

    let revoked = first_app
        .clone()
        .oneshot(revoke_device_session_request(
            &first_cookie,
            &second_session_id,
            &uuid::Uuid::new_v4().to_string(),
        ))
        .await
        .expect("device session revocation must receive a response");
    assert_eq!(revoked.status(), StatusCode::OK);

    let close_code = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        target_socket.read_close_code(),
    )
    .await
    .expect("cross-instance revocation must close the security channel within the p95 target");
    assert_eq!(close_code, 1008);
    server.abort();
}

#[tokio::test]
async fn participant_protection_revokes_every_session_and_recovery_link_atomically() {
    let (app, _database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let first_cookie = session_cookie(&created);
    let created = response_json(created).await;
    let original_token = created["recovery_token"]
        .as_str()
        .expect("room creation must return a recovery token")
        .to_owned();
    let recovered = app
        .clone()
        .oneshot(recover_participation_request(&original_token))
        .await
        .expect("participant recovery must receive a response");
    let second_cookie = session_cookie(&recovered);
    let recovered = response_json(recovered).await;
    let successor_token = recovered["recovery_token"]
        .as_str()
        .expect("recovery must rotate the participant link")
        .to_owned();
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let protected = app
        .clone()
        .oneshot(protect_participant_request(&first_cookie, &idempotency_key))
        .await
        .expect("participant protection must receive a response");
    assert_eq!(protected.status(), StatusCode::OK);
    assert!(
        protected
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.contains("__Host-session=") && value.contains("Max-Age=0")),
        "participant protection must clear the current browser cookie"
    );
    let protected = response_json(protected).await;
    assert_eq!(protected["status"], "protected");
    assert_eq!(protected["participant"]["position"], 1);
    assert_eq!(protected["participant"]["display_name"], "Minerva");
    assert_eq!(protected["revoked_sessions"], 2);
    assert_eq!(protected["recovery_generation"], 2);
    assert_eq!(protected["security_event"]["type"], "participant_protected");
    assert_eq!(protected["security_event"]["actor_position"], 1);
    assert_eq!(protected["security_event"]["target_position"], 1);
    assert_eq!(protected["security_event"]["revoked_sessions"], 2);
    assert_eq!(protected["security_event"]["recovery_generation"], 2);

    for cookie in [&first_cookie, &second_cookie] {
        let restored = app
            .clone()
            .oneshot(restore_session_request(cookie))
            .await
            .expect("each protected session must receive a response");
        assert_eq!(restored.status(), StatusCode::UNAUTHORIZED);
    }
    for token in [&original_token, &successor_token] {
        let recovered = app
            .clone()
            .oneshot(recover_participation_request(token))
            .await
            .expect("each protected recovery link must receive a response");
        assert_eq!(recovered.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response_json(recovered).await["error"]["code"],
            "RECOVERY_FAILED"
        );
    }

    let replay = app
        .clone()
        .oneshot(protect_participant_request(&first_cookie, &idempotency_key))
        .await
        .expect("the participant protection retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, protected);
}

#[tokio::test]
async fn participant_protection_rejects_a_credential_regeneration_authenticated_before_commit() {
    let (app, database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let cookie = session_cookie(&created);
    let created = response_json(created).await;
    let room_code = created["room"]["code"]
        .as_str()
        .expect("room creation must return its code")
        .to_owned();

    let mut fence = database
        .begin()
        .await
        .expect("the protection linearization fence must begin");
    let fence_pid = sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
        .fetch_one(&mut *fence)
        .await
        .expect("the protection fence backend must be identifiable");
    sqlx::query("SELECT id FROM rooms WHERE code = $1 FOR UPDATE")
        .bind(&room_code)
        .fetch_one(&mut *fence)
        .await
        .expect("the room access root must be lockable");

    let protection = {
        let app = app.clone();
        let cookie = cookie.clone();
        tokio::spawn(async move {
            app.oneshot(protect_participant_request(
                &cookie,
                &uuid::Uuid::new_v4().to_string(),
            ))
            .await
            .expect("the racing participant protection must receive a response")
        })
    };
    wait_for_requests_blocked_by(&database, fence_pid, 1).await;
    let regeneration = {
        let app = app.clone();
        let cookie = cookie.clone();
        tokio::spawn(async move {
            app.oneshot(regenerate_own_credential_request(&cookie))
                .await
                .expect("the racing credential regeneration must receive a response")
        })
    };
    wait_for_requests_blocked_by(&database, fence_pid, 2).await;

    fence
        .commit()
        .await
        .expect("the protection linearization fence must release");
    let protection = protection.await.expect("the protection task must finish");
    let regeneration = regeneration
        .await
        .expect("the regeneration task must finish");
    assert_eq!(protection.status(), StatusCode::OK);
    assert_eq!(regeneration.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response_json(regeneration).await["error"]["code"],
        "SESSION_INVALID"
    );

    let active_access = sqlx::query_as::<_, (i64, i64)>(
        r"
        SELECT
            (
                SELECT COUNT(*)
                FROM device_sessions
                WHERE device_sessions.participant_id = participants.id
                  AND device_sessions.status = 'active'
            ),
            (
                SELECT COUNT(*)
                FROM recovery_credentials
                WHERE recovery_credentials.participant_id = participants.id
                  AND recovery_credentials.status = 'active'
            )
        FROM participants
        JOIN rooms ON rooms.id = participants.room_id
        WHERE rooms.code = $1
          AND participants.role = 'host'
        ",
    )
    .bind(&room_code)
    .fetch_one(&database)
    .await
    .expect("the protected participant access must be queryable");
    assert_eq!(active_access, (0, 0));
}

#[tokio::test]
async fn protection_requires_confirmation_host_authority_and_the_current_password() {
    let (app, _database) = test_state().await;
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
    let guest_cookie = session_cookie(&joined);

    assert_error_response(
        &app,
        protect_participant_confirmation_request(
            &guest_cookie,
            &uuid::Uuid::new_v4().to_string(),
            false,
        ),
        StatusCode::UNPROCESSABLE_ENTITY,
        "PROTECTION_CONFIRMATION_REQUIRED",
    )
    .await;

    assert_error_response(
        &app,
        protect_room_with_values_request(
            &host_cookie,
            &uuid::Uuid::new_v4().to_string(),
            RECOVERY_PASSWORD,
            NEW_RECOVERY_PASSWORD,
            true,
            false,
        ),
        StatusCode::UNPROCESSABLE_ENTITY,
        "PROTECTION_CONFIRMATION_REQUIRED",
    )
    .await;

    assert_error_response(
        &app,
        protect_room_request(&guest_cookie, &uuid::Uuid::new_v4().to_string(), true),
        StatusCode::FORBIDDEN,
        "NOT_ROOM_HOST",
    )
    .await;

    assert_error_response(
        &app,
        protect_room_with_values_request(
            &host_cookie,
            &uuid::Uuid::new_v4().to_string(),
            "not the current recovery password",
            NEW_RECOVERY_PASSWORD,
            true,
            true,
        ),
        StatusCode::UNAUTHORIZED,
        "RECOVERY_CONFIRMATION_FAILED",
    )
    .await;

    for cookie in [&host_cookie, &guest_cookie] {
        let restored = app
            .clone()
            .oneshot(restore_session_request(cookie))
            .await
            .expect("a session must remain active after rejected protection");
        assert_eq!(restored.status(), StatusCode::OK);
    }

    let idempotency_key = uuid::Uuid::new_v4().to_string();
    let protected = app
        .clone()
        .oneshot(protect_room_request(&host_cookie, &idempotency_key, true))
        .await
        .expect("confirmed room protection must receive a response");
    assert_eq!(protected.status(), StatusCode::OK);
    let protected = response_json(protected).await;
    assert_eq!(protected["password_generation"], 2);
    assert_eq!(protected["recovery_epoch"], 2);

    let conflicting_replay = app
        .clone()
        .oneshot(protect_room_with_values_request(
            &host_cookie,
            &idempotency_key,
            RECOVERY_PASSWORD,
            NEW_RECOVERY_PASSWORD,
            false,
            true,
        ))
        .await
        .expect("conflicting room protection retry must receive a response");
    assert_eq!(conflicting_replay.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(conflicting_replay).await["error"]["code"],
        "IDEMPOTENCY_KEY_REUSED"
    );
}

#[tokio::test]
async fn room_protection_rotates_password_and_epoch_and_only_preserves_the_confirming_host_session()
{
    let (app, _database) = test_state().await;
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
    let host_recovery_token = created["recovery_token"]
        .as_str()
        .expect("room creation must return a recovery token")
        .to_owned();
    let joined = app
        .clone()
        .oneshot(join_room_request(&room_code))
        .await
        .expect("room join must receive a response");
    let guest_cookie = session_cookie(&joined);
    let joined = response_json(joined).await;
    let guest_recovery_token = joined["recovery_token"]
        .as_str()
        .expect("room join must return a recovery token")
        .to_owned();
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let protected = app
        .clone()
        .oneshot(protect_room_request(&host_cookie, &idempotency_key, true))
        .await
        .expect("room protection must receive a response");
    assert_eq!(protected.status(), StatusCode::OK);
    assert!(protected.headers().get(header::SET_COOKIE).is_none());
    let protected = response_json(protected).await;
    assert_preserved_room_protection(&protected);

    let retained_host = app
        .clone()
        .oneshot(restore_session_request(&host_cookie))
        .await
        .expect("the confirming host session must receive a response");
    assert_eq!(retained_host.status(), StatusCode::OK);
    let revoked_guest = app
        .clone()
        .oneshot(restore_session_request(&guest_cookie))
        .await
        .expect("the protected guest session must receive a response");
    assert_eq!(revoked_guest.status(), StatusCode::UNAUTHORIZED);

    for token in [&host_recovery_token, &guest_recovery_token] {
        for password in [RECOVERY_PASSWORD, NEW_RECOVERY_PASSWORD] {
            let recovered = app
                .clone()
                .oneshot(recover_participation_with_password_request(token, password))
                .await
                .expect("an obsolete room recovery must receive a response");
            assert_eq!(recovered.status(), StatusCode::UNAUTHORIZED);
        }
    }

    let regenerated = app
        .clone()
        .oneshot(regenerate_own_credential_request(&host_cookie))
        .await
        .expect("the retained host must regenerate a recovery credential");
    assert_eq!(regenerated.status(), StatusCode::OK);
    let regenerated = response_json(regenerated).await;
    let new_host_token = regenerated["recovery_token"]
        .as_str()
        .expect("credential regeneration must return the new token");
    let recovered_with_old_password = app
        .clone()
        .oneshot(recover_participation_with_password_request(
            new_host_token,
            RECOVERY_PASSWORD,
        ))
        .await
        .expect("recovery with the old password must receive a response");
    assert_eq!(
        recovered_with_old_password.status(),
        StatusCode::UNAUTHORIZED
    );
    let recovered_with_new_password = app
        .clone()
        .oneshot(recover_participation_with_password_request(
            new_host_token,
            NEW_RECOVERY_PASSWORD,
        ))
        .await
        .expect("recovery with the protected room password must receive a response");
    assert_eq!(recovered_with_new_password.status(), StatusCode::OK);

    let replay = app
        .clone()
        .oneshot(protect_room_request(&host_cookie, &idempotency_key, true))
        .await
        .expect("the room protection retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, protected);
}

#[tokio::test]
async fn room_protection_can_revoke_the_confirming_host_and_replay_after_logout() {
    let (app, _database) = test_state().await;
    let created = app
        .clone()
        .oneshot(create_room_request())
        .await
        .expect("room creation must receive a response");
    let host_cookie = session_cookie(&created);
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let protected = app
        .clone()
        .oneshot(protect_room_request(&host_cookie, &idempotency_key, false))
        .await
        .expect("room protection must receive a response");
    assert_eq!(protected.status(), StatusCode::OK);
    assert!(
        protected
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .any(|value| value.contains("__Host-session=") && value.contains("Max-Age=0"))
    );
    let protected = response_json(protected).await;
    assert_eq!(protected["revoked_sessions"], 1);
    assert_eq!(protected["current_session_preserved"], false);

    let restored = app
        .clone()
        .oneshot(restore_session_request(&host_cookie))
        .await
        .expect("the protected host session must receive a response");
    assert_eq!(restored.status(), StatusCode::UNAUTHORIZED);

    let replay = app
        .clone()
        .oneshot(protect_room_request(&host_cookie, &idempotency_key, false))
        .await
        .expect("the logged-out room protection retry must receive a response");
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(response_json(replay).await, protected);
}
