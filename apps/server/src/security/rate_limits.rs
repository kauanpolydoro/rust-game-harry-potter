use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(60);
const MAX_BUCKETS: usize = 8192;

#[derive(Default)]
pub(crate) struct RateLimits(Mutex<HashMap<[u8; 32], Bucket>>);

struct Bucket {
    started: Instant,
    used: u32,
}

impl RateLimits {
    pub(crate) fn admit(&self, key: [u8; 32], limit: u32) -> bool {
        let now = Instant::now();
        let mut buckets = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        buckets.retain(|_, bucket| now.duration_since(bucket.started) < WINDOW);
        if !buckets.contains_key(&key) && buckets.len() >= MAX_BUCKETS {
            return false;
        }
        let bucket = buckets.entry(key).or_insert(Bucket {
            started: now,
            used: 0,
        });
        if bucket.used >= limit {
            return false;
        }
        bucket.used += 1;
        true
    }
}
