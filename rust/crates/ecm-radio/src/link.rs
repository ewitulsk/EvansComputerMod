//! The `radio0` link layer: the kernel sees an Ethernet interface; on the air
//! every frame is an AX.25 UI frame (what a KISS TNC sends), as in Linux
//! `kissattach` + IP over AX.25.
//!
//! - IPv4 rides in UI frames with PID `0xCC`, ARP with PID `0xCD`. The
//!   Ethernet header is not sent: it is rebuilt on receive.
//! - Each station's MAC is derived from its callsign ([`mac_for`]), so the
//!   source MAC of a received frame follows from the AX.25 source address.
//!   Unicast destinations are looked up in a table learned from received
//!   frames; broadcasts (and unknown destinations) go to `QST`.
//! - Packets longer than [`MAX_INFO`] are split with AX.25 2.2 segmentation
//!   (PID `0x08`, first byte = first-flag | segments remaining).
//! - TCP SYNs have their MSS clamped to fit [`MAX_INFO`], so TCP segments
//!   normally fit one frame.
//! - Frames are wrapped in KISS (`FEND 0x00 ... FEND`) between the link and
//!   the modem, so the TNC half can be swapped for a real KISS device.

use ecm_dsp::coding::ax25::{Address, UiFrame};
use ecm_dsp::coding::crc::crc32;
use ecm_dsp::coding::kiss::{KissDecoder, KissFrame};
use std::collections::{BTreeMap, VecDeque};

use crate::modem::{FmPacketRx, FmPacketTx};
use ecm_dsp::C32;

pub const PID_IP: u8 = 0xCC;
pub const PID_ARP: u8 = 0xCD;
pub const PID_SEGMENT: u8 = 0x08;
/// AX.25 paclen: the most info bytes in one frame.
pub const MAX_INFO: usize = 256;
/// TCP MSS that keeps a segment (20 IP + 20 TCP + data) inside one frame.
pub const CLAMPED_MSS: u16 = (MAX_INFO - 40) as u16;
pub const ETH_IPV4: u16 = 0x0800;
pub const ETH_ARP: u16 = 0x0806;
pub const BROADCAST: [u8; 6] = [0xff; 6];

/// The MAC a station with this callsign uses on `radio0`: locally
/// administered, from a CRC of `CALL-SSID`.
pub fn mac_for(a: &Address) -> [u8; 6] {
    let c = crc32(format!("{}-{}", a.call, a.ssid).as_bytes()).to_be_bytes();
    [0x02, 0xAC, c[0], c[1], c[2], c[3]]
}

pub fn qst() -> Address {
    Address::new("QST", 0).expect("QST is a valid callsign")
}

/// Counters for `radiod` status lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkStats {
    pub tx_packets: u64,
    pub tx_frames: u64,
    pub rx_frames: u64,
    pub rx_packets: u64,
    pub dropped: u64,
    pub mss_clamped: u64,
}

struct Reasm {
    pid: u8,
    remaining: u8,
    data: Vec<u8>,
}

/// One station's link state.
pub struct Link {
    pub me: Address,
    pub mac: [u8; 6],
    peers: BTreeMap<[u8; 6], Address>,
    reasm: BTreeMap<String, Reasm>,
    pub stats: LinkStats,
}

impl Link {
    pub fn new(me: Address) -> Link {
        let mac = mac_for(&me);
        Link { me, mac, peers: BTreeMap::new(), reasm: BTreeMap::new(), stats: LinkStats::default() }
    }

    /// Stations heard so far (MAC -> callsign).
    pub fn peers(&self) -> impl Iterator<Item = (&[u8; 6], &Address)> {
        self.peers.iter()
    }

    /// An Ethernet frame from the kernel -> AX.25 UI frame bodies (no FCS).
    /// Non-IP/ARP frames are dropped (empty result).
    pub fn encode(&mut self, eth: &[u8]) -> Vec<Vec<u8>> {
        if eth.len() < 14 {
            self.stats.dropped += 1;
            return Vec::new();
        }
        let dst: [u8; 6] = eth[0..6].try_into().unwrap();
        let ethertype = u16::from_be_bytes([eth[12], eth[13]]);
        let pid = match ethertype {
            ETH_IPV4 => PID_IP,
            ETH_ARP => PID_ARP,
            _ => {
                self.stats.dropped += 1;
                return Vec::new();
            }
        };
        let mut payload = eth[14..].to_vec();
        if pid == PID_IP {
            payload.truncate(ip_total_len(&payload).unwrap_or(payload.len()));
            if clamp_mss(&mut payload, CLAMPED_MSS) {
                self.stats.mss_clamped += 1;
            }
        }
        let dest = if dst == BROADCAST || dst[0] & 1 == 1 {
            qst()
        } else {
            self.peers.get(&dst).cloned().unwrap_or_else(qst)
        };
        let mut out = Vec::new();
        if payload.len() <= MAX_INFO {
            out.push(self.ui(dest, pid, payload));
        } else {
            // AX.25 2.2 segmentation: first segment carries the original PID.
            let mut chunks: Vec<Vec<u8>> = Vec::new();
            let mut first = vec![pid];
            let take = (MAX_INFO - 2).min(payload.len());
            first.extend_from_slice(&payload[..take]);
            chunks.push(first);
            for c in payload[take..].chunks(MAX_INFO - 1) {
                chunks.push(c.to_vec());
            }
            if chunks.len() > 128 {
                self.stats.dropped += 1;
                return Vec::new();
            }
            let n = chunks.len();
            for (i, c) in chunks.into_iter().enumerate() {
                let remaining = (n - 1 - i) as u8;
                let mut info = vec![if i == 0 { 0x80 | remaining } else { remaining }];
                info.extend(c);
                out.push(self.ui(dest.clone(), PID_SEGMENT, info));
            }
        }
        self.stats.tx_packets += 1;
        self.stats.tx_frames += out.len() as u64;
        out
    }

    fn ui(&self, dest: Address, pid: u8, info: Vec<u8>) -> Vec<u8> {
        let mut f = UiFrame::new(dest, self.me.clone(), &[]);
        f.pid = pid;
        f.info = info;
        f.encode().expect("no digipeaters")
    }

    /// An AX.25 frame body from the air -> an Ethernet frame for the kernel,
    /// if it is for this station and complete.
    pub fn decode(&mut self, body: &[u8]) -> Option<Vec<u8>> {
        let f = UiFrame::parse(body).ok()?;
        self.stats.rx_frames += 1;
        if f.src.call == self.me.call && f.src.ssid == self.me.ssid {
            return None; // our own transmission
        }
        let to_me = f.dest.call == self.me.call && f.dest.ssid == self.me.ssid;
        let bcast = matches!(f.dest.call.as_str(), "QST" | "CQ");
        if !to_me && !bcast {
            return None;
        }
        let src_mac = mac_for(&f.src);
        let src_key = f.src.to_string();
        self.peers.insert(src_mac, Address { flag: false, ..f.src.clone() });
        let (pid, mut payload) = if f.pid == PID_SEGMENT {
            let (&hdr, rest) = f.info.split_first()?;
            let remaining = hdr & 0x7f;
            if hdr & 0x80 != 0 {
                let (&pid, data) = rest.split_first()?;
                self.reasm.insert(src_key.clone(), Reasm { pid, remaining, data: data.to_vec() });
            } else {
                let ok = self.reasm.get(&src_key).is_some_and(|r| r.remaining == remaining + 1);
                if !ok {
                    self.reasm.remove(&src_key);
                    self.stats.dropped += 1;
                    return None;
                }
                let r = self.reasm.get_mut(&src_key).unwrap();
                r.remaining = remaining;
                r.data.extend_from_slice(rest);
            }
            if remaining != 0 {
                return None;
            }
            let r = self.reasm.remove(&src_key)?;
            (r.pid, r.data)
        } else {
            (f.pid, f.info)
        };
        let ethertype = match pid {
            PID_IP => ETH_IPV4,
            PID_ARP => ETH_ARP,
            _ => {
                self.stats.dropped += 1;
                return None;
            }
        };
        if pid == PID_IP && clamp_mss(&mut payload, CLAMPED_MSS) {
            self.stats.mss_clamped += 1;
        }
        let mut eth = Vec::with_capacity(14 + payload.len());
        eth.extend_from_slice(if to_me { &self.mac } else { &BROADCAST });
        eth.extend_from_slice(&src_mac);
        eth.extend_from_slice(&ethertype.to_be_bytes());
        eth.extend(payload);
        self.stats.rx_packets += 1;
        Some(eth)
    }
}

/// Wrap an AX.25 body as a KISS data frame (port 0).
pub fn kiss_wrap(body: &[u8]) -> Vec<u8> {
    KissFrame::data(0, body).encode()
}

/// Split a KISS byte stream back into AX.25 bodies (data frames only).
pub struct KissStream(KissDecoder);

impl Default for KissStream {
    fn default() -> Self {
        Self::new()
    }
}

impl KissStream {
    pub fn new() -> KissStream {
        KissStream(KissDecoder::new())
    }
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.0
            .push_bytes(bytes)
            .into_iter()
            .filter(|f| f.command == ecm_dsp::coding::kiss::cmd::DATA)
            .map(|f| f.data)
            .collect()
    }
}

/// The whole `radiod` data path for one station, minus the device I/O:
/// kernel Ethernet frames -> [`Link`] (AX.25) -> KISS -> AFSK1200 over NBFM
/// IQ, and back. Frames queued between transmissions go out as one burst
/// (one TX delay preamble, like a KISS TNC with a full queue).
pub struct Tnc {
    link: Link,
    tx: FmPacketTx,
    rx: FmPacketRx,
    to_modem: KissStream,
    from_modem: KissStream,
    queue: VecDeque<Vec<u8>>,
    txdelay_flags: usize,
}

impl Tnc {
    pub fn new(me: Address, rate: f64) -> Tnc {
        Tnc {
            link: Link::new(me),
            tx: FmPacketTx::new(rate as f32),
            rx: FmPacketRx::new(rate as f32),
            to_modem: KissStream::new(),
            from_modem: KissStream::new(),
            queue: VecDeque::new(),
            txdelay_flags: 24,
        }
    }

    /// TX delay (keying time before the first frame of a burst).
    pub fn set_txdelay_ms(&mut self, ms: u32) {
        // one flag = 8 bits at 1200 baud = 6.67 ms
        self.txdelay_flags = ((ms as f64 / 6.667).ceil() as usize).max(2);
    }

    pub fn mac(&self) -> [u8; 6] {
        self.link.mac
    }

    pub fn link(&self) -> &Link {
        &self.link
    }

    /// Frames waiting to be transmitted.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// A frame the kernel sent on the interface.
    pub fn from_kernel(&mut self, eth: &[u8]) {
        for body in self.link.encode(eth) {
            // Through the KISS framing a hardware TNC would see.
            for f in self.to_modem.push(&kiss_wrap(&body)) {
                // A retry of a frame still waiting (ARP while the channel is
                // busy) adds nothing but airtime.
                if self.queue.contains(&f) {
                    continue;
                }
                if self.queue.len() >= 64 {
                    self.queue.pop_front();
                    self.link.stats.dropped += 1;
                }
                self.queue.push_back(f);
            }
        }
    }

    /// Everything queued, modulated as one transmission (None if idle).
    pub fn take_burst(&mut self) -> Option<Vec<C32>> {
        if self.queue.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        let mut first = true;
        while let Some(f) = self.queue.pop_front() {
            self.tx.set_preamble_flags(if first { self.txdelay_flags } else { 2 });
            first = false;
            out.extend(self.tx.send(&f));
        }
        Some(out)
    }

    /// Received IQ -> Ethernet frames for the kernel.
    pub fn from_air(&mut self, iq: &[C32]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for body in self.rx.push(iq) {
            for f in self.from_modem.push(&kiss_wrap(&body)) {
                if let Some(eth) = self.link.decode(&f) {
                    out.push(eth);
                }
            }
        }
        out
    }
}

/// Carrier sense for a half-duplex packet station: is someone on the channel?
///
/// It compares each received block's power in absolute terms (dBFS minus the receiver gain)
/// against a tracked noise floor. The floor follows drops at once and rises 10 dB/s (0.2 dB
/// per 20 ms block), so a burst can't pin it up for long.
///
/// A block that is all zeros (the noise sits below one ADC step, as with a fixed low gain and
/// cs16 samples) is a measurement too: the channel is quieter than one quantisation step. It
/// sets the floor to that step's level instead of being skipped; skipping it left the floor
/// unset until the first signal, which then *became* the floor, so a station never sensed the
/// other one's carrier and keyed up over it (radio0 pings lost to ARP retries colliding with
/// the replies).
#[derive(Clone, Debug)]
pub struct CarrierSense {
    floor: f32,
    /// How far above the floor counts as a carrier, dB.
    pub threshold_db: f32,
}

/// One cs16 quantisation step, as a power in dBFS (20 log10(1/32768)).
pub const CS16_STEP_DBFS: f32 = -90.3;

impl Default for CarrierSense {
    fn default() -> Self {
        CarrierSense { floor: f32::INFINITY, threshold_db: 10.0 }
    }
}

impl CarrierSense {
    /// Feed one block's mean power (linear, full scale = 1) at receiver gain `gain_db`;
    /// returns true if a carrier is on the channel.
    pub fn update(&mut self, mean_power: f32, gain_db: f32) -> bool {
        let dbfs = if mean_power > 0.0 { ecm_dsp::complex::to_db(mean_power) } else { CS16_STEP_DBFS };
        if !dbfs.is_finite() {
            return false;
        }
        let p = dbfs.max(CS16_STEP_DBFS) - gain_db;
        self.floor = if p < self.floor { p } else { self.floor + 0.2 };
        p > self.floor + self.threshold_db
    }

    /// The current noise floor (dBm + a constant), or +inf before any block.
    pub fn floor(&self) -> f32 {
        self.floor
    }
}

fn ip_total_len(ip: &[u8]) -> Option<usize> {
    if ip.len() < 20 || ip[0] >> 4 != 4 {
        return None;
    }
    let t = u16::from_be_bytes([ip[2], ip[3]]) as usize;
    (t >= 20 && t <= ip.len()).then_some(t)
}

fn checksum(data: &[u8], mut sum: u32) -> u16 {
    let mut i = 0;
    while i + 1 < data.len() {
        sum += u16::from_be_bytes([data[i], data[i + 1]]) as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// If `ip` is an IPv4 TCP SYN with an MSS option above `mss`, lower it and
/// fix the TCP checksum. Returns whether it changed anything.
pub fn clamp_mss(ip: &mut [u8], mss: u16) -> bool {
    let Some(total) = ip_total_len(ip) else { return false };
    let ihl = ((ip[0] & 0x0f) as usize) * 4;
    if ip[9] != 6 || ihl < 20 || total < ihl + 20 {
        return false;
    }
    // Fragments other than the first don't carry the TCP header.
    if u16::from_be_bytes([ip[6], ip[7]]) & 0x1fff != 0 {
        return false;
    }
    let tcp_len = total - ihl;
    let doff = ((ip[ihl + 12] >> 4) as usize) * 4;
    let flags = ip[ihl + 13];
    if flags & 0x02 == 0 || doff < 20 || doff > tcp_len {
        return false;
    }
    let mut i = ihl + 20;
    let end = ihl + doff;
    let mut changed = false;
    while i < end {
        match ip[i] {
            0 => break,
            1 => i += 1,
            kind => {
                let Some(&len) = ip.get(i + 1) else { break };
                let len = len as usize;
                if len < 2 || i + len > end {
                    break;
                }
                if kind == 2 && len == 4 {
                    let cur = u16::from_be_bytes([ip[i + 2], ip[i + 3]]);
                    if cur > mss {
                        ip[i + 2..i + 4].copy_from_slice(&mss.to_be_bytes());
                        changed = true;
                    }
                }
                i += len;
            }
        }
    }
    if changed {
        ip[ihl + 16] = 0;
        ip[ihl + 17] = 0;
        let mut pseudo = 0u32;
        for k in (12..20).step_by(2) {
            pseudo += u16::from_be_bytes([ip[k], ip[k + 1]]) as u32;
        }
        pseudo += 6 + tcp_len as u32;
        let c = checksum(&ip[ihl..total], pseudo);
        ip[ihl + 16..ihl + 18].copy_from_slice(&c.to_be_bytes());
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> Address {
        Address::parse(s).unwrap()
    }

    fn ipv4(proto: u8, src: [u8; 4], dst: [u8; 4], l4: &[u8]) -> Vec<u8> {
        let total = 20 + l4.len();
        let mut p = vec![0x45, 0, (total >> 8) as u8, total as u8, 0, 1, 0x40, 0, 64, proto, 0, 0];
        p.extend_from_slice(&src);
        p.extend_from_slice(&dst);
        let c = checksum(&p[..20], 0);
        p[10..12].copy_from_slice(&c.to_be_bytes());
        p.extend_from_slice(l4);
        p
    }

    fn eth(dst: [u8; 6], src: [u8; 6], ty: u16, payload: &[u8]) -> Vec<u8> {
        let mut e = dst.to_vec();
        e.extend_from_slice(&src);
        e.extend_from_slice(&ty.to_be_bytes());
        e.extend_from_slice(payload);
        e
    }

    fn tcp_checksum_ok(ip: &[u8]) -> bool {
        let ihl = ((ip[0] & 0xf) * 4) as usize;
        let total = u16::from_be_bytes([ip[2], ip[3]]) as usize;
        let mut pseudo = 0u32;
        for k in (12..20).step_by(2) {
            pseudo += u16::from_be_bytes([ip[k], ip[k + 1]]) as u32;
        }
        pseudo += 6 + (total - ihl) as u32;
        checksum(&ip[ihl..total], pseudo) == 0
    }

    /// IP packet -> AX.25 -> KISS -> (air) -> KISS -> AX.25 -> IP, both ways,
    /// with ARP learning the unicast callsign.
    #[test]
    fn ip_over_ax25_kiss_round_trip() {
        let mut a = Link::new(addr("N0CALL-1"));
        let mut b = Link::new(addr("N0CALL-2"));
        assert_ne!(a.mac, b.mac);
        // A broadcasts an ARP request -> QST.
        let arp = vec![0, 1, 8, 0, 6, 4, 0, 1, 1, 2, 3, 4, 5, 6, 10, 44, 0, 1, 0, 0, 0, 0, 0, 0, 10, 44, 0, 2];
        let frames = a.encode(&eth(BROADCAST, a.mac, ETH_ARP, &arp));
        assert_eq!(frames.len(), 1);
        let ui = UiFrame::parse(&frames[0]).unwrap();
        assert_eq!((ui.dest.call.as_str(), ui.src.to_string().as_str(), ui.pid), ("QST", "N0CALL-1", PID_ARP));
        let mut air = Vec::new();
        for f in &frames {
            air.extend(kiss_wrap(f));
        }
        let mut ks = KissStream::new();
        let bodies = ks.push(&air);
        let got = b.decode(&bodies[0]).unwrap();
        assert_eq!(got, eth(BROADCAST, a.mac, ETH_ARP, &arp));
        // B replies unicast to A's MAC: goes to N0CALL-1, not QST.
        let ping = ipv4(1, [10, 44, 0, 2], [10, 44, 0, 1], &[0, 0, 0xf7, 0xff, 0, 1, 0, 1]);
        let frames = b.encode(&eth(a.mac, b.mac, ETH_IPV4, &ping));
        let ui = UiFrame::parse(&frames[0]).unwrap();
        assert_eq!((ui.dest.to_string().as_str(), ui.pid), ("N0CALL-1", PID_IP));
        assert_eq!(ui.info, ping, "IP packet carried unchanged without the Ethernet header");
        let back = a.decode(&ks.push(&kiss_wrap(&frames[0]))[0]).unwrap();
        assert_eq!(back, eth(a.mac, b.mac, ETH_IPV4, &ping));
        // A third station ignores unicast between A and B.
        let mut c = Link::new(addr("N0CALL-3"));
        assert!(c.decode(&frames[0]).is_none());
        // A ignores its own echo.
        let echo = a.encode(&eth(BROADCAST, a.mac, ETH_ARP, &arp));
        assert!(a.decode(&echo[0]).is_none());
        // Non-IP ethertypes are dropped.
        assert!(a.encode(&eth(BROADCAST, a.mac, 0x86dd, &[0; 40])).is_empty());
    }

    #[test]
    fn large_packets_are_segmented_and_reassembled() {
        let mut a = Link::new(addr("AA1AA"));
        let mut b = Link::new(addr("BB2BB"));
        let udp: Vec<u8> = (0..1000u32).map(|i| (i * 31 % 251) as u8).collect();
        let pkt = ipv4(17, [10, 0, 0, 1], [10, 0, 0, 2], &udp);
        let frames = a.encode(&eth(BROADCAST, a.mac, ETH_IPV4, &pkt));
        assert_eq!(frames.len(), 5, "1020 bytes in 254 + 3x255 + 1 segments");
        let mut out = None;
        for (i, f) in frames.iter().enumerate() {
            let ui = UiFrame::parse(f).unwrap();
            assert_eq!(ui.pid, PID_SEGMENT);
            assert!(ui.info.len() <= MAX_INFO);
            assert!(f.len() <= crate::modem::MAX_FRAME);
            let r = b.decode(f);
            assert_eq!(r.is_some(), i == frames.len() - 1);
            out = out.or(r);
        }
        assert_eq!(out.unwrap(), eth(BROADCAST, a.mac, ETH_IPV4, &pkt));
        // A lost middle segment drops the packet; the next one still works.
        let frames = a.encode(&eth(BROADCAST, a.mac, ETH_IPV4, &pkt));
        assert!(b.decode(&frames[0]).is_none());
        assert!(b.decode(&frames[2]).is_none());
        assert!(b.decode(&frames[3]).is_none());
        let frames = a.encode(&eth(BROADCAST, a.mac, ETH_IPV4, &pkt));
        let last = frames.iter().map(|f| b.decode(f)).last().unwrap();
        assert!(last.is_some());
    }

    #[test]
    fn carrier_sense_works_when_the_noise_is_below_one_adc_step() {
        // Fixed gain 10 dB: the noise quantises to zero, then another station's burst arrives.
        let mut cs = CarrierSense::default();
        for _ in 0..50 {
            assert!(!cs.update(0.0, 10.0), "silence is not a carrier");
        }
        let burst = 10f32.powf(-12.0 / 10.0); // -12 dBFS
        assert!(cs.update(burst, 10.0), "the first block of a burst must read as a carrier");
        for i in 0..20 {
            assert!(cs.update(burst, 10.0), "the whole 0.4 s burst is a carrier (block {i})");
        }
        assert!(!cs.update(0.0, 10.0), "silence again");
        assert!(cs.update(burst, 10.0), "and the next burst is sensed too");
        // With real noise above one step (AGC), the floor is the noise.
        let mut agc = CarrierSense::default();
        let noise = 10f32.powf(-40.0 / 10.0);
        for _ in 0..10 {
            assert!(!agc.update(noise, 30.0));
        }
        assert!(agc.update(noise * 100.0, 30.0), "20 dB over the noise is a carrier");
    }

    #[test]
    fn syn_mss_is_clamped_with_valid_checksum() {
        // SYN with MSS 1460.
        let mut tcp = vec![0x30, 0x39, 0, 22, 0, 0, 0, 1, 0, 0, 0, 0, 0x60, 0x02, 0xff, 0xff, 0, 0, 0, 0];
        tcp.extend_from_slice(&[2, 4, 0x05, 0xb4]);
        let mut pkt = ipv4(6, [10, 44, 0, 1], [10, 44, 0, 2], &tcp);
        // fill in a valid TCP checksum first
        let ihl = 20;
        let mut pseudo = 0u32;
        for k in (12..20).step_by(2) {
            pseudo += u16::from_be_bytes([pkt[k], pkt[k + 1]]) as u32;
        }
        pseudo += 6 + tcp.len() as u32;
        let c = checksum(&pkt[ihl..], pseudo);
        pkt[ihl + 16..ihl + 18].copy_from_slice(&c.to_be_bytes());
        assert!(tcp_checksum_ok(&pkt));
        let mut a = Link::new(addr("N0CALL-1"));
        let frames = a.encode(&eth(BROADCAST, a.mac, ETH_IPV4, &pkt));
        let ui = UiFrame::parse(&frames[0]).unwrap();
        let mss = u16::from_be_bytes([ui.info[42], ui.info[43]]);
        assert_eq!(mss, CLAMPED_MSS);
        assert!(tcp_checksum_ok(&ui.info));
        assert_eq!(a.stats.mss_clamped, 1);
        // a non-SYN segment is untouched
        let mut ack = pkt.clone();
        ack[ihl + 13] = 0x10;
        assert!(!clamp_mss(&mut ack, 100));
    }
}
