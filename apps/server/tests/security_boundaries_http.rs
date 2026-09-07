use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use harry_potter_server::{AppState, build_router, initialize};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
#[path = "support/log_capture.rs"]
mod log_capture;

#[tokio::test]
async fn database_errors_cannot_copy_private_rows_into_http_responses_or_logs() {
    use sqlx::{AssertSqlSafe, postgres::PgConnectOptions};
    use std::str::FromStr;

    let url = std::env::var("TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&url).await.unwrap();
    let schema = format!("security_logging_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA {schema}")))
        .execute(&admin)
        .await
        .unwrap();
    let database = PgPoolOptions::new()
        .connect_with(
            PgConnectOptions::from_str(&url)
                .unwrap()
                .options([("search_path", schema.clone())]),
        )
        .await
        .unwrap();
    let state = AppState::new(database.clone());
    initialize(&state).await.unwrap();
    sqlx::raw_sql("CREATE FUNCTION reject_private_row() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION '%', NEW.display_name; END $$; CREATE TRIGGER reject_private_row BEFORE INSERT ON participants FOR EACH ROW EXECUTE FUNCTION reject_private_row();")
        .execute(&database).await.unwrap();
    let logs = log_capture::LogCapture::start();
    let response = build_router(state.clone()).oneshot(Request::builder().method("POST").uri("/api/rooms")
        .header(header::ORIGIN, ORIGIN).header("x-csrf-protection", "1")
        .header(header::CONTENT_TYPE, "application/json")
        .header("idempotency-key", uuid::Uuid::new_v4().to_string())
        .body(Body::from(json!({"display_name":"db-private-canary", "recovery_password":"a long uncommon secret passphrase"}).to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 4096).await.unwrap();
    state.begin_shutdown();
    drop(state);
    database.close().await;
    sqlx::query(AssertSqlSafe(format!("DROP SCHEMA {schema} CASCADE")))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], "INTERNAL_ERROR");
    assert_eq!(body["error"]["details"], json!({}));
    let logs = logs.text();
    assert!(logs.contains("internal operation failed"));
    assert!(!logs.contains("db-private-canary"));
    assert!(!logs.contains("a long uncommon secret passphrase"));
}

#[tokio::test]
async fn http_logs_never_contain_paths_queries_headers_or_rejected_payloads() {
    let output = log_capture::LogCapture::start();
    async {
        let app = app().await;
        for (method, uri, payload) in [
            ("GET", "/unknown/path-canary?secret=query-canary", ""),
            ("method-canary", "/api/rooms/path-canary", ""),
            ("GET", "/api/session/events?cursor=query-canary", ""),
            (
                "POST",
                "/api/rooms/current/participants/path-canary/recovery-credential",
                "{}",
            ),
            (
                "POST",
                "/api/rooms",
                "{\"payload-canary\":\"seed-hand-password-canary\"}",
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header(header::ORIGIN, ORIGIN)
                        .header("x-csrf-protection", "1")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::COOKIE, "__Host-session=cookie-canary")
                        .header(header::AUTHORIZATION, "Bearer authorization-canary")
                        .header("x-correlation-id", "correlation-canary")
                        .body(Body::from(payload))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(response.status().is_client_error());
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(!String::from_utf8_lossy(&body).contains("canary"));
        }
    }
    .await;
    let output = output.text();
    assert!(
        output.contains("HTTP request completed"),
        "the log capture must be live"
    );
    assert!(
        !output.contains("canary"),
        "private input reached logs: {output}"
    );
}

const ORIGIN: &str = "http://127.0.0.1:5173";

#[tokio::test]
async fn recovery_rate_limit_cannot_be_reset_with_forwarding_headers_or_new_attempts() {
    let app = app().await;
    for attempt in 0..61 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/session/recover")
                    .header(header::ORIGIN, ORIGIN)
                    .header("x-csrf-protection", "1")
                    .header("x-forwarded-for", format!("192.0.2.{attempt}"))
                    .header("forwarded", format!("for=192.0.2.{attempt}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        if attempt < 60 {
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        } else {
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            assert_eq!(response.headers()[header::RETRY_AFTER], "60");
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                    .unwrap();
            assert_eq!(body["error"]["code"], "RECOVERY_RATE_LIMITED");
        }
    }
}

#[tokio::test]
async fn rejected_payloads_have_bounded_redacted_errors_and_security_headers() {
    for (content_type, payload, status) in [
        (
            "application/json",
            "{\"private-canary\": 1}".to_owned(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "application/json",
            "{private-canary".to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "text/plain",
            "private-canary".to_owned(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            "application/json",
            "private-canary".repeat(2048),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
    ] {
        let response = app()
            .await
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/rooms")
                    .header(header::ORIGIN, ORIGIN)
                    .header("x-csrf-protection", "1")
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::from(payload))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let csp = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        for directive in [
            "default-src 'none'",
            "frame-ancestors 'none'",
            "object-src 'none'",
            "base-uri 'none'",
        ] {
            assert!(csp.contains(directive));
        }
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("private-canary"));
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error"]["code"], "INVALID_REQUEST");
    }
}

async fn app() -> axum::Router {
    let database = PgPoolOptions::new()
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let state = AppState::new(database);
    initialize(&state).await.unwrap();
    build_router(state)
}

#[tokio::test]
async fn same_origin_requests_still_require_explicit_csrf_protection() {
    for values in [vec![], vec!["0"], vec!["1", "1"]] {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/rooms")
            .header(header::ORIGIN, ORIGIN)
            .header(header::CONTENT_TYPE, "application/json");
        for value in values {
            request = request.header("x-csrf-protection", value);
        }
        let response = app()
            .await
            .oneshot(request.body(Body::from("{}")).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
        assert_eq!(body["error"]["code"], "CSRF_REQUIRED");
    }
}

#[tokio::test]
async fn every_mutation_rejects_missing_foreign_null_and_duplicate_origins_before_processing() {
    let app = app().await;
    for (method, path) in [
        ("POST", "/api/rooms"),
        ("POST", "/api/rooms/ABCDEFGH/participants"),
        ("POST", "/api/session/recover"),
        ("PUT", "/api/session/device-sessions/invalid/revocation"),
        ("PUT", "/api/session/protection"),
        ("PUT", "/api/rooms/current/protection"),
        ("PUT", "/api/session/recovery-password"),
        ("POST", "/api/session/recovery-credential"),
        (
            "POST",
            "/api/rooms/current/participants/2/recovery-credential",
        ),
        ("PUT", "/api/session/hero"),
        ("PUT", "/api/session/readiness"),
        ("POST", "/api/games"),
        ("POST", "/api/games/current/commands"),
    ] {
        for origins in [
            vec![],
            vec!["https://attacker.invalid"],
            vec!["null"],
            vec![ORIGIN, ORIGIN],
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(path)
                .header("x-csrf-protection", "1")
                .header(header::CONTENT_TYPE, "application/json");
            for origin in origins {
                request = request.header(header::ORIGIN, origin);
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from("{}")).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
            assert!(response.headers().get(header::SET_COOKIE).is_none());
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                    .unwrap();
            assert_eq!(body["error"]["code"], "ORIGIN_NOT_ALLOWED");
        }
    }
}
