//! The speaker side of a call.
//!
//! * [`JitterBuffer`]: one RTP stream (a VoLTE call). Frames go in by sequence number
//!   with their RTP timestamp and arrival time; 16 kHz samples come out with
//!   [`JitterBuffer::pull`]. It reorders, drops packets that arrive after their turn
//!   (late), and conceals one lost frame by repeating the last one fading out (more in
//!   a row are silence; running dry is silence too, and primes again). The playout delay adapts between 40 and 160 ms: it is set each
//!   time playout (re)starts from the measured interarrival jitter (RFC 3550 style,
//!   three times the jitter) plus a boost after late packets, and a buffer that grows
//!   past the maximum is trimmed back.
//! * [`Mixer`]: many talkers at once (a Discord voice channel), from Discord: a small
//!   queue per talker that primes before it is heard, all mixed into one stream.

use std::collections::{BTreeMap, VecDeque};

use crate::{FRAME_SAMPLES, RATE};

/// Lowest playout delay, ms.
pub const MIN_DELAY_MS: u32 = 40;
/// Highest playout delay, ms.
pub const MAX_DELAY_MS: u32 = 160;
/// Added to the playout delay for each late packet, ms (it decays at each restart).
pub const LATE_BOOST_MS: u32 = 20;

/// What happened to the packets a [`JitterBuffer`] was given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JitterStats {
    /// Packets accepted.
    pub received: u64,
    /// Sequence numbers never received by the time they were due.
    pub lost: u64,
    /// Packets that arrived after their turn (dropped).
    pub late: u64,
    /// Duplicates (dropped).
    pub duplicates: u64,
    /// Frames played by concealment (a faded repeat).
    pub concealed: u64,
    /// Frames dropped because the buffer had grown past the maximum delay.
    pub overflow: u64,
}

/// An adaptive jitter buffer for one RTP stream, 16 kHz out.
#[derive(Clone, Debug)]
pub struct JitterBuffer {
    clock_rate: u32,
    pub min_delay_ms: u32,
    pub max_delay_ms: u32,
    frames: BTreeMap<u64, Vec<i16>>,
    /// The next sequence number (extended) to play.
    next: Option<u64>,
    /// The highest sequence number seen (extended), for unwrapping.
    highest: Option<u64>,
    playing: bool,
    out: VecDeque<i16>,
    last: Vec<i16>,
    concealed_run: u32,
    last_transit: Option<f64>,
    jitter_ms: f64,
    late_boost_ms: u32,
    target_ms: u32,
    stats: JitterStats,
}

impl JitterBuffer {
    /// A buffer for a stream whose RTP clock runs at `clock_rate` Hz
    /// (`Codec::clock_rate`); the frames given to it are 16 kHz samples.
    pub fn new(clock_rate: u32) -> JitterBuffer {
        JitterBuffer {
            clock_rate: clock_rate.max(1),
            min_delay_ms: MIN_DELAY_MS,
            max_delay_ms: MAX_DELAY_MS,
            frames: BTreeMap::new(),
            next: None,
            highest: None,
            playing: false,
            out: VecDeque::new(),
            last: Vec::new(),
            concealed_run: 0,
            last_transit: None,
            jitter_ms: 0.0,
            late_boost_ms: 0,
            target_ms: MIN_DELAY_MS,
            stats: JitterStats::default(),
        }
    }

    fn extend(&self, seq: u16) -> u64 {
        match self.highest.or(self.next) {
            None => (1 << 16) + seq as u64, // room below for reordered early packets
            Some(r) => {
                let d = seq.wrapping_sub(r as u16) as i16 as i64;
                (r as i64 + d).max(0) as u64
            }
        }
    }

    /// A packet's decoded frame (16 kHz samples) arrived at `arrival_ms`.
    pub fn push(&mut self, seq: u16, timestamp: u32, pcm: Vec<i16>, arrival_ms: u64) {
        if pcm.is_empty() {
            return;
        }
        let ext = self.extend(seq);
        // Interarrival jitter (RFC 3550 6.4.1), in ms.
        let transit = arrival_ms as f64 - timestamp as f64 * 1000.0 / self.clock_rate as f64;
        if let Some(last) = self.last_transit {
            let mut d = (transit - last).abs();
            // A timestamp wrap shows up as a huge jump: ignore that sample.
            if d > 10_000.0 {
                d = self.jitter_ms;
            }
            self.jitter_ms += (d - self.jitter_ms) / 16.0;
        }
        self.last_transit = Some(transit);
        if self.next.is_some_and(|n| ext < n) {
            self.stats.late += 1;
            self.late_boost_ms = (self.late_boost_ms + LATE_BOOST_MS).min(self.max_delay_ms);
            return;
        }
        if self.frames.contains_key(&ext) {
            self.stats.duplicates += 1;
            return;
        }
        self.stats.received += 1;
        self.frames.insert(ext, pcm);
        self.highest = Some(self.highest.map_or(ext, |h| h.max(ext)));
        // Grown far past the maximum delay (a burst after a stall): catch up.
        let frame_ms = self.frame_ms();
        if self.buffered_ms() > self.max_delay_ms + 2 * frame_ms {
            while self.buffered_ms() > self.target_ms.max(self.min_delay_ms) {
                let Some((&k, _)) = self.frames.iter().next() else {
                    break;
                };
                self.frames.remove(&k);
                self.stats.overflow += 1;
                if self.next.is_some_and(|n| n <= k) {
                    self.next = Some(k + 1);
                }
            }
        }
    }

    fn frame_len(&self) -> usize {
        if self.last.is_empty() {
            self.frames
                .values()
                .next()
                .map_or(FRAME_SAMPLES, |f| f.len())
        } else {
            self.last.len()
        }
    }

    fn frame_ms(&self) -> u32 {
        (self.frame_len() * 1000 / RATE as usize) as u32
    }

    /// Audio waiting, ms (at 16 kHz).
    pub fn buffered_ms(&self) -> u32 {
        let samples: usize = self.frames.values().map(|f| f.len()).sum();
        (samples * 1000 / RATE as usize) as u32
    }

    /// The playout delay now in use, ms.
    pub fn target_ms(&self) -> u32 {
        self.target_ms
    }

    /// Measured interarrival jitter, ms.
    pub fn jitter_ms(&self) -> f32 {
        self.jitter_ms as f32
    }

    /// Playing (false while priming, before the first frame or after running dry).
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    pub fn stats(&self) -> JitterStats {
        self.stats
    }

    /// Lost packets as a percentage of those due.
    pub fn loss_percent(&self) -> f32 {
        let due = self.stats.received + self.stats.lost;
        if due == 0 {
            0.0
        } else {
            self.stats.lost as f32 * 100.0 / due as f32
        }
    }

    /// Drop everything (a new call); the statistics stay.
    pub fn reset(&mut self) {
        let stats = self.stats;
        *self = JitterBuffer {
            stats,
            min_delay_ms: self.min_delay_ms,
            max_delay_ms: self.max_delay_ms,
            ..JitterBuffer::new(self.clock_rate)
        };
    }

    /// Exactly `n` samples at 16 kHz (silence while priming or when nothing came).
    pub fn pull(&mut self, n: usize) -> Vec<i16> {
        while self.out.len() < n {
            let f = self.next_frame();
            self.out.extend(f);
        }
        self.out.drain(..n).collect()
    }

    fn next_frame(&mut self) -> Vec<i16> {
        if !self.playing {
            let base = (3.0 * self.jitter_ms).ceil() as u32 + self.late_boost_ms;
            self.target_ms =
                base.clamp(self.min_delay_ms, self.max_delay_ms.max(self.min_delay_ms));
            let Some((&first, _)) = self.frames.iter().next() else {
                return self.silence();
            };
            if self.buffered_ms() < self.target_ms {
                return self.silence();
            }
            if let Some(n) = self.next {
                self.stats.lost += first.saturating_sub(n);
            }
            self.next = Some(first);
            self.playing = true;
            self.late_boost_ms /= 2;
        }
        let n = self.next.unwrap_or(0);
        if let Some(f) = self.frames.remove(&n) {
            self.next = Some(n + 1);
            self.concealed_run = 0;
            self.last = f.clone();
            return f;
        }
        if self.frames.is_empty() {
            // Ran dry (the talker paused, or the network stalled): prime again. Nothing is
            // known lost yet: a packet that is merely slow still plays after the restart.
            self.playing = false;
            self.concealed_run = 1;
            return self.silence();
        }
        self.stats.lost += 1;
        self.next = Some(n + 1);
        self.conceal()
    }

    fn silence(&self) -> Vec<i16> {
        vec![0; self.frame_len()]
    }

    fn conceal(&mut self) -> Vec<i16> {
        self.concealed_run += 1;
        if self.concealed_run > 1 || self.last.is_empty() {
            return self.silence();
        }
        self.stats.concealed += 1;
        let len = self.last.len() as f32;
        self.last
            .iter()
            .enumerate()
            .map(|(i, s)| (*s as f32 * (1.0 - i as f32 / len)).round() as i16)
            .collect()
    }
}

// ---------------------------------------------------------------- multi-talker mixer

/// One talker's queue in the [`Mixer`].
#[derive(Default, Clone, Debug)]
struct Talker {
    queue: VecDeque<i16>,
    last_seq: Option<u16>,
    /// Waiting for [`Mixer::prebuffer`] samples before playing (after silence or a start).
    priming: bool,
    received: u64,
    lost: u64,
    late: u64,
}

/// Many talkers mixed into one stream (from Discord): a small jitter queue per talker.
/// A talker is heard once [`Mixer::prebuffer`] samples are queued, primes again when its
/// queue runs dry, and queues at most [`Mixer::max_queue`] (older samples are dropped:
/// we are late). Duplicates and packets older than the last one are dropped; sequence
/// gaps count as losses.
#[derive(Clone, Debug)]
pub struct Mixer {
    talkers: BTreeMap<u32, Talker>,
    /// Samples a talker queues before it is heard (absorbs network jitter).
    pub prebuffer: usize,
    /// Most samples a talker may queue.
    pub max_queue: usize,
}

/// Discord's defaults: 80 ms prebuffer, 400 ms most.
impl Default for Mixer {
    fn default() -> Self {
        Mixer::with_limits(1280, 6400)
    }
}

impl Mixer {
    pub fn new() -> Mixer {
        Mixer::default()
    }

    pub fn with_limits(prebuffer: usize, max_queue: usize) -> Mixer {
        Mixer {
            talkers: BTreeMap::new(),
            prebuffer,
            max_queue,
        }
    }

    /// A talker's decoded frame with its sequence number.
    pub fn push_frame(&mut self, talker: u32, seq: u16, pcm: &[i16]) {
        let t = self.talkers.entry(talker).or_insert_with(|| Talker {
            priming: true,
            ..Default::default()
        });
        if let Some(last) = t.last_seq {
            let gap = seq.wrapping_sub(last);
            if gap == 0 || gap > 0x8000 {
                t.late += 1;
                return; // a duplicate or a late, reordered packet
            }
            t.lost += (gap - 1) as u64;
        }
        t.last_seq = Some(seq);
        t.received += 1;
        t.queue.extend(pcm);
        while t.queue.len() > self.max_queue {
            t.queue.pop_front();
        }
    }

    /// Up to `n` samples of everyone mixed, or fewer (none when nobody is talking).
    /// A talker whose queue ran dry primes again.
    pub fn pull(&mut self, n: usize) -> Vec<i16> {
        let mut len = 0;
        for t in self.talkers.values_mut() {
            if t.priming && t.queue.len() >= self.prebuffer {
                t.priming = false;
            }
            if !t.priming {
                len = len.max(t.queue.len().min(n));
            }
        }
        if len == 0 {
            return Vec::new();
        }
        let mut mix = vec![0i32; len];
        for t in self.talkers.values_mut() {
            if t.priming {
                continue;
            }
            for (k, s) in t.queue.drain(..t.queue.len().min(len)).enumerate() {
                mix[k] += s as i32;
            }
            if t.queue.is_empty() {
                t.priming = true;
            }
        }
        mix.into_iter()
            .map(|s| s.clamp(-32768, 32767) as i16)
            .collect()
    }

    /// A talker left: drop their queue.
    pub fn forget(&mut self, talker: u32) {
        self.talkers.remove(&talker);
    }

    pub fn clear(&mut self) {
        self.talkers.clear();
    }

    /// Packets received and lost (sequence gaps) from a talker.
    pub fn stats(&self, talker: u32) -> (u64, u64) {
        self.talkers
            .get(&talker)
            .map(|t| (t.received, t.lost))
            .unwrap_or((0, 0))
    }

    /// Duplicate or late packets dropped from a talker.
    pub fn late(&self, talker: u32) -> u64 {
        self.talkers.get(&talker).map_or(0, |t| t.late)
    }

    /// Talkers known.
    pub fn talkers(&self) -> impl Iterator<Item = u32> + '_ {
        self.talkers.keys().copied()
    }

    /// Samples waiting (the longest talker queue; for tests and the status line).
    pub fn queued(&self) -> usize {
        self.talkers
            .values()
            .map(|t| t.queue.len())
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::sine;

    const F: usize = FRAME_SAMPLES;

    /// Frame `k`: a constant marker value so the output can be read back.
    fn frame(k: u16) -> Vec<i16> {
        vec![(k as i16 + 1) * 10; F]
    }

    /// Which frame each output frame was (0 = silence, -1 = something else).
    fn which(out: &[i16]) -> Vec<i32> {
        out.chunks(F)
            .map(|c| {
                if c.iter().all(|s| *s == 0) {
                    0
                } else if c.iter().all(|s| *s == c[0]) {
                    c[0] as i32 / 10
                } else {
                    -1
                }
            })
            .collect()
    }

    /// Push frames `seqs` at 20 ms ticks (packet i arriving at tick i + delay(i)), pulling one
    /// frame per tick; returns the frames played.
    fn run(jb: &mut JitterBuffer, seqs: &[u16], ticks: usize) -> Vec<i32> {
        let mut out = Vec::new();
        for t in 0..ticks {
            if let Some(&s) = seqs.get(t) {
                jb.push(s, s as u32 * 320, frame(s), t as u64 * 20);
            }
            out.extend(jb.pull(F));
        }
        which(&out)
    }

    #[test]
    fn plays_in_order_after_the_minimum_delay() {
        let mut jb = JitterBuffer::new(16000);
        let seqs: Vec<u16> = (0..10).collect();
        let got = run(&mut jb, &seqs, 14);
        // Two frames (40 ms) prime, then 0..9 in order, then dry.
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 0, 0, 0]);
        assert_eq!(jb.target_ms(), 40);
        assert_eq!(
            jb.stats(),
            JitterStats {
                received: 10,
                ..Default::default()
            }
        );
        assert!(!jb.is_playing());
        assert_eq!(jb.pull(5).len(), 5, "pull gives exactly n");
    }

    #[test]
    fn reorders() {
        let mut jb = JitterBuffer::new(16000);
        let got = run(&mut jb, &[0, 2, 1, 4, 3, 5], 9);
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5, 6, 0, 0]);
        assert_eq!(jb.stats().lost, 0);
    }

    #[test]
    fn conceals_one_lost_frame_then_silence() {
        let mut jb = JitterBuffer::new(16000);
        let mut out = Vec::new();
        for s in [0u16, 1, 2, 3, 6, 7] {
            jb.push(s, s as u32 * 320, sine(F, 500.0, 5000.0 + s as f64), 0);
        }
        for _ in 0..8 {
            out.push(jb.pull(F));
        }
        let st = jb.stats();
        assert_eq!(st.lost, 2);
        assert_eq!(st.concealed, 1);
        // Frame 4 is frame 3 fading out, frame 5 silence, then 6 and 7.
        let f3 = &out[3];
        assert_eq!(out[4][0], f3[0]);
        assert!(out[4]
            .iter()
            .zip(f3)
            .all(|(c, o)| c.unsigned_abs() <= o.unsigned_abs()));
        assert!(out[4][F - 10..].iter().all(|s| s.unsigned_abs() < 200));
        assert!(out[5].iter().all(|s| *s == 0));
        assert!(out[6].iter().any(|s| *s != 0));
        assert!((jb.loss_percent() - 25.0).abs() < 0.01);
    }

    #[test]
    fn drops_late_packets_and_grows_the_delay() {
        let mut jb = JitterBuffer::new(16000);
        run(&mut jb, &[0, 1, 2, 3, 4], 8);
        jb.push(1, 320, frame(1), 200);
        jb.push(3, 960, frame(3), 200);
        assert_eq!(jb.stats().late, 2);
        // Duplicates of a queued packet are dropped too.
        jb.push(9, 9 * 320, frame(9), 210);
        jb.push(9, 9 * 320, frame(9), 210);
        assert_eq!(jb.stats().duplicates, 1);
        // The next restart primes for longer (the late arrivals raised the jitter, plus 2 * 20 ms boost).
        let _ = jb.pull(F);
        assert!(jb.target_ms() >= 80, "{}", jb.target_ms());
    }

    #[test]
    fn the_delay_adapts_to_jitter_within_limits() {
        // Arrival jitter: packets alternate on time / 30 ms late.
        let mut jb = JitterBuffer::new(16000);
        for s in 0..200u16 {
            jb.push(
                s,
                s as u32 * 320,
                frame(s),
                s as u64 * 20 + if s % 2 == 1 { 30 } else { 0 },
            );
        }
        assert!(jb.jitter_ms() > 20.0, "{}", jb.jitter_ms());
        let _ = jb.pull(F);
        assert!(
            jb.target_ms() > 40 && jb.target_ms() <= 160,
            "{}",
            jb.target_ms()
        );
        // Huge jitter: capped at 160 ms.
        let mut jb = JitterBuffer::new(16000);
        for s in 0..100u16 {
            jb.push(
                s,
                s as u32 * 320,
                frame(s),
                s as u64 * 20 + if s % 2 == 1 { 300 } else { 0 },
            );
        }
        let _ = jb.pull(F);
        assert_eq!(jb.target_ms(), 160);
        // An 8 kHz clock (narrowband) measures jitter in the same ms.
        let mut nb = JitterBuffer::new(8000);
        for s in 0..100u16 {
            nb.push(s, s as u32 * 160, frame(s), s as u64 * 20);
        }
        assert!(nb.jitter_ms() < 0.01);
    }

    #[test]
    fn sequence_numbers_wrap() {
        let mut jb = JitterBuffer::new(16000);
        let seqs: Vec<u16> = (65532..=65535).chain(0..4).collect();
        let mut out = Vec::new();
        for (t, &s) in seqs.iter().enumerate() {
            jb.push(
                s,
                t as u32 * 320,
                vec![(t as i16 + 1) * 10; F],
                t as u64 * 20,
            );
            out.extend(jb.pull(F));
        }
        out.extend(jb.pull(F * 2));
        assert_eq!(which(&out), vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 0]);
        assert_eq!(jb.stats().lost, 0);
    }

    #[test]
    fn runs_dry_primes_again_and_trims_a_burst() {
        let mut jb = JitterBuffer::new(16000);
        run(&mut jb, &[0, 1, 2], 6);
        assert!(!jb.is_playing());
        // The talker comes back (seq continues after a pause): primes, plays, no loss.
        for s in 3..5u16 {
            jb.push(s, s as u32 * 320 + 16000, frame(s), 1000 + s as u64 * 20);
        }
        assert_eq!(which(&jb.pull(F * 2)), vec![4, 5]);
        assert_eq!(jb.stats().lost, 0);
        // A burst of 20 frames at once (a stall): trimmed back to the delay.
        for s in 5..25u16 {
            jb.push(s, s as u32 * 320, frame(s), 2000);
        }
        assert!(jb.stats().overflow > 0);
        assert!(jb.buffered_ms() <= 160 + 40, "{}", jb.buffered_ms());
        let got = which(&jb.pull(F * 3));
        assert!(got[0] > 15, "the newest frames play: {got:?}");
        jb.reset();
        assert_eq!(jb.buffered_ms(), 0);
        assert!(jb.stats().received > 0, "stats survive a reset");
    }

    // ---- mixer (Discord's behaviour)

    fn tone(n: usize, amp: f64) -> Vec<i16> {
        sine(n, 440.0, amp)
    }

    #[test]
    fn the_mixer_primes_mixes_and_counts_losses() {
        let a = tone(640 * 3, 4000.0);
        let fa: Vec<&[i16]> = a.chunks(640).collect();
        let mut m = Mixer::new();
        m.push_frame(10, 1, fa[0]);
        assert!(
            m.pull(640).is_empty(),
            "less than the prebuffer: silence for now"
        );
        m.push_frame(10, 2, fa[1]);
        let first = m.pull(640);
        assert_eq!(first, fa[0]);
        // Two talkers add up.
        let mut both = Mixer::new();
        for (i, f) in fa.iter().take(2).enumerate() {
            both.push_frame(1, i as u16, f);
            both.push_frame(2, i as u16, f);
        }
        let mixed = both.pull(640);
        assert!(mixed
            .iter()
            .zip(fa[0])
            .all(|(m, s)| *m as i32 == (*s as i32 * 2).clamp(-32768, 32767)));
        // A duplicate is dropped, a gap counts as losses.
        m.push_frame(10, 3, fa[2]);
        m.push_frame(10, 3, fa[2]);
        assert_eq!(m.stats(10), (3, 0));
        assert_eq!(m.late(10), 1);
        m.push_frame(10, 6, fa[2]);
        assert_eq!(m.stats(10).1, 2);
        assert_eq!(m.talkers().collect::<Vec<_>>(), vec![10]);
        m.forget(10);
        assert_eq!(m.stats(10), (0, 0));
    }

    #[test]
    fn the_mixer_clips_caps_and_reprimes() {
        let mut m = Mixer::with_limits(10, 30);
        m.push_frame(1, 0, &[30000; 20]);
        m.push_frame(2, 0, &[30000; 20]);
        assert_eq!(m.pull(5), vec![32767; 5], "clipped, not wrapped");
        m.push_frame(1, 1, &[1; 100]);
        assert_eq!(m.queued(), 30, "capped at max_queue");
        assert_eq!(m.pull(100).len(), 30);
        m.clear();
        assert_eq!(m.queued(), 0);
        m.push_frame(1, 2, &[5; 5]);
        assert!(m.pull(5).is_empty(), "primes again");
    }
}
