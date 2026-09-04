use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use std::{error::Error, fmt, fmt::Write as _};

use axum::{
    Json, Router,
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::get,
};
use hmac::{Hmac, KeyInit, Mac};
use serde::Serialize;
use sha2::Sha256;
use sqlx::{PgPool, migrate::Migrator};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, watch};
use tracing::Instrument;
use uuid::Uuid;

mod content_catalog;
mod current_session;
mod http_support;
mod identity_access;
mod match_runtime;
mod session;
mod session_events;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
const DEFAULT_APPLICATION_ORIGIN: &str = "http://127.0.0.1:5173";
const MAX_CONCURRENT_RECOVERY_PASSWORD_CHECKS: usize = 4;

tokio::task_local! {
    static REQUEST_CORRELATION_ID: Uuid;
}

#[derive(Clone)]
pub struct AppState {
    database: PgPool,
    migration_database: PgPool,
    started: Arc<AtomicBool>,
    content: content_catalog::ContentCatalog,
    application_origin: Arc<str>,
    game_event_fanout: EventFanout,
    game_presence_fanout: EventFanout,
    security_event_fanout: EventFanout,
    session_token_key: Arc<[u8; 32]>,
    recovery_token_key: Arc<[u8; 32]>,
    recovery_password_checks: Arc<Semaphore>,
    shutdown: watch::Sender<bool>,
}

#[derive(Clone, Default)]
struct EventFanout {
    channels: Arc<Mutex<HashMap<Uuid, broadcast::Sender<()>>>>,
}

impl EventFanout {
    fn subscribe(&self, game_id: Uuid) -> broadcast::Receiver<()> {
        let mut channels = self.channels.lock().unwrap_or_else(PoisonError::into_inner);
        channels
            .entry(game_id)
            .or_insert_with(|| broadcast::channel(64).0)
            .subscribe()
    }

    fn signal(&self, game_id: Uuid) {
        let mut channels = self.channels.lock().unwrap_or_else(PoisonError::into_inner);
        let remove = channels
            .get(&game_id)
            .is_some_and(|sender| sender.send(()).is_err());
        if remove {
            channels.remove(&game_id);
        }
    }

    fn prune(&self, game_id: Uuid) {
        let mut channels = self.channels.lock().unwrap_or_else(PoisonError::into_inner);
        let remove = channels
            .get(&game_id)
            .is_some_and(|sender| sender.receiver_count() == 0);
        if remove {
            channels.remove(&game_id);
        }
    }
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
    /// Builds application state from an explicitly supplied content catalog.
    ///
    /// # Panics
    ///
    /// Panics when the operating system cannot provide entropy for the
    /// default ephemeral session key. Production replaces it with the stable
    /// configured key before serving requests.
    pub fn with_content_manifests(
        database: PgPool,
        manifests: Vec<game_content::ContentManifest>,
    ) -> Self {
        let mut session_token_key = [0_u8; 32];
        getrandom::fill(&mut session_token_key)
            .expect("the operating system must provide session key entropy");
        let recovery_token_key = recovery_token_key(&session_token_key);
        let (shutdown, _) = watch::channel(false);
        Self {
            migration_database: database.clone(),
            database,
            started: Arc::new(AtomicBool::new(false)),
            content: content_catalog::ContentCatalog::new(manifests),
            application_origin: Arc::from(DEFAULT_APPLICATION_ORIGIN),
            game_event_fanout: EventFanout::default(),
            game_presence_fanout: EventFanout::default(),
            security_event_fanout: EventFanout::default(),
            session_token_key: Arc::new(session_token_key),
            recovery_token_key: Arc::new(recovery_token_key),
            recovery_password_checks: Arc::new(Semaphore::new(
                MAX_CONCURRENT_RECOVERY_PASSWORD_CHECKS,
            )),
            shutdown,
        }
    }

    /// Sets the one browser origin accepted by authenticated WebSocket handshakes.
    #[must_use]
    pub fn with_application_origin(mut self, application_origin: impl Into<Arc<str>>) -> Self {
        self.application_origin = application_origin.into();
        self
    }

    /// Configures the stable secret used to reproduce an idempotent session grant.
    #[must_use]
    pub fn with_session_token_key(mut self, session_token_key: [u8; 32]) -> Self {
        self.recovery_token_key = Arc::new(recovery_token_key(&session_token_key));
        self.session_token_key = Arc::new(session_token_key);
        self
    }

    /// Uses a dedicated pool for schema migrations.
    ///
    /// Production keeps migration connections free of request-level statement
    /// and transaction limits so a valid backfill cannot enter a retry loop.
    #[must_use]
    pub fn with_migration_database(mut self, migration_database: PgPool) -> Self {
        self.migration_database = migration_database;
        self
    }

    pub fn mark_started(&self) {
        self.started.store(true, Ordering::Release);
    }

    fn is_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    fn application_origin(&self) -> &str {
        &self.application_origin
    }

    fn subscribe_to_game_events(&self, game_id: Uuid) -> broadcast::Receiver<()> {
        self.game_event_fanout.subscribe(game_id)
    }

    fn signal_game_event(&self, game_id: Uuid) {
        self.game_event_fanout.signal(game_id);
    }

    fn prune_game_event_channel(&self, game_id: Uuid) {
        self.game_event_fanout.prune(game_id);
    }

    fn subscribe_to_game_presence(&self, game_id: Uuid) -> broadcast::Receiver<()> {
        self.game_presence_fanout.subscribe(game_id)
    }

    fn signal_game_presence(&self, game_id: Uuid) {
        self.game_presence_fanout.signal(game_id);
    }

    fn prune_game_presence_channel(&self, game_id: Uuid) {
        self.game_presence_fanout.prune(game_id);
    }

    fn subscribe_to_security_events(&self, room_id: Uuid) -> broadcast::Receiver<()> {
        self.security_event_fanout.subscribe(room_id)
    }

    fn signal_security_event(&self, room_id: Uuid) {
        self.security_event_fanout.signal(room_id);
    }

    fn prune_security_event_channel(&self, room_id: Uuid) {
        self.security_event_fanout.prune(room_id);
    }

    fn idempotent_session_token(
        &self,
        operation: &str,
        idempotency_key: &str,
        participant_id: Uuid,
    ) -> String {
        let mut hasher = blake3::Hasher::new_keyed(&self.session_token_key);
        for value in [
            b"hogwarts-session-grant-v1".as_slice(),
            operation.as_bytes(),
            idempotency_key.as_bytes(),
            participant_id.as_bytes(),
        ] {
            hasher.update(&(value.len() as u64).to_be_bytes());
            hasher.update(value);
        }
        hasher.finalize().to_hex().to_string()
    }

    fn idempotent_recovery_token(
        &self,
        operation: &str,
        idempotency_key: &str,
        participant_id: Uuid,
    ) -> String {
        encode_hex(&hmac_sha256(
            self.recovery_token_key.as_ref(),
            &[
                b"hogwarts-recovery-token-v1",
                operation.as_bytes(),
                idempotency_key.as_bytes(),
                participant_id.as_bytes(),
            ],
        ))
    }

    fn recovery_token_hmac(&self, token: &str) -> String {
        format!(
            "hmac-sha256:{}",
            encode_hex(&hmac_sha256(
                self.recovery_token_key.as_ref(),
                &[b"hogwarts-recovery-token-storage-v1", token.as_bytes()],
            ))
        )
    }

    fn recovery_request_fingerprint(
        &self,
        operation: &str,
        idempotency_key: &str,
        values: &[&[u8]],
    ) -> String {
        let mut fingerprint_values = Vec::with_capacity(values.len() + 3);
        fingerprint_values.push(b"hogwarts-recovery-request-v1".as_slice());
        fingerprint_values.push(operation.as_bytes());
        fingerprint_values.push(idempotency_key.as_bytes());
        fingerprint_values.extend_from_slice(values);
        format!(
            "hmac-sha256:{}",
            encode_hex(&hmac_sha256(
                self.recovery_token_key.as_ref(),
                &fingerprint_values,
            ))
        )
    }

    fn recovered_session_token(&self, token: &str, participant_id: Uuid) -> String {
        encode_hex(&hmac_sha256(
            self.session_token_key.as_ref(),
            &[
                b"hogwarts-recovered-session-v1",
                token.as_bytes(),
                participant_id.as_bytes(),
            ],
        ))
    }

    fn try_recovery_password_check(&self) -> Option<OwnedSemaphorePermit> {
        self.recovery_password_checks
            .clone()
            .try_acquire_owned()
            .ok()
    }

    fn subscribe_to_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    pub fn begin_shutdown(&self) {
        self.shutdown.send_replace(true);
    }
}

fn recovery_token_key(session_token_key: &[u8; 32]) -> [u8; 32] {
    hmac_sha256(session_token_key, &[b"hogwarts-recovery-token-key-v1"])
}

fn hmac_sha256(key: &[u8], values: &[&[u8]]) -> [u8; 32] {
    let mut hmac = <Hmac<Sha256> as KeyInit>::new_from_slice(key)
        .expect("HMAC-SHA-256 accepts keys of any size");
    for value in values {
        hmac.update(&(value.len() as u64).to_be_bytes());
        hmac.update(value);
    }
    hmac.finalize().into_bytes().into()
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
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
        .merge(current_session::router())
        .merge(match_runtime::router())
        .merge(session_events::router())
        .with_state(state)
        .layer(middleware::from_fn(correlate_request))
}

async fn correlate_request(request: Request, next: Next) -> Response {
    let correlation_id = Uuid::new_v4();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let span = tracing::info_span!(
        "http_request",
        correlation_id = %correlation_id,
        %method,
        %path
    );
    REQUEST_CORRELATION_ID
        .scope(correlation_id, async move {
            let mut response = next.run(request).await;
            if let Ok(value) = HeaderValue::from_str(&correlation_id.to_string()) {
                response.headers_mut().insert("x-correlation-id", value);
            }
            tracing::info!(
                status = response.status().as_u16(),
                "HTTP request completed"
            );
            response
        })
        .instrument(span)
        .await
}

pub(crate) fn current_correlation_id() -> Uuid {
    REQUEST_CORRELATION_ID
        .try_with(|correlation_id| *correlation_id)
        .unwrap_or_else(|_| Uuid::new_v4())
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
        .run(&state.migration_database)
        .await
        .map_err(InitializationError::Migration)?;
    state
        .content
        .publish(&state.database)
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
