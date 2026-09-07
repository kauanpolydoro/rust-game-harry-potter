use std::{env, error::Error, io, time::Duration};

use harry_potter_server::{
    AppState, initialize,
    lifecycle::{FileLedger, LifecycleWorker, S3Ledger, TombstoneLedger},
};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{EnvFilter, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    harry_potter_server::tracing_subscriber(io::stdout, EnvFilter::new("info")).init();
    std::panic::set_hook(Box::new(|_| tracing::error!("process panicked")));
    if run_application().await.is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        tracing::error!("process terminated after an operational failure");
        std::process::ExitCode::FAILURE
    }
}

async fn run_application() -> Result<(), Box<dyn Error>> {
    let audit_restore = audit_requested()?;
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
    let state = AppState::new(database.clone()).with_migration_database(migrations);
    if !audit_restore {
        initialize(&state)
            .await
            .map_err(|_| io::Error::other("worker initialization failed"))?;
    }
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
            execute(LifecycleWorker::new(database, ledger, key), audit_restore).await?;
        }
        (Err(_), Ok(directory)) => {
            execute(
                LifecycleWorker::new(database, FileLedger::new(directory.into()), key),
                audit_restore,
            )
            .await?;
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

fn audit_requested() -> Result<bool, io::Error> {
    let arguments: Vec<_> = env::args().skip(1).collect();
    match arguments.as_slice() {
        [] => Ok(false),
        [argument] if argument == "--audit-restore" => Ok(true),
        _ => Err(io::Error::other(
            "usage: lifecycle-worker [--audit-restore]",
        )),
    }
}

async fn execute<L: TombstoneLedger>(
    worker: LifecycleWorker<L>,
    audit_restore: bool,
) -> Result<(), Box<dyn Error>> {
    if audit_restore {
        let report = worker.audit_restore().await?;
        if report.resurrected > 0 {
            return Err(io::Error::other(
                "restore audit failed: external tombstones conflict with operational roots",
            )
            .into());
        }
    } else {
        run(worker).await;
    }
    Ok(())
}

async fn run<L: TombstoneLedger>(worker: LifecycleWorker<L>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => return,
            _ = interval.tick() => {
                if let Err(_error) = worker.tick().await { tracing::warn!("purge batch failed; retry scheduled"); }
                match worker.metrics().await {
                    Ok(metrics) => {
                        if metrics.overdue > 0 || metrics.purge_max_seconds >= 86400.0 || metrics.detection_p95_seconds > 300.0 || metrics.purge_p95_seconds > 3600.0 {
                            tracing::error!("lifecycle SLO breached");
                        }
                    }
                    Err(_error) => tracing::error!("lifecycle measurement unavailable"),
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
