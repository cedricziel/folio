//! Small in-memory rate limiters keyed by an arbitrary string (a peer IP).

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

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

struct Failures {
    count: u32,
    last: Instant,
    locked_until: Instant,
}

/// Counts failed attempts per key and locks the key out with exponentially
/// growing delays once `free_attempts` failures have happened.
pub struct FailureLimiter {
    free_attempts: u32,
    base: Duration,
    cap: Duration,
    /// A key that stays quiet this long starts over.
    forget_after: Duration,
    entries: Mutex<HashMap<String, Failures>>,
}

impl FailureLimiter {
    pub fn new(free_attempts: u32, base: Duration, cap: Duration, forget_after: Duration) -> Self {
        Self {
            free_attempts,
            base,
            cap,
            forget_after,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// `Err(seconds)` while the key is locked out.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        self.check_at(key, Instant::now())
    }

    pub fn record_failure(&self, key: &str) {
        self.record_failure_at(key, Instant::now());
    }

    pub fn record_success(&self, key: &str) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    fn check_at(&self, key: &str, now: Instant) -> Result<(), u64> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match entries.get(key) {
            Some(f) if f.locked_until > now => {
                Err((f.locked_until - now).as_secs_f64().ceil().max(1.0) as u64)
            }
            _ => Ok(()),
        }
    }

    fn record_failure_at(&self, key: &str, now: Instant) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if entries.len() > PRUNE_ABOVE {
            let forget = self.forget_after;
            entries.retain(|_, f| now.duration_since(f.last) < forget);
        }
        let f = entries.entry(key.to_string()).or_insert(Failures {
            count: 0,
            last: now,
            locked_until: now,
        });
        if now.duration_since(f.last) >= self.forget_after {
            f.count = 0;
        }
        f.count += 1;
        f.last = now;
        if f.count >= self.free_attempts {
            let doublings = (f.count - self.free_attempts).min(16);
            f.locked_until = now + self.base.saturating_mul(1 << doublings).min(self.cap);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_lock_out_with_growing_delay_and_reset_on_success() {
        let limiter = FailureLimiter::new(
            3,
            Duration::from_secs(10),
            Duration::from_secs(60),
            Duration::from_secs(3600),
        );
        let t0 = Instant::now();
        for _ in 0..2 {
            limiter.record_failure_at("u", t0);
        }
        assert!(limiter.check_at("u", t0).is_ok());
        limiter.record_failure_at("u", t0);
        assert_eq!(limiter.check_at("u", t0), Err(10));
        assert!(limiter.check_at("u", t0 + Duration::from_secs(10)).is_ok());
        limiter.record_failure_at("u", t0 + Duration::from_secs(10));
        assert_eq!(limiter.check_at("u", t0 + Duration::from_secs(10)), Err(20));
        for _ in 0..10 {
            limiter.record_failure_at("u", t0 + Duration::from_secs(10));
        }
        assert_eq!(
            limiter.check_at("u", t0 + Duration::from_secs(10)),
            Err(60),
            "delay is capped"
        );
        limiter.record_success("u");
        assert!(limiter.check_at("u", t0).is_ok());
        assert!(limiter.check_at("other", t0).is_ok());
    }

    #[test]
    fn quiet_keys_are_forgotten() {
        let limiter = FailureLimiter::new(
            1,
            Duration::from_secs(10),
            Duration::from_secs(60),
            Duration::from_secs(100),
        );
        let t0 = Instant::now();
        limiter.record_failure_at("u", t0);
        let later = t0 + Duration::from_secs(200);
        limiter.record_failure_at("u", later);
        assert_eq!(
            limiter.check_at("u", later),
            Err(10),
            "count restarted at 1"
        );
    }

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
