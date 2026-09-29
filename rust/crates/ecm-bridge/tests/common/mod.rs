//! In-process multi-bridge network simulator ("netsim").
//!
//! * Virtual clock advanced in fixed steps; every node is polled each step.
//! * Links are point-to-point queues between endpoints (bridge port or
//!   host), bounded to 64 frames per direction with drop-oldest (like the
//!   Java hub), with optional seeded random drop per direction.
//! * Frames move in "waves" within a step, so a broadcast storm is bounded
//!   per step instead of hanging the test.
//! * Hosts are trivial: they emit the frames they are told to and record
//!   everything they receive.
#![allow(dead_code)]

use ecm_bridge::cli::{self, CliSession};
use ecm_bridge::{Bridge, Output};
use ecm_bridge::MacAddr;
use std::collections::VecDeque;

pub const QUEUE_CAP: usize = 64;
const MAX_WAVES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// (bridge index, port)
    B(usize, usize),
    /// host index
    H(usize),
}

pub struct Link {
    pub a: End,
    pub b: End,
    pub up: bool,
    /// Drop probability per direction, in 1/1000.
    pub drop_ab: u32,
    pub drop_ba: u32,
    q_ab: VecDeque<Vec<u8>>,
    q_ba: VecDeque<Vec<u8>>,
    /// Frames that entered the link, per direction.
    pub sent_ab: u64,
    pub sent_ba: u64,
    /// Optional capture of every frame (both directions).
    pub capture: Option<Vec<(bool, Vec<u8>)>>,
}

pub struct Host {
    pub mac: [u8; 6],
    /// Data frames received (everything except 01:80:c2:00:00:0x).
    pub rx: Vec<Vec<u8>>,
    /// Link-local control frames received (LLDP/BPDU/LACP from the switch).
    pub ctrl_rx: Vec<Vec<u8>>,
    txq: VecDeque<Vec<u8>>,
}

impl Host {
    /// Frames received whose source MAC is `src`.
    pub fn count_from(&self, src: [u8; 6]) -> usize {
        self.rx.iter().filter(|f| f.get(6..12) == Some(&src[..])).count()
    }
}

pub struct Net {
    pub now: i64,
    pub bridges: Vec<Bridge>,
    pub sessions: Vec<CliSession>,
    pub hosts: Vec<Host>,
    pub links: Vec<Link>,
    pub locals: Vec<Vec<(u16, Vec<u8>)>>,
    pub logs: Vec<Vec<String>>,
    rng: u64,
    pub step_ms: i64,
}

pub fn bridge_mac(i: usize) -> [u8; 6] {
    [0x02, 0xb0, 0, 0, 0, i as u8 + 1]
}

pub fn port_mac(i: usize, p: usize) -> [u8; 6] {
    [0x02, 0xb1, i as u8, 0, 0, p as u8]
}

pub fn host_mac(n: u8) -> [u8; 6] {
    [0x02, 0xaa, 0, 0, 0, n]
}

impl Net {
    pub fn new(seed: u64) -> Self {
        Net {
            now: 1_000,
            bridges: Vec::new(),
            sessions: Vec::new(),
            hosts: Vec::new(),
            links: Vec::new(),
            locals: Vec::new(),
            logs: Vec::new(),
            rng: seed | 1,
            step_ms: 10,
        }
    }

    fn rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    pub fn add_bridge(&mut self, nports: usize) -> usize {
        let i = self.bridges.len();
        let macs: Vec<MacAddr> = (0..nports).map(|p| MacAddr(port_mac(i, p))).collect();
        self.bridges.push(Bridge::new(&macs, MacAddr(bridge_mac(i)), self.now));
        self.sessions.push(CliSession::new());
        self.locals.push(Vec::new());
        self.logs.push(Vec::new());
        i
    }

    pub fn add_host(&mut self, n: u8) -> usize {
        self.hosts.push(Host { mac: host_mac(n), rx: Vec::new(), ctrl_rx: Vec::new(), txq: VecDeque::new() });
        self.hosts.len() - 1
    }

    /// Run CLI lines on bridge `b`; panics on any `%` error line.
    pub fn cli(&mut self, b: usize, cmds: &str) -> String {
        let mut out = String::new();
        for l in cmds.lines() {
            let r = cli::exec(&mut self.bridges[b], &mut self.sessions[b], l, self.now);
            assert!(!r.output.contains("% "), "bridge {} command {:?} failed: {}", b, l, r.output);
            out.push_str(&r.output);
        }
        out
    }

    /// Convert ports to L2 access VLAN 1.
    pub fn l2_ports(&mut self, b: usize, ports: &[usize]) {
        for p in ports {
            self.cli(b, &format!("interface eth{}\nno routing\nexit", p));
        }
    }

    pub fn link(&mut self, a: End, b: End) -> usize {
        self.links.push(Link {
            a,
            b,
            up: false,
            drop_ab: 0,
            drop_ba: 0,
            q_ab: VecDeque::new(),
            q_ba: VecDeque::new(),
            sent_ab: 0,
            sent_ba: 0,
            capture: None,
        });
        let l = self.links.len() - 1;
        self.set_link(l, true);
        l
    }

    pub fn set_link(&mut self, l: usize, up: bool) {
        let now = self.now;
        let (a, b) = (self.links[l].a, self.links[l].b);
        self.links[l].up = up;
        self.links[l].q_ab.clear();
        self.links[l].q_ba.clear();
        for e in [a, b] {
            if let End::B(i, p) = e {
                self.bridges[i].set_link(p, up, now);
            }
        }
    }

    pub fn set_drop(&mut self, l: usize, ab: u32, ba: u32) {
        self.links[l].drop_ab = ab;
        self.links[l].drop_ba = ba;
    }

    pub fn capture(&mut self, l: usize) {
        self.links[l].capture = Some(Vec::new());
    }

    pub fn send(&mut self, h: usize, frame: Vec<u8>) {
        self.hosts[h].txq.push_back(frame);
    }

    fn enqueue(&mut self, from: End, frame: Vec<u8>) {
        let r = self.rand();
        let link = self.links.iter_mut().find(|l| l.a == from || l.b == from);
        let l = match link {
            Some(l) if l.up => l,
            _ => return,
        };
        let ab = l.a == from;
        let drop = if ab { l.drop_ab } else { l.drop_ba };
        if let Some(c) = &mut l.capture {
            c.push((ab, frame.clone()));
        }
        if ab {
            l.sent_ab += 1;
        } else {
            l.sent_ba += 1;
        }
        if (r % 1000) < drop as u64 {
            return;
        }
        let q = if ab { &mut l.q_ab } else { &mut l.q_ba };
        if q.len() >= QUEUE_CAP {
            q.pop_front();
        }
        q.push_back(frame);
    }

    fn drain_outputs(&mut self) {
        for i in 0..self.bridges.len() {
            while let Some(o) = self.bridges[i].pop_output() {
                match o {
                    Output::Tx { port, frame } => self.enqueue(End::B(i, port), frame),
                    Output::Local { vlan, frame } => self.locals[i].push((vlan, frame)),
                    Output::Log { msg, .. } => self.logs[i].push(msg),
                }
            }
        }
        for h in 0..self.hosts.len() {
            while let Some(f) = self.hosts[h].txq.pop_front() {
                self.enqueue(End::H(h), f);
            }
        }
    }

    fn deliver_wave(&mut self) -> bool {
        let mut any = false;
        let now = self.now;
        for l in 0..self.links.len() {
            let ab: Vec<Vec<u8>> = self.links[l].q_ab.drain(..).collect();
            let ba: Vec<Vec<u8>> = self.links[l].q_ba.drain(..).collect();
            let (a, b) = (self.links[l].a, self.links[l].b);
            for (to, frames) in [(b, ab), (a, ba)] {
                for f in frames {
                    any = true;
                    match to {
                        End::B(i, p) => self.bridges[i].handle_frame(p, &f, now),
                        End::H(h) => {
                            let ctrl = f.get(..6).map(|d| d[..5] == [0x01, 0x80, 0xc2, 0, 0] && d[5] <= 0x0f).unwrap_or(false);
                            if ctrl {
                                self.hosts[h].ctrl_rx.push(f)
                            } else {
                                self.hosts[h].rx.push(f)
                            }
                        }
                    }
                }
            }
        }
        any
    }

    /// Advance by one step: poll every bridge, then move frames.
    pub fn step(&mut self) {
        self.now += self.step_ms;
        let now = self.now;
        for b in self.bridges.iter_mut() {
            let dl = b.poll(now);
            if let Some(d) = dl {
                assert!(d > now, "poll returned a deadline that is already due (busy loop)");
            }
        }
        self.settle();
    }

    /// Move frames until quiet (bounded number of waves).
    pub fn settle(&mut self) {
        for _ in 0..MAX_WAVES {
            self.drain_outputs();
            if !self.deliver_wave() {
                break;
            }
        }
        self.drain_outputs();
    }

    pub fn run_for(&mut self, ms: i64) {
        let end = self.now + ms;
        while self.now < end {
            self.step();
        }
    }

    /// Step until `cond` holds or `max_ms` elapses; returns elapsed ms.
    pub fn run_until(&mut self, max_ms: i64, mut cond: impl FnMut(&Net) -> bool) -> Option<i64> {
        let start = self.now;
        while self.now - start <= max_ms {
            if cond(self) {
                return Some(self.now - start);
            }
            self.step();
        }
        None
    }

    pub fn clear_host_rx(&mut self) {
        for h in self.hosts.iter_mut() {
            h.rx.clear();
            h.ctrl_rx.clear();
        }
    }

    pub fn link_total(&self) -> u64 {
        self.links.iter().map(|l| l.sent_ab + l.sent_ba).sum()
    }
}

// ---------------------------------------------------------------------
// Frame builders
// ---------------------------------------------------------------------

pub const BCAST: [u8; 6] = [0xff; 6];

pub fn eth(dst: [u8; 6], src: [u8; 6], ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(14 + payload.len());
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f.extend_from_slice(payload);
    if f.len() < 60 {
        f.resize(60, 0);
    }
    f
}

/// A data frame with a recognisable payload.
pub fn data(dst: [u8; 6], src: [u8; 6]) -> Vec<u8> {
    eth(dst, src, 0x88b5, &[0x5a; 46])
}

pub fn ipv4(dst: [u8; 6], src: [u8; 6], sip: [u8; 4], dip: [u8; 4], sport: u16, dport: u16) -> Vec<u8> {
    let mut p = vec![0x45, 0, 0, 28, 0, 0, 0, 0, 64, 17, 0, 0];
    p.extend_from_slice(&sip);
    p.extend_from_slice(&dip);
    p.extend_from_slice(&sport.to_be_bytes());
    p.extend_from_slice(&dport.to_be_bytes());
    p.extend_from_slice(&[0, 8, 0, 0]);
    eth(dst, src, 0x0800, &p)
}

pub fn vid_of(f: &[u8]) -> Option<u16> {
    ecm_bridge::frame::parse_eth(f).and_then(|e| e.vid())
}

pub fn src_of(f: &[u8]) -> [u8; 6] {
    let mut m = [0u8; 6];
    if let Some(s) = f.get(6..12) {
        m.copy_from_slice(s);
    }
    m
}
