use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use harry_potter_server::{AppState, build_router};
use sqlx::postgres::PgPoolOptions;
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
    time::Duration,
};
use tower::ServiceExt;
use tracing::instrument::WithSubscriber;

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);
impl Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn telemetry_records_registered_routes_without_identifiable_paths_bodies_or_credentials() {
    let capture = CapturedLog::default();
    let writer = capture.clone();
    let subscriber = harry_potter_server::tracing_subscriber(
        move || writer.clone(),
        tracing_subscriber::EnvFilter::new("trace"),
    );
    let database = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(50))
        .connect_lazy("postgres://unavailable:unavailable@127.0.0.1:1/unavailable")
        .unwrap();
    let app = build_router(AppState::new(database));
    async {
        let unknown = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/unmatched/PRIVATE-ROOM")
                    .header("cookie", "session=PRIVATE-COOKIE")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let join = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/rooms/PRIVATE-ROOM/participants")
                    .header("content-type", "application/json")
                    .header("origin", "http://127.0.0.1:5173")
                    .header("x-csrf-protection", "1")
                    .header("idempotency-key", "PRIVATE-IDEMPOTENCY")
                    .body(Body::from(
                        r#"{"display_name":"PRIVATE-NAME","hero_id":"hermione"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(join.status().is_client_error() || join.status().is_server_error());
    }
    .with_subscriber(subscriber)
    .await;
    let log = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
    assert!(log.contains("HTTP request completed"));
    assert!(log.contains("unmatched"));
    assert!(log.contains("/api/rooms/{room_code}/participants"));
    for secret in [
        "PRIVATE-ROOM",
        "PRIVATE-COOKIE",
        "PRIVATE-IDEMPOTENCY",
        "PRIVATE-NAME",
        "unavailable:unavailable",
    ] {
        assert!(
            !log.contains(secret),
            "identifiable value leaked to telemetry"
        );
    }
}
