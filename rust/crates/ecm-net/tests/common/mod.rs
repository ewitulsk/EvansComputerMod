//! In-process network harness: several `Stack`s wired through hub segments with a
//! virtual clock, optional seeded loss / reordering, and frame capture.
#![allow(dead_code)]

use ecm_net::eth::EthHeader;
use ecm_net::{Ipv4Addr, MacAddr, SocketAddr, Stack, StackConfig};

/// Deterministic PRNG (splitmix64).
#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next_u64() % n
        }
    }
    pub fn chance(&mut self, p: f64) -> bool {
        ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) < p
    }
    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next_u64() as u8).collect()
    }
}

pub fn mac(n: u8) -> MacAddr {
    MacAddr([0x02, 0, 0, 0, 0, n])
}

pub fn ip(s: &str) -> Ipv4Addr {
    Ipv4Addr::parse(s).unwrap()
}

pub fn sa(s: &str, port: u16) -> SocketAddr {
    SocketAddr::new(ip(s), port)
}

pub struct Segment {
    pub ends: Vec<(usize, usize)>,
    pub loss: f64,
    /// Probability that a frame gets 1..=5 ms of extra delay.
    pub reorder: f64,
    pub latency: i64,
    pub up: bool,
}

struct InFlight {
    due: i64,
    seq: u64,
    node: usize,
    iface: usize,
    frame: Vec<u8>,
}

/// A captured transmitted frame.
#[derive(Clone, Debug)]
pub struct Captured {
    pub time: i64,
    pub node: usize,
    pub iface: usize,
    pub frame: Vec<u8>,
}

impl Captured {
    pub fn eth(&self) -> EthHeader {
        EthHeader::parse(&self.frame).unwrap().0
    }
}

pub struct Net {
    pub stacks: Vec<Stack>,
    pub segments: Vec<Segment>,
    inflight: Vec<InFlight>,
    deadlines: Vec<Option<i64>>,
    pub now: i64,
    pub rng: Rng,
    seq: u64,
    pub capture: Vec<Captured>,
    pub capture_on: bool,
    pub delivered: u64,
    pub lost: u64,
}

impl Net {
    pub fn new(seed: u64) -> Self {
        Net {
            stacks: Vec::new(),
            segments: Vec::new(),
            inflight: Vec::new(),
            deadlines: Vec::new(),
            now: 1_000_000, // arbitrary non-zero start
            rng: Rng(seed ^ 0x5eed),
            seq: 0,
            capture: Vec::new(),
            capture_on: true,
            delivered: 0,
            lost: 0,
        }
    }

    /// Add a host with one interface "eth0" configured with `cidr`.
    pub fn host(&mut self, macn: u8, cidr: &str) -> usize {
        let mut s = Stack::new(StackConfig { seed: 1000 + macn as u64 + self.rng.next_u64() });
        let i = s.add_interface("eth0", mac(macn)).unwrap();
        let (a, p) = Ipv4Addr::parse_cidr(cidr).unwrap();
        s.configure_addr(i, a, p, self.now);
        self.stacks.push(s);
        self.deadlines.push(None);
        self.stacks.len() - 1
    }

    pub fn add_stack(&mut self, s: Stack) -> usize {
        self.stacks.push(s);
        self.deadlines.push(None);
        self.stacks.len() - 1
    }

    pub fn segment(&mut self, ends: &[(usize, usize)]) -> usize {
        self.segments.push(Segment { ends: ends.to_vec(), loss: 0.0, reorder: 0.0, latency: 1, up: true });
        self.segments.len() - 1
    }

    pub fn s(&mut self, n: usize) -> &mut Stack {
        &mut self.stacks[n]
    }

    fn schedule(&mut self, node: usize, iface: usize, frame: Vec<u8>) {
        if self.capture_on {
            self.capture.push(Captured { time: self.now, node, iface, frame: frame.clone() });
        }
        for si in 0..self.segments.len() {
            if !self.segments[si].ends.contains(&(node, iface)) || !self.segments[si].up {
                continue;
            }
            let ends = self.segments[si].ends.clone();
            for (dn, di) in ends {
                if (dn, di) == (node, iface) {
                    continue;
                }
                let seg = &self.segments[si];
                let (loss, reorder, latency) = (seg.loss, seg.reorder, seg.latency);
                if self.rng.chance(loss) {
                    self.lost += 1;
                    continue;
                }
                let extra = if self.rng.chance(reorder) { 1 + self.rng.below(5) as i64 } else { 0 };
                self.seq += 1;
                self.inflight.push(InFlight { due: self.now + latency + extra, seq: self.seq, node: dn, iface: di, frame: frame.clone() });
            }
        }
    }

    fn collect_tx(&mut self) {
        for n in 0..self.stacks.len() {
            while let Some((i, f)) = self.stacks[n].pop_tx() {
                self.schedule(n, i, f);
            }
        }
    }

    /// Move frames, deliver everything due at `now`, run timers.
    pub fn pump(&mut self) {
        loop {
            self.collect_tx();
            let now = self.now;
            let mut due: Vec<InFlight> = Vec::new();
            let mut rest = Vec::new();
            for f in self.inflight.drain(..) {
                if f.due <= now {
                    due.push(f);
                } else {
                    rest.push(f);
                }
            }
            self.inflight = rest;
            due.sort_by_key(|f| (f.due, f.seq));
            let any = !due.is_empty();
            for f in due {
                self.delivered += 1;
                self.stacks[f.node].handle_frame(f.iface, &f.frame, now);
            }
            for n in 0..self.stacks.len() {
                self.deadlines[n] = self.stacks[n].poll(now);
            }
            self.collect_tx();
            if !any && !self.inflight.iter().any(|f| f.due <= self.now) {
                break;
            }
        }
    }

    fn next_event(&self) -> Option<i64> {
        let a = self.inflight.iter().map(|f| f.due).min();
        let b = self.deadlines.iter().flatten().copied().min();
        match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Run until `f` returns true or `max_ms` of virtual time elapses. `f` is the
    /// application: it runs at every event. Returns whether `f` succeeded.
    pub fn run(&mut self, max_ms: i64, mut f: impl FnMut(&mut Net) -> bool) -> bool {
        let end = self.now + max_ms;
        let mut same_time_iters = 0;
        loop {
            if f(self) {
                self.pump();
                return true;
            }
            self.pump();
            // let the application react to what was just delivered, then flush its output
            if f(self) {
                self.pump();
                return true;
            }
            self.pump();
            let next = self.next_event().unwrap_or(self.now + 1000).max(self.now);
            if next == self.now {
                same_time_iters += 1;
                assert!(same_time_iters < 10_000, "no progress at t={}", self.now);
            } else {
                same_time_iters = 0;
            }
            if next > end {
                self.now = end;
                self.pump();
                return f(self);
            }
            self.now = next;
        }
    }

    /// Advance virtual time by `ms`, processing everything on the way.
    pub fn advance(&mut self, ms: i64) {
        self.run(ms, |_| false);
    }

    pub fn clear_capture(&mut self) {
        self.capture.clear();
    }
}

/// Two hosts on one segment: A = 10.0.0.1 (node 0), B = 10.0.0.2 (node 1).
pub fn two_hosts(seed: u64) -> Net {
    let mut net = Net::new(seed);
    let a = net.host(1, "10.0.0.1/24");
    let b = net.host(2, "10.0.0.2/24");
    net.segment(&[(a, 0), (b, 0)]);
    net.pump();
    net
}
