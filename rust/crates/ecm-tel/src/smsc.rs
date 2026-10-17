//! `smscd`, sans-IO: the carrier's SMSC and IP-SM-GW (docs/TELEPHONY.md).
//!
//! Texts reach it as SIP `MESSAGE`s that `imsd` proxies (3GPP TS 24.341: an
//! RP-DATA carrying an SMS-SUBMIT). It acknowledges the submit at once
//! (RP-ACK: the sender sees **Sent**), stores the message, and delivers it as an
//! SMS-DELIVER to the recipient (through `imsd`, which pages an idle phone). When
//! the phone's RP-ACK comes back, a status report (**Delivered**) goes to the
//! sender if it asked for one.
//!
//! * **Store and forward.** A recipient that isn't registered (`480`) or doesn't
//!   answer keeps its texts here; they go out when `imsd` says it registered
//!   (`NOTIFY Event: reg`) and on a back-off (30 s doubling to 10 min). Texts
//!   expire after `validity_ms` (default 7 in-game days) with a failure report.
//!   The queue is saved after every change ([`SmscOut::Save`]) and reloaded
//!   ([`Smsc::load`]), so a restart loses nothing.
//! * **Other carriers.** A text for a number that isn't ours goes to it the same
//!   way; `imsd`'s border routes it (ENUM) to that carrier, whose SMSC stores it
//!   (texts arriving from a peer come with `P-Called-Party-ID`).
//! * **Block lists** (the TAS's, from the HSS) drop a blocked sender's text quietly.
//! * **Rate limit**: at most `rate_per_min` submits a minute per sender
//!   (RP-ERROR 42 congestion beyond).
//! * **Short codes** (5-6 digits) go to a program over the local socket
//!   ([`SmscOut::ShortCode`]; it answers with [`Smsc::short_code_reply`]); the
//!   carrier's own `BAL` service on 22225 answers with the data balance.
//! * **WAP push** for MMS: `mmscd` sends `MESSAGE sip:wap-push@...` with the push
//!   PDU; it is delivered as a binary SMS to port 2948 (split when longer).
//! * One CDR line per text submitted ([`SmscOut::Cdr`]).

use std::collections::{BTreeMap, VecDeque};

use crate::dialog::{number_uri, response_target, uri_number, Dialog};
use crate::ims::kv;
use crate::sip::{self, Message, Method, Uri};
use crate::sms::{self, Address, Body, Deliver, Direction as RpDir, Ie, Rp, StatusReport, Timestamp, Tpdu, UserData};
use crate::txn::{Addr, Rng, TxEvent, Txns};
use crate::util::{hex, unhex};

pub const SIP_PORT: u16 = 5062;
/// The carrier's balance service (text `BAL`).
pub const BALANCE_CODE: &str = "22225";

#[derive(Clone, Debug)]
pub struct SmscConfig {
    pub domain: String,
    pub carrier: String,
    pub ip: [u8; 4],
    pub sip_port: u16,
    /// `imsd`.
    pub ims: Addr,
    /// The SMSC's own number (RP-OA / RP-DA).
    pub address: String,
    pub validity_ms: u64,
    pub rate_per_min: u32,
}

impl SmscConfig {
    pub fn new(domain: &str, carrier: &str, ip: [u8; 4]) -> SmscConfig {
        SmscConfig {
            domain: domain.into(),
            carrier: carrier.into(),
            ip,
            sip_port: SIP_PORT,
            ims: Addr::new(ip, 5060),
            address: "+10000000001".into(),
            // 7 in-game days of 20 minutes
            validity_ms: 7 * 20 * 60 * 1000,
            rate_per_min: 30,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum SmscOut {
    Sip { to: Addr, data: Vec<u8> },
    /// The whole queue file (write it).
    Save(String),
    Hss { id: u64, line: String },
    /// A text for short code `code` (a program on the local socket answers).
    ShortCode { code: String, from: String, text: String },
    Cdr(String),
    Log(String),
}

/// One stored text (one SMS part).
#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub id: u64,
    pub from: String,
    pub to: String,
    /// The SMS-DELIVER user data (with its UDH).
    pub ud: UserData,
    /// The sender asked for a status report, with this MR (`None`: from a peer, or WAP push).
    pub report_mr: Option<u8>,
    pub submitted_ms: u64,
    pub attempts: u32,
    pub next_try_ms: u64,
    /// Waiting for the phone's RP-ACK under this MR.
    pub inflight: Option<u8>,
}

enum Pending {
    /// Block-list check before storing.
    Udr { msg: Stored },
    /// Balance for the BAL service.
    Balance { to: String },
}

pub struct Smsc {
    pub cfg: SmscConfig,
    rng: Rng,
    tx: Txns,
    queue: BTreeMap<u64, Stored>,
    next_id: u64,
    next_mr: u8,
    /// Our MR towards a phone -> stored id.
    mr: BTreeMap<u8, u64>,
    /// MESSAGE Call-ID -> stored id (its SIP outcome).
    sent: BTreeMap<String, u64>,
    pending: BTreeMap<u64, Pending>,
    /// Submits per sender in the last minute.
    rate: BTreeMap<String, VecDeque<u64>>,
    next_ref: u8,
    out: Vec<SmscOut>,
    dirty: bool,
}

/// SMS-DELIVER user data as hex (the queue file).
fn ud_hex(ud: &UserData) -> String {
    let d = Deliver { more_messages: false, status_report_indication: false, reply_path: false, originator: Address::parse("+1"), pid: 0, timestamp: Timestamp::from_game_millis(0), ud: ud.clone() };
    hex(&Tpdu::Deliver(d).encode())
}

fn ud_from_hex(h: &str) -> Option<UserData> {
    match Tpdu::decode_mt(&unhex(h)?).ok()? {
        Tpdu::Deliver(d) => Some(d.ud),
        _ => None,
    }
}

impl Smsc {
    pub fn new(cfg: SmscConfig, seed: u64) -> Smsc {
        Smsc {
            cfg,
            rng: Rng::new(seed),
            tx: Txns::new(),
            queue: BTreeMap::new(),
            next_id: 1,
            next_mr: 1,
            mr: BTreeMap::new(),
            sent: BTreeMap::new(),
            pending: BTreeMap::new(),
            rate: BTreeMap::new(),
            next_ref: (seed as u8).max(1),
            out: Vec::new(),
            dirty: false,
        }
    }

    pub fn take_out(&mut self) -> Vec<SmscOut> {
        for (to, data) in self.tx.take_sends() {
            self.out.push(SmscOut::Sip { to, data });
        }
        if self.dirty {
            self.dirty = false;
            self.out.push(SmscOut::Save(self.save_text()));
        }
        std::mem::take(&mut self.out)
    }

    fn log(&mut self, s: String) {
        self.out.push(SmscOut::Log(s));
    }

    /// The queue file: `msg <id> <from> <to> <submitted> <attempts> <report mr|-> <ud hex>`.
    pub fn save_text(&self) -> String {
        let mut s = String::from("# smscd queue (store and forward)\n");
        for m in self.queue.values() {
            s.push_str(&format!(
                "msg {} {} {} {} {} {} {}\n",
                m.id,
                m.from,
                m.to,
                m.submitted_ms,
                m.attempts,
                m.report_mr.map(|r| r.to_string()).unwrap_or_else(|| "-".into()),
                ud_hex(&m.ud)
            ));
        }
        s
    }

    /// Reload a saved queue (`now` restarts the retry clock).
    pub fn load(&mut self, text: &str, now: u64) -> usize {
        let mut n = 0;
        for l in text.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            if let ["msg", id, from, to, sub, att, rmr, ud] = w.as_slice() {
                let (Ok(id), Ok(sub), Ok(att)) = (id.parse::<u64>(), sub.parse::<u64>(), att.parse::<u32>()) else { continue };
                let Some(ud) = ud_from_hex(ud) else { continue };
                let report_mr = rmr.parse().ok();
                self.queue.insert(id, Stored { id, from: from.to_string(), to: to.to_string(), ud, report_mr, submitted_ms: sub, attempts: att, next_try_ms: now, inflight: None });
                self.next_id = self.next_id.max(id + 1);
                n += 1;
            }
        }
        n
    }

    pub fn queued(&self) -> Vec<&Stored> {
        self.queue.values().collect()
    }

    fn host(&self) -> String {
        Addr::new(self.cfg.ip, 0).host()
    }

    fn respond(&mut self, req: &Message, from: Addr, code: u16, now: u64) {
        if let Ok(r) = req.response_to(code, Some(&sip::new_tag(self.rng.next()))) {
            self.tx.respond(response_target(req, from), &r, now);
        }
    }

    /// A `MESSAGE` with an RP body to `to` (a number), through imsd.
    fn send_rp(&mut self, to: &str, from_id: &str, body: Vec<u8>, track: Option<u64>, now: u64) {
        let host = self.host();
        let me = Uri::sip(Some(from_id), &self.cfg.domain, None);
        let mut d = Dialog::outgoing(&mut self.rng, me, None, number_uri(to, &self.cfg.domain), self.cfg.ims, &host);
        let mut m = d.request(&mut self.rng, Method::Message, &host, self.cfg.sip_port);
        m.set_body("application/vnd.3gpp.sms", body);
        if let Some(id) = track {
            self.sent.insert(d.call_id.clone(), id);
        }
        let ims = self.cfg.ims;
        self.tx.request(track.unwrap_or(0), ims, &m, now);
    }

    /// The sender's number on a request imsd proxied (P-Asserted-Identity).
    fn sender(m: &Message) -> Option<String> {
        let h = m.headers();
        h.p_asserted_identity().ok().and_then(|v| v.into_iter().find_map(|n| uri_number(&n.uri))).or_else(|| h.from().ok().and_then(|f| uri_number(&f.uri)))
    }

    pub fn on_sip(&mut self, from: Addr, data: &[u8], now: u64) {
        let Ok(m) = sip::parse(data) else { return };
        if !m.is_request() {
            self.tx.on_response(&m);
            return self.on_response(m, now);
        }
        if !self.tx.on_request(from, &m, now) {
            return;
        }
        match m.method() {
            Some(Method::Message) => self.message(from, m, now),
            Some(Method::Notify) => {
                self.respond(&m, from, 200, now);
                let body = String::from_utf8_lossy(m.body()).to_string();
                for l in body.lines() {
                    if let Some(n) = l.trim().strip_prefix("registered ") {
                        self.retry_for(n.trim(), now);
                    }
                }
            }
            Some(Method::Options) => self.respond(&m, from, 200, now),
            _ => self.respond(&m, from, 405, now),
        }
    }

    fn message(&mut self, from: Addr, m: Message, now: u64) {
        let user = m.uri().and_then(|u| u.user().map(String::from)).unwrap_or_default();
        // WAP push from mmscd: a binary SMS to port 2948
        if user == "wap-push" {
            let to = m.headers().to().ok().and_then(|t| uri_number(&t.uri));
            let Some(to) = to else { return self.respond(&m, from, 400, now) };
            self.respond(&m, from, 202, now);
            let r = self.next_ref;
            self.next_ref = self.next_ref.wrapping_add(1).max(1);
            let parts = binary_parts(m.body(), sms::WAP_PUSH_PORT, r);
            for ud in parts {
                self.store(Stored { id: 0, from: "MMS".into(), to: to.clone(), ud, report_mr: None, submitted_ms: now, attempts: 0, next_try_ms: now, inflight: None }, now);
            }
            return;
        }
        let ct = m.headers().content_type().ok().flatten().map(|c| c.value.to_ascii_lowercase()).unwrap_or_default();
        if ct != "application/vnd.3gpp.sms" {
            return self.respond(&m, from, 415, now);
        }
        let called = m.headers().p_called_party_id().ok().flatten().and_then(|n| uri_number(&n.uri));
        if let Some(to) = called {
            // from another carrier: an RP-DATA (network to MS) for one of our numbers
            self.respond(&m, from, 202, now);
            if let Ok(Rp::Data { tpdu, .. }) = Rp::decode(m.body(), RpDir::NetworkToMs) {
                match Tpdu::decode_mt(&tpdu) {
                    Ok(Tpdu::Deliver(d)) => {
                        let s = Stored { id: 0, from: d.originator.to_string(), to: to.clone(), ud: d.ud, report_mr: None, submitted_ms: now, attempts: 0, next_try_ms: now, inflight: None };
                        self.check_and_store(s, now);
                    }
                    Ok(Tpdu::StatusReport(sr)) => {
                        // a report for one of our senders: pass it on
                        let body = Rp::Data { mr: self.mr_next(), smsc: Address::parse(&self.cfg.address), tpdu: Tpdu::StatusReport(sr).encode() }.encode(RpDir::NetworkToMs);
                        self.send_rp(&to, "smsc", body, None, now);
                    }
                    _ => {}
                }
            }
            return;
        }
        let Some(sender) = Self::sender(&m) else {
            return self.respond(&m, from, 403, now);
        };
        self.respond(&m, from, 202, now);
        let Ok(rp) = Rp::decode(m.body(), RpDir::MsToNetwork) else {
            self.log(format!("bad RP from {}", sender));
            return;
        };
        match rp {
            Rp::Data { mr, tpdu, .. } => self.submit(&sender, mr, &tpdu, now),
            Rp::Ack { mr, .. } => self.delivered(mr, now),
            Rp::Error { mr, cause, .. } => self.refused(mr, cause, now),
            Rp::Smma { .. } => self.retry_for(&sender, now),
        }
    }

    fn mr_next(&mut self) -> u8 {
        let mr = self.next_mr;
        self.next_mr = self.next_mr.wrapping_add(1);
        mr
    }

    fn rp_reply(&mut self, to: &str, rp: Rp, now: u64) {
        let body = rp.encode(RpDir::NetworkToMs);
        self.send_rp(to, "smsc", body, None, now);
    }

    fn submit(&mut self, sender: &str, mr: u8, tpdu: &[u8], now: u64) {
        let Ok(Tpdu::Submit(s)) = Tpdu::decode_mo(tpdu) else {
            return self.rp_reply(sender, Rp::Error { mr, cause: 96, diagnostic: None, tpdu: None }, now);
        };
        // rate limit
        let q = self.rate.entry(sender.to_string()).or_default();
        while q.front().is_some_and(|t| now.saturating_sub(*t) >= 60_000) {
            q.pop_front();
        }
        if q.len() as u32 >= self.cfg.rate_per_min {
            self.log(format!("{}: over {} texts a minute, refused", sender, self.cfg.rate_per_min));
            return self.rp_reply(sender, Rp::Error { mr, cause: 42, diagnostic: None, tpdu: None }, now);
        }
        q.push_back(now);
        let to = s.destination.to_string();
        let text = match &s.ud.body {
            Body::Text(t) => t.clone(),
            Body::Binary(b) => String::from_utf8_lossy(b).into(),
        };
        self.out.push(SmscOut::Cdr(format!("sms time={} from={} to={} parts={} chars={}", now / 1000, sender, to, s.ud.concat().map(|c| c.1).unwrap_or(1), text.chars().count())));
        // short codes
        if (5..=6).contains(&to.len()) && to.bytes().all(|b| b.is_ascii_digit()) {
            self.rp_reply(sender, Rp::Ack { mr, tpdu: None }, now);
            if to == BALANCE_CODE && text.trim().eq_ignore_ascii_case("BAL") {
                let id = self.next_pending();
                self.pending.insert(id, Pending::Balance { to: sender.to_string() });
                self.out.push(SmscOut::Hss { id, line: format!("UDR {}", sender) });
                return;
            }
            self.out.push(SmscOut::ShortCode { code: to.clone(), from: sender.to_string(), text });
            return;
        }
        if !to.starts_with('+') {
            return self.rp_reply(sender, Rp::Error { mr, cause: 1, diagnostic: None, tpdu: None }, now);
        }
        // accepted: the sender sees Sent
        self.rp_reply(sender, Rp::Ack { mr, tpdu: None }, now);
        let st = Stored { id: 0, from: sender.to_string(), to, ud: s.ud, report_mr: s.status_report_request.then_some(mr), submitted_ms: now, attempts: 0, next_try_ms: now, inflight: None };
        self.check_and_store(st, now);
    }

    fn next_pending(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id + 1_000_000
    }

    /// The recipient's block list first (our numbers only; the HSS says `unknown user` otherwise).
    fn check_and_store(&mut self, s: Stored, now: u64) {
        let id = self.next_pending();
        let line = format!("UDR {}", s.to);
        self.pending.insert(id, Pending::Udr { msg: s });
        self.out.push(SmscOut::Hss { id, line });
        let _ = now;
    }

    pub fn on_hss(&mut self, id: u64, reply: Result<String, String>, now: u64) {
        let Some(p) = self.pending.remove(&id) else { return };
        match p {
            Pending::Udr { msg } => {
                let blocked = reply.as_ref().map(|b| kv(b, "block").split(',').any(|n| n == msg.from)).unwrap_or(false);
                if blocked {
                    self.log(format!("text from {} to {} dropped (blocked)", msg.from, msg.to));
                    return;
                }
                self.store(msg, now);
            }
            Pending::Balance { to } => {
                let text = match reply {
                    Ok(b) => {
                        let used: u64 = kv(&b, "used").parse().unwrap_or(0);
                        let quota: u64 = kv(&b, "quota").parse().unwrap_or(0);
                        let n = kv(&b, "msisdn");
                        let q = if quota == 0 { "no limit".to_string() } else { format_bytes(quota) };
                        format!("{}: you have used {} of {} of data. Your number is {}.", self.cfg.carrier, format_bytes(used), q, n)
                    }
                    Err(e) => format!("{}: balance not available ({})", self.cfg.carrier, e),
                };
                self.send_text(BALANCE_CODE, &to, &text, now);
            }
        }
    }

    /// A program on the short-code socket (or the carrier) answers a text.
    pub fn short_code_reply(&mut self, code: &str, to: &str, text: &str, now: u64) {
        self.send_text(code, to, text, now);
    }

    /// Store a text from `from` (a short code, a service) to `to`.
    pub fn send_text(&mut self, from: &str, to: &str, text: &str, now: u64) {
        let r = self.next_ref;
        self.next_ref = self.next_ref.wrapping_add(1).max(1);
        for ud in sms::split_text(text, r) {
            self.store(Stored { id: 0, from: from.into(), to: to.into(), ud, report_mr: None, submitted_ms: now, attempts: 0, next_try_ms: now, inflight: None }, now);
        }
    }

    fn store(&mut self, mut s: Stored, now: u64) {
        s.id = self.next_id;
        self.next_id += 1;
        let id = s.id;
        self.queue.insert(id, s);
        self.dirty = true;
        self.try_deliver(id, now);
    }

    fn try_deliver(&mut self, id: u64, now: u64) {
        let Some(s) = self.queue.get(&id).cloned() else { return };
        if s.inflight.is_some() {
            return;
        }
        let mr = self.mr_next();
        let d = Deliver {
            more_messages: false,
            status_report_indication: false,
            reply_path: false,
            originator: Address::parse(&s.from),
            pid: 0,
            timestamp: Timestamp::from_game_millis(s.submitted_ms),
            ud: s.ud.clone(),
        };
        let body = Rp::Data { mr, smsc: Address::parse(&self.cfg.address), tpdu: Tpdu::Deliver(d).encode() }.encode(RpDir::NetworkToMs);
        if let Some(q) = self.queue.get_mut(&id) {
            q.inflight = Some(mr);
            q.attempts += 1;
            q.next_try_ms = now + backoff(q.attempts);
        }
        self.mr.insert(mr, id);
        self.dirty = true;
        let to = s.to.clone();
        self.send_rp(&to, "smsc", body, Some(id), now);
    }

    /// Everything waiting for `number` goes out now (it registered).
    fn retry_for(&mut self, number: &str, now: u64) {
        let ids: Vec<u64> = self.queue.values().filter(|s| s.to == number && s.inflight.is_none()).map(|s| s.id).collect();
        if !ids.is_empty() {
            self.log(format!("{} registered: delivering {} stored text(s)", number, ids.len()));
        }
        for id in ids {
            self.try_deliver(id, now);
        }
    }

    fn on_response(&mut self, m: Message, now: u64) {
        let code = m.code().unwrap_or(0);
        if code < 200 {
            return;
        }
        let cid = m.headers().call_id().unwrap_or("").to_string();
        let Some(id) = self.sent.remove(&cid) else { return };
        if (200..300).contains(&code) {
            // 200: the phone has it (its RP-ACK follows); 202: another carrier's SMSC took it over
            if code == 202 {
                if let Some(mr) = self.queue.get(&id).and_then(|s| s.inflight) {
                    self.delivered(mr, now);
                }
            }
            return;
        }
        // not delivered now: stays queued (404: nobody has that number)
        let Some(s) = self.queue.get_mut(&id) else { return };
        if let Some(mr) = s.inflight.take() {
            self.mr.remove(&mr);
        }
        self.dirty = true;
        if code == 404 || code == 403 {
            let s = self.queue.remove(&id).expect("present");
            self.log(format!("text {} -> {}: {} (dropped)", s.from, s.to, code));
            self.report(&s, sms::status::PERM_INCOMPATIBLE_DESTINATION, now);
        }
    }

    fn delivered(&mut self, mr: u8, now: u64) {
        let Some(id) = self.mr.remove(&mr) else { return };
        let Some(s) = self.queue.remove(&id) else { return };
        self.dirty = true;
        self.log(format!("text {} -> {} delivered", s.from, s.to));
        self.report(&s, sms::status::DELIVERED, now);
    }

    fn refused(&mut self, mr: u8, cause: u8, now: u64) {
        let Some(id) = self.mr.remove(&mr) else { return };
        if cause == 22 {
            // memory full: keep it, the phone sends RP-SMMA when there is room
            if let Some(s) = self.queue.get_mut(&id) {
                s.inflight = None;
            }
            return;
        }
        let Some(s) = self.queue.remove(&id) else { return };
        self.dirty = true;
        self.report(&s, sms::status::PERM_REMOTE_PROCEDURE_ERROR, now);
    }

    fn report(&mut self, s: &Stored, status: u8, now: u64) {
        let Some(rmr) = s.report_mr else { return };
        let sr = StatusReport {
            more_messages: false,
            srq: false,
            mr: rmr,
            recipient: Address::parse(&s.to),
            sc_timestamp: Timestamp::from_game_millis(s.submitted_ms),
            discharge_time: Timestamp::from_game_millis(now),
            status,
        };
        let body = Rp::Data { mr: self.mr_next(), smsc: Address::parse(&self.cfg.address), tpdu: Tpdu::StatusReport(sr).encode() }.encode(RpDir::NetworkToMs);
        let to = s.from.clone();
        if to.starts_with('+') {
            self.send_rp(&to, "smsc", body, None, now);
        }
    }

    pub fn tick(&mut self, now: u64) {
        self.tx.tick(now);
        for e in self.tx.take_events() {
            let TxEvent::Timeout { owner, .. } = e;
            if let Some(s) = self.queue.get_mut(&owner) {
                if let Some(mr) = s.inflight.take() {
                    self.mr.remove(&mr);
                }
            }
        }
        // a sent MESSAGE that got a 200 but whose RP-ACK never came: try again later
        let stale: Vec<u64> = self.queue.values().filter(|s| s.inflight.is_some() && now >= s.next_try_ms + 30_000).map(|s| s.id).collect();
        for id in stale {
            if let Some(s) = self.queue.get_mut(&id) {
                if let Some(mr) = s.inflight.take() {
                    self.mr.remove(&mr);
                }
            }
        }
        let expired: Vec<u64> = self.queue.values().filter(|s| now.saturating_sub(s.submitted_ms) >= self.cfg.validity_ms).map(|s| s.id).collect();
        for id in expired {
            let s = self.queue.remove(&id).expect("listed");
            self.dirty = true;
            self.log(format!("text {} -> {} expired", s.from, s.to));
            self.report(&s, sms::status::PERM_VALIDITY_EXPIRED, now);
        }
        let due: Vec<u64> = self.queue.values().filter(|s| s.inflight.is_none() && now >= s.next_try_ms).map(|s| s.id).collect();
        for id in due {
            self.try_deliver(id, now);
        }
    }
}

/// 30 s, 1 min, 2 min, ... at most 10 min.
fn backoff(attempts: u32) -> u64 {
    (30_000u64 << attempts.saturating_sub(1).min(5)).min(600_000)
}

fn format_bytes(b: u64) -> String {
    if b >= 1 << 20 {
        format!("{:.1} MB", b as f64 / (1u64 << 20) as f64)
    } else if b >= 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{} bytes", b)
    }
}

/// A binary payload as 8-bit SMS parts to an application port (WAP push):
/// ports IE plus, when it needs more than one part, the concatenation IE.
pub fn binary_parts(data: &[u8], port: u16, reference: u8) -> Vec<UserData> {
    // 140 octets: UDH length (1) + ports IE (6) leaves 133; with concat (5 more) 128
    let single = 140 - 1 - 6;
    let chunks: Vec<&[u8]> = if data.len() <= single { vec![data] } else { data.chunks(128).collect() };
    let total = chunks.len().min(255) as u8;
    chunks
        .iter()
        .take(255)
        .enumerate()
        .map(|(i, c)| {
            let mut ud = UserData::binary(c);
            ud.udh.push(Ie::Ports16 { dest: port, src: sms::WAP_PUSH_SOURCE_PORT });
            if total > 1 {
                ud.udh.push(Ie::Concat8 { reference, total, seq: i as u8 + 1 });
            }
            ud
        })
        .collect()
}
