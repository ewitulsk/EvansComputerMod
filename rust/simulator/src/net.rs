//! Cable segments with the same wire semantics as the Java `NetworkHub` +
//! `CableNetworkManager`:
//!
//! - every frame a NIC transmits is offered to every *other* NIC on the same
//!   segment; each receiving NIC filters: its own MAC, broadcast, any
//!   multicast (I/G bit), or everything when promiscuous;
//! - a NIC that is administratively down neither sends nor receives (and
//!   its receive queue is cleared when it goes down);
//! - per-NIC receive queue of 256 frames, drop-oldest;
//! - one coalesced IRQ_NETWORK pending per computer (a flag here; the host
//!   loop clears it right before delivering the interrupt).
//!
//! Additions the simulator needs: explicit topology (2-member links and
//! multi-member hub segments), cable pull (link down) that really stops
//! delivery and drops carrier, per-segment fault injection with a seeded
//! PRNG, per-segment frame counters and pcap capture.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};
use std::path::Path;

use crate::pcap::PcapWriter;
use crate::util::Rng;

pub const RX_QUEUE_CAP: usize = 256;
pub const PCAP_QUEUE_CAP: usize = 256;

#[derive(Clone, Debug, Default)]
pub struct Faults {
    /// Percent of transmissions dropped.
    pub drop_pct: f64,
    /// One-way propagation delay in ms.
    pub delay_ms: i64,
    /// Percent of transmissions delivered twice.
    pub dup_pct: f64,
    /// Percent of transmissions held back by `reorder_ms` extra, so later
    /// frames overtake them.
    pub reorder_pct: f64,
    pub reorder_ms: i64,
}

impl Faults {
    pub fn is_clean(&self) -> bool {
        self.drop_pct <= 0.0 && self.delay_ms <= 0 && self.dup_pct <= 0.0 && self.reorder_pct <= 0.0
    }

    pub fn describe(&self) -> String {
        format!(
            "drop={}% delay={}ms dup={}% reorder={}%/{}ms",
            self.drop_pct, self.delay_ms, self.dup_pct, self.reorder_pct, self.reorder_ms
        )
    }
}

pub struct Nic {
    pub node: usize,
    pub iface: usize,
    pub mac: [u8; 6],
    pub admin_up: bool,
    pub promisc: bool,
    pub rxq: VecDeque<Vec<u8>>,
    pub seg: Option<usize>,
    /// Cable plugged in (only meaningful for members of multi-member
    /// segments; 2-member links are pulled as a whole).
    pub attached: bool,
    pub pcap_on: bool,
    pub pcapq: VecDeque<Vec<u8>>,
    pub tx_frames: u64,
    pub rx_frames: u64,
    pub rx_overflow: u64,
}

impl Nic {
    fn accepts(&self, frame: &[u8]) -> bool {
        if self.promisc {
            return true;
        }
        if frame[0] & 0x01 != 0 {
            return true;
        }
        frame[..6] == self.mac
    }
}

pub struct Segment {
    pub name: String,
    pub members: Vec<usize>,
    pub up: bool,
    pub faults: Faults,
    rng: Rng,
    /// Transmissions put onto this segment (before fault injection).
    pub frames: u64,
    pub bytes: u64,
    pub dropped: u64,
    pub pcap: Option<PcapWriter>,
}

struct Delayed {
    seg: usize,
    src: usize,
    frame: Vec<u8>,
}

pub struct Network {
    pub nics: Vec<Nic>,
    pub node_names: Vec<String>,
    node_nics: Vec<Vec<usize>>,
    pub segs: Vec<Segment>,
    delayed: BinaryHeap<Reverse<(i64, u64, usize)>>,
    delayed_frames: std::collections::HashMap<u64, Delayed>,
    seq: u64,
    irq: Vec<bool>,
    seed: u64,
}

pub fn iface_name(i: usize) -> String {
    format!("eth{}", i)
}

impl Network {
    pub fn new(seed: u64) -> Self {
        Network {
            nics: Vec::new(),
            node_names: Vec::new(),
            node_nics: Vec::new(),
            segs: Vec::new(),
            delayed: BinaryHeap::new(),
            delayed_frames: Default::default(),
            seq: 0,
            irq: Vec::new(),
            seed,
        }
    }

    /// Register a computer with `ifaces` NICs. MACs are deterministic:
    /// `02:<iface>:5e:00:<node hi>:<node lo+1>` (locally administered,
    /// iface in byte 1 like the Java `deriveMac`).
    pub fn add_node(&mut self, name: &str, ifaces: usize) -> usize {
        let node = self.node_nics.len();
        let mut list = Vec::with_capacity(ifaces);
        for i in 0..ifaces {
            let n = node + 1;
            let mac = [0x02, i as u8, 0x5e, 0x00, (n >> 8) as u8, n as u8];
            list.push(self.nics.len());
            self.nics.push(Nic {
                node,
                iface: i,
                mac,
                admin_up: true,
                promisc: false,
                rxq: VecDeque::new(),
                seg: None,
                attached: true,
                pcap_on: false,
                pcapq: VecDeque::new(),
                tx_frames: 0,
                rx_frames: 0,
                rx_overflow: 0,
            });
        }
        self.node_nics.push(list);
        self.node_names.push(name.to_string());
        self.irq.push(false);
        node
    }

    pub fn node_by_name(&self, name: &str) -> Option<usize> {
        self.node_names.iter().position(|n| n == name)
    }

    pub fn iface_count(&self, node: usize) -> usize {
        self.node_nics.get(node).map(|v| v.len()).unwrap_or(0)
    }

    pub fn nic(&self, node: usize, iface: usize) -> Option<usize> {
        self.node_nics.get(node)?.get(iface).copied()
    }

    /// Resolve "node:ethN" (or "node:N").
    pub fn endpoint(&self, spec: &str) -> Result<usize, String> {
        let (n, i) = spec.split_once(':').ok_or_else(|| format!("endpoint '{}' must be node:ethN", spec))?;
        let node = self.node_by_name(n.trim()).ok_or_else(|| format!("unknown node '{}' in '{}'", n, spec))?;
        let i = i.trim();
        let idx: usize = i
            .strip_prefix("eth")
            .unwrap_or(i)
            .parse()
            .map_err(|_| format!("bad interface '{}' in '{}' (use ethN)", i, spec))?;
        self.nic(node, idx).ok_or_else(|| {
            format!("{} has {} interfaces; '{}' does not exist", n, self.iface_count(node), spec)
        })
    }

    pub fn endpoint_name(&self, nic: usize) -> String {
        let n = &self.nics[nic];
        format!("{}:{}", self.node_names[n.node], iface_name(n.iface))
    }

    pub fn add_segment(&mut self, name: Option<String>, members: &[usize], faults: Faults) -> Result<usize, String> {
        if members.len() < 2 {
            return Err("a segment needs at least 2 members".into());
        }
        for (k, &m) in members.iter().enumerate() {
            if members[..k].contains(&m) {
                return Err(format!("{} listed twice in one segment", self.endpoint_name(m)));
            }
            if let Some(s) = self.nics[m].seg {
                return Err(format!(
                    "{} is already cabled to segment '{}' (one cable per face)",
                    self.endpoint_name(m),
                    self.segs[s].name
                ));
            }
        }
        let id = self.segs.len();
        let name = name.unwrap_or_else(|| members.iter().map(|&m| self.endpoint_name(m)).collect::<Vec<_>>().join("--"));
        for &m in members {
            self.nics[m].seg = Some(id);
        }
        let rng = Rng::derive(self.seed, &format!("segment:{}", name));
        self.segs.push(Segment {
            name,
            members: members.to_vec(),
            up: true,
            faults,
            rng,
            frames: 0,
            bytes: 0,
            dropped: 0,
            pcap: None,
        });
        Ok(id)
    }

    /// Segment by name or by any member endpoint.
    pub fn segment(&self, spec: &str) -> Result<usize, String> {
        if let Some(i) = self.segs.iter().position(|s| s.name == spec) {
            return Ok(i);
        }
        let nic = self.endpoint(spec)?;
        self.nics[nic].seg.ok_or_else(|| format!("{} is not cabled to anything", spec))
    }

    // ------------------------------------------------------------ state

    /// Carrier as the kernel sees it (`net_get_link_state`): the face is
    /// administratively up and its cable is plugged into a live segment.
    /// (Java reports carrier whenever a cable block sits on the face; the
    /// simulator additionally drops carrier when the cable is pulled.)
    pub fn carrier(&self, nic: usize) -> bool {
        let n = &self.nics[nic];
        n.admin_up && n.attached && n.seg.is_some_and(|s| self.segs[s].up)
    }

    pub fn set_admin(&mut self, nic: usize, up: bool) {
        let n = &mut self.nics[nic];
        n.admin_up = up;
        if !up {
            n.rxq.clear();
        }
    }

    pub fn set_promisc(&mut self, nic: usize, on: bool) {
        self.nics[nic].promisc = on;
    }

    pub fn set_pcap_mirror(&mut self, nic: usize, on: bool) {
        let n = &mut self.nics[nic];
        n.pcap_on = on;
        if !on {
            n.pcapq.clear();
        }
    }

    pub fn pcap_mirror_rx(&mut self, nic: usize) -> Option<Vec<u8>> {
        self.nics[nic].pcapq.pop_front()
    }

    /// Pull or re-plug a cable. For a 2-member link the whole link goes
    /// down (both ends lose carrier); for a hub segment only that member is
    /// detached.
    pub fn set_cable(&mut self, spec: &str, up: bool) -> Result<String, String> {
        if let Some(s) = self.segs.iter().position(|s| s.name == spec) {
            self.segs[s].up = up;
            return Ok(format!("segment {}", self.segs[s].name));
        }
        let nic = self.endpoint(spec)?;
        let s = self.nics[nic].seg.ok_or_else(|| format!("{} is not cabled to anything", spec))?;
        if self.segs[s].members.len() == 2 {
            self.segs[s].up = up;
            Ok(format!("link {}", self.segs[s].name))
        } else {
            self.nics[nic].attached = up;
            Ok(format!("{} on segment {}", spec, self.segs[s].name))
        }
    }

    pub fn set_faults(&mut self, seg: usize, f: Faults) {
        self.segs[seg].faults = f;
    }

    pub fn start_pcap(&mut self, seg: usize, path: &Path) -> Result<(), String> {
        let w = PcapWriter::create(path).map_err(|e| format!("pcap {}: {}", path.display(), e))?;
        self.segs[seg].pcap = Some(w);
        Ok(())
    }

    pub fn stop_pcap(&mut self, seg: usize) -> Option<(std::path::PathBuf, u64)> {
        self.segs[seg].pcap.take().map(|w| (w.path.clone(), w.records))
    }

    // ------------------------------------------------------------ I/O

    /// `net_tx_frame_on`. Returns false only for invalid frames.
    pub fn transmit(&mut self, nic: usize, frame: &[u8], now: i64) -> bool {
        if frame.len() < 14 || frame.len() > 1518 {
            return false;
        }
        let n = &mut self.nics[nic];
        if !n.admin_up {
            return true;
        }
        n.tx_frames += 1;
        // tcpdump on the sender sees its own TX.
        if n.pcap_on {
            if n.pcapq.len() >= PCAP_QUEUE_CAP {
                n.pcapq.pop_front();
            }
            n.pcapq.push_back(frame.to_vec());
        }
        let Some(s) = n.seg else { return true };
        if !n.attached || !self.segs[s].up {
            return true;
        }
        let seg = &mut self.segs[s];
        seg.frames += 1;
        seg.bytes += frame.len() as u64;
        if let Some(p) = seg.pcap.as_mut() {
            p.write(now, frame);
        }
        if seg.faults.is_clean() {
            self.deliver(s, nic, frame);
            return true;
        }
        let f = seg.faults.clone();
        if seg.rng.chance(f.drop_pct) {
            seg.dropped += 1;
            return true;
        }
        let copies = if seg.rng.chance(f.dup_pct) { 2 } else { 1 };
        let mut delays = Vec::with_capacity(copies);
        for _ in 0..copies {
            let mut d = f.delay_ms.max(0);
            if seg.rng.chance(f.reorder_pct) {
                d += f.reorder_ms.max(1);
            }
            delays.push(d);
        }
        for d in delays {
            if d == 0 {
                self.deliver(s, nic, frame);
            } else {
                self.seq += 1;
                let id = self.seq;
                self.delayed.push(Reverse((now + d, id, s)));
                self.delayed_frames.insert(id, Delayed { seg: s, src: nic, frame: frame.to_vec() });
            }
        }
        true
    }

    fn deliver(&mut self, s: usize, src: usize, frame: &[u8]) {
        if !self.segs[s].up {
            return;
        }
        for k in 0..self.segs[s].members.len() {
            let m = self.segs[s].members[k];
            if m == src {
                continue;
            }
            let n = &mut self.nics[m];
            if !n.attached || !n.admin_up || !n.accepts(frame) {
                continue;
            }
            if n.rxq.len() >= RX_QUEUE_CAP {
                n.rxq.pop_front();
                n.rx_overflow += 1;
            }
            n.rxq.push_back(frame.to_vec());
            n.rx_frames += 1;
            if n.pcap_on {
                if n.pcapq.len() >= PCAP_QUEUE_CAP {
                    n.pcapq.pop_front();
                }
                n.pcapq.push_back(frame.to_vec());
            }
            self.irq[n.node] = true;
        }
    }

    /// Deliver delayed frames whose time has come.
    pub fn deliver_due(&mut self, now: i64) -> bool {
        let mut any = false;
        while let Some(Reverse((t, id, _))) = self.delayed.peek().copied() {
            if t > now {
                break;
            }
            self.delayed.pop();
            if let Some(d) = self.delayed_frames.remove(&id) {
                // A cable pulled while the frame was in flight loses it.
                let src_ok = self.nics[d.src].attached;
                if src_ok {
                    self.deliver(d.seg, d.src, &d.frame);
                }
                any = true;
            }
        }
        any
    }

    pub fn next_delivery(&self) -> Option<i64> {
        self.delayed.peek().map(|Reverse((t, _, _))| *t)
    }

    /// `net_rx_frame_any`: round-robin across the node's NICs.
    pub fn rx_any(&mut self, node: usize, rr: &mut usize) -> Option<(usize, Vec<u8>)> {
        let list = self.node_nics.get(node)?;
        let n = list.len();
        for k in 0..n {
            let i = (*rr + k) % n;
            let nic = list[i];
            if let Some(f) = self.nics[nic].rxq.pop_front() {
                *rr = (i + 1) % n;
                return Some((i, f));
            }
        }
        None
    }

    /// Take (and clear) the node's coalesced IRQ_NETWORK flag.
    pub fn take_irq(&mut self, node: usize) -> bool {
        std::mem::replace(&mut self.irq[node], false)
    }

    pub fn has_rx(&self, node: usize) -> bool {
        self.node_nics[node].iter().any(|&n| !self.nics[n].rxq.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(dst: [u8; 6], src: [u8; 6]) -> Vec<u8> {
        let mut f = vec![0u8; 60];
        f[..6].copy_from_slice(&dst);
        f[6..12].copy_from_slice(&src);
        f
    }

    fn three_hosts_on_hub() -> (Network, [usize; 3]) {
        let mut n = Network::new(1);
        for name in ["a", "b", "c"] {
            n.add_node(name, 2);
        }
        let a = n.endpoint("a:eth0").unwrap();
        let b = n.endpoint("b:eth0").unwrap();
        let c = n.endpoint("c:eth0").unwrap();
        n.add_segment(Some("hub".into()), &[a, b, c], Faults::default()).unwrap();
        (n, [a, b, c])
    }

    #[test]
    fn wire_semantics_filter_at_receiver() {
        let (mut n, [a, b, c]) = three_hosts_on_hub();
        let (ma, mb) = (n.nics[a].mac, n.nics[b].mac);
        // Unicast to b: only b accepts; c filters it out.
        n.transmit(a, &frame(mb, ma), 0);
        assert_eq!(n.nics[b].rxq.len(), 1);
        assert_eq!(n.nics[c].rxq.len(), 0);
        assert_eq!(n.nics[a].rxq.len(), 0, "sender never hears itself");
        // Promiscuous c sees unicast for b.
        n.set_promisc(c, true);
        n.transmit(a, &frame(mb, ma), 0);
        assert_eq!(n.nics[c].rxq.len(), 1);
        // Multicast (01:80:c2:..) reaches everyone.
        n.set_promisc(c, false);
        n.transmit(a, &frame([0x01, 0x80, 0xc2, 0, 0, 0], ma), 0);
        assert_eq!(n.nics[c].rxq.len(), 2);
        assert!(n.take_irq(1) && n.take_irq(2) && !n.take_irq(0));
        assert!(!n.take_irq(1), "IRQ flag is coalesced and cleared");
        assert_eq!(n.segs[0].frames, 3);
    }

    #[test]
    fn admin_down_and_cable_pull() {
        let (mut n, [a, b, _c]) = three_hosts_on_hub();
        let (ma, mb) = (n.nics[a].mac, n.nics[b].mac);
        n.set_admin(b, false);
        assert!(!n.carrier(b));
        n.transmit(a, &frame(mb, ma), 0);
        assert!(n.nics[b].rxq.is_empty());
        n.set_admin(b, true);
        n.set_cable("b:eth0", false).unwrap();
        assert!(!n.carrier(b) && n.carrier(a));
        n.transmit(a, &frame(mb, ma), 0);
        assert!(n.nics[b].rxq.is_empty());
        // A lone NIC has no carrier.
        let lone = n.endpoint("a:eth1").unwrap();
        assert!(!n.carrier(lone));
    }

    #[test]
    fn two_member_link_pull_drops_both_ends() {
        let mut n = Network::new(1);
        n.add_node("x", 1);
        n.add_node("y", 1);
        let (x, y) = (n.endpoint("x:eth0").unwrap(), n.endpoint("y:0").unwrap());
        n.add_segment(None, &[x, y], Faults::default()).unwrap();
        n.set_cable("x:eth0", false).unwrap();
        assert!(!n.carrier(x) && !n.carrier(y));
        n.set_cable("y:eth0", true).unwrap();
        assert!(n.carrier(x) && n.carrier(y));
        assert!(n.add_segment(None, &[x, y], Faults::default()).is_err(), "one cable per face");
    }

    #[test]
    fn queue_drops_oldest() {
        let (mut n, [a, b, _]) = three_hosts_on_hub();
        let (ma, mb) = (n.nics[a].mac, n.nics[b].mac);
        for i in 0..300u32 {
            let mut f = frame(mb, ma);
            f[20..24].copy_from_slice(&i.to_le_bytes());
            n.transmit(a, &f, 0);
        }
        assert_eq!(n.nics[b].rxq.len(), RX_QUEUE_CAP);
        assert_eq!(&n.nics[b].rxq[0][20..24], &44u32.to_le_bytes());
    }

    #[test]
    fn faults_are_seeded_and_delay_orders_delivery() {
        let mut n = Network::new(9);
        n.add_node("x", 1);
        n.add_node("y", 1);
        let (x, y) = (n.endpoint("x:eth0").unwrap(), n.endpoint("y:eth0").unwrap());
        let f = Faults { delay_ms: 5, drop_pct: 50.0, ..Default::default() };
        n.add_segment(None, &[x, y], f).unwrap();
        let my = n.nics[y].mac;
        for _ in 0..100 {
            n.transmit(x, &frame(my, [2; 6]), 0);
        }
        assert!(n.nics[y].rxq.is_empty());
        assert_eq!(n.next_delivery(), Some(5));
        n.deliver_due(5);
        let got = n.nics[y].rxq.len();
        assert!((30..70).contains(&got), "got {}", got);
        assert_eq!(got as u64 + n.segs[0].dropped, 100);
    }

    #[test]
    fn rx_any_is_round_robin() {
        let mut n = Network::new(1);
        n.add_node("s", 2);
        n.add_node("p", 1);
        n.add_node("q", 1);
        let (s0, s1) = (n.endpoint("s:eth0").unwrap(), n.endpoint("s:eth1").unwrap());
        let (p, q) = (n.endpoint("p:eth0").unwrap(), n.endpoint("q:eth0").unwrap());
        n.add_segment(None, &[s0, p], Faults::default()).unwrap();
        n.add_segment(None, &[s1, q], Faults::default()).unwrap();
        for _ in 0..3 {
            n.transmit(p, &frame([0xff; 6], [1; 6]), 0);
            n.transmit(q, &frame([0xff; 6], [2; 6]), 0);
        }
        let mut rr = 0;
        let order: Vec<usize> = (0..6).map(|_| n.rx_any(0, &mut rr).unwrap().0).collect();
        assert_eq!(order, vec![0, 1, 0, 1, 0, 1]);
        assert!(n.rx_any(0, &mut rr).is_none());
    }
}
