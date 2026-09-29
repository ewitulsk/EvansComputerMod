//! Small shared helpers: a seeded PRNG, duration parsing, formatting.

/// SplitMix64: tiny, fast, seedable, good enough for fault injection and
/// simulated entropy. NOT cryptographically secure (the simulator hands
/// deterministic "entropy" to guests on purpose so runs are reproducible).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    /// Derive an independent stream from a seed and a label.
    pub fn derive(seed: u64, label: &str) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in label.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        Rng::new(seed ^ h.rotate_left(17))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }

    /// True with probability `pct` percent.
    pub fn chance(&mut self, pct: f64) -> bool {
        if pct <= 0.0 {
            return false;
        }
        if pct >= 100.0 {
            return true;
        }
        let x = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        x * 100.0 < pct
    }
}

/// Parse "10s", "250ms", "2m", "1.5s", or a bare number of milliseconds.
pub fn parse_duration_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let (num, mult) = if let Some(n) = s.strip_suffix("ms") {
        (n, 1.0)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1000.0)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60_000.0)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, 3_600_000.0)
    } else {
        (s, 1.0)
    };
    let v: f64 = num.trim().parse().ok()?;
    if !v.is_finite() || v < 0.0 {
        return None;
    }
    Some((v * mult).round() as i64)
}

pub fn fmt_ms(ms: i64) -> String {
    if ms % 1000 == 0 {
        format!("{}s", ms / 1000)
    } else {
        format!("{:.3}s", ms as f64 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration_ms("10s"), Some(10_000));
        assert_eq!(parse_duration_ms("250ms"), Some(250));
        assert_eq!(parse_duration_ms("1.5s"), Some(1500));
        assert_eq!(parse_duration_ms("2m"), Some(120_000));
        assert_eq!(parse_duration_ms("42"), Some(42));
        assert_eq!(parse_duration_ms("x"), None);
        assert_eq!(parse_duration_ms("-1s"), None);
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::derive(7, "x");
        let mut b = Rng::derive(7, "x");
        let mut c = Rng::derive(7, "y");
        let va: Vec<u64> = (0..4).map(|_| a.next_u64()).collect();
        let vb: Vec<u64> = (0..4).map(|_| b.next_u64()).collect();
        let vc: Vec<u64> = (0..4).map(|_| c.next_u64()).collect();
        assert_eq!(va, vb);
        assert_ne!(va, vc);
        let mut r = Rng::new(1);
        let hits = (0..10_000).filter(|_| r.chance(25.0)).count();
        assert!((2000..3000).contains(&hits));
    }
}
