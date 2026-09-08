use axum::{Router, http::StatusCode, routing::post};
use serde::Deserialize;

use crate::{
    AppState,
    http_support::{ApiError, StrictJson},
};

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/api/telemetry", post(record))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientObservation {
    metric: ClientMetric,
    value: f64,
    outcome: ClientOutcome,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClientMetric {
    WebLcpSeconds,
    WebInpSeconds,
    WebClsRatio,
    RecoveryJourneys,
    RecoveryHumanSeconds,
    ReconnectAttempts,
    ReplaySeconds,
    SnapshotSeconds,
    LongTaskSeconds,
    TouchFeedbackSeconds,
}

impl ClientMetric {
    const fn name(&self) -> &'static str {
        match self {
            Self::WebLcpSeconds => "web_lcp_seconds",
            Self::WebInpSeconds => "web_inp_seconds",
            Self::WebClsRatio => "web_cls_ratio",
            Self::RecoveryJourneys => "recovery_journeys",
            Self::RecoveryHumanSeconds => "recovery_human_seconds",
            Self::ReconnectAttempts => "reconnect_attempts",
            Self::ReplaySeconds => "replay_seconds",
            Self::SnapshotSeconds => "snapshot_seconds",
            Self::LongTaskSeconds => "long_task_seconds",
            Self::TouchFeedbackSeconds => "touch_feedback_seconds",
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClientOutcome {
    Success,
    Error,
    Abandoned,
    Started,
}

impl ClientOutcome {
    const fn name(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Abandoned => "abandoned",
            Self::Started => "started",
        }
    }
}

async fn record(
    StrictJson(observation): StrictJson<ClientObservation>,
) -> Result<StatusCode, ApiError> {
    if !observation.value.is_finite()
        || observation.value < 0.0
        || (matches!(
            observation.metric,
            ClientMetric::RecoveryJourneys | ClientMetric::ReconnectAttempts
        ) && observation.value.to_bits() != 1.0_f64.to_bits())
    {
        return Err(ApiError::invalid_request(StatusCode::UNPROCESSABLE_ENTITY));
    }
    // RUM is untrusted, anonymous and isolated from authoritative P0 signals.
    // Saturating an extreme duration preserves every SLO breach without allowing
    // an unbounded numeric payload to become an observation or a label.
    super::observe(
        observation.metric.name(),
        "browser",
        observation.outcome.name(),
        observation.value.min(86_400.0),
    );
    Ok(StatusCode::NO_CONTENT)
}
