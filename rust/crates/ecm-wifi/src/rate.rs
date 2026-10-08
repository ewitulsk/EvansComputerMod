//! 802.11b/g/n rate table and a minstrel-style rate controller.
//!
//! Rates are exchanged with the SoftMAC as one-byte *rate codes*: legacy rates use the
//! Supported Rates encoding (units of 500 kb/s, e.g. 2 = 1 Mb/s, 108 = 54 Mb/s) and HT
//! rates are `0x80 | MCS` (20 MHz, long GI, one stream).
//!
//! Minstrel keeps, per rate, the attempts/successes of the current statistics interval,
//! folds them into an EWMA success probability every interval, and transmits at the rate
//! with the best expected throughput (probability × bitrate). A fraction of frames are
//! *sample* frames sent at another rate so that the controller notices when conditions
//! improve. Randomness comes from a small seeded generator so behaviour is reproducible.

/// Modulation family.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phy {
    /// 802.11b DSSS/CCK.
    Dsss,
    /// 802.11a/g OFDM.
    Ofdm,
    /// 802.11n HT (one spatial stream, 20 MHz, long GI).
    Ht,
}

/// One entry of the rate table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    pub phy: Phy,
    /// Rate code on the host ABI (see module docs).
    pub code: u8,
    /// Bit rate in kb/s.
    pub kbps: u32,
}

impl Rate {
    pub fn mbps(&self) -> f32 {
        self.kbps as f32 / 1000.0
    }

    /// HT MCS index, if HT.
    pub fn mcs(&self) -> Option<u8> {
        if self.phy == Phy::Ht {
            Some(self.code & 0x7f)
        } else {
            None
        }
    }

    /// Display like `iw`: "54.0 MBit/s" or "65.0 MBit/s MCS 7".
    pub fn describe(&self) -> String {
        match self.mcs() {
            Some(m) => format!("{:.1} MBit/s MCS {}", self.mbps(), m),
            None => format!("{:.1} MBit/s", self.mbps()),
        }
    }

    pub fn from_code(code: u8) -> Option<Rate> {
        RATES.iter().copied().find(|r| r.code == code)
    }
}

const fn r(phy: Phy, code: u8, kbps: u32) -> Rate {
    Rate { phy, code, kbps }
}

/// All rates this stack can use, in ascending bit-rate order within each family.
pub const RATES: [Rate; 20] = [
    r(Phy::Dsss, 2, 1000),
    r(Phy::Dsss, 4, 2000),
    r(Phy::Dsss, 11, 5500),
    r(Phy::Dsss, 22, 11000),
    r(Phy::Ofdm, 12, 6000),
    r(Phy::Ofdm, 18, 9000),
    r(Phy::Ofdm, 24, 12000),
    r(Phy::Ofdm, 36, 18000),
    r(Phy::Ofdm, 48, 24000),
    r(Phy::Ofdm, 72, 36000),
    r(Phy::Ofdm, 96, 48000),
    r(Phy::Ofdm, 108, 54000),
    r(Phy::Ht, 0x80, 6500),
    r(Phy::Ht, 0x81, 13000),
    r(Phy::Ht, 0x82, 19500),
    r(Phy::Ht, 0x83, 26000),
    r(Phy::Ht, 0x84, 39000),
    r(Phy::Ht, 0x85, 52000),
    r(Phy::Ht, 0x86, 58500),
    r(Phy::Ht, 0x87, 65000),
];

/// Lowest mandatory rate for a channel: 1 Mb/s on 2.4 GHz, 6 Mb/s on 5 GHz.
pub fn basic_rate(channel: u8) -> Rate {
    if channel <= 14 {
        RATES[0]
    } else {
        RATES[4]
    }
}

/// Our supported-rates list for the band (Supported Rates IE encoding, basic bit on the
/// mandatory rates).
pub fn our_rates(channel: u8) -> Vec<u8> {
    if channel <= 14 {
        vec![0x82, 0x84, 0x8b, 0x96, 12, 18, 24, 36, 48, 72, 96, 108]
    } else {
        vec![0x8c, 18, 0x98, 36, 0xb0, 72, 96, 108]
    }
}

/// The rates usable with a peer: intersection of the peer's legacy rates (basic bit
/// ignored) with ours for the band, plus HT MCS present in the peer's mask.
pub fn usable_rates(channel: u8, peer_rates: &[u8], peer_ht_mcs: u8) -> Vec<Rate> {
    let ours: Vec<u8> = our_rates(channel).iter().map(|x| x & 0x7f).collect();
    let mut v: Vec<Rate> = RATES
        .iter()
        .copied()
        .filter(|rt| match rt.phy {
            Phy::Ht => peer_ht_mcs & (1 << (rt.code & 7)) != 0,
            _ => ours.contains(&rt.code) && peer_rates.iter().any(|p| p & 0x7f == rt.code),
        })
        .collect();
    if v.is_empty() {
        v.push(basic_rate(channel));
    }
    v.sort_by_key(|x| x.kbps);
    v
}

/// Per-rate statistics.
#[derive(Clone, Debug)]
pub struct RateStats {
    pub rate: Rate,
    /// Attempts / successes in the current interval.
    pub attempts: u32,
    pub success: u32,
    /// EWMA success probability (0..=1).
    pub prob: f32,
    /// Expected throughput in kb/s (prob × bitrate, 0 when prob < 10 %).
    pub tp: f32,
    pub total_attempts: u64,
    pub total_success: u64,
    /// Intervals since this rate was last tried (for sampling freshness).
    pub stale: u32,
}

/// Minstrel parameters.
#[derive(Clone, Debug)]
pub struct MinstrelConfig {
    /// Statistics interval in ms.
    pub update_interval_ms: u64,
    /// EWMA weight of the old value in percent.
    pub ewma_level: u32,
    /// Percentage of frames used for sampling.
    pub sample_percent: u32,
}

impl Default for MinstrelConfig {
    fn default() -> Self {
        MinstrelConfig { update_interval_ms: 100, ewma_level: 75, sample_percent: 10 }
    }
}

/// Minstrel rate controller for one peer.
#[derive(Clone, Debug)]
pub struct Minstrel {
    cfg: MinstrelConfig,
    stats: Vec<RateStats>,
    max_tp: usize,
    max_prob: usize,
    last_update: u64,
    rng: u64,
    frames: u64,
    /// Index of the rate a sample frame is in flight on, if any.
    pub last_sample: Option<usize>,
}

impl Minstrel {
    pub fn new(rates: Vec<Rate>, seed: u64) -> Self {
        Self::with_config(rates, seed, MinstrelConfig::default())
    }

    pub fn with_config(mut rates: Vec<Rate>, seed: u64, cfg: MinstrelConfig) -> Self {
        if rates.is_empty() {
            rates.push(RATES[0]);
        }
        rates.sort_by_key(|x| x.kbps);
        let stats: Vec<RateStats> = rates
            .into_iter()
            .map(|rate| RateStats {
                rate,
                attempts: 0,
                success: 0,
                prob: 0.0,
                tp: 0.0,
                total_attempts: 0,
                total_success: 0,
                stale: 0,
            })
            .collect();
        // Start in the middle of the table, as minstrel does before it has statistics.
        let start = stats.len() / 2;
        Minstrel {
            cfg,
            stats,
            max_tp: start,
            max_prob: 0,
            last_update: 0,
            rng: seed | 1,
            frames: 0,
            last_sample: None,
        }
    }

    fn rand(&mut self) -> u32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 32) as u32
    }

    pub fn stats(&self) -> &[RateStats] {
        &self.stats
    }

    /// The current best-throughput rate.
    pub fn best_rate(&self) -> Rate {
        self.stats[self.max_tp].rate
    }

    /// The most reliable rate (used for retries by a low MAC that supports chains).
    pub fn reliable_rate(&self) -> Rate {
        self.stats[self.max_prob].rate
    }

    /// Choose the rate for the next data frame.
    pub fn tx_rate(&mut self, now: u64) -> Rate {
        self.maybe_update(now);
        self.frames += 1;
        self.last_sample = None;
        if self.stats.len() > 1 && self.rand() % 100 < self.cfg.sample_percent {
            // Sample a rate other than the current best. Prefer faster rates (finding an
            // improvement) and stale ones; never sample rates far below the best, which
            // cannot raise throughput.
            let n = self.stats.len();
            let pick = (self.rand() as usize) % n;
            let cur = &self.stats[self.max_tp];
            let floor = if cur.prob >= 0.5 { cur.rate.kbps / 2 } else { 0 };
            if pick != self.max_tp && self.stats[pick].rate.kbps > floor {
                self.last_sample = Some(pick);
                return self.stats[pick].rate;
            }
        }
        self.stats[self.max_tp].rate
    }

    /// Report the outcome of one transmitted frame: `attempts` transmissions on `rate`
    /// (1 + retries), and whether it was finally acknowledged.
    pub fn tx_status(&mut self, rate: Rate, attempts: u32, acked: bool, now: u64) {
        if let Some(s) = self.stats.iter_mut().find(|s| s.rate.code == rate.code) {
            let attempts = attempts.max(1);
            s.attempts += attempts;
            s.success += if acked { 1 } else { 0 };
            s.total_attempts += attempts as u64;
            s.total_success += if acked { 1 } else { 0 };
        }
        self.maybe_update(now);
    }

    fn maybe_update(&mut self, now: u64) {
        if now >= self.last_update + self.cfg.update_interval_ms {
            self.update_stats();
            self.last_update = now;
        }
    }

    /// Fold the interval counters into the EWMA and pick the new best rates.
    pub fn update_stats(&mut self) {
        let w = self.cfg.ewma_level as f32 / 100.0;
        for s in self.stats.iter_mut() {
            if s.attempts > 0 {
                let p = s.success as f32 / s.attempts as f32;
                s.prob = if s.total_attempts == s.attempts as u64 { p } else { s.prob * w + p * (1.0 - w) };
                s.stale = 0;
            } else {
                s.stale = s.stale.saturating_add(1);
            }
            s.attempts = 0;
            s.success = 0;
            s.tp = if s.prob < 0.1 { 0.0 } else { s.prob * s.rate.kbps as f32 };
        }
        let mut best = 0usize;
        let mut best_prob = 0usize;
        let mut any = false;
        for (i, s) in self.stats.iter().enumerate() {
            if s.total_attempts == 0 {
                continue;
            }
            if !any || s.tp > self.stats[best].tp {
                best = i;
            }
            if !any || s.prob > self.stats[best_prob].prob
                || (s.prob == self.stats[best_prob].prob && s.rate.kbps > self.stats[best_prob].rate.kbps)
            {
                best_prob = i;
            }
            any = true;
        }
        if any {
            if self.stats[best].tp == 0.0 {
                // Nothing tried works: step down one rate from the current one, which
                // walks the table towards the robust rates even without samples.
                best = self.max_tp.saturating_sub(1);
            }
            self.max_tp = best;
            self.max_prob = best_prob;
        }
    }
}
