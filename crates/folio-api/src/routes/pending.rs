//! Short-lived server-side state for multi-step logins and ceremonies.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Hard cap so an unauthenticated client cannot grow the map without bound.
const MAX_ENTRIES: usize = 10_000;

/// Values that expire, keyed by the hash of a browser-bound token.
pub struct Pending<T> {
    ttl: Duration,
    entries: Mutex<HashMap<String, (T, Instant)>>,
}

impl<T> Pending<T> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (T, Instant)>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stores `value`. Returns false when the map is full of live entries.
    pub fn insert(&self, key: String, value: T) -> bool {
        let now = Instant::now();
        let mut entries = self.lock();
        if entries.len() >= MAX_ENTRIES {
            entries.retain(|_, (_, deadline)| *deadline > now);
            if entries.len() >= MAX_ENTRIES {
                return false;
            }
        }
        entries.insert(key, (value, now + self.ttl));
        true
    }

    /// Removes and returns the value if it has not expired.
    pub fn take(&self, key: &str) -> Option<T> {
        let (value, deadline) = self.lock().remove(key)?;
        (deadline > Instant::now()).then_some(value)
    }

    /// Puts a value taken with [`Self::take_with_remaining`] back, keeping
    /// its original deadline, so failed attempts do not extend its life.
    pub fn restore(&self, key: String, value: T, remaining: Duration) {
        self.lock().insert(key, (value, Instant::now() + remaining));
    }

    /// Reads a live value without removing it.
    pub fn peek<R>(&self, key: &str, read: impl FnOnce(&T) -> R) -> Option<R> {
        let entries = self.lock();
        let (value, deadline) = entries.get(key)?;
        (*deadline > Instant::now()).then(|| read(value))
    }

    /// Like [`Self::take`] but also reports how long the entry had left.
    pub fn take_with_remaining(&self, key: &str) -> Option<(T, Duration)> {
        let (value, deadline) = self.lock().remove(key)?;
        let remaining = deadline.checked_duration_since(Instant::now())?;
        Some((value, remaining))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_taken_once_and_expire() {
        let p = Pending::new(Duration::from_millis(30));
        assert!(p.insert("a".into(), 1));
        assert_eq!(p.take("a"), Some(1));
        assert_eq!(p.take("a"), None);
        assert!(p.insert("b".into(), 2));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(p.take("b"), None);
    }
}
