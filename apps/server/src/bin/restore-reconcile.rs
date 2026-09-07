use harry_potter_server::{
    AppState,
    lifecycle::{FileLedger, RestoredAccess, S3Ledger, reconcile_restore},
};
use sqlx::postgres::PgPoolOptions;
use std::{
    env,
    error::Error,
    fs::{File, OpenOptions},
    io,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::Duration,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Never enable protocol tracing or print restored identifiers and secrets.
    let session_key = secret("SESSION_TOKEN_KEY")?;
    let ledger_key = secret("TOMBSTONE_HMAC_KEY")?;
    let epoch = env::var("DEPLOYMENT_EPOCH")?.parse()?;
    let restore_point_ms = env::var("RESTORE_POINT_MS")?.parse()?;
    let access_path = PathBuf::from(env::var("RESTORE_ACCESS_FILE")?);
    let mut access_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&access_path)
        .map_err(|_| io::Error::other("RESTORE_ACCESS_FILE must be a new private file"))?;
    let database = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&env::var("DATABASE_URL")?)
        .await
        .map_err(|_| io::Error::other("restore database connection failed"))?;
    let state = AppState::new(database)
        .with_session_token_key(session_key)
        .with_deployment_epoch(epoch);
    let operation = async {
        let access = match (
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
                reconcile_restore(&state, &ledger, ledger_key, restore_point_ms).await?
            }
            (Err(_), Ok(directory)) => {
                reconcile_restore(
                    &state,
                    &FileLedger::new(directory.into()),
                    ledger_key,
                    restore_point_ms,
                )
                .await?
            }
            _ => {
                return Err(
                    io::Error::other("configure exactly one external tombstone ledger").into(),
                );
            }
        };
        Ok::<Vec<RestoredAccess>, Box<dyn Error>>(access)
    };
    let access = tokio::time::timeout(Duration::from_secs(3600), operation)
        .await
        .map_err(|_| io::Error::other("restore reconciliation exceeded 60 minutes"))??;
    serde_json::to_writer(&mut access_file, &access)?;
    access_file.sync_all()?;
    let parent = access_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    File::open(parent)?.sync_all()?;
    println!("restore reconciliation completed; private access file persisted");
    Ok(())
}

fn secret(name: &str) -> Result<[u8; 32], io::Error> {
    let value = env::var(name).map_err(|_| io::Error::other(format!("{name} is required")))?;
    value
        .as_bytes()
        .try_into()
        .map_err(|_| io::Error::other(format!("{name} must contain exactly 32 bytes")))
}
