use std::{env, error::Error, io, net::SocketAddr, time::Duration};

use harry_potter_server::{AppState, build_router, initialize};
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

const INITIALIZATION_RETRY_DELAY: Duration = Duration::from_secs(2);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    initialize_tracing();

    let database_url = env::var("DATABASE_URL")?;
    let session_token_key = session_token_key()?;
    let bind_address = env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8080".to_owned());
    let bind_address: SocketAddr = bind_address.parse()?;
    let migration_database = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect_lazy(&database_url)?;
    let database = PgPoolOptions::new()
        .max_connections(20)
        .acquire_timeout(Duration::from_secs(1))
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                for statement in [
                    "SET statement_timeout = '5s'",
                    "SET lock_timeout = '2s'",
                    "SET transaction_timeout = '15s'",
                    "SET idle_in_transaction_session_timeout = '15s'",
                ] {
                    sqlx::query(statement).execute(&mut *connection).await?;
                }
                Ok(())
            })
        })
        .connect_lazy(&database_url)?;
    let mut state = AppState::new(database)
        .with_migration_database(migration_database)
        .with_session_token_key(session_token_key);
    if let Ok(origin) = env::var("APPLICATION_ORIGIN") {
        state = state.with_application_origin(origin);
    }
    let initialization = tokio::spawn(initialize_until_ready(state.clone()));

    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    tracing::info!(address = %bind_address, "HTTP server listening");

    let shutdown_state = state.clone();
    let result = axum::serve(listener, build_router(state))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            shutdown_state.begin_shutdown();
        })
        .await;

    initialization.abort();
    result?;
    Ok(())
}

fn session_token_key() -> Result<[u8; 32], Box<dyn Error>> {
    let value = env::var("SESSION_TOKEN_KEY")?;
    value.as_bytes().try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "SESSION_TOKEN_KEY must contain exactly 32 bytes",
        )
        .into()
    })
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
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %error, "failed to install Ctrl+C signal handler");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => {
                tracing::error!(error = %error, "failed to install SIGTERM signal handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
    tracing::info!("shutdown signal received");
}
