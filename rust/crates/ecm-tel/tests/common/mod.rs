//! An in-process carrier for the telephony tests: phones (`Ua`), `imsd`,
//! `smscd`, `mmscd` and `mrfd` machines exchanging real SIP/RTP bytes over an
//! ideal UDP network, with a small HSS that speaks `ecm_cell::hss`'s protocol.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};

use ecm_cell::simauth::ims_aka;
use ecm_tel::ims::{EnumAnswer, Ims, ImsConfig, ImsOut, Peer};
use ecm_tel::smsc::{Smsc, SmscConfig, SmscOut};
use ecm_tel::txn::Addr;
use ecm_tel::ua::{Bearer, Ua, UaConfig, UaEvent, UaOut};

pub const CORE: [u8; 4] = [100, 65, 2, 1];

#[derive(Clone, Debug, Default)]
pub struct Sub {
    pub imsi: String,
    pub msisdn: String,
    pub key: [u8; 16],
    pub ims_ip: [u8; 4],
    pub attached: bool,
    pub fields: BTreeMap<String, String>,
}

/// The HSS side of `cored` for these tests (the real one is tested in ecm-cell).
#[derive(Default)]
pub struct Hss {
    pub subs: Vec<Sub>,
    pub regs: BTreeMap<String, String>,
    pub bearers: Vec<(String, u16, String, u16)>,
    pub blocks: Vec<String>,
    pub domain: String,
    next_rand: u8,
}

impl Hss {
    fn find(&self, who: &str) -> Option<usize> {
        self.subs.iter().position(|s| s.imsi == who || s.msisdn == who)
    }

    pub fn answer(&mut self, line: &str) -> Result<String, String> {
        let w: Vec<&str> = line.split_whitespace().collect();
        let d = |s: &str| if s.is_empty() { "-".to_string() } else { s.to_string() };
        match w.as_slice() {
            ["MAR", who] => {
                let i = self.find(who).ok_or("unknown user")?;
                self.next_rand = self.next_rand.wrapping_add(1);
                let rand = [self.next_rand; 16];
                let v = ims_aka(&self.subs[i].key, &rand);
                let hex = |b: &[u8]| b.iter().map(|x| format!("{:02x}", x)).collect::<String>();
                Ok(format!("imsi {}\nmsisdn {}\nrand {}\nxres {}\nck {}\n", self.subs[i].imsi, self.subs[i].msisdn, hex(&rand), hex(&v.res), hex(&v.ks)))
            }
            ["SAR", who, what, server] => {
                let i = self.find(who).ok_or("unknown user")?;
                if *what == "register" {
                    self.regs.insert(self.subs[i].imsi.clone(), server.to_string());
                } else {
                    self.regs.remove(&self.subs[i].imsi);
                }
                let imsi = self.subs[i].imsi.clone();
                self.answer(&format!("UDR {}", imsi))
            }
            ["UDR", who] => {
                let i = self.find(who).ok_or("unknown user")?;
                let s = &self.subs[i];
                let f = |k: &str, def: &str| s.fields.get(k).cloned().unwrap_or_else(|| def.to_string());
                Ok(format!(
                    "imsi {}\nmsisdn {}\nlabel -\nused 1500\nquota 0\ncfu {}\ncfb {}\ncfnr {}\ncfnrc {}\ncw {}\nclir 0\ndnd {}\nvm {}\nblock {}\nport -\n",
                    s.imsi,
                    s.msisdn,
                    d(&f("cfu", "")),
                    d(&f("cfb", "")),
                    d(&f("cfnr", "")),
                    d(&f("cfnrc", "")),
                    f("cw", "1"),
                    f("dnd", "0"),
                    f("vm", "1"),
                    d(&f("block", ""))
                ))
            }
            ["PUR", who, k, v] => {
                let i = self.find(who).ok_or("unknown user")?;
                let v = if *v == "off" { "" } else { *v };
                self.subs[i].fields.insert(k.to_string(), v.to_string());
                Ok(String::new())
            }
            ["LIR", number] => {
                if let Some(i) = self.find(number) {
                    let s = &self.subs[i];
                    let ip = if s.attached { Addr::new(s.ims_ip, 0).host() } else { "-".into() };
                    return Ok(format!("result local\nimsi {}\nims_ip {}\nidle 0\nregistered {}\n", s.imsi, ip, self.regs.get(&s.imsi).cloned().unwrap_or_else(|| "-".into())));
                }
                if self.blocks.iter().any(|b| number.starts_with(&format!("+1{}", b.replace('-', "")))) {
                    Ok("result unassigned\n".into())
                } else {
                    Ok("result foreign\n".into())
                }
            }
            ["IPR", ip] => {
                let s = self.subs.iter().find(|s| s.attached && Addr::new(s.ims_ip, 0).host() == *ip).ok_or("no IMS session at that address")?;
                Ok(format!("imsi {}\nmsisdn {}\n", s.imsi, s.msisdn))
            }
            ["AAR", ue, up, r, rp] => {
                self.bearers.push((ue.to_string(), up.parse().unwrap_or(0), r.to_string(), rp.parse().unwrap_or(0)));
                Ok(format!("bearer {}\nsent 1\n", self.bearers.len()))
            }
            ["STR", ue, _id] => {
                self.bearers.retain(|b| b.0 != *ue);
                Ok(String::new())
            }
            ["NUM"] => Ok(format!("cc 1\ndomain {}\n", self.domain)),
            _ => Err(format!("unknown request {}", line)),
        }
    }
}

pub struct Phone {
    pub ua: Ua,
    pub ip: [u8; 4],
    pub key: [u8; 16],
    pub events: Vec<UaEvent>,
    /// Energy of what reached the speaker (sum of |sample|) since the last read.
    pub heard: u64,
    /// A tone the microphone hears (amplitude; 0 = silence).
    pub mic_level: i16,
    pub on: bool,
}

pub struct Carrier {
    pub hss: Hss,
    pub ims: Ims,
    pub smsc: Smsc,
    pub cdrs: Vec<String>,
    pub logs: Vec<String>,
    pub saved_queue: String,
    pub short_codes: Vec<(String, String, String)>,
}

pub struct Net {
    pub now: u64,
    pub carriers: Vec<Carrier>,
    pub phones: Vec<Phone>,
    /// (from, to, bytes) in flight.
    pub wire: VecDeque<(Addr, Addr, Vec<u8>)>,
    /// ENUM: number prefix -> (domain, border).
    pub enum_db: Vec<(String, String, Addr)>,
    /// Drop datagrams to this address (a phone switched off).
    pub dead: Vec<[u8; 4]>,
    pub delivered: u64,
}

impl Net {
    pub fn new() -> Net {
        Net { now: 1_000, carriers: Vec::new(), phones: Vec::new(), wire: VecDeque::new(), enum_db: Vec::new(), dead: Vec::new(), delivered: 0 }
    }

    /// A carrier whose IMS computer is `ip` (imsd 5060, smscd 5062, mmscd 5063, mrfd 5064).
    pub fn add_carrier(&mut self, domain: &str, ip: [u8; 4], pool: [u8; 4], blocks: &[&str]) -> usize {
        let mut cfg = ImsConfig::new(domain, domain.split('.').next().unwrap_or(domain), ip);
        cfg.ims_pool = (pool, 16);
        cfg.no_answer_ms = 30_000;
        let ims = Ims::new(cfg, 7 + self.carriers.len() as u64);
        let mut scfg = SmscConfig::new(domain, "test", ip);
        scfg.ims = Addr::new(ip, 5060);
        let smsc = Smsc::new(scfg, 99);
        let hss = Hss { domain: domain.into(), blocks: blocks.iter().map(|s| s.to_string()).collect(), ..Default::default() };
        self.carriers.push(Carrier { hss, ims, smsc, cdrs: Vec::new(), logs: Vec::new(), saved_queue: String::new(), short_codes: Vec::new() });
        self.carriers.len() - 1
    }

    pub fn peer(&mut self, a: usize, b: usize, secret: &str) {
        let (da, ia) = (self.carriers[a].ims.cfg.domain.clone(), self.carriers[a].ims.cfg.public_ip);
        let (db, ib) = (self.carriers[b].ims.cfg.domain.clone(), self.carriers[b].ims.cfg.public_ip);
        self.carriers[a].ims.cfg.peers.push(Peer { domain: db, addr: Some(Addr::new(ib, 5060)), secret: secret.into() });
        self.carriers[b].ims.cfg.peers.push(Peer { domain: da, addr: Some(Addr::new(ia, 5060)), secret: secret.into() });
    }

    /// A phone on carrier `c` with number `msisdn`, IMS address `ip`; returns its index.
    pub fn add_phone(&mut self, c: usize, imsi: &str, msisdn: &str, ip: [u8; 4]) -> usize {
        let key = [imsi.len() as u8 ^ ip[3]; 16];
        self.carriers[c].hss.subs.push(Sub { imsi: imsi.into(), msisdn: msisdn.into(), key, ims_ip: ip, attached: true, fields: BTreeMap::new() });
        let ua = Ua::new(UaConfig::new(imsi), ip[3] as u64 * 31 + 5);
        self.phones.push(Phone { ua, ip, key, events: Vec::new(), heard: 0, mic_level: 0, on: false });
        self.phones.len() - 1
    }

    pub fn carrier_of(&self, ip: [u8; 4]) -> Option<usize> {
        self.carriers.iter().position(|c| c.ims.cfg.ims_ip == ip || c.ims.cfg.public_ip == ip)
    }

    /// Bring the phone's IMS bearer up (it registers).
    pub fn power_on(&mut self, p: usize) {
        let ip = self.phones[p].ip;
        let c = self.carriers.iter().position(|c| c.hss.subs.iter().any(|s| s.ims_ip == ip)).expect("phone's carrier");
        let domain = self.carriers[c].ims.cfg.domain.clone();
        let pcscf = Addr::new(self.carriers[c].ims.cfg.ims_ip, 5060);
        if let Some(s) = self.carriers[c].hss.subs.iter_mut().find(|s| s.ims_ip == ip) {
            s.attached = true;
        }
        self.dead.retain(|d| *d != ip);
        self.phones[p].on = true;
        let now = self.now;
        self.phones[p].ua.bearer_up(Bearer { ip, pcscf, domain, msisdn: String::new() }, now);
        self.pump();
    }

    pub fn power_off(&mut self, p: usize) {
        let ip = self.phones[p].ip;
        self.dead.push(ip);
        self.phones[p].on = false;
        let now = self.now;
        self.phones[p].ua.bearer_down(now);
        for c in &mut self.carriers {
            if let Some(s) = c.hss.subs.iter_mut().find(|s| s.ims_ip == ip) {
                s.attached = false;
            }
        }
        self.pump();
    }

    pub fn send(&mut self, from: Addr, to: Addr, data: Vec<u8>) {
        self.wire.push_back((from, to, data));
    }

    /// Drain every machine's outputs and deliver datagrams until quiet.
    pub fn pump(&mut self) {
        for _ in 0..10_000 {
            let mut busy = self.collect_outputs();
            while let Some((from, to, data)) = self.wire.pop_front() {
                busy = true;
                self.deliver(from, to, &data);
            }
            if !busy {
                return;
            }
        }
        panic!("network never went quiet");
    }

    fn collect_outputs(&mut self) -> bool {
        {
            let mut busy = false;
            let now = self.now;
            for i in 0..self.phones.len() {
                let ip = self.phones[i].ip;
                let outs = self.phones[i].ua.take_out();
                for o in outs {
                    busy = true;
                    match o {
                        UaOut::Sip { to, data } => self.wire.push_back((Addr::new(ip, 5060), to, data)),
                        UaOut::Rtp { port, to, data } => self.wire.push_back((Addr::new(ip, port), to, data)),
                        UaOut::Aka { rand } => {
                            let key = self.phones[i].key;
                            let r = ims_aka(&key, &rand);
                            self.phones[i].ua.on_aka(Some(r.res), now);
                        }
                        UaOut::Event(e) => self.phones[i].events.push(e),
                        UaOut::OpenRtp(_) | UaOut::CloseRtp(_) | UaOut::Log(_) => {}
                    }
                }
            }
            for ci in 0..self.carriers.len() {
                let ip = self.carriers[ci].ims.cfg.ims_ip;
                let outs = self.carriers[ci].ims.take_out();
                for o in outs {
                    busy = true;
                    match o {
                        ImsOut::Sip { to, data } => self.wire.push_back((Addr::new(ip, 5060), to, data)),
                        ImsOut::Relay { port, to, data } => self.wire.push_back((Addr::new(ip, port), to, data)),
                        ImsOut::Hss { id, line } => {
                            let r = self.carriers[ci].hss.answer(&line);
                            self.carriers[ci].ims.on_hss(id, r, now);
                        }
                        ImsOut::Enum { id, number } => {
                            let a = self.enum_db.iter().find(|(p, _, _)| number.starts_with(p.as_str())).map(|(_, d, b)| EnumAnswer { domain: d.clone(), border: Some(*b) });
                            self.carriers[ci].ims.on_enum(id, a, now);
                        }
                        ImsOut::Cdr(l) => self.carriers[ci].cdrs.push(l),
                        ImsOut::Log(l) => self.carriers[ci].logs.push(format!("ims: {}", l)),
                        ImsOut::OpenRelay(_) | ImsOut::CloseRelay(_) => {}
                    }
                }
                let outs = self.carriers[ci].smsc.take_out();
                for o in outs {
                    busy = true;
                    match o {
                        SmscOut::Sip { to, data } => self.wire.push_back((Addr::new(ip, 5062), to, data)),
                        SmscOut::Hss { id, line } => {
                            let r = self.carriers[ci].hss.answer(&line);
                            self.carriers[ci].smsc.on_hss(id, r, now);
                        }
                        SmscOut::Save(t) => self.carriers[ci].saved_queue = t,
                        SmscOut::ShortCode { code, from, text } => self.carriers[ci].short_codes.push((code, from, text)),
                        SmscOut::Cdr(l) => self.carriers[ci].cdrs.push(l),
                        SmscOut::Log(l) => self.carriers[ci].logs.push(format!("smsc: {}", l)),
                    }
                }
            }
            busy
        }
    }

    pub fn pump_once_trace(&mut self) {
        let before = self.wire.len();
        let _ = before;
        // move outputs onto the wire without delivering, print, then deliver
        let mut w = std::mem::take(&mut self.wire);
        self.pump_collect();
        w.extend(std::mem::take(&mut self.wire));
        for (f, t, d) in w {
            let first = String::from_utf8_lossy(&d).lines().next().unwrap_or("").to_string();
            let body_kind = ecm_tel::sip::parse(&d).ok().map(|m| (m.headers().call_id().unwrap_or("").to_string(), m.body().iter().take(4).map(|b| format!("{:02x}", b)).collect::<String>())).unwrap_or_default();
            eprintln!("{} -> {}: {} {:?}", f, t, first, body_kind);
            self.deliver(f, t, &d);
        }
    }

    fn pump_collect(&mut self) {
        let saved: Vec<u8> = Vec::new();
        let _ = saved;
        // run one round of output collection (same as pump, without delivery)
        let wire = std::mem::take(&mut self.wire);
        self.wire = wire;
        let n = self.wire.len();
        let _ = n;
        self.collect_outputs();
    }

    fn deliver(&mut self, from: Addr, to: Addr, data: &[u8]) {
        if self.dead.contains(&to.ip) {
            return;
        }
        self.delivered += 1;
        let now = self.now;
        if let Some(p) = self.phones.iter_mut().find(|p| p.ip == to.ip) {
            if to.port == 5060 {
                p.ua.on_sip(from, data, now);
            } else {
                p.ua.on_rtp(to.port, from, data, now);
            }
            return;
        }
        if let Some(c) = self.carrier_of(to.ip) {
            let car = &mut self.carriers[c];
            match to.port {
                5060 => car.ims.on_sip(from, data, now),
                5062 => car.smsc.on_sip(from, data, now),
                p if (30000..31000).contains(&p) => car.ims.on_relay(p, from, data, now),
                _ => {}
            }
        }
    }

    /// Advance the clock in 20 ms steps: phones' microphones and speakers run, machines tick.
    pub fn run(&mut self, ms: u64) {
        let end = self.now + ms;
        while self.now < end {
            self.now += 20;
            let now = self.now;
            for p in &mut self.phones {
                if !p.on {
                    continue;
                }
                let pcm: Vec<i16> = (0..320).map(|i| if p.mic_level == 0 { 0 } else { ((i as f32 * 0.35).sin() * p.mic_level as f32) as i16 }).collect();
                p.ua.mic(&pcm, now);
                let out = p.ua.speaker(320);
                p.heard += out.iter().map(|s| s.unsigned_abs() as u64).sum::<u64>();
                p.ua.tick(now);
            }
            for c in &mut self.carriers {
                c.ims.tick(now);
                c.smsc.tick(now);
            }
            self.pump();
        }
    }

    pub fn run_until(&mut self, ms: u64, f: impl Fn(&Net) -> bool) -> bool {
        let end = self.now + ms;
        while self.now < end {
            if f(self) {
                return true;
            }
            self.run(20);
        }
        f(self)
    }

    pub fn registered(&self, p: usize) -> bool {
        self.phones[p].ua.registration() == ecm_tel::ua::RegState::Registered
    }

    pub fn dump(&self) -> String {
        let mut s = String::new();
        for (i, c) in self.carriers.iter().enumerate() {
            s += &format!("carrier {} ({}):\n", i, c.ims.cfg.domain);
            for l in &c.logs {
                s += &format!("  {}\n", l);
            }
            for l in &c.cdrs {
                s += &format!("  CDR {}\n", l);
            }
        }
        for (i, p) in self.phones.iter().enumerate() {
            s += &format!("phone {} {:?} reg {:?} calls {:?}\n  events {:?}\n", i, p.ip, p.ua.registration(), p.ua.calls(), p.events);
        }
        s
    }
}
