//! The phone's IMS client (`telephonyd`), sans-IO: what iOS CommCenter or the
//! Android telephony stack does (docs/TELEPHONY.md).
//!
//! * **Registration.** Once the IMS bearer is up ([`Ua::bearer_up`]), `REGISTER`
//!   to the P-CSCF; the 401's nonce is the AKA challenge, answered by the SIM
//!   ([`UaOut::Aka`], the `sim` peripheral's `respond_ims`); the 200 says the
//!   number. Refreshed at half the expiry; lost registrations retry with back-off.
//! * **Calls** (at most two: one active and one held or waiting). The phone
//!   offers ADPCM-WB and ADPCM-NB and telephone-event; the network relays the
//!   media. Hold is a re-INVITE `sendonly`; merge (three-way) a `REFER` with
//!   `merge=`; DTMF is RFC 4733 events. Audio goes through [`Ua::mic`] and
//!   [`Ua::speaker`] (16 kHz mono), ringing through [`Ua::ring`]; the local tones
//!   (ringback, busy, intercept, call waiting) are generated here.
//! * **Texts**: SMS-SUBMIT in RP-DATA inside `MESSAGE` to the SMSC, split into
//!   parts with the concatenation header and status reports requested; incoming
//!   SMS-DELIVERs are acknowledged (RP-ACK) and rejoined. A WAP push on port 2948
//!   is an MMS notification ([`UaEvent::MmsNotice`]; the daemon fetches it).
//! * **Voicemail** indications (`NOTIFY` `message-summary`).

use std::collections::BTreeMap;

use ecm_rtp::codec::{Codec as RtpCodec, FrameEncoder};
use ecm_rtp::dtmf::{DtmfReceiver, DtmfSender, TelephoneEvent};
use ecm_rtp::jitter::JitterBuffer;
use ecm_rtp::rtp::RtpPacket;
use ecm_rtp::tones::Tone;

use crate::dialog::{ack_for, ack_non2xx, cancel_for, number_uri, response_target, uri_number, Dialog};
use crate::sdp::{self, Caps, Direction, Negotiated, SessionDescription};
use crate::sip::{self, new_tag, Message, Method, Uri};
use crate::sms::{self, Address, Body, Direction as RpDir, Reassembler, Rp, Submit, Tpdu};
use crate::txn::{Addr, Rng, TxEvent, Txns};

pub const SIP_PORT: u16 = 5060;
pub const RTP_BASE: u16 = 40000;
/// Samples per 20 ms frame at 16 kHz.
pub const FRAME: usize = 320;

#[derive(Clone, Debug)]
pub struct UaConfig {
    /// The SIM's IMSI (the private identity is `IMSI@domain`).
    pub imsi: String,
    pub sip_port: u16,
    pub rtp_base: u16,
    pub wideband: bool,
    pub expires: u32,
    /// Seconds a call rings before the phone gives up on its side (the network sends voicemail first).
    pub ring_timeout_ms: u64,
    pub cc: String,
    /// This phone downloads MMS (tells the network with `+ecm.mms` on its Contact).
    pub mms: bool,
}

impl UaConfig {
    pub fn new(imsi: &str) -> UaConfig {
        UaConfig { imsi: imsi.into(), sip_port: SIP_PORT, rtp_base: RTP_BASE, wideband: true, expires: 3600, ring_timeout_ms: 60_000, cc: "1".into(), mms: true }
    }
}

/// The IMS bearer as the attach gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct Bearer {
    pub ip: [u8; 4],
    pub pcscf: Addr,
    pub domain: String,
    /// From the attach (may be empty until the registration says it).
    pub msisdn: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegState {
    Off,
    Registering,
    Authenticating,
    Registered,
    Failed,
}

impl RegState {
    pub fn name(self) -> &'static str {
        match self {
            RegState::Off => "off",
            RegState::Registering => "registering",
            RegState::Authenticating => "authenticating",
            RegState::Registered => "registered",
            RegState::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallState {
    /// We sent the INVITE.
    Dialing,
    /// The far end rings (180), or early media plays (183).
    Alerting,
    /// An incoming call is ringing here.
    Incoming,
    Active,
    /// We put it on hold.
    Held,
    Ended,
}

impl CallState {
    pub fn name(self) -> &'static str {
        match self {
            CallState::Dialing => "dialing",
            CallState::Alerting => "alerting",
            CallState::Incoming => "incoming",
            CallState::Active => "active",
            CallState::Held => "held",
            CallState::Ended => "ended",
        }
    }
}

/// Why a call ended, as the phone shows it.
pub fn end_reason(code: u16) -> &'static str {
    match code {
        0 | 200 => "call ended",
        404 | 604 => "the number you dialed is not in service",
        480 | 408 => "the person you called is not available",
        486 | 600 => "busy",
        487 => "cancelled",
        403 => "call not allowed",
        488 => "no common codec",
        603 => "declined",
        _ => "call failed",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UaEvent {
    Registration { state: RegState, msisdn: String, why: String },
    Incoming { call: u32, from: String, hidden: bool, waiting: bool },
    CallState { call: u32, state: CallState, reason: String },
    /// A DTMF digit the far end sent (IVRs).
    Dtmf { call: u32, digit: char },
    SmsSent { id: u32 },
    SmsDelivered { id: u32 },
    SmsFailed { id: u32, why: String },
    Sms { from: String, text: String, timestamp_ms: u64 },
    /// An MMS is waiting at the MMSC (fetch `location` over HTTP).
    MmsNotice { from: String, location: String, transaction: String, size: u64, subject: String },
    /// The MMSC says a recipient fetched (or refused) an MMS this phone sent.
    MmsDelivered { message_id: String, to: String, status: String },
    /// A WAP-push / binary SMS for another port (passed up).
    BinarySms { from: String, port: u16, data: Vec<u8> },
    Voicemail { waiting: u32 },
}

#[derive(Debug, PartialEq)]
pub enum UaOut {
    Sip { to: Addr, data: Vec<u8> },
    /// An RTP datagram from local port `port`.
    Rtp { port: u16, to: Addr, data: Vec<u8> },
    OpenRtp(u16),
    CloseRtp(u16),
    /// Ask the SIM for IMS AKA on this RAND; answer with [`Ua::on_aka`].
    Aka { rand: [u8; 16] },
    Event(UaEvent),
    Log(String),
}

struct Media {
    port: u16,
    remote: Option<Addr>,
    neg: Option<Negotiated>,
    enc: Option<FrameEncoder>,
    jitter: JitterBuffer,
    seq: u16,
    ts: u32,
    ssrc: u32,
    dtmf_rx: DtmfReceiver,
    /// DTMF packets waiting to go out: (due ms, bytes).
    dtmf_tx: Vec<(u64, Vec<u8>)>,
    rx_packets: u64,
    tx_packets: u64,
    /// The sender's SSRC: a new one (the network switched the far end, e.g. to a
    /// conference mixer) starts the jitter buffer over.
    rx_ssrc: Option<u32>,
}

struct UaCall {
    id: u32,
    dialog: Dialog,
    state: CallState,
    outgoing: bool,
    /// The far end's number (or "anonymous"), and whether they hid it.
    remote: String,
    hidden: bool,
    media: Media,
    /// The INVITE (ours, to CANCEL/ACK it; theirs, to answer it).
    invite: Message,
    invite_from: Addr,
    offer: Option<SessionDescription>,
    started_ms: u64,
    answered_ms: Option<u64>,
    deadline: Option<u64>,
    muted: bool,
    /// Local tone being played into the speaker (ringback, busy, intercept, waiting beep).
    tone: Option<Tone>,
    tone_until: Option<u64>,
    end_code: u16,
    merged: bool,
}

struct OutSms {
    id: u32,
    /// per part MR: (sent to the SMSC, delivered)
    acked: BTreeMap<u8, (bool, bool)>,
    failed: bool,
}

pub struct Ua {
    pub cfg: UaConfig,
    rng: Rng,
    tx: Txns,
    bearer: Option<Bearer>,
    reg: RegState,
    reg_dialog: Option<Dialog>,
    reg_until: u64,
    reg_retry_at: Option<u64>,
    reg_failures: u32,
    pending_nonce: Option<(String, String)>,
    msisdn: String,
    calls: BTreeMap<u32, UaCall>,
    next_call: u32,
    /// Carrier settings from the registration: the MMSC and visual voicemail URLs.
    pub mmsc_url: String,
    pub vvm_url: String,
    next_port: u16,
    next_mr: u8,
    next_sms: u32,
    next_ref: u8,
    sms: BTreeMap<u32, OutSms>,
    /// MR -> outgoing SMS id.
    mr: BTreeMap<u8, u32>,
    /// MESSAGE Call-ID -> outgoing SMS id (a refused part fails its message).
    sms_by_cid: BTreeMap<String, u32>,
    reasm: Reassembler,
    /// Ringing (for the ringtone stream).
    ringing: Option<Tone>,
    out: Vec<UaOut>,
    now: u64,
}

impl Ua {
    pub fn new(cfg: UaConfig, seed: u64) -> Ua {
        let next_port = cfg.rtp_base;
        Ua {
            cfg,
            rng: Rng::new(seed),
            tx: Txns::new(),
            bearer: None,
            reg: RegState::Off,
            reg_dialog: None,
            reg_until: 0,
            reg_retry_at: None,
            reg_failures: 0,
            pending_nonce: None,
            msisdn: String::new(),
            calls: BTreeMap::new(),
            next_call: 1,
            mmsc_url: String::new(),
            vvm_url: String::new(),
            next_port,
            next_mr: 1,
            next_sms: 1,
            next_ref: (seed as u8).max(1),
            sms: BTreeMap::new(),
            mr: BTreeMap::new(),
            sms_by_cid: BTreeMap::new(),
            reasm: Reassembler::new(7 * 24 * 3600 * 1000),
            ringing: None,
            out: Vec::new(),
            now: 0,
        }
    }

    pub fn take_out(&mut self) -> Vec<UaOut> {
        for (to, data) in self.tx.take_sends() {
            self.out.push(UaOut::Sip { to, data });
        }
        std::mem::take(&mut self.out)
    }

    fn ev(&mut self, e: UaEvent) {
        self.out.push(UaOut::Event(e));
    }

    fn log(&mut self, s: String) {
        self.out.push(UaOut::Log(s));
    }

    pub fn registration(&self) -> RegState {
        self.reg
    }

    pub fn msisdn(&self) -> &str {
        &self.msisdn
    }

    pub fn bearer(&self) -> Option<&Bearer> {
        self.bearer.as_ref()
    }

    fn host(&self) -> String {
        self.bearer.as_ref().map(|b| Addr::new(b.ip, 0).host()).unwrap_or_default()
    }

    fn domain(&self) -> String {
        self.bearer.as_ref().map(|b| b.domain.clone()).unwrap_or_default()
    }

    fn pcscf(&self) -> Addr {
        self.bearer.as_ref().map(|b| b.pcscf).unwrap_or_default()
    }

    /// The home area code, for 7-digit dialling.
    pub fn home_npa(&self) -> Option<String> {
        let d = self.msisdn.strip_prefix('+')?.strip_prefix(&self.cfg.cc)?;
        (d.len() == 10).then(|| d[..3].to_string())
    }

    // ------------------------------------------------------------ bearer and registration

    /// The IMS bearer came up (attach or resume): register.
    pub fn bearer_up(&mut self, b: Bearer, now: u64) {
        self.now = now;
        let changed = self.bearer.as_ref() != Some(&b);
        if !b.msisdn.is_empty() {
            self.msisdn = b.msisdn.clone();
        }
        self.bearer = Some(b);
        if changed || self.reg != RegState::Registered {
            self.register(now);
        }
    }

    /// The bearer went (detach, SIM out): every call ends, the registration with it.
    pub fn bearer_down(&mut self, now: u64) {
        self.now = now;
        self.bearer = None;
        let ids: Vec<u32> = self.calls.keys().copied().collect();
        for id in ids {
            self.end_call(id, 480, now);
        }
        if self.reg != RegState::Off {
            self.reg = RegState::Off;
            let m = self.msisdn.clone();
            self.ev(UaEvent::Registration { state: RegState::Off, msisdn: m, why: "no IMS bearer".into() });
        }
    }

    fn register(&mut self, now: u64) {
        let Some(b) = self.bearer.clone() else { return };
        let impi = format!("{}@{}", self.cfg.imsi, b.domain);
        let aor = Uri::sip(Some(&self.cfg.imsi), &b.domain, None);
        let mut d = match self.reg_dialog.take() {
            Some(d) => d,
            None => Dialog::outgoing(&mut self.rng, aor.clone(), None, aor.clone(), b.pcscf, &Addr::new(b.ip, 0).host()),
        };
        d.target = Uri::sip(None, &b.domain, None);
        d.next_hop = b.pcscf;
        let host = Addr::new(b.ip, 0).host();
        let mut m = d.request(&mut self.rng, Method::Register, &host, self.cfg.sip_port);
        let h = m.headers_mut();
        let caps = if self.cfg.mms { ";+g.3gpp.smsip;+ecm.mms" } else { ";+g.3gpp.smsip" };
        h.push("Contact", format!("<sip:{}@{}:{}>;expires={}{}", self.cfg.imsi, host, self.cfg.sip_port, self.cfg.expires, caps));
        h.push("Expires", self.cfg.expires.to_string());
        h.push("Supported", "path, gruu");
        h.push("User-Agent", "ECM-telephonyd/1");
        match self.pending_nonce.take() {
            Some((nonce, response)) => {
                let a = sip::Digest::new("Digest")
                    .with("username", &impi)
                    .with("realm", &b.domain)
                    .with("nonce", &nonce)
                    .with("uri", &format!("sip:{}", b.domain))
                    .with("response", &response)
                    .with("algorithm", sip::AKA_ALGORITHM);
                h.push("Authorization", a.to_string());
                self.reg = RegState::Authenticating;
            }
            None => {
                self.reg = RegState::Registering;
            }
        }
        self.reg_dialog = Some(d);
        self.tx.request(0, b.pcscf, &m, now);
    }

    /// The SIM answered the AKA challenge: `res` (16 bytes), or None (no SIM / failed).
    pub fn on_aka(&mut self, res: Option<[u8; 16]>, now: u64) {
        let Some((nonce, _)) = self.pending_nonce.take() else { return };
        let Some(res) = res else {
            self.reg_failed("the SIM didn't answer the IMS challenge".into(), now);
            return;
        };
        let domain = self.domain();
        let impi = format!("{}@{}", self.cfg.imsi, domain);
        let response = sip::digest_response(&impi, &domain, &nonce, &format!("sip:{}", domain), "REGISTER", &res);
        self.pending_nonce = Some((nonce, response));
        self.register(now);
    }

    fn reg_failed(&mut self, why: String, now: u64) {
        self.reg = RegState::Failed;
        self.reg_failures += 1;
        let back = (5_000u64 << self.reg_failures.min(5)).min(120_000);
        self.reg_retry_at = Some(now + back);
        let m = self.msisdn.clone();
        self.log(format!("registration failed: {} (again in {} s)", why, back / 1000));
        self.ev(UaEvent::Registration { state: RegState::Failed, msisdn: m, why });
    }

    fn reg_response(&mut self, m: Message, now: u64) {
        let code = m.code().unwrap_or(0);
        match code {
            100..=199 => {}
            200..=299 => {
                let pau = m.headers().p_associated_uri().ok().unwrap_or_default();
                if let Some(n) = pau.iter().find_map(|u| uri_number(&u.uri)) {
                    self.msisdn = n;
                }
                if let Some(svc) = m.headers().get("P-ECM-Services") {
                    for part in svc.split(';').map(str::trim) {
                        if let Some(u) = part.strip_prefix("mmsc=") {
                            self.mmsc_url = u.to_string();
                        } else if let Some(u) = part.strip_prefix("vvm=") {
                            self.vvm_url = u.to_string();
                        }
                    }
                }
                let exp = m.headers().expires().ok().flatten().unwrap_or(self.cfg.expires).max(60);
                self.reg_until = now + exp as u64 * 1000;
                self.reg_retry_at = Some(now + exp as u64 * 500);
                self.reg_failures = 0;
                let first = self.reg != RegState::Registered;
                self.reg = RegState::Registered;
                if first {
                    let msisdn = self.msisdn.clone();
                    self.log(format!("registered as {}", msisdn));
                    self.ev(UaEvent::Registration { state: RegState::Registered, msisdn, why: String::new() });
                }
            }
            401 => {
                let ch = m.headers().www_authenticate().ok().flatten();
                let Some(ch) = ch else { return self.reg_failed("401 without a challenge".into(), now) };
                let Some(nonce) = ch.nonce().map(String::from) else { return self.reg_failed("401 without a nonce".into(), now) };
                let Ok(rand) = sip::aka_rand(&nonce) else { return self.reg_failed("bad AKA nonce".into(), now) };
                if let Some(d) = self.reg_dialog.as_mut() {
                    d.learn(&m);
                }
                self.pending_nonce = Some((nonce, String::new()));
                self.out.push(UaOut::Aka { rand });
            }
            403 => self.reg_failed("the network refused this SIM (403)".into(), now),
            _ => self.reg_failed(format!("REGISTER answered {}", code), now),
        }
    }

    // ------------------------------------------------------------ input

    pub fn on_sip(&mut self, from: Addr, data: &[u8], now: u64) {
        self.now = now;
        let Ok(m) = sip::parse(data) else { return };
        if m.is_request() {
            if !self.tx.on_request(from, &m, now) {
                return;
            }
            self.on_request(from, m, now);
        } else {
            self.tx.on_response(&m);
            self.on_response(m, now);
        }
    }

    fn respond(&mut self, req: &Message, from: Addr, code: u16, tag: Option<&str>, now: u64) {
        if let Ok(r) = req.response_to(code, tag) {
            self.tx.respond(response_target(req, from), &r, now);
        }
    }

    fn call_by_cid(&self, cid: &str) -> Option<u32> {
        self.calls.values().find(|c| c.dialog.call_id == cid).map(|c| c.id)
    }

    fn on_request(&mut self, from: Addr, m: Message, now: u64) {
        let method = m.method().cloned().unwrap_or(Method::Other(String::new()));
        let cid = m.headers().call_id().unwrap_or("").to_string();
        if let Some(id) = self.call_by_cid(&cid) {
            return self.in_call(id, from, m, now);
        }
        match method {
            Method::Invite => self.incoming(from, m, now),
            Method::Message => self.message(from, m, now),
            Method::Notify => {
                let ev = m.headers().event().ok().flatten().map(|e| e.value).unwrap_or_default();
                self.respond(&m, from, 200, None, now);
                if ev.eq_ignore_ascii_case("message-summary") {
                    let body = String::from_utf8_lossy(m.body()).to_string();
                    let waiting = body
                        .lines()
                        .find_map(|l| l.strip_prefix("Voice-Message:"))
                        .and_then(|v| v.trim().split('/').next().and_then(|n| n.trim().parse().ok()))
                        .unwrap_or(0);
                    self.ev(UaEvent::Voicemail { waiting });
                }
            }
            Method::Options => self.respond(&m, from, 200, None, now),
            Method::Ack => {}
            Method::Bye | Method::Cancel => self.respond(&m, from, 481, None, now),
            _ => self.respond(&m, from, 405, None, now),
        }
    }

    fn on_response(&mut self, m: Message, now: u64) {
        let Some(cseq) = m.headers().cseq().ok() else { return };
        let cid = m.headers().call_id().unwrap_or("").to_string();
        if cseq.method == Method::Register && self.reg_dialog.as_ref().is_some_and(|d| d.call_id == cid) {
            return self.reg_response(m, now);
        }
        if cseq.method == Method::Message {
            let code = m.code().unwrap_or(0);
            if code >= 300 {
                // the network refused a part: which message?
                if let Some(id) = self.sms_by_cid.remove(&cid) {
                    self.sms_failed(id, format!("refused ({})", code));
                }
            } else if code >= 200 {
                self.sms_by_cid.remove(&cid);
            }
            return;
        }
        if let Some(id) = self.call_by_cid(&cid) {
            self.call_response(id, m, now);
        }
    }

    // ------------------------------------------------------------ calls

    fn alloc_port(&mut self) -> u16 {
        let p = self.next_port;
        self.next_port = if p >= self.cfg.rtp_base + 400 { self.cfg.rtp_base } else { p + 2 };
        self.out.push(UaOut::OpenRtp(p));
        p
    }

    fn new_media(&mut self, port: u16) -> Media {
        Media {
            port,
            remote: None,
            neg: None,
            enc: None,
            jitter: JitterBuffer::new(16_000),
            seq: (self.rng.next() & 0x7fff) as u16,
            ts: self.rng.next() as u32,
            ssrc: self.rng.next() as u32,
            dtmf_rx: DtmfReceiver::new(),
            dtmf_tx: Vec::new(),
            rx_packets: 0,
            tx_packets: 0,
            rx_ssrc: None,
        }
    }

    fn apply_neg(m: &mut Media, neg: Negotiated) {
        let codec = match neg.codec {
            sdp::Codec::AdpcmWb => RtpCodec::AdpcmWb,
            sdp::Codec::AdpcmNb => RtpCodec::AdpcmNb,
        };
        m.remote = Addr::parse_ip(&neg.remote_addr).map(|ip| Addr::new(ip, neg.remote_port));
        if m.enc.as_ref().map(|e| e.codec()) != Some(codec) {
            m.enc = Some(FrameEncoder::new(codec));
        }
        m.neg = Some(neg);
    }

    /// Place a call. `number` is E.164 (or a service code: `*86`, `611`, `*72...`).
    pub fn dial(&mut self, number: &str, hide_id: bool, now: u64) -> Result<u32, String> {
        self.now = now;
        if self.reg != RegState::Registered {
            return Err("not registered with the network (no VoLTE)".into());
        }
        if self.calls.values().filter(|c| c.state != CallState::Ended).count() >= 2 {
            return Err("already in two calls".into());
        }
        // a second call puts the first on hold
        let active: Vec<u32> = self.calls.values().filter(|c| c.state == CallState::Active).map(|c| c.id).collect();
        for a in active {
            self.hold(a, true, now)?;
        }
        let domain = self.domain();
        let host = self.host();
        let me = number_uri(&self.msisdn, &domain);
        let target = number_uri(number, &domain);
        let pcscf = self.pcscf();
        let mut d = Dialog::outgoing(&mut self.rng, me, None, target, pcscf, &host);
        let port = self.alloc_port();
        let sess = self.rng.next() >> 16;
        let offer = sdp::offer(&host, port, sess, 1, self.cfg.wideband);
        let mut inv = d.request(&mut self.rng, Method::Invite, &host, self.cfg.sip_port);
        let h = inv.headers_mut();
        h.push("Contact", format!("<sip:{}@{}:{}>", self.msisdn, host, self.cfg.sip_port));
        h.push("P-Preferred-Identity", format!("<{}>", number_uri(&self.msisdn, &domain)));
        if hide_id {
            h.push("Privacy", "id");
        }
        h.push("Allow", "INVITE, ACK, BYE, CANCEL, OPTIONS, MESSAGE, NOTIFY, REFER, UPDATE");
        inv.set_body("application/sdp", offer.to_bytes());
        let id = self.next_call;
        self.next_call += 1;
        let media = self.new_media(port);
        let pcscf = self.pcscf();
        self.calls.insert(
            id,
            UaCall {
                id,
                dialog: d,
                state: CallState::Dialing,
                outgoing: true,
                remote: number.into(),
                hidden: hide_id,
                media,
                invite: inv.clone(),
                invite_from: pcscf,
                offer: Some(offer),
                started_ms: now,
                answered_ms: None,
                deadline: None,
                muted: false,
                tone: None,
                tone_until: None,
                end_code: 0,
                merged: false,
            },
        );
        self.tx.request(id as u64, pcscf, &inv, now);
        self.log(format!("calling {}", number));
        self.ev(UaEvent::CallState { call: id, state: CallState::Dialing, reason: String::new() });
        Ok(id)
    }

    fn incoming(&mut self, from: Addr, m: Message, now: u64) {
        if self.reg != RegState::Registered {
            return self.respond(&m, from, 480, None, now);
        }
        let live = self.calls.values().filter(|c| c.state != CallState::Ended).count();
        if live >= 2 {
            return self.respond(&m, from, 486, None, now);
        }
        let Ok(offer) = SessionDescription::parse(m.body()) else {
            return self.respond(&m, from, 488, None, now);
        };
        let tag = new_tag(self.rng.next());
        let Some(dialog) = Dialog::incoming(&m, &tag, from) else {
            return self.respond(&m, from, 400, None, now);
        };
        let h = m.headers();
        let hidden = h.privacy().iter().any(|p| p.eq_ignore_ascii_case("id"))
            || h.from().ok().and_then(|f| f.uri.user().map(|u| u == "anonymous")).unwrap_or(false);
        let caller = if hidden {
            "anonymous".to_string()
        } else {
            h.p_asserted_identity().ok().and_then(|v| v.into_iter().find_map(|n| uri_number(&n.uri))).or_else(|| h.from().ok().and_then(|f| uri_number(&f.uri))).unwrap_or_else(|| "unknown".into())
        };
        let id = self.next_call;
        self.next_call += 1;
        let port = self.alloc_port();
        let media = self.new_media(port);
        let waiting = live > 0;
        self.calls.insert(
            id,
            UaCall {
                id,
                dialog,
                state: CallState::Incoming,
                outgoing: false,
                remote: caller.clone(),
                hidden,
                media,
                invite: m.clone(),
                invite_from: from,
                offer: Some(offer),
                started_ms: now,
                answered_ms: None,
                deadline: Some(now + self.cfg.ring_timeout_ms),
                muted: false,
                tone: if waiting { Some(Tone::call_waiting()) } else { None },
                tone_until: None,
                end_code: 0,
                merged: false,
            },
        );
        if let Ok(r) = m.response_to(180, Some(&tag)) {
            self.tx.respond(response_target(&m, from), &r, now);
        }
        if !waiting {
            self.ringing = Some(Tone::ringtone());
        }
        self.log(format!("incoming call from {}{}", caller, if waiting { " (waiting)" } else { "" }));
        self.ev(UaEvent::Incoming { call: id, from: caller, hidden, waiting });
    }

    /// Answer a ringing call (a waiting call puts the active one on hold).
    pub fn answer(&mut self, id: u32, now: u64) -> Result<(), String> {
        self.now = now;
        let st = self.calls.get(&id).map(|c| c.state).ok_or("no such call")?;
        if st != CallState::Incoming {
            return Err("that call isn't ringing".into());
        }
        let others: Vec<u32> = self.calls.values().filter(|c| c.id != id && c.state == CallState::Active).map(|c| c.id).collect();
        for o in others {
            self.hold(o, true, now)?;
        }
        let host = self.host();
        let caps = if self.cfg.wideband { Caps::ALL } else { Caps::NARROWBAND };
        let sess = self.rng.next() >> 16;
        let c = self.calls.get_mut(&id).expect("checked");
        let offer = c.offer.clone().ok_or("no offer")?;
        let (ans, neg) = match sdp::answer(&offer, &host, c.media.port, sess, 1, &caps) {
            Ok(x) => x,
            Err(e) => {
                let inv = c.invite.clone();
                let from = c.invite_from;
                let tag = c.dialog.local.tag().map(String::from);
                self.respond(&inv, from, e.sip_code(), tag.as_deref(), now);
                self.end_call(id, 488, now);
                return Err("no common codec".into());
            }
        };
        Self::apply_neg(&mut c.media, neg);
        let mut r = c.invite.response_to(200, c.dialog.local.tag()).map_err(|e| e.to_string())?;
        r.headers_mut().push("Contact", format!("<sip:{}@{}:{}>", self.msisdn, host, self.cfg.sip_port));
        r.set_body("application/sdp", ans.to_bytes());
        let to = response_target(&c.invite, c.invite_from);
        c.state = CallState::Active;
        c.answered_ms = Some(now);
        c.deadline = None;
        c.tone = None;
        self.ringing = None;
        self.tx.respond(to, &r, now);
        self.ev(UaEvent::CallState { call: id, state: CallState::Active, reason: String::new() });
        Ok(())
    }

    /// Decline a ringing call (the caller hears busy), or hang up any other.
    pub fn hangup(&mut self, id: u32, now: u64) -> Result<(), String> {
        self.now = now;
        let c = self.calls.get_mut(&id).ok_or("no such call")?;
        let host = Addr::new(self.bearer.as_ref().map(|b| b.ip).unwrap_or_default(), 0).host();
        match c.state {
            CallState::Incoming => {
                let inv = c.invite.clone();
                let from = c.invite_from;
                let tag = c.dialog.local.tag().map(String::from);
                self.respond(&inv, from, 603, tag.as_deref(), now);
            }
            CallState::Dialing | CallState::Alerting => {
                if let Some(cancel) = cancel_for(&c.invite) {
                    let hop = c.dialog.next_hop;
                    self.tx.request(id as u64, hop, &cancel, now);
                }
            }
            CallState::Active | CallState::Held => {
                let bye = c.dialog.request(&mut self.rng, Method::Bye, &host, self.cfg.sip_port);
                let hop = c.dialog.next_hop;
                self.tx.request(id as u64, hop, &bye, now);
            }
            CallState::Ended => {}
        }
        self.end_call(id, 200, now);
        Ok(())
    }

    /// Hold (re-INVITE sendonly) or resume a call.
    pub fn hold(&mut self, id: u32, on: bool, now: u64) -> Result<(), String> {
        let host = self.host();
        let c = self.calls.get_mut(&id).ok_or("no such call")?;
        match (c.state, on) {
            (CallState::Active, true) | (CallState::Held, false) => {}
            _ => return Ok(()),
        }
        let offer = c.offer.clone().unwrap_or_else(|| sdp::offer(&host, c.media.port, 1, 1, true));
        let dir = if on { Direction::SendOnly } else { Direction::SendRecv };
        let mut sd = offer.with_direction(dir);
        // our own offer for the re-INVITE: our address and port
        sd.origin.addr = host.clone();
        sd.connection = Some(host.clone());
        for m in sd.media.iter_mut() {
            m.port = c.media.port;
            m.connection = None;
        }
        let mut re = c.dialog.request(&mut self.rng, Method::Invite, &host, self.cfg.sip_port);
        re.headers_mut().push("Contact", format!("<sip:{}@{}:{}>", self.msisdn, host, self.cfg.sip_port));
        re.set_body("application/sdp", sd.to_bytes());
        c.state = if on { CallState::Held } else { CallState::Active };
        let hop = c.dialog.next_hop;
        self.tx.request(id as u64, hop, &re, now);
        let st = c.state;
        self.ev(UaEvent::CallState { call: id, state: st, reason: String::new() });
        Ok(())
    }

    /// Swap: hold the active call, resume the held one.
    pub fn swap(&mut self, now: u64) -> Result<(), String> {
        let active = self.calls.values().find(|c| c.state == CallState::Active).map(|c| c.id);
        let held = self.calls.values().find(|c| c.state == CallState::Held).map(|c| c.id).ok_or("no held call")?;
        if let Some(a) = active {
            self.hold(a, true, now)?;
        }
        self.hold(held, false, now)
    }

    /// Merge the two calls into a three-way call (the network's conference).
    pub fn merge(&mut self, now: u64) -> Result<(), String> {
        let ids: Vec<u32> = self.calls.values().filter(|c| matches!(c.state, CallState::Active | CallState::Held)).map(|c| c.id).collect();
        if ids.len() != 2 {
            return Err("merge needs two calls".into());
        }
        // both active, then REFER in the first with the second's Call-ID
        for id in &ids {
            self.hold(*id, false, now)?;
        }
        let host = self.host();
        let domain = self.domain();
        let other = self.calls[&ids[1]].dialog.call_id.clone();
        let c = self.calls.get_mut(&ids[0]).expect("listed");
        let mut r = c.dialog.request(&mut self.rng, Method::Refer, &host, self.cfg.sip_port);
        r.headers_mut().push("Refer-To", format!("<sip:conference@{};merge={}>", domain, other));
        r.headers_mut().push("Contact", format!("<sip:{}@{}:{}>", self.msisdn, host, self.cfg.sip_port));
        let hop = c.dialog.next_hop;
        c.merged = true;
        self.tx.request(ids[0] as u64, hop, &r, now);
        // the merged audio comes on the first call's stream; the second stays up but quiet
        if let Some(c2) = self.calls.get_mut(&ids[1]) {
            c2.merged = true;
            c2.muted = true;
        }
        Ok(())
    }

    pub fn mute(&mut self, id: u32, on: bool) -> Result<(), String> {
        self.calls.get_mut(&id).ok_or("no such call")?.muted = on;
        Ok(())
    }

    /// Send keypad digits as RFC 4733 events (IVRs read them).
    pub fn send_dtmf(&mut self, id: u32, digits: &str, now: u64) -> Result<(), String> {
        let c = self.calls.get_mut(&id).ok_or("no such call")?;
        let pt = c.media.neg.as_ref().and_then(|n| n.dtmf_pt).ok_or("the far end takes no keypad tones")?;
        let rate = c.media.neg.as_ref().map(|n| n.rate).unwrap_or(16_000);
        let mut s = DtmfSender::new(rate, c.media.ts);
        for p in s.packets(digits) {
            let mut rtp = RtpPacket::new(pt, 0, p.timestamp, c.media.ssrc, p.event.encode().to_vec());
            rtp.marker = p.marker;
            c.media.dtmf_tx.push((now + p.at_ms as u64, rtp.encode()));
        }
        Ok(())
    }

    fn end_call(&mut self, id: u32, code: u16, now: u64) {
        let Some(mut c) = self.calls.remove(&id) else { return };
        c.state = CallState::Ended;
        c.end_code = code;
        self.out.push(UaOut::CloseRtp(c.media.port));
        if !self.calls.values().any(|x| x.state == CallState::Incoming) {
            self.ringing = None;
        }
        let reason = end_reason(code).to_string();
        let _ = now;
        self.ev(UaEvent::CallState { call: id, state: CallState::Ended, reason });
    }

    fn call_response(&mut self, id: u32, m: Message, now: u64) {
        let code = m.code().unwrap_or(0);
        let method = m.headers().cseq().map(|c| c.method).unwrap_or(Method::Other(String::new()));
        let host = self.host();
        let Some(c) = self.calls.get_mut(&id) else { return };
        if method != Method::Invite {
            return; // BYE/CANCEL/REFER answers
        }
        let initial = matches!(c.state, CallState::Dialing | CallState::Alerting);
        if !initial {
            // a re-INVITE's answer
            if (200..300).contains(&code) {
                c.dialog.learn(&m);
                if let (Some(off), Ok(ans)) = (c.offer.clone(), SessionDescription::parse(m.body())) {
                    if let Ok(neg) = sdp::accept_answer(&off, &ans) {
                        Self::apply_neg(&mut c.media, neg);
                    }
                }
                let ack = ack_for(&c.dialog, &mut self.rng, &host, self.cfg.sip_port);
                let hop = c.dialog.next_hop;
                self.tx.request(id as u64, hop, &ack, now);
            }
            return;
        }
        match code {
            100 => {}
            180 | 183 => {
                c.dialog.learn(&m);
                let early = !m.body().is_empty();
                if early {
                    if let (Some(off), Ok(ans)) = (c.offer.clone(), SessionDescription::parse(m.body())) {
                        if let Ok(neg) = sdp::accept_answer(&off, &ans) {
                            Self::apply_neg(&mut c.media, neg);
                        }
                    }
                    c.tone = None; // the network plays the audio
                } else if c.tone.is_none() {
                    c.tone = Some(Tone::ringback());
                }
                if c.state != CallState::Alerting {
                    c.state = CallState::Alerting;
                    self.ev(UaEvent::CallState { call: id, state: CallState::Alerting, reason: String::new() });
                }
            }
            200..=299 => {
                c.dialog.learn(&m);
                if let (Some(off), Ok(ans)) = (c.offer.clone(), SessionDescription::parse(m.body())) {
                    match sdp::accept_answer(&off, &ans) {
                        Ok(neg) => Self::apply_neg(&mut c.media, neg),
                        Err(_) => {}
                    }
                }
                let ack = ack_for(&c.dialog, &mut self.rng, &host, self.cfg.sip_port);
                let hop = c.dialog.next_hop;
                self.tx.request(id as u64, hop, &ack, now);
                c.state = CallState::Active;
                c.answered_ms = Some(now);
                c.tone = None;
                self.ev(UaEvent::CallState { call: id, state: CallState::Active, reason: String::new() });
            }
            _ => {
                if let Some(ack) = ack_non2xx(&c.invite, &m) {
                    let hop = c.dialog.next_hop;
                    self.tx.request(id as u64, hop, &ack, now);
                }
                // busy: the phone plays the busy tone a few seconds before the call screen goes
                let reason = end_reason(code).to_string();
                let line = format!("call to {} ended: {} ({})", c.remote, reason, code);
                self.out.push(UaOut::Log(line));
                let Some(c) = self.calls.get_mut(&id) else { return };
                if code == 486 || code == 600 {
                    c.tone = Some(Tone::busy());
                    c.tone_until = Some(now + 3000);
                    c.state = CallState::Ended;
                    c.end_code = code;
                    self.ev(UaEvent::CallState { call: id, state: CallState::Ended, reason });
                    return;
                }
                self.end_call(id, code, now);
            }
        }
    }

    fn in_call(&mut self, id: u32, from: Addr, m: Message, now: u64) {
        let method = m.method().cloned().unwrap_or(Method::Other(String::new()));
        match method {
            Method::Ack => {}
            Method::Bye => {
                self.respond(&m, from, 200, None, now);
                self.end_call(id, 200, now);
            }
            Method::Cancel => {
                self.respond(&m, from, 200, None, now);
                if let Some(c) = self.calls.get(&id) {
                    if c.state == CallState::Incoming {
                        let inv = c.invite.clone();
                        let f = c.invite_from;
                        let tag = c.dialog.local.tag().map(String::from);
                        self.respond(&inv, f, 487, tag.as_deref(), now);
                        // a missed call
                        self.end_call(id, 487, now);
                    }
                }
            }
            Method::Invite | Method::Update => {
                // the network's re-INVITE (the far end held or resumed us)
                let host = self.host();
                let Some(c) = self.calls.get_mut(&id) else { return };
                let Ok(off) = SessionDescription::parse(m.body()) else {
                    return self.respond(&m, from, 488, None, now);
                };
                let sess = self.rng.next() >> 16;
                let caps = if self.cfg.wideband { Caps::ALL } else { Caps::NARROWBAND };
                match sdp::answer(&off, &host, c.media.port, sess, 2, &caps) {
                    Ok((ans, neg)) => {
                        Self::apply_neg(&mut c.media, neg);
                        if let Ok(mut r) = m.response_to(200, None) {
                            r.headers_mut().push("Contact", format!("<sip:{}@{}:{}>", self.msisdn, host, self.cfg.sip_port));
                            r.set_body("application/sdp", ans.to_bytes());
                            self.tx.respond(response_target(&m, from), &r, now);
                        }
                    }
                    Err(e) => self.respond(&m, from, e.sip_code(), None, now),
                }
            }
            Method::Notify | Method::Info | Method::Options | Method::Message => self.respond(&m, from, 200, None, now),
            _ => self.respond(&m, from, 405, None, now),
        }
    }

    // ------------------------------------------------------------ media

    /// RTP arrived on local port `port`.
    pub fn on_rtp(&mut self, port: u16, _from: Addr, data: &[u8], now: u64) {
        let Some(c) = self.calls.values_mut().find(|c| c.media.port == port) else { return };
        let Some(p) = RtpPacket::decode(data) else { return };
        let Some(neg) = c.media.neg.clone() else { return };
        c.media.rx_packets += 1;
        if c.media.rx_ssrc != Some(p.ssrc) {
            if c.media.rx_ssrc.is_some() {
                c.media.jitter.reset();
            }
            c.media.rx_ssrc = Some(p.ssrc);
        }
        if Some(p.payload_type) == neg.dtmf_pt {
            if let Some(d) = c.media.dtmf_rx.push(p.timestamp, &p.payload) {
                let id = c.id;
                self.out.push(UaOut::Event(UaEvent::Dtmf { call: id, digit: d }));
            }
            let _ = TelephoneEvent::decode(&p.payload);
            return;
        }
        if p.payload_type != neg.pt {
            return;
        }
        let codec = match neg.codec {
            sdp::Codec::AdpcmWb => RtpCodec::AdpcmWb,
            sdp::Codec::AdpcmNb => RtpCodec::AdpcmNb,
        };
        if let Some(pcm) = codec.decode_frame(&p.payload) {
            // the jitter buffer runs on a 16 kHz clock whatever the codec
            let ts = if codec == RtpCodec::AdpcmNb { p.timestamp.wrapping_mul(2) } else { p.timestamp };
            c.media.jitter.push(p.seq, ts, pcm, now);
        }
    }

    /// 20 ms of microphone audio (16 kHz mono): sent on the call that is active.
    pub fn mic(&mut self, pcm: &[i16], now: u64) {
        self.now = now;
        for c in self.calls.values_mut() {
            if c.state != CallState::Active || (c.merged && c.muted) {
                continue;
            }
            let (Some(remote), Some(neg), Some(enc)) = (c.media.remote, c.media.neg.clone(), c.media.enc.as_mut()) else { continue };
            if !neg.direction.sends() {
                continue;
            }
            for chunk in pcm.chunks(FRAME) {
                let mut frame = chunk.to_vec();
                frame.resize(FRAME, 0);
                if c.muted {
                    frame.iter_mut().for_each(|s| *s = 0);
                }
                let payload = enc.encode_frame(&frame);
                let step = if neg.rate == 8000 { 160 } else { 320 };
                let p = RtpPacket::new(neg.pt, c.media.seq, c.media.ts, c.media.ssrc, payload);
                c.media.seq = c.media.seq.wrapping_add(1);
                c.media.ts = c.media.ts.wrapping_add(step);
                c.media.tx_packets += 1;
                self.out.push(UaOut::Rtp { port: c.media.port, to: remote, data: p.encode() });
            }
        }
    }

    /// `n` samples for the earpiece/speaker: the active call's voice and local tones.
    pub fn speaker(&mut self, n: usize) -> Vec<i16> {
        let mut out = vec![0i32; n];
        for c in self.calls.values_mut() {
            if c.state == CallState::Active && !(c.merged && c.muted) {
                for (o, s) in out.iter_mut().zip(c.media.jitter.pull(n)) {
                    *o += s as i32;
                }
            } else if matches!(c.state, CallState::Alerting) && c.tone.is_none() {
                // early media (an announcement)
                for (o, s) in out.iter_mut().zip(c.media.jitter.pull(n)) {
                    *o += s as i32;
                }
            }
            if let Some(t) = c.tone.as_mut() {
                for (o, s) in out.iter_mut().zip(t.next_samples(n)) {
                    *o += s as i32;
                }
            }
        }
        out.into_iter().map(|s| s.clamp(i16::MIN as i32, i16::MAX as i32) as i16).collect()
    }

    /// `n` samples of ringtone while a call rings (none otherwise).
    pub fn ring(&mut self, n: usize) -> Option<Vec<i16>> {
        self.ringing.as_mut().map(|t| t.next_samples(n))
    }

    pub fn is_ringing(&self) -> bool {
        self.ringing.is_some()
    }

    // ------------------------------------------------------------ texts

    fn smsc_uri(&self) -> Uri {
        Uri::sip(Some("smsc"), &self.domain(), None)
    }

    /// Send a text: split into parts (GSM 7-bit 160 / 153, UCS-2 70 / 67), each a
    /// `MESSAGE` to the SMSC with a status report requested. Returns its id.
    pub fn send_sms(&mut self, to: &str, text: &str, now: u64) -> Result<u32, String> {
        if self.reg != RegState::Registered {
            return Err("not registered with the network".into());
        }
        let r = self.next_ref;
        self.next_ref = self.next_ref.wrapping_add(1).max(1);
        let parts = sms::split_text(text, r);
        let id = self.next_sms;
        self.next_sms += 1;
        let mut mrs = Vec::new();
        for ud in parts {
            let mr = self.next_mr;
            self.next_mr = self.next_mr.wrapping_add(1);
            let mut sub = Submit::new(mr, Address::parse(to), ud);
            sub.status_report_request = true;
            let tpdu = Tpdu::Submit(sub).encode();
            let rp = Rp::Data { mr, smsc: Address::parse("+0"), tpdu };
            self.send_rp(&rp.encode(RpDir::MsToNetwork), Some(id), now);
            self.mr.insert(mr, id);
            mrs.push(mr);
        }
        self.sms.insert(id, OutSms { id, acked: mrs.iter().map(|m| (*m, (false, false))).collect(), failed: false });
        Ok(id)
    }

    fn send_rp(&mut self, body: &[u8], sms_id: Option<u32>, now: u64) {
        let host = self.host();
        let domain = self.domain();
        let (smsc, pcscf) = (self.smsc_uri(), self.pcscf());
        let mut d = Dialog::outgoing(&mut self.rng, number_uri(&self.msisdn, &domain), None, smsc, pcscf, &host);
        let mut m = d.request(&mut self.rng, Method::Message, &host, self.cfg.sip_port);
        m.headers_mut().push("P-Preferred-Identity", format!("<{}>", number_uri(&self.msisdn, &domain)));
        m.headers_mut().push("Request-Disposition", "no-fork");
        m.set_body("application/vnd.3gpp.sms", body.to_vec());
        if let Some(id) = sms_id {
            self.sms_by_cid.insert(d.call_id.clone(), id);
        }
        let hop = self.pcscf();
        self.tx.request(0, hop, &m, now);
    }

    fn sms_failed(&mut self, id: u32, why: String) {
        if let Some(s) = self.sms.get_mut(&id) {
            if !s.failed {
                s.failed = true;
                self.out.push(UaOut::Event(UaEvent::SmsFailed { id, why }));
            }
        }
    }

    fn message(&mut self, from: Addr, m: Message, now: u64) {
        let ct = m.headers().content_type().ok().flatten().map(|c| c.value.to_ascii_lowercase()).unwrap_or_default();
        if ct != "application/vnd.3gpp.sms" {
            return self.respond(&m, from, 415, None, now);
        }
        self.respond(&m, from, 200, None, now);
        let Ok(rp) = Rp::decode(m.body(), RpDir::NetworkToMs) else { return };
        match rp {
            Rp::Ack { mr, .. } => {
                if let Some(&id) = self.mr.get(&mr) {
                    let done = {
                        let s = self.sms.get_mut(&id).expect("mr map in sync");
                        if let Some(e) = s.acked.get_mut(&mr) {
                            e.0 = true;
                        }
                        s.acked.values().all(|a| a.0)
                    };
                    if done {
                        self.ev(UaEvent::SmsSent { id });
                    }
                }
            }
            Rp::Error { mr, cause, .. } => {
                if let Some(&id) = self.mr.get(&mr) {
                    self.sms_failed(id, sms_cause(cause).into());
                }
            }
            Rp::Data { mr, smsc: _, tpdu } => {
                // acknowledge to the SMSC first (RP-ACK), then use it
                let ack = Rp::Ack { mr, tpdu: None }.encode(RpDir::MsToNetwork);
                self.send_rp(&ack, None, now);
                match Tpdu::decode_mt(&tpdu) {
                    Ok(Tpdu::Deliver(d)) => self.delivered(d, now),
                    Ok(Tpdu::StatusReport(sr)) => {
                        if let Some(&id) = self.mr.get(&sr.mr) {
                            let ok = sr.status < 0x20;
                            if !ok {
                                self.sms_failed(id, format!("not delivered (status {:#04x})", sr.status));
                                return;
                            }
                            let done = {
                                let s = self.sms.get_mut(&id).expect("in sync");
                                if let Some(e) = s.acked.get_mut(&sr.mr) {
                                    *e = (true, true);
                                }
                                s.acked.values().all(|a| a.1)
                            };
                            if done {
                                self.ev(UaEvent::SmsDelivered { id });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Rp::Smma { .. } => {}
        }
    }

    fn delivered(&mut self, d: sms::Deliver, now: u64) {
        let from = d.originator.to_string();
        let ts = d.timestamp.to_game_millis();
        if let Some((dst, _)) = d.ud.ports() {
            // a binary SMS to an application port: WAP push (MMS) or something else
            let full = if d.ud.concat().is_some() { self.reasm.push(&from, d.ud.clone(), now) } else { Some(d.ud.clone()) };
            let Some(ud) = full else { return };
            let data = match &ud.body {
                Body::Binary(b) => b.clone(),
                Body::Text(t) => t.as_bytes().to_vec(),
            };
            if dst == sms::WAP_PUSH_PORT {
                if let Ok((ct, body)) = crate::mms::wap_push_decode(&data) {
                    if ct.contains("mms-message") {
                        if let Ok(p) = crate::mms::Pdu::decode(&body) {
                            if p.message_type == crate::mms::MessageType::DeliveryInd {
                                let to = p.to.first().map(|a| crate::mms::parse_address(a).0.to_string()).unwrap_or_default();
                                let status = match p.status {
                                    Some(crate::mms::Status::Retrieved) => "delivered",
                                    Some(crate::mms::Status::Rejected) => "rejected",
                                    Some(crate::mms::Status::Expired) => "expired",
                                    Some(crate::mms::Status::Unreachable) => "unreachable",
                                    Some(crate::mms::Status::Forwarded) => "forwarded",
                                    _ => "unknown",
                                };
                                self.ev(UaEvent::MmsDelivered { message_id: p.message_id.clone().unwrap_or_default(), to, status: status.into() });
                                return;
                            }
                            if p.message_type == crate::mms::MessageType::NotificationInd {
                                self.ev(UaEvent::MmsNotice {
                                    from: p.from_address().map(|a| crate::mms::parse_address(a).0.to_string()).unwrap_or_else(|| from.clone()),
                                    location: p.content_location.clone().unwrap_or_default(),
                                    transaction: p.transaction_id.clone().unwrap_or_default(),
                                    size: p.message_size.unwrap_or(0),
                                    subject: p.subject.clone().unwrap_or_default(),
                                });
                                return;
                            }
                        }
                    }
                }
            }
            self.ev(UaEvent::BinarySms { from, port: dst, data });
            return;
        }
        let ud = if d.ud.concat().is_some() {
            match self.reasm.push(&from, d.ud.clone(), now) {
                Some(u) => u,
                None => return,
            }
        } else {
            d.ud.clone()
        };
        let text = match ud.body {
            Body::Text(t) => t,
            Body::Binary(b) => String::from_utf8_lossy(&b).into(),
        };
        self.ev(UaEvent::Sms { from, text, timestamp_ms: ts });
    }

    // ------------------------------------------------------------ time

    pub fn tick(&mut self, now: u64) {
        self.now = now;
        self.tx.tick(now);
        for e in self.tx.take_events() {
            let TxEvent::Timeout { owner, method, .. } = e;
            match method {
                Method::Register => self.reg_failed("the P-CSCF didn't answer".into(), now),
                Method::Message => {
                    // can't tell which part from the owner; fail every pending text still waiting
                    let waiting: Vec<u32> = self.sms.values().filter(|s| !s.failed && s.acked.values().any(|a| !a.0)).map(|s| s.id).collect();
                    for id in waiting {
                        self.sms_failed(id, "the network didn't answer".into());
                    }
                }
                Method::Invite => {
                    let id = owner as u32;
                    if self.calls.get(&id).is_some_and(|c| c.state == CallState::Dialing) {
                        self.end_call(id, 408, now);
                    }
                }
                _ => {}
            }
        }
        if let Some(t) = self.reg_retry_at {
            if now >= t && self.bearer.is_some() {
                self.reg_retry_at = None;
                self.register(now);
            }
        }
        if self.reg == RegState::Registered && now >= self.reg_until {
            self.reg = RegState::Registering;
            self.register(now);
        }
        // calls: ring timeouts, busy tones ending, DTMF packets due
        let ids: Vec<u32> = self.calls.keys().copied().collect();
        for id in ids {
            let (deadline, tone_done, state) = {
                let c = &self.calls[&id];
                (c.deadline, c.tone_until.is_some_and(|t| now >= t), c.state)
            };
            if state == CallState::Ended && tone_done {
                let code = self.calls[&id].end_code;
                let c = self.calls.remove(&id).expect("listed");
                self.out.push(UaOut::CloseRtp(c.media.port));
                let _ = code;
                continue;
            }
            if deadline.is_some_and(|d| now >= d) && state == CallState::Incoming {
                let _ = self.hangup(id, now);
                continue;
            }
            let c = self.calls.get_mut(&id).expect("listed");
            if let Some(remote) = c.media.remote {
                let port = c.media.port;
                let due: Vec<Vec<u8>> = c.media.dtmf_tx.iter().filter(|(t, _)| *t <= now).map(|(_, b)| b.clone()).collect();
                c.media.dtmf_tx.retain(|(t, _)| *t > now);
                for b in due {
                    self.out.push(UaOut::Rtp { port, to: remote, data: b });
                }
            }
        }
    }

    // ------------------------------------------------------------ status

    pub fn calls(&self) -> Vec<CallInfo> {
        self.calls
            .values()
            .map(|c| CallInfo {
                id: c.id,
                state: c.state,
                remote: c.remote.clone(),
                hidden: c.hidden,
                outgoing: c.outgoing,
                started_ms: c.started_ms,
                answered_ms: c.answered_ms,
                muted: c.muted,
                hd: c.media.neg.as_ref().is_some_and(|n| n.codec == sdp::Codec::AdpcmWb),
                codec: c.media.neg.as_ref().map(|n| n.codec.name().to_string()).unwrap_or_default(),
                loss: c.media.jitter.loss_percent(),
                rx_packets: c.media.rx_packets,
                tx_packets: c.media.tx_packets,
                merged: c.merged,
            })
            .collect()
    }
}

/// A call as the Phone app shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct CallInfo {
    pub id: u32,
    pub state: CallState,
    pub remote: String,
    pub hidden: bool,
    pub outgoing: bool,
    pub started_ms: u64,
    pub answered_ms: Option<u64>,
    pub muted: bool,
    /// HD Voice (ADPCM-WB both ends).
    pub hd: bool,
    pub codec: String,
    pub loss: f32,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub merged: bool,
}

/// RP-Cause in words.
pub fn sms_cause(c: u8) -> &'static str {
    match c {
        1 => "unassigned number",
        8 => "operator determined barring",
        10 => "call barred",
        21 => "short message transfer rejected",
        22 => "the phone's memory is full",
        27 => "the phone is off",
        28 => "unidentified subscriber",
        29 => "facility rejected",
        30 => "unknown subscriber",
        38 => "network out of order",
        41 => "temporary failure",
        42 => "congestion",
        _ => "not delivered",
    }
}

