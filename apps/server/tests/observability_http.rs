use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use harry_potter_server::{AppState, build_router};
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

#[path = "support/log_capture.rs"]
mod log_capture;

#[tokio::test]
async fn http_observations_are_exportable_and_private_fields_never_reach_the_writer() {
    let logs = log_capture::LogCapture::start();
    let app = build_router(AppState::new(
        PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap(),
    ));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health/live?token=telemetry-private-canary")
                .header("cookie", "telemetry-private-canary")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let correlation = response.headers()["x-correlation-id"].to_str().unwrap();
    tracing::info!(target: "harry_potter_server", route = "/api/rooms/telemetry-private-route-canary", "HTTP request completed");
    // A future accidental first-party log must be contained at the writer,
    // including private span fields, even with maximum verbosity enabled.
    let span = tracing::info_span!(target: "harry_potter_server", "unsafe", password = "telemetry-private-canary");
    let _entered = span.enter();
    tracing::info!(target: "harry_potter_server", payload = "telemetry-private-canary", "unsafe input");
    let output = logs.text();
    assert!(!output.contains("telemetry-private-canary"));
    assert!(!output.contains("telemetry-private-route-canary"));
    let events: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let request = events
        .iter()
        .find(|event| event["metric"] == "http_requests" && event["correlation_id"] == correlation)
        .expect("HTTP denominator must be exported");
    assert_eq!(request["value"], 1.0);
    assert_eq!(request["operation"], "health");
    assert_eq!(request["outcome"], "success");
    assert_eq!(
        request["_aws"]["CloudWatchMetrics"][0]["Namespace"],
        "Hogwarts"
    );
    assert!(
        events
            .iter()
            .any(|event| event["metric"] == "http_duration_seconds"
                && event["correlation_id"] == correlation)
    );
    assert!(
        events
            .iter()
            .any(|event| event["metric"] == "p0_secret_detected")
    );
}

#[tokio::test]
async fn browser_observations_have_a_closed_numeric_schema_and_cannot_raise_p0() {
    let logs = log_capture::LogCapture::start();
    let app = build_router(AppState::new(
        PgPoolOptions::new()
            .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
            .unwrap(),
    ));
    for (body, expected) in [
        (
            r#"{"metric":"web_lcp_seconds","value":1.2,"outcome":"success"}"#,
            StatusCode::NO_CONTENT,
        ),
        (
            r#"{"metric":"web_lcp_seconds","value":1.2,"outcome":"success","url":"private-browser-canary"}"#,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            r#"{"metric":"p0_restore_resurrection","value":1,"outcome":"success"}"#,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            r#"{"metric":"web_lcp_seconds","value":-1,"outcome":"success"}"#,
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/telemetry")
                    .header("origin", "http://127.0.0.1:5173")
                    .header("x-csrf-protection", "1")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let output = logs.text();
    assert!(!output.contains("private-browser-canary"));
    assert!(
        output
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .any(|event| event["metric"] == "web_lcp_seconds" && event["value"] == 1.2)
    );
}
