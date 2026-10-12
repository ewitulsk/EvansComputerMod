//! A simulated framed-PHY radio medium (std only): what the simulator and
//! this crate's end-to-end tests put under the state machines instead of the
//! game's SDRs. It mirrors the framed PHY's semantics (SDR_PROGRAMS.md "What
//! the radio models") in a simpler world:
//!
//! - **Propagation.** Log-distance path loss `FSPL(1 m, f) + 10 n log10(d)`
//!   (exponent `n` per medium, default 3.0), plus a fixed loss for every wall
//!   box the straight line crosses. Antenna gains and noise figures per profile
//!   and role as in `PhyProfile` (LTE: tower 43 dBm, 14 dBi panel, NF 3 dB;
//!   phone 23 dBm, 2.15 dBi, NF 7 dB).
//! - **Channels.** FDD: a `bs` transmits on the channel's downlink frequency
//!   and hears its uplink; a `ue` the reverse. Different channels never meet.
//! - **Slots.** Frames start on slot boundaries of the airtime clock (the next
//!   one, or the slot a TX names, at most 1 s ahead; a past slot is rejected);
//!   one transmitter sends one frame at a time (later frames queue behind it).
//! - **Link adaptation.** Airtime from the CQI table; unicast frames go at the
//!   CQI the destination last reported, broadcasts and beacons at CQI 1; each
//!   frame carries the sender's measured CQI of the destination.
//! - **Collisions.** A frame's SINR counts every other frame on the same
//!   frequency overlapping it in time and in sub-channel (sub-channel 0 is the
//!   whole carrier). Below the CQI's threshold - 1 dB it is lost, within 1 dB
//!   of it 10 % are lost (a deterministic roll per frame and receiver).
//! - **HARQ.** A lost HARQ frame is resent 8 slots after its start, two CQI
//!   steps lower, at most 3 times, then STATUS `failed`; delivered = `acked`
//!   (ACK frames themselves are not simulated). Beacons: up to 4 per radio,
//!   every N ms on boundaries of the airtime clock.

use std::collections::{BTreeMap, VecDeque};

use crate::phy::{
    airtime_us, cqi_for, FramedPhy, PhyConfig, PhyError, PhyEvent, Profile, Role, RxFrame, TxReq, TxResult, TxStatus, BROADCAST, FLAG_BEACON,
    FLAG_HARQ, FLAG_RETX, MAX_PAYLOAD, REQUIRED_SINR_DB,
};

pub type RadioId = usize;

/// An axis-aligned box that attenuates every path crossing it.
#[derive(Clone, Debug)]
pub struct Wall {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub loss_db: f64,
}

#[derive(Clone, Debug)]
pub struct MediumConfig {
    pub exponent: f64,
    pub seed: u64,
    pub walls: Vec<Wall>,
}

impl Default for MediumConfig {
    fn default() -> Self {
        MediumConfig { exponent: 3.0, seed: 1, walls: Vec::new() }
    }
}

#[derive(Clone, Copy, Debug)]
struct Motion {
    from: [f64; 3],
    to: [f64; 3],
    t0: u64,
    t1: u64,
}

impl Motion {
    fn at(&self, t: u64) -> [f64; 3] {
        if t >= self.t1 || self.t1 <= self.t0 {
            return self.to;
        }
        if t <= self.t0 {
            return self.from;
        }
        let k = (t - self.t0) as f64 / (self.t1 - self.t0) as f64;
        [0, 1, 2].map(|i| self.from[i] + (self.to[i] - self.from[i]) * k)
    }
}

struct Beacon {
    period_us: u64,
    next_us: u64,
    payload: Vec<u8>,
}

#[derive(Default, Clone, Debug)]
pub struct RadioStats {
    pub tx_frames: u64,
    pub rx_frames: u64,
    pub lost: u64,
    pub harq_retx: u64,
    pub harq_failed: u64,
}

struct Radio {
    name: String,
    motion: Motion,
    cfg: Option<PhyConfig>,
    on: bool,
    inbox: VecDeque<PhyEvent>,
    beacons: [Option<Beacon>; 4],
    busy_until: u64,
    seq: u16,
    /// What each peer reported about how it hears us (used to pick our CQI to it).
    reported: BTreeMap<u32, u8>,
    /// Our measurement of each peer (sent back in our frames' headers).
    heard: BTreeMap<u32, u8>,
    stats: RadioStats,
}

#[derive(Clone)]
struct Frame {
    src: RadioId,
    src_addr: u32,
    dst: u32,
    profile: Profile,
    freq: f64,
    sub: u8,
    start: u64,
    end: u64,
    cqi: u8,
    power_dbm: f64,
    flags: u8,
    seq: u16,
    cqi_report: u8,
    tag: u16,
    attempt: u8,
    payload: Vec<u8>,
    resolved: bool,
}

pub struct Medium {
    pub cfg: MediumConfig,
    radios: Vec<Radio>,
    frames: Vec<Frame>,
    now: u64,
}

/// Radio header bytes on the air in front of every payload.
const AIR_HEADER: usize = 16;
const MAX_AHEAD_US: u64 = 1_000_000;

fn gains(p: Profile, role: Role) -> (f64, f64) {
    // (antenna gain dBi, noise figure dB)
    match (p, role) {
        (Profile::Lte, Role::Bs) => (14.0, 3.0),
        (Profile::Lte, Role::Ue) => (2.15, 7.0),
        (Profile::Mmwave, Role::Bs) => (20.0, 5.0),
        (Profile::Mmwave, Role::Ue) => (8.0, 9.0),
    }
}

fn tx_freq(c: &PhyConfig) -> f64 {
    match c.role {
        Role::Bs => c.profile.dl_hz(c.channel),
        Role::Ue => c.profile.ul_hz(c.channel),
    }
}

fn rx_freq(c: &PhyConfig) -> f64 {
    match c.role {
        Role::Bs => c.profile.ul_hz(c.channel),
        Role::Ue => c.profile.dl_hz(c.channel),
    }
}

fn db_to_mw(db: f64) -> f64 {
    10f64.powf(db / 10.0)
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^ (x >> 33)
}

impl Medium {
    pub fn new(cfg: MediumConfig) -> Medium {
        Medium { cfg, radios: Vec::new(), frames: Vec::new(), now: 0 }
    }

    pub fn now_us(&self) -> u64 {
        self.now
    }

    pub fn add_radio(&mut self, name: &str, pos: [f64; 3]) -> RadioId {
        self.radios.push(Radio {
            name: name.into(),
            motion: Motion { from: pos, to: pos, t0: 0, t1: 0 },
            cfg: None,
            on: true,
            inbox: VecDeque::new(),
            beacons: [None, None, None, None],
            busy_until: 0,
            seq: 0,
            reported: BTreeMap::new(),
            heard: BTreeMap::new(),
            stats: RadioStats::default(),
        });
        self.radios.len() - 1
    }

    pub fn radio_by_name(&self, name: &str) -> Option<RadioId> {
        self.radios.iter().position(|r| r.name == name)
    }

    pub fn position(&self, id: RadioId, now_us: u64) -> [f64; 3] {
        self.radios[id].motion.at(now_us)
    }

    /// Move in a straight line to `to`, arriving after `duration_us` (0 = jump).
    pub fn move_to(&mut self, id: RadioId, to: [f64; 3], now_us: u64, duration_us: u64) {
        let from = self.radios[id].motion.at(now_us);
        self.radios[id].motion = Motion { from, to, t0: now_us, t1: now_us + duration_us };
    }

    /// Power the radio on or off (off: nothing sent or heard, beacons and queue dropped).
    pub fn set_on(&mut self, id: RadioId, on: bool) {
        let r = &mut self.radios[id];
        r.on = on;
        if !on {
            r.beacons = [None, None, None, None];
            r.inbox.clear();
            self.frames.retain(|f| f.src != id || f.resolved);
        }
    }

    pub fn stats(&self, id: RadioId) -> RadioStats {
        self.radios[id].stats.clone()
    }

    pub fn configure(&mut self, id: RadioId, c: &PhyConfig) -> Result<(), PhyError> {
        if c.channel == 0 || c.channel > c.profile.channels() {
            return Err(PhyError(format!("channel {} out of range", c.channel)));
        }
        let r = &mut self.radios[id];
        // a retune flushes queued frames of the old channel
        let retune = r.cfg.as_ref().is_some_and(|o| o.channel != c.channel || o.profile != c.profile || o.role != c.role);
        if retune {
            self.frames.retain(|f| f.src != id || f.resolved || f.start <= self.now);
        }
        let r = &mut self.radios[id];
        if r.cfg.as_ref().is_some_and(|o| o.profile != c.profile) {
            r.beacons = [None, None, None, None];
        }
        r.cfg = Some(c.clone());
        Ok(())
    }

    pub fn slot(&self, id: RadioId, now_us: u64) -> u64 {
        let p = self.radios[id].cfg.as_ref().map(|c| c.profile).unwrap_or(Profile::Lte);
        now_us / p.slot_us()
    }

    fn status(&mut self, id: RadioId, result: TxResult, dst: u32, tag: u16, attempts: u8, cqi: u8, slot: u64, airtime: u64) {
        self.radios[id].inbox.push_back(PhyEvent::Status(TxStatus { result, dst, tag, attempts, cqi, slot, airtime_us: airtime as u32 }));
    }

    pub fn send(&mut self, id: RadioId, tx: &TxReq, now_us: u64) -> Result<(), PhyError> {
        self.advance(now_us);
        let Some(c) = self.radios[id].cfg.clone() else {
            self.status(id, TxResult::Rejected, tx.dst, tx.tag, 0, 0, 0, 0);
            return Ok(());
        };
        let slot_us = c.profile.slot_us();
        let reject = !self.radios[id].on
            || tx.payload.len() > MAX_PAYLOAD
            || tx.subchannel > 4
            || (tx.slot != 0 && (tx.slot * slot_us < now_us || tx.slot * slot_us > now_us + MAX_AHEAD_US));
        if reject {
            self.status(id, TxResult::Rejected, tx.dst, tx.tag, 0, 0, tx.slot, 0);
            return Ok(());
        }
        let want = if tx.slot == 0 { (now_us / slot_us + 1) * slot_us } else { tx.slot * slot_us };
        let r = &mut self.radios[id];
        let cqi = if tx.cqi != 0 {
            tx.cqi
        } else if tx.dst == BROADCAST {
            1
        } else {
            r.reported.get(&tx.dst).copied().unwrap_or(1)
        }
        .clamp(1, c.profile.max_cqi());
        let start = want.max(r.busy_until.div_ceil(slot_us) * slot_us);
        let air = airtime_us(c.profile, cqi, tx.subchannel, tx.payload.len() + AIR_HEADER);
        r.busy_until = start + air;
        r.seq = r.seq.wrapping_add(1);
        r.stats.tx_frames += 1;
        let cqi_report = if tx.dst == BROADCAST { 0 } else { r.heard.get(&tx.dst).copied().unwrap_or(0) };
        let f = Frame {
            src: id,
            src_addr: c.address,
            dst: tx.dst,
            profile: c.profile,
            freq: tx_freq(&c),
            sub: tx.subchannel,
            start,
            end: start + air,
            cqi,
            power_dbm: c.power_dbm.unwrap_or(c.profile.default_power_dbm(c.role)) + gains(c.profile, c.role).0,
            flags: if tx.harq && tx.dst != BROADCAST { FLAG_HARQ } else { 0 },
            seq: r.seq,
            cqi_report,
            tag: tx.tag,
            attempt: 1,
            payload: tx.payload.clone(),
            resolved: false,
        };
        if f.flags & FLAG_HARQ == 0 {
            self.status(id, TxResult::Sent, tx.dst, tx.tag, 1, cqi, start / slot_us, air);
        }
        self.frames.push(f);
        Ok(())
    }

    pub fn beacon(&mut self, id: RadioId, index: u8, period_ms: u32, payload: &[u8], now_us: u64) -> Result<(), PhyError> {
        self.advance(now_us);
        if index > 3 || payload.len() > MAX_PAYLOAD || (period_ms != 0 && !(5..=10_000).contains(&period_ms)) {
            return Err(PhyError("bad beacon".into()));
        }
        let r = &mut self.radios[id];
        if period_ms == 0 {
            r.beacons[index as usize] = None;
            return Ok(());
        }
        let p = period_ms as u64 * 1000;
        let next = r.beacons[index as usize].as_ref().filter(|b| b.period_us == p).map(|b| b.next_us).unwrap_or((now_us / p + 1) * p);
        r.beacons[index as usize] = Some(Beacon { period_us: p, next_us: next, payload: payload.to_vec() });
        Ok(())
    }

    pub fn poll(&mut self, id: RadioId, now_us: u64) -> Option<PhyEvent> {
        self.advance(now_us);
        self.radios[id].inbox.pop_front()
    }

    /// Path gain (negative dB) between two radios at time `t`, walls included.
    fn path_loss(&self, a: RadioId, b: RadioId, freq: f64, t: u64) -> f64 {
        let pa = self.radios[a].motion.at(t);
        let pb = self.radios[b].motion.at(t);
        let d = ((pa[0] - pb[0]).powi(2) + (pa[1] - pb[1]).powi(2) + (pa[2] - pb[2]).powi(2)).sqrt().max(1.0);
        let fspl1 = 20.0 * freq.log10() - 147.55;
        let mut loss = fspl1 + 10.0 * self.cfg.exponent * d.log10();
        for w in &self.cfg.walls {
            if segment_hits_box(pa, pb, w) {
                loss += w.loss_db;
            }
        }
        loss
    }

    fn rx_power(&self, f: &Frame, r: RadioId) -> f64 {
        let c = self.radios[r].cfg.as_ref().expect("listening radios are configured");
        f.power_dbm + gains(c.profile, c.role).0 - self.path_loss(f.src, r, f.freq, f.start)
    }

    /// Run the medium up to `now_us`: emit due beacons, resolve finished frames.
    pub fn advance(&mut self, now_us: u64) {
        if now_us < self.now {
            return;
        }
        self.now = now_us;
        // beacons are put on the air up to 150 ms ahead
        for id in 0..self.radios.len() {
            if !self.radios[id].on {
                continue;
            }
            let Some(c) = self.radios[id].cfg.clone() else { continue };
            for bi in 0..4 {
                loop {
                    let r = &mut self.radios[id];
                    let Some(b) = r.beacons[bi].as_mut() else { break };
                    if b.next_us > now_us + 150_000 {
                        break;
                    }
                    let start = b.next_us;
                    b.next_us += b.period_us;
                    let payload = b.payload.clone();
                    let air = airtime_us(c.profile, 1, 0, payload.len() + AIR_HEADER);
                    r.seq = r.seq.wrapping_add(1);
                    r.stats.tx_frames += 1;
                    let seq = r.seq;
                    self.frames.push(Frame {
                        src: id,
                        src_addr: c.address,
                        dst: BROADCAST,
                        profile: c.profile,
                        freq: tx_freq(&c),
                        sub: 0,
                        start,
                        end: start + air,
                        cqi: 1,
                        power_dbm: c.power_dbm.unwrap_or(c.profile.default_power_dbm(c.role)) + gains(c.profile, c.role).0,
                        flags: FLAG_BEACON,
                        seq,
                        cqi_report: 0,
                        tag: 0,
                        attempt: 1,
                        payload,
                        resolved: false,
                    });
                }
            }
        }
        // resolve, in a fixed order
        // (retransmissions scheduled while resolving may already be due too)
        loop {
            let mut due: Vec<usize> = (0..self.frames.len()).filter(|&i| !self.frames[i].resolved && self.frames[i].end <= now_us).collect();
            if due.is_empty() {
                break;
            }
            due.sort_by_key(|&i| {
                let f = &self.frames[i];
                (f.end, f.start, f.src_addr, f.seq)
            });
            for i in due {
                self.resolve(i);
            }
        }
        let keep_after = now_us.saturating_sub(200_000);
        self.frames.retain(|f| !f.resolved || f.end >= keep_after);
    }

    fn resolve(&mut self, i: usize) {
        self.frames[i].resolved = true;
        let f = self.frames[i].clone();
        let slot_us = f.profile.slot_us();
        let mut delivered_to_dst = false;
        let mut dst_exists = false;
        for r in 0..self.radios.len() {
            if r == f.src || !self.radios[r].on {
                continue;
            }
            let Some(c) = self.radios[r].cfg.clone() else { continue };
            if c.profile != f.profile || (rx_freq(&c) - f.freq).abs() > 1.0 {
                continue;
            }
            if f.dst == c.address {
                dst_exists = true;
            }
            let p = self.rx_power(&f, r);
            let bw = f.profile.bandwidth_hz() / if f.sub == 0 { 1.0 } else { 4.0 };
            let (_, nf) = gains(c.profile, c.role);
            let noise_dbm = -174.0 + 10.0 * bw.log10() + nf;
            if p < noise_dbm - 15.0 {
                continue; // far below the noise: not even detected
            }
            let mut int_mw = 0.0;
            for g in &self.frames {
                if std::ptr::eq(g, &self.frames[i]) || g.src == r || g.src == f.src {
                    continue;
                }
                if (g.freq - f.freq).abs() > 1.0 || g.end <= f.start || g.start >= f.end {
                    continue;
                }
                if !(f.sub == 0 || g.sub == 0 || f.sub == g.sub) {
                    continue;
                }
                if !self.radios[g.src].on {
                    continue;
                }
                int_mw += db_to_mw(self.rx_power(g, r));
            }
            let sinr = p - 10.0 * (db_to_mw(noise_dbm) + int_mw).log10();
            let need = REQUIRED_SINR_DB[f.cqi.clamp(1, f.profile.max_cqi()) as usize];
            let roll = mix(self.cfg.seed ^ (f.src_addr as u64) << 32 ^ f.start ^ (c.address as u64).rotate_left(17) ^ f.attempt as u64) % 100;
            let ok = sinr >= need + 1.0 || (sinr >= need - 1.0 && roll >= 10);
            let supported = cqi_for(f.profile, sinr);
            let rr = &mut self.radios[r];
            if !ok {
                if f.dst == c.address {
                    rr.stats.lost += 1;
                }
                continue;
            }
            if f.dst != c.address && f.dst != BROADCAST {
                continue;
            }
            rr.heard.insert(f.src_addr, supported);
            if f.dst == c.address && f.cqi_report != 0 {
                rr.reported.insert(f.src_addr, f.cqi_report);
            }
            rr.stats.rx_frames += 1;
            if f.dst == c.address {
                delivered_to_dst = true;
            }
            rr.inbox.push_back(PhyEvent::Rx(RxFrame {
                flags: f.flags | if f.attempt > 1 { FLAG_RETX } else { 0 },
                src: f.src_addr,
                dst: f.dst,
                seq: f.seq,
                cqi: supported,
                cqi_sent: f.cqi,
                rssi_dbm: (p * 10.0).round() / 10.0,
                sinr_db: (sinr * 10.0).round() / 10.0,
                timestamp_us: f.end,
                slot: f.start / slot_us,
                subchannel: f.sub,
                cqi_report: f.cqi_report,
                payload: f.payload.clone(),
            }));
        }
        let _ = dst_exists;
        if f.flags & FLAG_HARQ == 0 {
            return;
        }
        let air = f.end - f.start;
        if delivered_to_dst {
            self.status(f.src, TxResult::Acked, f.dst, f.tag, f.attempt, f.cqi, f.start / slot_us, air);
            return;
        }
        if f.attempt >= 4 || !self.radios[f.src].on {
            self.radios[f.src].stats.harq_failed += 1;
            self.status(f.src, TxResult::Failed, f.dst, f.tag, f.attempt, f.cqi, f.start / slot_us, air);
            return;
        }
        // retransmit 8 slots after the start, two CQI steps lower
        let src = &mut self.radios[f.src];
        src.stats.harq_retx += 1;
        let cqi = f.cqi.saturating_sub(2).max(1);
        let want = (f.start / slot_us + 8) * slot_us;
        let start = want.max(src.busy_until.div_ceil(slot_us) * slot_us);
        let air = airtime_us(f.profile, cqi, f.sub, f.payload.len() + AIR_HEADER);
        src.busy_until = start + air;
        let mut g = f;
        g.start = start;
        g.end = start + air;
        g.cqi = cqi;
        g.attempt += 1;
        g.resolved = false;
        self.frames.push(g);
    }
}

fn segment_hits_box(a: [f64; 3], b: [f64; 3], w: &Wall) -> bool {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for i in 0..3 {
        let d = b[i] - a[i];
        if d.abs() < 1e-12 {
            if a[i] < w.min[i] || a[i] > w.max[i] {
                return false;
            }
        } else {
            let (mut ta, mut tb) = ((w.min[i] - a[i]) / d, (w.max[i] - a[i]) / d);
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// One radio of a medium at one instant, as a [`FramedPhy`].
pub struct Port<'a> {
    pub medium: &'a mut Medium,
    pub id: RadioId,
    pub now_us: u64,
}

impl FramedPhy for Port<'_> {
    fn configure(&mut self, c: &PhyConfig) -> Result<(), PhyError> {
        self.medium.configure(self.id, c)
    }
    fn send(&mut self, tx: &TxReq) -> Result<(), PhyError> {
        self.medium.send(self.id, tx, self.now_us)
    }
    fn beacon(&mut self, index: u8, period_ms: u32, payload: &[u8]) -> Result<(), PhyError> {
        self.medium.beacon(self.id, index, period_ms, payload, self.now_us)
    }
    fn poll(&mut self) -> Option<PhyEvent> {
        self.medium.poll(self.id, self.now_us)
    }
    fn slot(&mut self) -> u64 {
        self.medium.slot(self.id, self.now_us)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(role: Role, ch: u8, addr: u32) -> PhyConfig {
        PhyConfig { profile: Profile::Lte, role, channel: ch, power_dbm: None, address: addr }
    }

    fn drain(m: &mut Medium, id: RadioId, now: u64) -> Vec<PhyEvent> {
        let mut v = Vec::new();
        while let Some(e) = m.poll(id, now) {
            v.push(e);
        }
        v
    }

    fn rx(v: &[PhyEvent]) -> Vec<&RxFrame> {
        v.iter()
            .filter_map(|e| match e {
                PhyEvent::Rx(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn beacons_unicast_harq_and_range() {
        let mut m = Medium::new(MediumConfig::default());
        let bs = m.add_radio("bs", [0.0, 0.0, 0.0]);
        let ue = m.add_radio("ue", [100.0, 0.0, 0.0]);
        let far = m.add_radio("far", [1e7, 0.0, 0.0]);
        let other = m.add_radio("other", [50.0, 0.0, 0.0]);
        m.configure(bs, &cfg(Role::Bs, 1, 0xA)).unwrap();
        m.configure(ue, &cfg(Role::Ue, 1, 0xB)).unwrap();
        m.configure(far, &cfg(Role::Ue, 1, 0xC)).unwrap();
        m.configure(other, &cfg(Role::Ue, 2, 0xD)).unwrap();
        m.beacon(bs, 0, 100, b"sib", 0).unwrap();
        let t = 1_000_000;
        let got = drain(&mut m, ue, t);
        let b = rx(&got);
        assert!((9..=11).contains(&b.len()), "beacons in 1 s: {}", b.len());
        assert!(b[0].is_beacon() && b[0].cqi > 10 && b[0].rssi_dbm > -80.0, "{:?}", b[0]);
        assert_eq!(b[1].timestamp_us - b[0].timestamp_us, 100_000);
        assert!(drain(&mut m, far, t).is_empty(), "out of range");
        assert!(drain(&mut m, other, t).is_empty(), "another channel");
        // uplink unicast with HARQ: acked, and the BS learns the UE's CQI report
        m.send(ue, &TxReq { dst: 0xA, harq: true, tag: 7, payload: b"hi".to_vec(), ..Default::default() }, t).unwrap();
        let t2 = t + 20_000;
        let st: Vec<_> = drain(&mut m, ue, t2);
        assert!(st.iter().any(|e| matches!(e, PhyEvent::Status(s) if s.tag == 7 && s.result == TxResult::Acked)), "{:?}", st);
        let at_bs = drain(&mut m, bs, t2);
        let f = rx(&at_bs);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].payload, b"hi");
        assert!(f[0].cqi_report > 10, "the UE reported how it hears the BS");
        // downlink to an absent address fails after 4 attempts
        m.send(bs, &TxReq { dst: 0xEE, harq: true, tag: 9, payload: vec![1; 10], ..Default::default() }, t2).unwrap();
        let st = drain(&mut m, bs, t2 + 100_000);
        assert!(st.iter().any(|e| matches!(e, PhyEvent::Status(s) if s.tag == 9 && s.result == TxResult::Failed && s.attempts == 4)), "{:?}", st);
        // a past slot is rejected
        m.send(bs, &TxReq { dst: 0xB, slot: 5, tag: 10, payload: vec![1], ..Default::default() }, t2 + 100_000).unwrap();
        assert!(drain(&mut m, bs, t2 + 100_000).iter().any(|e| matches!(e, PhyEvent::Status(s) if s.tag == 10 && s.result == TxResult::Rejected)));
    }

    #[test]
    fn collisions_per_subchannel_and_walls() {
        let mut m = Medium::new(MediumConfig::default());
        let bs = m.add_radio("bs", [0.0, 0.0, 0.0]);
        let a = m.add_radio("a", [100.0, 0.0, 0.0]);
        let b = m.add_radio("b", [-100.0, 0.0, 0.0]);
        m.configure(bs, &cfg(Role::Bs, 1, 1)).unwrap();
        m.configure(a, &cfg(Role::Ue, 1, 2)).unwrap();
        m.configure(b, &cfg(Role::Ue, 1, 3)).unwrap();
        let slot = 50;
        // same slot, same sub-channel, equal power: both lost (no HARQ)
        m.send(a, &TxReq { dst: 1, slot, cqi: 10, subchannel: 1, payload: vec![0; 200], ..Default::default() }, 0).unwrap();
        m.send(b, &TxReq { dst: 1, slot, cqi: 10, subchannel: 1, payload: vec![0; 200], ..Default::default() }, 0).unwrap();
        assert!(rx(&drain(&mut m, bs, 100_000)).is_empty(), "collision");
        // different sub-channels: both heard
        m.send(a, &TxReq { dst: 1, slot: slot + 200, cqi: 10, subchannel: 1, payload: vec![0; 200], ..Default::default() }, 100_000).unwrap();
        m.send(b, &TxReq { dst: 1, slot: slot + 200, cqi: 10, subchannel: 2, payload: vec![0; 200], ..Default::default() }, 100_000).unwrap();
        assert_eq!(rx(&drain(&mut m, bs, 400_000)).len(), 2);
        // a wall between bs and a costs its loss
        let before = {
            m.send(a, &TxReq { dst: 1, payload: vec![1], ..Default::default() }, 400_000).unwrap();
            rx(&drain(&mut m, bs, 450_000))[0].rssi_dbm
        };
        m.cfg.walls.push(Wall { min: [40.0, -5.0, -5.0], max: [41.0, 5.0, 5.0], loss_db: 20.0 });
        m.send(a, &TxReq { dst: 1, payload: vec![1], ..Default::default() }, 450_000).unwrap();
        let after = rx(&drain(&mut m, bs, 500_000))[0].rssi_dbm;
        assert!((before - after - 20.0).abs() < 0.2, "{} -> {}", before, after);
    }

    #[test]
    fn motion_and_power() {
        let mut m = Medium::new(MediumConfig::default());
        let bs = m.add_radio("bs", [0.0, 0.0, 0.0]);
        let ue = m.add_radio("ue", [10.0, 0.0, 0.0]);
        m.configure(bs, &cfg(Role::Bs, 1, 1)).unwrap();
        m.configure(ue, &cfg(Role::Ue, 1, 2)).unwrap();
        m.move_to(ue, [1000.0, 0.0, 0.0], 0, 10_000_000);
        assert_eq!(m.position(ue, 5_000_000), [505.0, 0.0, 0.0]);
        m.beacon(bs, 0, 100, b"x", 0).unwrap();
        let near = rx(&drain(&mut m, ue, 1_000_000))[0].rssi_dbm;
        let _ = drain(&mut m, ue, 9_000_000);
        let far = rx(&drain(&mut m, ue, 10_000_000)).last().unwrap().rssi_dbm;
        assert!(near - far > 40.0, "{} vs {}", near, far);
        m.set_on(bs, false);
        assert!(rx(&drain(&mut m, ue, 11_000_000)).is_empty(), "powered off");
    }
}
