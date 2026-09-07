//! Resumable deletion of expired aggregates and external, opaque evidence.
use std::{error::Error, fmt, future::Future, time::Duration};

use serde::{Deserialize, Serialize};
use sqlx::{Acquire, PgPool};
use uuid::Uuid;

mod ledger;
mod postgres;
pub use ledger::{FileLedger, S3Ledger};

/// External stores must acknowledge durable writes and tolerate duplicate calls.
/// Neither keys nor bodies may contain operational identifiers or credentials.
pub trait TombstoneLedger: Send + Sync {
    /// Stores with native TTL may keep the default; local stores expire proofs here.
    fn maintain(&self, _now_ms: i64) -> impl Future<Output = Result<(), PurgeError>> + Send {
        async { Ok(()) }
    }

    fn record(
        &self,
        key: &str,
        proof: &PurgeProof,
    ) -> impl Future<Output = Result<(), PurgeError>> + Send;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeProof {
    pub version: u8,
    pub expires_at_ms: i64,
    pub detected_at_ms: i64,
    pub completed_at_ms: Option<i64>,
}

/// Deliberately excludes upstream messages which can carry SQL row contents,
/// object keys, request URLs or credentials into long-lived telemetry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PurgeError {
    Database,
    Ledger,
    Orphans,
    Timeout,
}

impl fmt::Display for PurgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}",
            match self {
                Self::Database => "purge database operation failed",
                Self::Ledger => "external tombstone operation failed",
                Self::Orphans => "operational inventory verification failed",
                Self::Timeout => "purge stage timed out",
            }
        )
    }
}
impl Error for PurgeError {}
impl From<sqlx::Error> for PurgeError {
    fn from(_: sqlx::Error) -> Self {
        Self::Database
    }
}

#[derive(Debug, Default, Serialize, sqlx::FromRow)]
pub struct LifecycleMetrics {
    pub pending: i64,
    pub completed: i64,
    pub failed_attempts: i64,
    pub detection_p95_seconds: f64,
    pub purge_p95_seconds: f64,
    pub purge_max_seconds: f64,
    pub oldest_pending_seconds: f64,
    pub undetected: i64,
    pub overdue: i64,
    pub orphan_jobs: i64,
    pub stages: serde_json::Value,
}

pub struct LifecycleWorker<L> {
    database: PgPool,
    ledger: L,
    key: [u8; 32],
}

impl<L: TombstoneLedger> LifecycleWorker<L> {
    #[must_use]
    pub fn new(database: PgPool, ledger: L, key: [u8; 32]) -> Self {
        Self {
            database,
            ledger,
            key,
        }
    }

    /// Runs a bounded scan and at most 100 resumable stages, using database time.
    ///
    /// # Errors
    /// Returns a sanitized failure; failed jobs remain durable and retryable.
    pub async fn tick(&self) -> Result<usize, PurgeError> {
        postgres::enqueue(&self.database).await?;
        let now_ms: i64 =
            sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000)::BIGINT")
                .fetch_one(&self.database)
                .await?;
        self.ledger.maintain(now_ms).await?;
        let mut processed = 0;
        let mut failure = None;
        let batch_started = std::time::Instant::now();
        for _ in 0..100 {
            if batch_started.elapsed() >= Duration::from_secs(5) {
                break;
            }
            match self.step().await {
                Ok(true) => processed += 1,
                Ok(false) => break,
                Err(error) => failure = Some(error),
            }
        }
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(processed)
    }

    /// Measures completed and unfinished work; stalled jobs cannot hide breaches.
    ///
    /// # Errors
    /// Returns a sanitized database failure.
    pub async fn metrics(&self) -> Result<LifecycleMetrics, PurgeError> {
        postgres::metrics(&self.database).await.map_err(Into::into)
    }

    fn opaque_key(&self, game_id: Uuid) -> String {
        crate::encode_hex(&crate::hmac_sha256(
            &self.key,
            &[b"hogwarts-tombstone-v1", game_id.as_bytes()],
        ))
    }

    async fn step(&self) -> Result<bool, PurgeError> {
        let mut transaction = self.database.begin().await?;
        postgres::configure_transaction(&mut transaction).await?;
        let Some(job) = postgres::claim(&mut transaction).await? else {
            return Ok(false);
        };
        let mut stage = transaction.begin().await?;
        let result = tokio::time::timeout(Duration::from_secs(5), async {
            match job.stage.as_str() {
                "tombstone" => {
                    self.ledger
                        .record(&self.opaque_key(job.game_id), &job.proof())
                        .await?;
                    postgres::advance(&mut stage, job.game_id, "detach").await?;
                }
                "detach" => postgres::detach(&mut stage, &job).await?,
                "purge" => {
                    // Re-acknowledge before destructive work, including after long outages.
                    self.ledger
                        .record(&self.opaque_key(job.game_id), &job.proof())
                        .await?;
                    postgres::purge(&mut stage, &job).await?;
                }
                "verify" => postgres::verify(&mut stage, &job).await?,
                "complete" => {
                    // Verification is repeated immediately before publishing completion.
                    postgres::verify_inventory(&mut stage, &job).await?;
                    self.ledger
                        .record(&self.opaque_key(job.game_id), &job.proof())
                        .await?;
                    postgres::complete(&mut stage, &job).await?;
                }
                _ => return Err(PurgeError::Database),
            }
            Ok::<_, PurgeError>(())
        })
        .await
        .unwrap_or(Err(PurgeError::Timeout));
        match result {
            Ok(()) => {
                stage.commit().await?;
                transaction.commit().await?;
                Ok(true)
            }
            Err(error) => {
                stage.rollback().await?;
                postgres::retry_later(&mut transaction, job.game_id, error).await?;
                transaction.commit().await?;
                Err(error)
            }
        }
    }
}
