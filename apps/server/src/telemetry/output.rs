use std::{
    fmt,
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{Layer, fmt::MakeWriter, layer::Context, registry::LookupSpan};

pub(super) struct SafeOutput<W>(pub W);

impl<S, W> Layer<S> for SafeOutput<W>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    fn on_event(&self, event: &Event<'_>, context: Context<'_, S>) {
        let mut fields = SafeFields::default();
        event.record(&mut fields);
        if let Some(scope) = context.event_scope(event) {
            for span in scope {
                if span
                    .metadata()
                    .fields()
                    .iter()
                    .any(|field| !matches!(field.name(), "correlation_id" | "method" | "route"))
                {
                    fields.rejected = true;
                }
            }
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let mut document = if fields.rejected {
            // Do not include the rejected value, message, field name or span.
            json!({"metric":"p0_secret_detected", "operation":"telemetry", "outcome":"violation", "value":1.0})
        } else {
            Value::Object(fields.values)
        };
        document["timestamp_ms"] = json!(timestamp);
        document["environment"] = json!(environment());
        if let Ok(id) = crate::REQUEST_CORRELATION_ID.try_with(ToString::to_string) {
            document["correlation_id"] = json!(id);
        }
        if let Ok(trace) = super::COMMAND_TRACE.try_with(ToString::to_string) {
            document["command_trace"] = json!(trace);
        }
        if let Some(metric) = document["metric"].as_str().map(str::to_owned) {
            let unit = if metric.ends_with("_seconds") {
                "Seconds"
            } else if metric.ends_with("_bytes") {
                "Bytes"
            } else if metric.ends_with("_ratio") {
                "None"
            } else {
                "Count"
            };
            document[&metric] = document["value"].clone();
            document["_aws"] = json!({
                "Timestamp": timestamp,
                "CloudWatchMetrics": [{
                    "Namespace": "Hogwarts",
                    "Dimensions": [["environment", "operation"], ["environment", "operation", "outcome"]],
                    "Metrics": [{"Name": metric, "Unit": unit}]
                }]
            });
        } else {
            document["level"] = json!(event.metadata().level().as_str());
        }
        if let Ok(mut bytes) = serde_json::to_vec(&document) {
            bytes.push(b'\n');
            // Failure is not retried into another unbounded or unsanitized sink.
            let _ = self.0.make_writer_for(event.metadata()).write_all(&bytes);
        }
    }
}

#[derive(Default)]
struct SafeFields {
    values: Map<String, Value>,
    rejected: bool,
}

impl SafeFields {
    fn string(&mut self, name: &str, value: &str) {
        let allowed = match name {
            "message" => value == "observation" || super::allowlist::MESSAGES.contains(&value),
            "operation" => {
                super::allowlist::OPERATION_LABELS.contains(&value)
                    || super::allowlist::OPERATIONS.contains(&value)
            }
            "outcome" => super::allowlist::OUTCOMES.contains(&value),
            "metric" => super::allowlist::METRICS.contains(&value),
            "method" => matches!(
                value,
                "GET" | "HEAD" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" | "OTHER"
            ),
            "route" => super::allowlist::ROUTES.contains(&value),
            "address" | "reason" => return,
            _ => false,
        };
        if allowed {
            self.values.insert(name.to_owned(), json!(value));
        } else {
            self.rejected = true;
        }
    }
}

impl Visit for SafeFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.string(field.name(), value);
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.string(field.name(), &format!("{value:?}"));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "value" && value.is_finite() && value >= 0.0 {
            self.values.insert("value".to_owned(), json!(value));
        } else {
            self.rejected = true;
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        if matches!(field.name(), "status" | "code") && value <= 4999 {
            self.values.insert(field.name().to_owned(), json!(value));
        } else {
            self.rejected = true;
        }
    }
}

fn environment() -> &'static str {
    static ENVIRONMENT: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    ENVIRONMENT.get_or_init(|| match std::env::var("TELEMETRY_ENVIRONMENT").as_deref() {
        Ok("production") => "production",
        Ok("staging") => "staging",
        Ok("development") | Err(_) => "development",
        Ok(_) => "unconfigured",
    })
}
