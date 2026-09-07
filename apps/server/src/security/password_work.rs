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
        let admitted = self
            .admitted
            .clone()
            .try_acquire_owned()
            .map_err(|_| ApiError::rate_limited(true))?;
        let running =
            tokio::time::timeout(Duration::from_secs(1), self.running.clone().acquire_owned())
                .await
                .map_err(|_| ApiError::rate_limited(true))?
                .map_err(|_| ApiError::internal())?;
        tokio::task::spawn_blocking(move || {
            // Blocking work cannot be aborted with its HTTP future. Keep both
            // permits until the actual work finishes, including executor queue time.
            let _permits = (admitted, running);
            work()
        })
        .await
        .map_err(|error| ApiError::internal_with("password work failed", error))?
        .map_err(|()| ApiError::internal())
    }
}
