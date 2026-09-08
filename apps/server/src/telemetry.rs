//! Sanitized observations. Metric labels are a closed vocabulary, never user data.
use std::time::Instant;

use tracing::Subscriber;
use tracing_subscriber::{
    EnvFilter,
    filter::{FilterExt, filter_fn},
    fmt::MakeWriter,
    prelude::*,
};

mod access_end;
mod allowlist;
pub(crate) use access_end::close_access;
pub(crate) mod client;
pub(crate) mod delivery;
mod evidence;
mod output;
mod runtime;
pub(crate) use evidence::{access_ended, commit, durable_acceptance, http};
pub use runtime::observe_runtime;
#[cfg(test)]
mod tests;

tokio::task_local! {
    pub(crate) static OPERATION: &'static str;
    pub(crate) static COMMAND_TRACE: uuid::Uuid;
}

pub(crate) fn current_operation() -> &'static str {
    OPERATION
        .try_with(|operation| *operation)
        .unwrap_or("runtime")
}

/// Builds the production subscriber. Dependency logs and unapproved fields never
/// reach the writer. Operational metrics remain enabled regardless of verbosity.
pub fn tracing_subscriber<W>(writer: W, filter: EnvFilter) -> impl Subscriber + Send + Sync
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let targets = filter_fn(|metadata| {
        metadata.target() == "harry_potter_server"
            || metadata.target().starts_with("harry_potter_server::")
            || metadata.target() == "lifecycle_worker"
    });
    let metrics = filter_fn(|metadata| metadata.target() == "harry_potter_server::telemetry");
    tracing_subscriber::registry()
        .with(output::SafeOutput(writer).with_filter(targets.and(filter.or(metrics))))
}

pub(crate) fn observe(
    metric: &'static str,
    operation: &'static str,
    outcome: &'static str,
    value: f64,
) {
    tracing::info!(metric, operation, outcome, value, "observation");
}

pub(crate) async fn measure<T, E>(
    metric: &'static str,
    operation: &'static str,
    work: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let timer = Timer::new(metric, operation);
    let result = work.await;
    timer.finish(if result.is_ok() {
        "success"
    } else if metric == "commit_seconds" {
        "uncertain"
    } else {
        "error"
    });
    result
}

pub(crate) fn measure_sync<T, E>(
    metric: &'static str,
    operation: &'static str,
    work: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let timer = Timer::new(metric, operation);
    let result = work();
    timer.finish(if result.is_ok() { "success" } else { "error" });
    result
}

pub(crate) fn http_operation(route: &str) -> &'static str {
    match route {
        "/api/telemetry" => "telemetry_ingest",
        "/api/games/current/commands" => "command",
        "/api/session/recover" => "recovery",
        "/api/games/current/events" | "/api/session/events" => "handshake",
        "/health/live" | "/health/ready" | "/health/startup" => "health",
        _ if route.starts_with("/api/") => "api",
        _ => "unmatched",
    }
}

pub(crate) struct Timer {
    metric: &'static str,
    operation: &'static str,
    started: Instant,
    outcome: &'static str,
    counter: Option<&'static str>,
    correlation: Option<uuid::Uuid>,
}

impl Timer {
    pub(crate) fn new(metric: &'static str, operation: &'static str) -> Self {
        Self {
            metric,
            operation,
            started: Instant::now(),
            outcome: "cancelled",
            counter: None,
            correlation: crate::REQUEST_CORRELATION_ID.try_with(|id| *id).ok(),
        }
    }

    pub(crate) fn with_counter(mut self, counter: &'static str) -> Self {
        self.counter = Some(counter);
        self
    }

    pub(crate) fn with_failure_outcome(mut self, outcome: &'static str) -> Self {
        self.outcome = outcome;
        self
    }

    pub(crate) fn set_operation(&mut self, operation: &'static str) {
        self.operation = operation;
    }

    pub(crate) fn finish(mut self, outcome: &'static str) {
        self.outcome = outcome;
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        let emit = || {
            observe(
                self.metric,
                self.operation,
                self.outcome,
                self.started.elapsed().as_secs_f64(),
            );
            if let Some(counter) = self.counter {
                observe(counter, self.operation, self.outcome, 1.0);
            }
        };
        if let Some(correlation) = self.correlation {
            crate::REQUEST_CORRELATION_ID.sync_scope(correlation, emit);
        } else {
            emit();
        }
    }
}
