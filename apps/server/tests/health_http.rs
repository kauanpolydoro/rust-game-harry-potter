use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use harry_potter_server::{AppState, build_router, initialize};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

fn unavailable_database() -> sqlx::PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://health:health@127.0.0.1:1/health")
        .expect("the test database URL must be syntactically valid")
}

#[tokio::test]
async fn liveness_reports_the_running_http_process_before_startup() {
    let response = build_router(AppState::new(unavailable_database()))
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .expect("the request must be valid"),
        )
        .await
        .expect("the health router must respond");

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn startup_stays_unavailable_until_initialization_finishes() {
    let state = AppState::new(unavailable_database());
    let app = build_router(state.clone());

    let before_initialization = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/startup")
                .body(Body::empty())
                .expect("the request must be valid"),
        )
        .await
        .expect("the health router must respond");

    state.mark_started();

    let after_initialization = app
        .oneshot(
            Request::builder()
                .uri("/health/startup")
                .body(Body::empty())
                .expect("the request must be valid"),
        )
        .await
        .expect("the health router must respond");

    assert_eq!(
        before_initialization.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(after_initialization.status(), StatusCode::OK);
}

#[tokio::test]
async fn readiness_rejects_traffic_when_the_database_is_unavailable() {
    let state = AppState::new(unavailable_database());
    state.mark_started();

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("the request must be valid"),
        )
        .await
        .expect("the health router must respond");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn readiness_accepts_traffic_after_the_database_is_migrated() {
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

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .expect("the request must be valid"),
        )
        .await
        .expect("the health router must respond");

    assert_eq!(response.status(), StatusCode::OK);
}
