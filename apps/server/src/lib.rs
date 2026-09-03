use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::Serialize;
use sqlx::{PgPool, migrate::Migrator};

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

#[derive(Clone)]
pub struct AppState {
    database: PgPool,
    started: Arc<AtomicBool>,
}

impl AppState {
    #[must_use]
    pub fn new(database: PgPool) -> Self {
        Self {
            database,
            started: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn mark_started(&self) {
        self.started.store(true, Ordering::Release);
    }

    fn is_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/health/startup", get(startup))
        .with_state(state)
}

/// Applies every pending migration before opening the startup gate.
///
/// # Errors
///
/// Returns the migration error and keeps startup closed when PostgreSQL cannot
/// be reached or a migration cannot be applied.
pub async fn initialize(state: &AppState) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(&state.database).await?;
    state.mark_started();
    Ok(())
}

async fn liveness() -> Json<HealthResponse> {
    Json(HealthResponse { status: "alive" })
}

async fn startup(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    if state.is_started() {
        (StatusCode::OK, Json(HealthResponse { status: "started" }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse { status: "starting" }),
        )
    }
}

async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let database_is_ready = if state.is_started() {
        tokio::time::timeout(
            Duration::from_secs(1),
            sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.database),
        )
        .await
        .is_ok_and(|result| result.is_ok_and(|value| value == 1))
    } else {
        false
    };

    if database_is_ready {
        (StatusCode::OK, Json(HealthResponse { status: "ready" }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "unavailable",
            }),
        )
    }
}
