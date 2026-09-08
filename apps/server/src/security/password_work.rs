use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

use crate::http_support::ApiError;

pub(crate) struct PasswordWork {
    admitted: Arc<Semaphore>,
    running: Arc<Semaphore>,
}

impl Default for PasswordWork {
    fn default() -> Self {
        Self {
            admitted: Arc::new(Semaphore::new(12)),
            running: Arc::new(Semaphore::new(4)),
        }
    }
}

impl PasswordWork {
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, ()> + Send + 'static,
    ) -> Result<T, ApiError> {
        let operation = crate::telemetry::current_operation();
        let queue = crate::telemetry::Timer::new("queue_wait_seconds", operation);
        let Ok(admitted) = self.admitted.clone().try_acquire_owned() else {
            queue.finish("rate_limited");
            return Err(ApiError::rate_limited(true));
        };
        let running = match tokio::time::timeout(
            Duration::from_secs(1),
            self.running.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Err(_) => {
                queue.finish("rate_limited");
                return Err(ApiError::rate_limited(true));
            }
            Ok(Err(_)) => {
                queue.finish("error");
                return Err(ApiError::internal());
            }
        };
        let correlation = crate::REQUEST_CORRELATION_ID.try_with(|id| *id).ok();
        tokio::task::spawn_blocking(move || {
            // Blocking work cannot be aborted with its HTTP future. Keep both
            // permits until the actual work finishes, including executor queue time.
            let _permits = (admitted, running);
            queue.finish("success");
            let execute = || crate::telemetry::measure_sync("password_seconds", operation, work);
            if let Some(correlation) = correlation {
                crate::REQUEST_CORRELATION_ID.sync_scope(correlation, execute)
            } else {
                execute()
            }
        })
        .await
        .map_err(|error| ApiError::internal_with("password work failed", error))?
        .map_err(|()| ApiError::internal())
    }
}
