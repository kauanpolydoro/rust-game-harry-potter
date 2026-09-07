use std::{env, error::Error, io, time::Duration};

use harry_potter_server::{
    AppState, initialize,
    lifecycle::{FileLedger, LifecycleWorker, S3Ledger, TombstoneLedger},
};
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // AWS/SQL wire debugging can expose identifiers and credentials. Production
    // only emits this binary's closed, anonymous operational events.
    tracing_subscriber::fmt()
        .with_env_filter("lifecycle_worker=info")
        .json()
        .init();
    let value = env::var("TOMBSTONE_HMAC_KEY")?;
    let key: [u8; 32] = value
        .as_bytes()
        .try_into()
        .map_err(|_| io::Error::other("TOMBSTONE_HMAC_KEY must contain exactly 32 bytes"))?;
    let database = PgPoolOptions::new()
        .max_connections(6)
        .acquire_timeout(Duration::from_secs(5))
        .after_connect(|connection, _| {
            Box::pin(async move {
                for setting in [
                    "SET lock_timeout = '2s'",
                    "SET statement_timeout = '4s'",
                    "SET transaction_timeout = '15s'",
                    "SET idle_in_transaction_session_timeout = '15s'",
                ] {
                    sqlx::query(setting).execute(&mut *connection).await?;
                }
                Ok(())
            })
        })
        .connect(&env::var("DATABASE_URL")?)
        .await?;
    let migrations = PgPoolOptions::new()
        .max_connections(2)
        .connect(&env::var("DATABASE_URL")?)
        .await?;
    let session_value = env::var("SESSION_TOKEN_KEY")?;
    let session_key = session_value
        .as_bytes()
        .try_into()
        .map_err(|_| io::Error::other("SESSION_TOKEN_KEY must contain exactly 32 bytes"))?;
    let state = AppState::new(database.clone())
        .with_migration_database(migrations)
        .with_session_token_key(session_key)
        .with_deployment_epoch(env::var("DEPLOYMENT_EPOCH")?.parse()?);
    initialize(&state)
        .await
        .map_err(|_| io::Error::other("worker initialization failed"))?;
    match (
        env::var("TOMBSTONE_BUCKET"),
        env::var("TOMBSTONE_LOCAL_DIRECTORY"),
    ) {
        (Ok(bucket), Err(_)) => {
            let config = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .timeout_config(
                    aws_sdk_s3::config::timeout::TimeoutConfig::builder()
                        .operation_timeout(Duration::from_secs(3))
                        .build(),
                )
                .load()
                .await;
            let ledger = S3Ledger::new(aws_sdk_s3::Client::new(&config), bucket);
            ledger.validate_retention().await?;
            run(LifecycleWorker::new(database, ledger, key), &state).await;
        }
        (Err(_), Ok(directory)) => {
            run(
                LifecycleWorker::new(database, FileLedger::new(directory.into()), key),
                &state,
            )
            .await;
        }
        _ => {
            return Err(io::Error::other(
                "configure exactly one of TOMBSTONE_BUCKET or TOMBSTONE_LOCAL_DIRECTORY",
            )
            .into());
        }
    }
    state.begin_shutdown();
    Ok(())
}

async fn run<L: TombstoneLedger>(worker: LifecycleWorker<L>, state: &AppState) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => return,
            _ = interval.tick() => {
                if !state.accepts_traffic().await {
                    tracing::warn!("worker deployment gate closed");
                    continue;
                }
                if let Err(error) = worker.tick().await { tracing::warn!(%error, "purge batch failed; retry scheduled"); }
                match worker.metrics().await {
                    Ok(metrics) => {
                        tracing::info!(pending = metrics.pending, completed = metrics.completed,
                            failed_attempts = metrics.failed_attempts, orphan_jobs = metrics.orphan_jobs, stages = %metrics.stages, undetected = metrics.undetected,
                            detection_p95_seconds = metrics.detection_p95_seconds,
                            purge_p95_seconds = metrics.purge_p95_seconds, purge_max_seconds = metrics.purge_max_seconds,
                            oldest_pending_seconds = metrics.oldest_pending_seconds, overdue = metrics.overdue,
                            "lifecycle metrics");
                        if metrics.overdue > 0 || metrics.purge_max_seconds >= 86400.0 || metrics.detection_p95_seconds > 300.0 || metrics.purge_p95_seconds > 3600.0 {
                            tracing::error!("lifecycle SLO breached");
                        }
                    }
                    Err(error) => tracing::error!(%error, "lifecycle measurement unavailable"),
                }
            }
        }
    }
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler must be available");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
