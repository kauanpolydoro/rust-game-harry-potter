use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;
use std::{error::Error, fmt};

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::Serialize;
use sqlx::{PgPool, migrate::Migrator};

mod identity_access;
mod match_runtime;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

#[derive(Clone)]
pub struct AppState {
    database: PgPool,
    started: Arc<AtomicBool>,
    content: match_runtime::ContentCatalog,
}

impl AppState {
    /// Builds application state with the checked-in candidate content bundle.
    ///
    /// # Panics
    ///
    /// Panics when the compile-time bundle no longer satisfies the content
    /// schema. The content test suite protects this startup invariant.
    #[must_use]
    pub fn new(database: PgPool) -> Self {
        let manifest = game_content::import_base_bundle(include_bytes!(
            "../../../content/bundles/base-en-candidate-2026-09-02.json"
        ))
        .expect("the checked-in candidate content bundle must remain structurally valid");
        Self::with_content_manifests(database, vec![manifest])
    }

    #[must_use]
    pub fn with_content_manifests(
        database: PgPool,
        manifests: Vec<game_content::ContentManifest>,
    ) -> Self {
        Self {
            database,
            started: Arc::new(AtomicBool::new(false)),
            content: match_runtime::ContentCatalog::new(manifests),
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
        .merge(identity_access::router())
        .merge(match_runtime::router())
        .with_state(state)
}

#[derive(Debug)]
pub enum InitializationError {
    Migration(sqlx::migrate::MigrateError),
    Content(sqlx::Error),
}

impl fmt::Display for InitializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Migration(error) => write!(formatter, "database migration failed: {error}"),
            Self::Content(error) => write!(formatter, "content publication failed: {error}"),
        }
    }
}

impl Error for InitializationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Migration(error) => Some(error),
            Self::Content(error) => Some(error),
        }
    }
}

/// Applies every pending migration before opening the startup gate.
///
/// # Errors
///
/// Returns the migration error and keeps startup closed when PostgreSQL cannot
/// be reached or a migration cannot be applied.
pub async fn initialize(state: &AppState) -> Result<(), InitializationError> {
    MIGRATOR
        .run(&state.database)
        .await
        .map_err(InitializationError::Migration)?;
    match_runtime::publish_content(state)
        .await
        .map_err(InitializationError::Content)?;
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
