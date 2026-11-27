//! In-memory rate limits: at most `limit` hits per key in a sliding window.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A limit: `count` hits per `window`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limit {
    pub count: u32,
    pub window: Duration,
}

impl Limit {
    pub const fn per_minute(count: u32) -> Limit {
        Limit { count, window: Duration::from_secs(60) }
    }
    pub const fn per_hour(count: u32) -> Limit {
        Limit { count, window: Duration::from_secs(3600) }
    }
}

const SWEEP_AT: usize = 20_000;
const LONGEST_WINDOW: Duration = Duration::from_secs(3600);

#[derive(Default)]
pub struct RateLimiter {
    hits: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new() -> RateLimiter {
        RateLimiter::default()
    }

    /// Count one hit for `key`. False when the key is over its limit (the hit is not counted).
    pub fn hit(&self, key: &str, limit: Limit) -> bool {
        self.hit_at(key, limit, Instant::now())
    }

    fn hit_at(&self, key: &str, limit: Limit, now: Instant) -> bool {
        let mut map = self.hits.lock().unwrap();
        if map.len() >= SWEEP_AT {
            map.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) < LONGEST_WINDOW));
        }
        let q = map.entry(key.to_string()).or_default();
        while q.front().is_some_and(|t| now.duration_since(*t) >= limit.window) {
            q.pop_front();
        }
        if q.len() as u32 >= limit.count {
            return false;
        }
        q.push_back(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_per_key_and_recovers() {
        let r = RateLimiter::new();
        let l = Limit { count: 3, window: Duration::from_secs(60) };
        let t0 = Instant::now();
        for _ in 0..3 {
            assert!(r.hit_at("a", l, t0));
        }
        assert!(!r.hit_at("a", l, t0 + Duration::from_secs(59)));
        assert!(r.hit_at("b", l, t0));
        assert!(r.hit_at("a", l, t0 + Duration::from_secs(60)));
    }
}
