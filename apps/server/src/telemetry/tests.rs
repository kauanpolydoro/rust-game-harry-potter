use axum::response::IntoResponse;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::post,
};
use serde_json::Value;
use tower::ServiceExt;

mod log_capture {
    use crate as harry_potter_server;
    include!("../../tests/support/log_capture.rs");
}

#[tokio::test]
async fn an_acceptance_after_an_access_denial_is_contained_even_with_a_commit() {
    let logs = log_capture::LogCapture::start();
    let app = Router::new()
        .route(
            "/api/games/current/commands",
            post(|| async {
                super::durable_acceptance();
                let mut response = crate::http_support::ApiError::game_expired().into_response();
                // Model a faulty response layer turning a known denial into success.
                *response.status_mut() = StatusCode::OK;
                response
            }),
        )
        .layer(middleware::from_fn(crate::correlate_request));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/games/current/commands")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(
        logs.text()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|event| event["metric"] == "p0_authorization_after_access_end")
    );
}

#[tokio::test]
async fn an_http_acceptance_without_commit_evidence_is_contained_and_alerted() {
    let logs = log_capture::LogCapture::start();
    // Inject a faulty HTTP handler at the observation boundary. Actual commit
    // and retry behavior is exercised separately with the real PostgreSQL adapter.
    let app = Router::new()
        .route(
            "/api/games/current/commands",
            post(|| async { StatusCode::OK }),
        )
        .layer(middleware::from_fn(crate::correlate_request));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/games/current/commands")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let events: Vec<Value> = logs
        .text()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        events
            .iter()
            .any(|event| event["metric"] == "p0_ack_before_commit")
    );
}
