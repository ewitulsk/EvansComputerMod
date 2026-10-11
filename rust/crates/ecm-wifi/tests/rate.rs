//! Minstrel behaviour against a synthetic channel: it climbs to the best rate when the
//! channel is clean, steps down when the high rates start failing, and samples its way
//! back up when conditions recover. Also checks the rate table helpers.

use ecm_wifi::rate::{self, Minstrel, Phy, Rate, RATES};

/// Deterministic channel: each rate succeeds with probability `p(rate)` per attempt
/// (up to 4 attempts, like a low MAC with retries).
struct Channel {
    seed: u64,
}

impl Channel {
    fn roll(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.seed >> 33) as f32) / ((1u64 << 31) as f32)
    }
    fn send(&mut self, p: f32) -> (u32, bool) {
        for a in 1..=4 {
            if self.roll() < p {
                return (a, true);
            }
        }
        (4, false)
    }
}

fn run(m: &mut Minstrel, ch: &mut Channel, now: &mut u64, ms: u64, p: impl Fn(&Rate) -> f32) -> Vec<Rate> {
    let mut used = Vec::new();
    let end = *now + ms;
    while *now < end {
        // 2 frames per ms.
        for _ in 0..2 {
            let r = m.tx_rate(*now);
            let (att, ok) = ch.send(p(&r));
            m.tx_status(r, att, ok, *now);
            used.push(r);
        }
        *now += 1;
    }
    used
}

fn g_rates() -> Vec<Rate> {
    rate::usable_rates(6, &rate::our_rates(6), 0)
}

#[test]
fn climbs_to_highest_rate_on_clean_channel() {
    let mut m = Minstrel::new(g_rates(), 42);
    let mut ch = Channel { seed: 1 };
    let mut now = 0;
    run(&mut m, &mut ch, &mut now, 3000, |_| 0.99);
    assert_eq!(m.best_rate().kbps, 54_000);
}

#[test]
fn steps_down_under_losses_and_back_up() {
    let mut m = Minstrel::new(g_rates(), 7);
    let mut ch = Channel { seed: 3 };
    let mut now = 0;
    run(&mut m, &mut ch, &mut now, 3000, |_| 0.99);
    assert_eq!(m.best_rate().kbps, 54_000);

    // Walk away: everything above 18 Mb/s fails, 18 Mb/s is lossy, below is fine.
    let bad = |r: &Rate| match r.kbps {
        k if k > 18_000 => 0.0,
        18_000 => 0.5,
        _ => 0.98,
    };
    let used = run(&mut m, &mut ch, &mut now, 3000, bad);
    let best = m.best_rate().kbps;
    assert!(best <= 18_000 && best >= 9_000, "stepped down to {best}");
    // After the first second most frames go at the new best rate (sampling ≈ 10 %).
    let tail = &used[used.len() / 2..];
    let at_best = tail.iter().filter(|r| r.kbps == best).count() as f32 / tail.len() as f32;
    assert!(at_best > 0.8, "at best {at_best}");
    // Throughput-optimal choice: 12 Mb/s × 0.98 beats 18 Mb/s × ~0.5 per attempt.
    assert_eq!(best, 12_000);

    // Walk back: the channel is clean again; sampling finds the high rates.
    run(&mut m, &mut ch, &mut now, 5000, |_| 0.99);
    assert_eq!(m.best_rate().kbps, 54_000, "back up");
}

#[test]
fn falls_back_when_every_tried_rate_fails() {
    let mut m = Minstrel::new(g_rates(), 9);
    let mut ch = Channel { seed: 5 };
    let mut now = 0;
    // From the start, only 1-2 Mb/s work.
    run(&mut m, &mut ch, &mut now, 4000, |r| if r.kbps <= 2000 { 0.95 } else { 0.0 });
    assert!(m.best_rate().kbps <= 2000, "best {}", m.best_rate().kbps);
}

#[test]
fn ht_rates_used_when_peer_supports_them() {
    let rates = rate::usable_rates(1, &rate::our_rates(1), 0xff);
    assert!(rates.iter().any(|r| r.phy == Phy::Ht));
    assert_eq!(rates.last().unwrap().kbps, 65_000);
    let mut m = Minstrel::new(rates, 11);
    let mut ch = Channel { seed: 8 };
    let mut now = 0;
    run(&mut m, &mut ch, &mut now, 3000, |_| 0.99);
    assert_eq!(m.best_rate().mcs(), Some(7));
    assert_eq!(m.best_rate().describe(), "65.0 MBit/s MCS 7");
}

#[test]
fn rate_table_helpers() {
    assert_eq!(rate::basic_rate(1).kbps, 1000);
    assert_eq!(rate::basic_rate(36).kbps, 6000);
    assert_eq!(Rate::from_code(108).unwrap().kbps, 54_000);
    assert_eq!(Rate::from_code(0x84).unwrap().mcs(), Some(4));
    assert!(Rate::from_code(3).is_none());
    // 5 GHz has no DSSS.
    let a = rate::usable_rates(36, &[0x8c, 0x98, 0xb0, 108], 0);
    assert!(a.iter().all(|r| r.phy == Phy::Ofdm));
    assert_eq!(a.len(), 4);
    // Codes are unique.
    for (i, r) in RATES.iter().enumerate() {
        assert!(RATES[i + 1..].iter().all(|o| o.code != r.code));
    }
}
