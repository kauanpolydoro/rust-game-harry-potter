use std::{cell::Cell, future::Future};

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sqlx::{Postgres, Transaction};

use crate::http_support::ApiError;

#[derive(Default)]
struct Evidence {
    durable_acceptance: Cell<bool>,
    access_ended: Cell<bool>,
}

tokio::task_local! {
    static EVIDENCE: Evidence;
}

/// Called only after observing a pre-existing receipt or a confirmed commit.
pub(crate) fn durable_acceptance() {
    let _ = EVIDENCE.try_with(|evidence| evidence.durable_acceptance.set(true));
}

pub(crate) fn access_ended() {
    let _ = EVIDENCE.try_with(|evidence| evidence.access_ended.set(true));
}

pub(crate) async fn commit(
    transaction: Transaction<'_, Postgres>,
    operation: &'static str,
) -> Result<(), sqlx::Error> {
    super::measure("commit_seconds", operation, transaction.commit()).await?;
    durable_acceptance();
    Ok(())
}

pub(crate) async fn http(
    operation: &'static str,
    work: impl Future<Output = Response>,
) -> Response {
    EVIDENCE
        .scope(Evidence::default(), async move {
            let timer =
                super::Timer::new("http_duration_seconds", operation).with_counter("http_requests");
            let mut response = work.await;
            if matches!(operation, "command" | "recovery") && response.status().is_success() {
                let violation = EVIDENCE.with(|evidence| {
                    if evidence.access_ended.get() {
                        Some("p0_authorization_after_access_end")
                    } else if !evidence.durable_acceptance.get() {
                        Some("p0_ack_before_commit")
                    } else {
                        None
                    }
                });
                if let Some(metric) = violation {
                    super::observe(metric, "integrity", "violation", 1.0);
                    response = ApiError::internal().into_response();
                }
            }
            let outcome = if response.status().is_server_error() {
                "error"
            } else if response.status() == StatusCode::TOO_MANY_REQUESTS {
                "rate_limited"
            } else if response.status().is_client_error() {
                "rejected"
            } else {
                "success"
            };
            timer.finish(outcome);
            response
        })
        .await
}
