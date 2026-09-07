//! One-time adoption of the original database when upgrading from schema 22.
use harry_potter_server::{
    AppState,
    lifecycle::{FileLedger, S3Ledger, adopt_existing_source},
};
use sqlx::postgres::PgPoolOptions;
use std::{env, error::Error, io, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let session_key = secret("SESSION_TOKEN_KEY")?;
    let ledger_key = secret("TOMBSTONE_HMAC_KEY")?;
    let database = PgPoolOptions::new()
        .max_connections(3)
        .connect(&env::var("DATABASE_URL")?)
        .await
        .map_err(|_| io::Error::other("source database connection failed"))?;
    let state = AppState::new(database)
        .with_session_token_key(session_key)
        .with_deployment_epoch(env::var("DEPLOYMENT_EPOCH")?.parse()?);
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
            adopt_existing_source(&state, &ledger, ledger_key).await?;
        }
        (Err(_), Ok(directory)) => {
            adopt_existing_source(&state, &FileLedger::new(directory.into()), ledger_key).await?;
        }
        _ => {
            return Err(io::Error::other("configure exactly one external tombstone ledger").into());
        }
    }
    println!("original source adopted; existing participation preserved");
    Ok(())
}
fn secret(name: &str) -> Result<[u8; 32], io::Error> {
    let value = env::var(name).map_err(|_| io::Error::other(format!("{name} is required")))?;
    value
        .as_bytes()
        .try_into()
        .map_err(|_| io::Error::other(format!("{name} must contain exactly 32 bytes")))
}
