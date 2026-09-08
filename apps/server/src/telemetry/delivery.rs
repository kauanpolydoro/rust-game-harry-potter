use std::time::Instant;

use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct DeliveryEvidence {
    pub(crate) commit_age: Option<f64>,
    pub(crate) command_trace: Option<Uuid>,
}

impl DeliveryEvidence {
    fn observe(&self, metric: &'static str, outcome: &'static str, value: f64) {
        let emit = || super::observe(metric, "event_delivery", outcome, value);
        if let Some(trace) = self.command_trace {
            super::COMMAND_TRACE.sync_scope(trace, emit);
        } else {
            emit();
        }
    }
}

/// One bounded, ephemeral probe per connection. No game or session identifiers
/// reach logs. A matching Pong proves transport receipt, not JavaScript rendering.
pub(crate) struct ReceiptProbe {
    nonce: Uuid,
    measured: Instant,
    sent: Instant,
    commit_ages: Vec<DeliveryEvidence>,
    outcome: &'static str,
}

impl ReceiptProbe {
    pub(crate) fn new(measured: Instant, commit_ages: Vec<DeliveryEvidence>) -> Self {
        for age in &commit_ages {
            age.observe(
                "commit_time_observations",
                if age.commit_age.is_some() {
                    "success"
                } else {
                    "unavailable"
                },
                1.0,
            );
            age.observe("delivery_events", "started", 1.0);
        }
        Self {
            nonce: Uuid::new_v4(),
            measured,
            sent: Instant::now(),
            commit_ages,
            outcome: "cancelled",
        }
    }

    pub(crate) fn finish(mut self, outcome: &'static str) {
        self.outcome = outcome;
    }

    pub(crate) fn nonce(&self) -> &[u8] {
        self.nonce.as_bytes()
    }

    pub(crate) fn acknowledge(&mut self, payload: &[u8]) -> bool {
        if payload != self.nonce.as_bytes() {
            return false;
        }
        if self.commit_ages.is_empty() {
            return true;
        }
        self.outcome = "success";
        super::observe(
            "delivery_round_trip_seconds",
            "event_delivery",
            "success",
            self.sent.elapsed().as_secs_f64(),
        );
        for evidence in &self.commit_ages {
            let Some(age) = evidence.commit_age else {
                continue;
            };
            // DB-clock commit age plus the entire local query/send/receipt interval
            // deliberately overestimates receipt latency by query and return transit.
            evidence.observe(
                "commit_to_receipt_upper_bound_seconds",
                "success",
                age + self.measured.elapsed().as_secs_f64(),
            );
        }
        true
    }

    pub(crate) fn deadline(&self) -> Option<tokio::time::Instant> {
        (!self.commit_ages.is_empty())
            .then(|| tokio::time::Instant::from_std(self.sent) + std::time::Duration::from_secs(5))
    }

    pub(crate) fn expire(&mut self) {
        if self.sent.elapsed().as_secs() < 5 || self.commit_ages.is_empty() {
            return;
        }
        self.outcome = "timeout";
        self.record_outcome();
        // Keep only the nonce until its late Pong or connection close. Replacing
        // it would turn a solicited response into an unsolicited rate-limit hit.
    }

    fn record_outcome(&mut self) {
        for evidence in self.commit_ages.drain(..) {
            evidence.observe("delivery_events", self.outcome, 1.0);
        }
    }
}

impl Drop for ReceiptProbe {
    fn drop(&mut self) {
        self.record_outcome();
    }
}
