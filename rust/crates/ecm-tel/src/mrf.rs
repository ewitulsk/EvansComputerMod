//! `mrfd`, sans-IO: the carrier's media server (MRF), docs/TELEPHONY.md.
//!
//! `imsd` sends it calls with the service as the Request-URI user:
//!
//! | Service | Box (`P-Called-Party-ID`) | What it does |
//! |---|---|---|
//! | `voicemail` | the callee | greeting tones and a beep, then records the caller (up to `max_record_ms`) into the box; the box's phone gets a waiting indication (`NOTIFY` `message-summary`) |
//! | `voicemail-retrieve` | the caller (`*86`) | plays the new messages, then the saved ones; keys: `7` delete, `9` save, `1` replay, `#` next |
//! | `announcement` | the SIP code (`404`, `480`, ...) | the special information tones (intercept) for `announce_ms`, then hangs up |
//! | `support` | the caller (`611`) | an IVR: `1` texts the data balance, `2` texts your number, `0` repeats; ends after 30 s |
//! | `conference` | (`X-Conference: <id>`) | one participant of a three-way call: everyone hears everyone else |
//!
//! Voicemail boxes are files ([`MrfOut::Save`]): `/var/spool/vm/<number>/<id>.wav`
//! (16 kHz mono PCM) and an index. Phones read their box over HTTP too (visual
//! voicemail, [`Mrf::http`]): only from the phone's own IMS address, which
//! `imsd`'s registration notices tell.

use std::collections::BTreeMap;

use ecm_rtp::codec::{Codec as RtpCodec, FrameEncoder};
use ecm_rtp::dtmf::DtmfReceiver;
use ecm_rtp::jitter::JitterBuffer;
use ecm_rtp::rtp::RtpPacket;
use ecm_rtp::tones::Tone;

use crate::dialog::{number_uri, response_target, uri_number, Dialog};
use crate::sdp::{self, Caps, Negotiated, SessionDescription};
use crate::sip::{self, Message, Method, Uri};
use crate::txn::{Addr, Rng, Txns};

pub const SIP_PORT: u16 = 5064;
pub const HTTP_PORT: u16 = 8003;
pub const SPOOL: &str = "/var/spool/vm";
const FRAME: usize = 320;

#[derive(Clone, Debug)]
pub struct MrfConfig {
    pub domain: String,
    pub carrier: String,
    pub ip: [u8; 4],
    pub sip_port: u16,
    pub ims: Addr,
    /// smscd (texts from the support line).
    pub smsc: Addr,
    pub rtp_ports: (u16, u16),
    pub max_record_ms: u64,
    pub announce_ms: u64,
    pub greeting_ms: u64,
}

impl MrfConfig {
    pub fn new(domain: &str, carrier: &str, ip: [u8; 4]) -> MrfConfig {
        MrfConfig {
            domain: domain.into(),
            carrier: carrier.into(),
            ip,
            sip_port: SIP_PORT,
            ims: Addr::new(ip, 5060),
            smsc: Addr::new(ip, 5062),
            rtp_ports: (31000, 32000),
            max_record_ms: 60_000,
            announce_ms: 4_000,
            greeting_ms: 2_000,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum MrfOut {
    Sip { to: Addr, data: Vec<u8> },
    Rtp { port: u16, to: Addr, data: Vec<u8> },
    OpenRtp(u16),
    CloseRtp(u16),
    /// Write a file (a recording or a box index).
    Save { path: String, data: Vec<u8> },
    Delete { path: String },
    /// Ask the HSS (the support line's balance).
    Hss { id: u64, line: String },
    Log(String),
}

/// One voicemail.
#[derive(Clone, Debug, PartialEq)]
pub struct Voicemail {
    pub id: u64,
    pub from: String,
    pub at_ms: u64,
    pub ms: u64,
    pub heard: bool,
    pub samples: Vec<i16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Service {
    Deposit,
    Retrieve,
    Announcement,
    Support,
    Conference,
}

struct Session {
    port: u16,
    dialog: Dialog,
    service: Service,
    /// The box (number) or the announcement's code.
    target: String,
    caller: String,
    neg: Negotiated,
    remote: Addr,
    enc: FrameEncoder,
    jitter: JitterBuffer,
    dtmf: DtmfReceiver,
    seq: u16,
    ts: u32,
    ssrc: u32,
    started_ms: u64,
    /// What it plays now: samples queued for the far end.
    play: Vec<i16>,
    tone: Option<Tone>,
    recording: Option<Vec<i16>>,
    /// Retrieval: the message playing.
    current: Option<u64>,
    end_at: Option<u64>,
    conf: u64,
    /// The latest 20 ms this participant sent (conference mixing).
    last: Vec<i16>,
}

pub struct Mrf {
    pub cfg: MrfConfig,
    rng: Rng,
    tx: Txns,
    sessions: BTreeMap<u16, Session>,
    by_cid: BTreeMap<String, u16>,
    boxes: BTreeMap<String, Vec<Voicemail>>,
    /// IMS address -> number (from imsd's registration notices).
    phones: BTreeMap<[u8; 4], String>,
    next_port: u16,
    next_vm: u64,
    next_id: u64,
    pending: BTreeMap<u64, (String, String)>,
    out: Vec<MrfOut>,
}

fn wav_bytes(pcm: &[i16]) -> Vec<u8> {
    ecm_rtp::wav::write(16_000, pcm)
}

impl Mrf {
    pub fn new(cfg: MrfConfig, seed: u64) -> Mrf {
        let next_port = cfg.rtp_ports.0;
        Mrf { cfg, rng: Rng::new(seed), tx: Txns::new(), sessions: BTreeMap::new(), by_cid: BTreeMap::new(), boxes: BTreeMap::new(), phones: BTreeMap::new(), next_port, next_vm: 1, next_id: 1, pending: BTreeMap::new(), out: Vec::new() }
    }

    pub fn take_out(&mut self) -> Vec<MrfOut> {
        for (to, data) in self.tx.take_sends() {
            self.out.push(MrfOut::Sip { to, data });
        }
        std::mem::take(&mut self.out)
    }

    fn log(&mut self, s: String) {
        self.out.push(MrfOut::Log(s));
    }

    fn host(&self) -> String {
        Addr::new(self.cfg.ip, 0).host()
    }

    pub fn mailbox(&self, number: &str) -> &[Voicemail] {
        self.boxes.get(number).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Load a box from its index and recordings (at start).
    pub fn load_box(&mut self, number: &str, index: &str, read: &dyn Fn(&str) -> Option<Vec<u8>>) {
        let mut v = Vec::new();
        for l in index.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            if let ["msg", id, from, at, ms, st] = w.as_slice() {
                let Ok(id) = id.parse::<u64>() else { continue };
                let samples = read(&format!("{}/{}/{}.wav", SPOOL, number, id)).and_then(|b| ecm_rtp::wav::read_16k(&b)).unwrap_or_default();
                v.push(Voicemail { id, from: from.to_string(), at_ms: at.parse().unwrap_or(0), ms: ms.parse().unwrap_or(0), heard: *st == "heard", samples });
                self.next_vm = self.next_vm.max(id + 1);
            }
        }
        self.boxes.insert(number.to_string(), v);
    }

    fn save_index(&mut self, number: &str) {
        let mut s = String::from("# voicemail box\n");
        for m in self.boxes.get(number).into_iter().flatten() {
            s.push_str(&format!("msg {} {} {} {} {}\n", m.id, m.from, m.at_ms, m.ms, if m.heard { "heard" } else { "new" }));
        }
        self.out.push(MrfOut::Save { path: format!("{}/{}/index", SPOOL, number), data: s.into_bytes() });
    }

    fn respond(&mut self, req: &Message, from: Addr, code: u16, tag: Option<&str>, now: u64) {
        if let Ok(r) = req.response_to(code, tag) {
            self.tx.respond(response_target(req, from), &r, now);
        }
    }

    pub fn on_sip(&mut self, from: Addr, data: &[u8], now: u64) {
        let Ok(m) = sip::parse(data) else { return };
        if !m.is_request() {
            self.tx.on_response(&m);
            return;
        }
        if !self.tx.on_request(from, &m, now) {
            return;
        }
        let cid = m.headers().call_id().unwrap_or("").to_string();
        match m.method().cloned() {
            Some(Method::Invite) => {
                if let Some(&port) = self.by_cid.get(&cid) {
                    // a re-INVITE: keep the session, answer with our media
                    let sdp = self.sessions.get(&port).map(|s| self.our_sdp(s.port, &s.neg));
                    if let (Some(sdp), Ok(mut r)) = (sdp, m.response_to(200, None)) {
                        r.set_body("application/sdp", sdp.to_bytes());
                        self.tx.respond(response_target(&m, from), &r, now);
                    }
                    return;
                }
                self.invite(from, m, now)
            }
            Some(Method::Ack) => {}
            Some(Method::Bye) => {
                self.respond(&m, from, 200, None, now);
                if let Some(port) = self.by_cid.get(&cid).copied() {
                    self.end(port, false, now);
                }
            }
            Some(Method::Cancel) => self.respond(&m, from, 200, None, now),
            Some(Method::Notify) => {
                self.respond(&m, from, 200, None, now);
                // imsd: "registered <number> <ims address>"
                let body = String::from_utf8_lossy(m.body()).to_string();
                for l in body.lines() {
                    let w: Vec<&str> = l.split_whitespace().collect();
                    if let ["registered", n, rest @ ..] = w.as_slice() {
                        if let Some(ip) = rest.first().and_then(|a| Addr::parse_ip(a)) {
                            self.phones.retain(|_, v| v != n);
                            self.phones.insert(ip, n.to_string());
                        }
                        self.mwi(n, now);
                    }
                }
            }
            Some(Method::Subscribe) => {
                self.respond(&m, from, 200, None, now);
                if let Some(n) = m.headers().from().ok().and_then(|f| uri_number(&f.uri)) {
                    self.mwi(&n, now);
                }
            }
            _ => self.respond(&m, from, 405, None, now),
        }
    }

    fn our_sdp(&self, port: u16, neg: &Negotiated) -> SessionDescription {
        let host = self.host();
        let mut s = sdp::offer(&host, port, 1, 1, neg.codec == sdp::Codec::AdpcmWb);
        if let Some(m) = s.media.first_mut() {
            // only what was agreed
            m.formats.retain(|f| *f == neg.pt || Some(*f) == neg.dtmf_pt);
        }
        s
    }

    fn invite(&mut self, from: Addr, m: Message, now: u64) {
        let user = m.uri().and_then(|u| u.user().map(String::from)).unwrap_or_default();
        let service = match user.as_str() {
            "voicemail" => Service::Deposit,
            "voicemail-retrieve" => Service::Retrieve,
            "announcement" => Service::Announcement,
            "support" => Service::Support,
            "conference" => Service::Conference,
            _ => return self.respond(&m, from, 404, None, now),
        };
        let h = m.headers();
        let target = h.p_called_party_id().ok().flatten().and_then(|n| uri_number(&n.uri)).unwrap_or_default();
        let caller = h.p_asserted_identity().ok().and_then(|v| v.into_iter().find_map(|n| uri_number(&n.uri))).unwrap_or_else(|| "anonymous".into());
        let conf: u64 = h.get("X-Conference").and_then(|v| v.split(';').next().and_then(|x| x.trim().parse().ok())).unwrap_or(0);
        let Ok(offer) = SessionDescription::parse(m.body()) else {
            return self.respond(&m, from, 488, None, now);
        };
        let port = self.next_port;
        self.next_port = if port + 2 >= self.cfg.rtp_ports.1 { self.cfg.rtp_ports.0 } else { port + 2 };
        let host = self.host();
        let Ok((ans, neg)) = sdp::answer(&offer, &host, port, self.rng.next() >> 16, 1, &Caps::ALL) else {
            return self.respond(&m, from, 488, None, now);
        };
        let tag = sip::new_tag(self.rng.next());
        let Some(dialog) = Dialog::incoming(&m, &tag, from) else { return };
        let Ok(mut r) = m.response_to(200, Some(&tag)) else { return };
        r.headers_mut().push("Contact", format!("<sip:{}@{}:{}>", user, host, self.cfg.sip_port));
        r.set_body("application/sdp", ans.to_bytes());
        self.tx.respond(response_target(&m, from), &r, now);
        self.out.push(MrfOut::OpenRtp(port));
        let codec = match neg.codec {
            sdp::Codec::AdpcmWb => RtpCodec::AdpcmWb,
            sdp::Codec::AdpcmNb => RtpCodec::AdpcmNb,
        };
        let remote = Addr::parse_ip(&neg.remote_addr).map(|ip| Addr::new(ip, neg.remote_port)).unwrap_or_default();
        let mut s = Session {
            port,
            dialog,
            service,
            target: target.clone(),
            caller: caller.clone(),
            neg,
            remote,
            enc: FrameEncoder::new(codec),
            jitter: JitterBuffer::new(16_000),
            dtmf: DtmfReceiver::new(),
            seq: 1,
            ts: self.rng.next() as u32,
            ssrc: self.rng.next() as u32,
            started_ms: now,
            play: Vec::new(),
            tone: None,
            recording: None,
            current: None,
            end_at: None,
            conf,
            last: vec![0; FRAME],
        };
        match service {
            Service::Deposit => {
                // greeting: two rising tones, then the beep; recording starts after it
                s.tone = Some(Tone::custom(&[(&[523.0], 400), (&[], 100), (&[659.0], 400), (&[], 300), (&[1000.0], 300)], false));
                s.end_at = Some(now + self.cfg.greeting_ms + self.cfg.max_record_ms);
                self.log(format!("voicemail for {} from {}", target, caller));
            }
            Service::Retrieve => {
                let n = self.boxes.get(&target).map(|b| b.len()).unwrap_or(0);
                self.log(format!("{} listens to voicemail ({} messages)", target, n));
                s.tone = Some(Tone::custom(&[(&[880.0], 200), (&[], 200)], false));
            }
            Service::Announcement => {
                s.tone = Some(Tone::sit());
                s.end_at = Some(now + self.cfg.announce_ms);
            }
            Service::Support => {
                s.tone = Some(Tone::custom(&[(&[660.0], 300), (&[], 150), (&[660.0], 300), (&[], 1000)], true));
                s.end_at = Some(now + 30_000);
                self.log(format!("{} called support", caller));
            }
            Service::Conference => self.log(format!("conference {}: a participant joins", conf)),
        }
        self.by_cid.insert(s.dialog.call_id.clone(), port);
        self.sessions.insert(port, s);
    }

    pub fn on_rtp(&mut self, port: u16, _from: Addr, data: &[u8], now: u64) {
        let Some(s) = self.sessions.get_mut(&port) else { return };
        let Some(p) = RtpPacket::decode(data) else { return };
        if Some(p.payload_type) == s.neg.dtmf_pt {
            if let Some(d) = s.dtmf.push(p.timestamp, &p.payload) {
                self.key(port, d, now);
            }
            return;
        }
        if p.payload_type != s.neg.pt {
            return;
        }
        let codec = match s.neg.codec {
            sdp::Codec::AdpcmWb => RtpCodec::AdpcmWb,
            sdp::Codec::AdpcmNb => RtpCodec::AdpcmNb,
        };
        if let Some(pcm) = codec.decode_frame(&p.payload) {
            let ts = if codec == RtpCodec::AdpcmNb { p.timestamp.wrapping_mul(2) } else { p.timestamp };
            s.jitter.push(p.seq, ts, pcm, now);
        }
    }

    /// A keypad digit in a session.
    fn key(&mut self, port: u16, d: char, now: u64) {
        let Some(s) = self.sessions.get(&port) else { return };
        let (service, target, caller, cur) = (s.service, s.target.clone(), s.caller.clone(), s.current);
        match service {
            Service::Retrieve => {
                let Some(cur) = cur else { return };
                match d {
                    '7' => {
                        if let Some(b) = self.boxes.get_mut(&target) {
                            b.retain(|m| m.id != cur);
                        }
                        self.out.push(MrfOut::Delete { path: format!("{}/{}/{}.wav", SPOOL, target, cur) });
                        self.save_index(&target);
                        self.next_message(port, now);
                    }
                    '9' | '#' => self.next_message(port, now),
                    '1' => {
                        let samples = self.boxes.get(&target).and_then(|b| b.iter().find(|m| m.id == cur)).map(|m| m.samples.clone()).unwrap_or_default();
                        if let Some(s) = self.sessions.get_mut(&port) {
                            s.play = samples;
                        }
                    }
                    _ => {}
                }
            }
            Service::Support => match d {
                '1' => {
                    let id = self.next_id;
                    self.next_id += 1;
                    self.pending.insert(id, (caller.clone(), "balance".into()));
                    self.out.push(MrfOut::Hss { id, line: format!("UDR {}", caller) });
                }
                '2' => {
                    let t = format!("{} support: your number is {}.", self.cfg.carrier, crate::number::E164::parse(&caller).map(|n| n.display("1")).unwrap_or(caller.clone()));
                    self.text(&caller, &t, now);
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// The HSS answered the support line.
    pub fn on_hss(&mut self, id: u64, reply: Result<String, String>, now: u64) {
        let Some((to, _)) = self.pending.remove(&id) else { return };
        let text = match reply {
            Ok(b) => {
                let used: u64 = crate::ims::kv(&b, "used").parse().unwrap_or(0);
                let quota: u64 = crate::ims::kv(&b, "quota").parse().unwrap_or(0);
                if quota == 0 {
                    format!("{} support: you have used {} KB of data this period (no limit).", self.cfg.carrier, used / 1024)
                } else {
                    format!("{} support: you have used {} KB of your {} KB.", self.cfg.carrier, used / 1024, quota / 1024)
                }
            }
            Err(e) => format!("{} support: your balance isn't available ({}).", self.cfg.carrier, e),
        };
        self.text(&to, &text, now);
    }

    /// A text from the support line, through the SMSC.
    fn text(&mut self, to: &str, text: &str, now: u64) {
        let host = self.host();
        let mut d = Dialog::outgoing(&mut self.rng, Uri::sip(Some("611"), &self.cfg.domain, None), None, Uri::sip(Some("service-text"), &self.cfg.smsc.host(), Some(self.cfg.smsc.port)), self.cfg.smsc, &host);
        let mut m = d.request(&mut self.rng, Method::Message, &host, self.cfg.sip_port);
        m.headers_mut().set("To", format!("<{}>", number_uri(to, &self.cfg.domain)));
        m.headers_mut().push("X-Service-From", "611");
        m.set_body("text/plain;charset=utf-8", text.as_bytes().to_vec());
        let smsc = self.cfg.smsc;
        self.tx.request(0, smsc, &m, now);
    }

    fn next_message(&mut self, port: u16, now: u64) {
        let Some(s) = self.sessions.get(&port) else { return };
        let target = s.target.clone();
        let cur = s.current;
        if let (Some(c), Some(b)) = (cur, self.boxes.get_mut(&target)) {
            if let Some(m) = b.iter_mut().find(|m| m.id == c) {
                if !m.heard {
                    m.heard = true;
                }
            }
        }
        if cur.is_some() {
            self.save_index(&target);
            self.mwi(&target, now);
        }
        // new ones first, then the saved, after the current
        let next = self.boxes.get(&target).and_then(|b| {
            let mut v: Vec<&Voicemail> = b.iter().filter(|m| Some(m.id) > cur.or(Some(0)) || cur.is_none()).collect();
            v.sort_by_key(|m| (m.heard, m.id));
            v.into_iter().find(|m| Some(m.id) != cur).map(|m| (m.id, m.samples.clone()))
        });
        let s = self.sessions.get_mut(&port).expect("checked");
        match next {
            Some((id, samples)) => {
                s.current = Some(id);
                // a short beep, the message
                let mut p = Tone::custom(&[(&[1000.0], 150), (&[], 150)], false).next_samples(4800);
                p.extend(samples);
                s.play = p;
            }
            None => {
                s.current = None;
                s.play = Tone::custom(&[(&[880.0], 200), (&[660.0], 200), (&[], 200)], false).next_samples(9600);
                s.end_at = Some(now + 700);
            }
        }
    }

    /// Message-waiting indication to the box's phone.
    fn mwi(&mut self, number: &str, now: u64) {
        let (new, old) = self.boxes.get(number).map(|b| (b.iter().filter(|m| !m.heard).count(), b.iter().filter(|m| m.heard).count())).unwrap_or((0, 0));
        if new + old == 0 && !self.boxes.contains_key(number) {
            return;
        }
        let host = self.host();
        let mut d = Dialog::outgoing(&mut self.rng, Uri::sip(Some("voicemail"), &self.cfg.domain, None), None, number_uri(number, &self.cfg.domain), self.cfg.ims, &host);
        let mut m = d.request(&mut self.rng, Method::Notify, &host, self.cfg.sip_port);
        m.headers_mut().push("Event", "message-summary");
        m.headers_mut().push("Subscription-State", "active");
        let body = format!("Messages-Waiting: {}\r\nMessage-Account: sip:voicemail@{}\r\nVoice-Message: {}/{}\r\n", if new > 0 { "yes" } else { "no" }, self.cfg.domain, new, old);
        m.set_body("application/simple-message-summary", body.into_bytes());
        let ims = self.cfg.ims;
        self.tx.request(0, ims, &m, now);
    }

    fn end(&mut self, port: u16, send_bye: bool, now: u64) {
        let Some(mut s) = self.sessions.remove(&port) else { return };
        self.by_cid.remove(&s.dialog.call_id);
        self.out.push(MrfOut::CloseRtp(port));
        if send_bye {
            let host = self.host();
            let bye = s.dialog.request(&mut self.rng, Method::Bye, &host, self.cfg.sip_port);
            let hop = s.dialog.next_hop;
            self.tx.request(0, hop, &bye, now);
        }
        if s.service == Service::Deposit {
            if let Some(rec) = s.recording.take() {
                // trim trailing silence, drop recordings under half a second
                let mut end = rec.len();
                while end > 0 && rec[end - 1].unsigned_abs() < 200 {
                    end -= 1;
                }
                let rec = rec[..end].to_vec();
                if rec.len() >= 8000 {
                    let id = self.next_vm;
                    self.next_vm += 1;
                    let ms = rec.len() as u64 * 1000 / 16_000;
                    self.out.push(MrfOut::Save { path: format!("{}/{}/{}.wav", SPOOL, s.target, id), data: wav_bytes(&rec) });
                    self.boxes.entry(s.target.clone()).or_default().push(Voicemail { id, from: s.caller.clone(), at_ms: now, ms, heard: false, samples: rec });
                    let t = s.target.clone();
                    self.save_index(&t);
                    self.log(format!("voicemail {} for {} from {} ({} ms)", id, s.target, s.caller, ms));
                    self.mwi(&t, now);
                }
            }
        }
    }

    /// Every 20 ms: play, record and mix.
    pub fn tick(&mut self, now: u64) {
        self.tx.tick(now);
        let _ = self.tx.take_events();
        let ports: Vec<u16> = self.sessions.keys().copied().collect();
        // conference mixing: each participant hears the sum of the others in its conference
        let mut heard: BTreeMap<u16, Vec<i16>> = BTreeMap::new();
        for p in &ports {
            let s = self.sessions.get_mut(p).expect("listed");
            let input = s.jitter.pull(FRAME);
            match s.service {
                Service::Conference => s.last = input,
                Service::Deposit if s.tone.as_ref().is_none_or(|t| t.is_done()) => {
                    s.tone = None;
                    s.recording.get_or_insert_with(Vec::new).extend(input);
                }
                _ => {}
            }
        }
        for p in &ports {
            let s = &self.sessions[p];
            if s.service != Service::Conference {
                continue;
            }
            let mut mix = vec![0i32; FRAME];
            for q in &ports {
                let o = &self.sessions[q];
                if q != p && o.service == Service::Conference && o.conf == s.conf {
                    for (m, x) in mix.iter_mut().zip(&o.last) {
                        *m += *x as i32;
                    }
                }
            }
            heard.insert(*p, mix.into_iter().map(|x| x.clamp(-32768, 32767) as i16).collect());
        }
        let mut ended = Vec::new();
        let mut start_retrieval = Vec::new();
        for p in &ports {
            let s = self.sessions.get_mut(p).expect("listed");
            let mut frame: Vec<i16> = if let Some(h) = heard.remove(p) {
                h
            } else if let Some(t) = s.tone.as_mut() {
                let f = t.next_samples(FRAME);
                if t.is_done() && s.service == Service::Retrieve && s.current.is_none() && s.play.is_empty() {
                    start_retrieval.push(*p);
                    s.tone = None;
                }
                f
            } else if !s.play.is_empty() {
                let n = FRAME.min(s.play.len());
                let f: Vec<i16> = s.play.drain(..n).collect();
                if s.play.is_empty() && s.service == Service::Retrieve && s.current.is_some() {
                    // the message finished: the next one after a moment
                    start_retrieval.push(*p);
                }
                f
            } else {
                vec![0; FRAME]
            };
            frame.resize(FRAME, 0);
            let payload = s.enc.encode_frame(&frame);
            let step = if s.neg.rate == 8000 { 160 } else { 320 };
            let pkt = RtpPacket::new(s.neg.pt, s.seq, s.ts, s.ssrc, payload);
            s.seq = s.seq.wrapping_add(1);
            s.ts = s.ts.wrapping_add(step);
            self.out.push(MrfOut::Rtp { port: *p, to: s.remote, data: pkt.encode() });
            if s.end_at.is_some_and(|t| now >= t) {
                ended.push(*p);
            }
            let _ = s.started_ms;
        }
        for p in start_retrieval {
            if !ended.contains(&p) {
                self.next_message(p, now);
            }
        }
        for p in ended {
            self.end(p, true, now);
        }
    }

    /// Visual voicemail over HTTP: `GET /vm` (the list), `GET /vm/<id>.wav`,
    /// `DELETE /vm/<id>`, `POST /vm/<id>/heard`, only from a registered phone's
    /// own IMS address. Returns (status, content type, body).
    pub fn http(&mut self, method: &str, path: &str, from: [u8; 4], now: u64) -> (u16, &'static str, Vec<u8>) {
        let Some(number) = self.phones.get(&from).cloned() else {
            return (403, "text/plain", b"not a registered phone of this carrier\n".to_vec());
        };
        let rest = path.trim_start_matches('/').strip_prefix("vm").unwrap_or("x");
        let rest = rest.trim_start_matches('/');
        match (method, rest) {
            ("GET", "") => {
                let mut s = String::new();
                for m in self.mailbox(&number) {
                    s.push_str(&format!("msg {} {} {} {} {}\n", m.id, m.from, m.at_ms, m.ms, if m.heard { "heard" } else { "new" }));
                }
                (200, "text/plain", s.into_bytes())
            }
            ("GET", r) if r.ends_with(".wav") => {
                let id: u64 = r.trim_end_matches(".wav").parse().unwrap_or(0);
                match self.mailbox(&number).iter().find(|m| m.id == id) {
                    Some(m) => (200, "audio/wav", wav_bytes(&m.samples)),
                    None => (404, "text/plain", b"no such message\n".to_vec()),
                }
            }
            ("POST", r) if r.ends_with("/heard") => {
                let id: u64 = r.trim_end_matches("/heard").parse().unwrap_or(0);
                if let Some(m) = self.boxes.get_mut(&number).and_then(|b| b.iter_mut().find(|m| m.id == id)) {
                    m.heard = true;
                    self.save_index(&number);
                    self.mwi(&number, now);
                    return (200, "text/plain", b"ok\n".to_vec());
                }
                (404, "text/plain", b"no such message\n".to_vec())
            }
            ("DELETE", r) => {
                let id: u64 = r.parse().unwrap_or(0);
                let had = self.mailbox(&number).iter().any(|m| m.id == id);
                if !had {
                    return (404, "text/plain", b"no such message\n".to_vec());
                }
                if let Some(b) = self.boxes.get_mut(&number) {
                    b.retain(|m| m.id != id);
                }
                self.out.push(MrfOut::Delete { path: format!("{}/{}/{}.wav", SPOOL, number, id) });
                self.save_index(&number);
                self.mwi(&number, now);
                (200, "text/plain", b"deleted\n".to_vec())
            }
            _ => (404, "text/plain", b"not found\n".to_vec()),
        }
    }

    /// Conference participants: (conference, RTP port, loudness of the last 20 ms, RTP packets in).
    pub fn conference_levels(&self) -> Vec<(u64, u16, u64)> {
        self.sessions.values().filter(|s| s.service == Service::Conference).map(|s| (s.conf, s.port, s.last.iter().map(|x| x.unsigned_abs() as u64).sum())).collect()
    }

    pub fn sessions(&self) -> usize {
        self.sessions.len()
    }
}
