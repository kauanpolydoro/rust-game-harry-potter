use std::time::Duration;

use crate::AppState;

/// Samples the running process independently of user traffic until shutdown.
/// Process memory is available on Linux; unavailable measurements are counted.
pub async fn observe_runtime(state: AppState) {
    let mut shutdown = state.subscribe_to_shutdown();
    let mut interval = tokio::time::interval(Duration::from_secs(10));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return; }
            }
            scheduled = interval.tick() => {
                super::observe("server_heartbeat", "runtime", "success", 1.0);
                super::observe("executor_lag_seconds", "runtime", "success", tokio::time::Instant::now().saturating_duration_since(scheduled).as_secs_f64());
                super::observe("pool_connections", "runtime", "success", f64::from(state.database.size()));
                let idle = u32::try_from(state.database.num_idle()).unwrap_or(u32::MAX);
                super::observe("pool_idle_connections", "runtime", "success", f64::from(idle));
                match tokio::task::spawn_blocking(resident_bytes).await {
                    Ok(Some(bytes)) => super::observe("process_resident_bytes", "runtime", "success", bytes),
                    _ => super::observe("runtime_measurements", "runtime", "unavailable", 1.0),
                }
            }
        }
    }
}

fn resident_bytes() -> Option<f64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?;
    let mut fields = line.split_whitespace();
    let kibibytes = fields.next()?.parse::<f64>().ok()?;
    if fields.next()? != "kB" || !kibibytes.is_finite() || kibibytes < 0.0 {
        return None;
    }
    Some(kibibytes * 1024.0)
}
