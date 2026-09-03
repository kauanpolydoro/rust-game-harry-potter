use std::{env, error::Error, net::SocketAddr, time::Duration};

use harry_potter_server::{AppState, build_router, initialize};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

const INITIALIZATION_RETRY_DELAY: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    initialize_tracing();

    let database_url = env::var("DATABASE_URL")?;
    let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
    let bind_address: SocketAddr = bind_address.parse()?;
    let database = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(1))
        .connect_lazy(&database_url)?;
    let state = AppState::new(database);
    let initialization = tokio::spawn(initialize_until_ready(state.clone()));

    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    tracing::info!(address = %bind_address, "HTTP server listening");

    let result = axum::serve(listener, build_router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await;

    initialization.abort();
    result?;
    Ok(())
}

fn initialize_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .init();
}

async fn initialize_until_ready(state: AppState) {
    loop {
        match initialize(&state).await {
            Ok(()) => {
                tracing::info!("startup initialization completed");
                return;
            }
            Err(error) => {
                tracing::warn!(error = %error, "startup initialization failed; retrying");
                tokio::time::sleep(INITIALIZATION_RETRY_DELAY).await;
            }
        }
    }
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %error, "failed to install shutdown signal handler");
    }
}
