//! Small in-memory rate limiters keyed by an arbitrary string (a peer IP).

use std::{collections::HashMap, sync::Mutex, time::Instant};

/// Above this many tracked keys, idle entries are dropped on the next check.
const PRUNE_ABOVE: usize = 10_000;

/// Token bucket: `capacity` requests may burst, then one more every
/// `1 / refill_per_sec` seconds.
pub struct RateLimiter {
    capacity: f64,
    refill_per_sec: f64,
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl RateLimiter {
    pub fn new(capacity: u32, refill_per_sec: f64) -> Self {
        Self {
            capacity: f64::from(capacity),
            refill_per_sec,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Takes a token for `key`. On refusal returns the seconds to wait.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        self.check_at(key, Instant::now())
    }

    fn check_at(&self, key: &str, now: Instant) -> Result<(), u64> {
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if buckets.len() > PRUNE_ABOVE {
            let (capacity, rate) = (self.capacity, self.refill_per_sec);
            buckets.retain(|_, (tokens, at)| {
                *tokens + now.duration_since(*at).as_secs_f64() * rate < capacity
            });
        }
        let (tokens, at) = buckets
            .entry(key.to_string())
            .or_insert((self.capacity, now));
        let elapsed = now.duration_since(*at).as_secs_f64();
        *tokens = (*tokens + elapsed * self.refill_per_sec).min(self.capacity);
        *at = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - *tokens) / self.refill_per_sec).ceil().max(1.0) as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bucket_bursts_then_refuses_then_refills() {
        let limiter = RateLimiter::new(3, 0.5);
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(limiter.check_at("a", t0).is_ok());
        }
        assert_eq!(limiter.check_at("a", t0), Err(2));
        assert!(limiter.check_at("b", t0).is_ok(), "keys are independent");
        assert!(limiter.check_at("a", t0 + Duration::from_secs(2)).is_ok());
        assert!(limiter.check_at("a", t0 + Duration::from_secs(2)).is_err());
    }
}
