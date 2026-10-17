//! `imsd`, sans-IO: the carrier's IMS core (docs/TELEPHONY.md).
//!
//! One machine plays every CSCF role, as small carriers run them on one box:
//!
//! * **P-CSCF**: the phones' entry point (they learn its address in the attach).
//!   Requests from a phone must come from the IMS address registered for its
//!   number, so a phone can't send as another one; the network adds
//!   `P-Asserted-Identity` itself.
//! * **S-CSCF + registrar**: `REGISTER` with IMS AKA. The HSS (`cored`, [`ImsOut::Hss`])
//!   gives a vector (`MAR`); the phone answers the 401 with its SIM's RES; the
//!   registration is recorded at the HSS (`SAR`), and the 200 carries the number
//!   (`P-Associated-URI`).
//! * **TAS** (telephony application server): caller-ID restriction (`*67`,
//!   `Privacy: id`, the profile's CLIR), network block lists, DND, call
//!   forwarding (unconditional, busy, no answer after `no_answer_ms`, not
//!   reachable), call waiting, voicemail and announcements on `mrfd`, three-way
//!   calls (a merge REFER moves both calls into an `mrfd` conference).
//! * **I-CSCF / border (IBCF)**: numbers that aren't ours are found with ENUM
//!   ([`ImsOut::Enum`]); calls and texts go to the other carrier's border, signed
//!   with the peering secret (`P-ECM-Peer`); requests from a peer that isn't
//!   configured get `403`.
//! * **Media relay**: calls are a back-to-back user agent; every leg's RTP goes
//!   through a relay port pair here ([`ImsOut::OpenRelay`]), so the voice bearer
//!   (`AAR` at the PCRF) has a fixed far end and a carrier's phones never see
//!   another carrier's addresses.
//!
//! Texts (SIP `MESSAGE` with 3GPP RP/TPDUs) are proxied: from a phone to `smscd`,
//! from `smscd` to a registered phone or a peer, from a peer to `smscd`.
//! Registrations are announced to `smscd`, `mmscd` and `mrfd` with `NOTIFY`
//! (`Event: reg`), so stored texts and voicemail indications go out at once.
//! Every call ends in a CDR line ([`ImsOut::Cdr`]).

use std::collections::BTreeMap;

use crate::dialog::{ack_for, ack_non2xx, cancel_for, number_uri, response_target, stamp_via, uri_number, Dialog};
use crate::sdp::SessionDescription;
use crate::sip::{self, new_branch, new_tag, Message, Method, NameAddr, Uri, Via};
use crate::txn::{Addr, Rng, TxEvent, Txns};

pub const SIP_PORT: u16 = 5060;

#[derive(Clone, Debug, PartialEq)]
pub struct Peer {
    /// The other carrier's SIP domain.
    pub domain: String,
    /// Its border (found by SRV when unset).
    pub addr: Option<Addr>,
    /// The shared secret both sides configured.
    pub secret: String,
}

#[derive(Clone, Debug)]
pub struct ImsConfig {
    pub domain: String,
    pub carrier: String,
    /// Country code.
    pub cc: String,
    /// This computer's address towards the phones (the P-CSCF the attach hands out).
    pub ims_ip: [u8; 4],
    /// Towards other carriers (the border); often the same.
    pub public_ip: [u8; 4],
    pub sip_port: u16,
    /// The phones' IMS pool: a request from inside it is a phone's.
    pub ims_pool: ([u8; 4], u8),
    pub smsc: Addr,
    pub mrf: Addr,
    /// `mmscd`'s SIP port (registration notices), when it runs.
    pub mmsc: Option<Addr>,
    /// Relay ports `[from, to)`.
    pub relay_ports: (u16, u16),
    pub peers: Vec<Peer>,
    /// Accept and send calls with any carrier that publishes ENUM + SRV (no shared secret).
    pub public_peering: bool,
    pub no_answer_ms: u64,
    pub max_expires: u32,
}

impl ImsConfig {
    pub fn new(domain: &str, carrier: &str, ims_ip: [u8; 4]) -> ImsConfig {
        ImsConfig {
            domain: domain.into(),
            carrier: carrier.into(),
            cc: "1".into(),
            ims_ip,
            public_ip: ims_ip,
            sip_port: SIP_PORT,
            ims_pool: ([10, 250, 0, 0], 16),
            smsc: Addr::new(ims_ip, 5062),
            mrf: Addr::new(ims_ip, 5064),
            mmsc: Some(Addr::new(ims_ip, 5063)),
            relay_ports: (30000, 31000),
            peers: Vec::new(),
            public_peering: false,
            no_answer_ms: 30_000,
            max_expires: 3600,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum ImsOut {
    /// A SIP datagram from `sip_port`.
    Sip { to: Addr, data: Vec<u8> },
    /// A datagram from relay port `port`.
    Relay { port: u16, to: Addr, data: Vec<u8> },
    OpenRelay(u16),
    CloseRelay(u16),
    /// A request to the HSS (`cored`, crate `ecm_cell::hss`); answer with [`Ims::on_hss`].
    Hss { id: u64, line: String },
    /// Find the carrier of a number: ENUM NAPTR, then `_sip._udp` SRV; answer with [`Ims::on_enum`].
    Enum { id: u64, number: String },
    Cdr(String),
    Log(String),
}

/// What ENUM says about a number.
#[derive(Clone, Debug, PartialEq)]
pub struct EnumAnswer {
    pub domain: String,
    /// The border from the domain's SRV record.
    pub border: Option<Addr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Registration {
    pub imsi: String,
    pub msisdn: String,
    pub contact: Addr,
    pub expires_ms: u64,
    /// Calls this phone is in (for busy and call waiting).
    pub calls: usize,
}

/// The parts of a subscriber profile the TAS uses (from `UDR`/`SAR`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub imsi: String,
    pub msisdn: String,
    pub cfu: String,
    pub cfb: String,
    pub cfnr: String,
    pub cfnrc: String,
    pub cw: bool,
    pub clir: bool,
    pub dnd: bool,
    pub vm: bool,
    pub blocked: Vec<String>,
}

impl Profile {
    pub fn parse(body: &str) -> Profile {
        let v = |k: &str| -> String {
            body.lines()
                .find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix(' ')))
                .map(|s| if s == "-" { String::new() } else { s.to_string() })
                .unwrap_or_default()
        };
        Profile {
            imsi: v("imsi"),
            msisdn: v("msisdn"),
            cfu: v("cfu"),
            cfb: v("cfb"),
            cfnr: v("cfnr"),
            cfnrc: v("cfnrc"),
            cw: v("cw") != "0",
            clir: v("clir") == "1",
            dnd: v("dnd") == "1",
            vm: v("vm") != "0",
            blocked: v("block").split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
        }
    }
}

/// The SDP we give a leg: its relay port on our address facing it.
fn relay_sdp(cfg: &ImsConfig, sdp: &SessionDescription, kind: LegKind, port: u16) -> SessionDescription {
    let ip = match kind {
        LegKind::Peer => cfg.public_ip,
        _ => cfg.ims_ip,
    };
    let host = Addr::new(ip, 0).host();
    let mut s = sdp.clone();
    s.origin.addr = host.clone();
    s.connection = Some(host);
    for m in s.media.iter_mut() {
        m.port = port;
        m.connection = None;
    }
    s
}

/// `key value` from an HSS reply body.
pub fn kv(body: &str, k: &str) -> String {
    body.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix(' '))).map(String::from).unwrap_or_default()
}

/// The interconnect signature: `HMAC-SHA256(secret, "ECM-NNI" || domain || call-id)`, hex.
pub fn peer_proof(secret: &str, domain: &str, call_id: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut m = <Hmac<sha2::Sha256> as Mac>::new_from_slice(secret.as_bytes()).expect("any key");
    m.update(b"ECM-NNI");
    m.update(domain.as_bytes());
    m.update(call_id.as_bytes());
    m.finalize().into_bytes().iter().map(|b| format!("{:02x}", b)).collect()
}

fn in_net(ip: [u8; 4], net: ([u8; 4], u8)) -> bool {
    let mask = if net.1 == 0 { 0 } else { u32::MAX << (32 - net.1 as u32) };
    u32::from_be_bytes(ip) & mask == u32::from_be_bytes(net.0) & mask
}

/// Why a call ended where it did, for the CDR and the announcement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Answered,
    Busy,
    Declined,
    NoAnswer,
    Unreachable,
    NotInService,
    Cancelled,
    Rejected,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LegKind {
    /// One of our phones.
    Phone,
    /// Another carrier's border.
    Peer,
    /// The media server.
    Mrf,
}

struct Leg {
    kind: LegKind,
    dialog: Dialog,
    /// Our relay port facing this leg.
    relay: u16,
    /// Where this leg wants its RTP.
    media: Option<Addr>,
    /// A phone leg: its IMS address and RTP port (for the voice bearer) and number.
    ims_ip: Option<[u8; 4]>,
    number: String,
    bearer: Option<u8>,
    /// The INVITE we received (A leg) or sent (B leg).
    invite: Message,
    /// Where responses to `invite` go (A leg).
    reply_to: Addr,
    /// The latest SDP this leg sent us.
    sdp: Option<SessionDescription>,
    answered: bool,
    /// In a conference: this leg's audio goes to mrfd's mixer.
    mix: Option<Mix>,
    /// In a conference: this stream is the merging phone's second one (silent).
    muted: bool,
}

/// One participant's connection to the conference mixer on mrfd.
struct Mix {
    /// Our relay port facing mrfd for this participant.
    port: u16,
    /// Where mrfd wants this participant's audio.
    mrf_media: Option<Addr>,
    dialog: Dialog,
}

/// What a relay port is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RelayRole {
    /// Facing a call leg (A or B side).
    Leg { call: u64, a_side: bool },
    /// Facing mrfd's mixer for that leg.
    Mix { call: u64, a_side: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallSt {
    /// Looking up / waiting for HSS or ENUM.
    Routing,
    /// INVITE sent to B, not answered.
    Calling,
    Active,
    Ended,
}

struct Call {
    id: u64,
    st: CallSt,
    a: Leg,
    b: Option<Leg>,
    /// Who is calling (E.164 or a peer's URI) and whether they hid their number.
    caller: String,
    hide_id: bool,
    /// The number being called now (after forwarding) and the number first dialled.
    target: String,
    dialled: String,
    forwards: u8,
    /// The B leg is a voicemail deposit / an announcement / a service.
    to_service: bool,
    answer_deadline: Option<u64>,
    started_ms: u64,
    answered_ms: Option<u64>,
    outcome: Option<Outcome>,
    /// The callee's profile (TAS), once known.
    callee: Option<Profile>,
    /// In a conference (three-way): the conference id at mrfd.
    conf: Option<u64>,
}

enum Pending {
    Mar { req: Message, from: Addr, imsi: String },
    RegCheck { req: Message, from: Addr, imsi: String },
    Sar { req: Message, from: Addr, imsi: String, expires: u32 },
    /// Locate the callee of a call (LIR then UDR).
    CallLir { call: u64 },
    CallUdr { call: u64, imsi: String },
    CallEnum { call: u64 },
    /// A MESSAGE/NOTIFY for a number (LIR / ENUM).
    MsgLir { req: Message, from: Addr, number: String, from_peer: bool },
    MsgEnum { req: Message, from: Addr, number: String },
    /// A profile update from a phone (*72 / *73).
    Feature,
    /// A voice bearer for a call leg (the answer carries its id).
    Aar { call: u64, a_side: bool },
    /// Bearer setup answers (just logged).
    Bearer,
}

/// What the IMS core is doing, for `imsd status` and the IMS page.
#[derive(Clone, Debug, PartialEq)]
pub struct ImsStatus {
    pub registrations: Vec<Registration>,
    pub calls: Vec<(String, String, &'static str, u64)>,
    pub relays: usize,
    pub retransmits: u64,
}

pub struct Ims {
    pub cfg: ImsConfig,
    rng: Rng,
    tx: Txns,
    regs: BTreeMap<String, Registration>,
    /// AKA vectors in flight: IMSI -> (nonce, xres).
    vectors: BTreeMap<String, (String, Vec<u8>, u64)>,
    calls: BTreeMap<u64, Call>,
    /// Call-ID (of either leg) -> call id.
    by_call_id: BTreeMap<String, u64>,
    relays: BTreeMap<u16, RelayRole>,
    /// Conference mixer dialogs at mrfd: Call-ID -> (call, side).
    mixers: BTreeMap<String, (u64, bool)>,
    next_relay: u16,
    pending: BTreeMap<u64, Pending>,
    /// Client transaction owners: our id -> what it belongs to.
    next_id: u64,
    /// Proxied requests: our branch -> (the request as received, where its response goes).
    proxied: BTreeMap<String, (Message, Addr)>,
    /// Conferences at mrfd: id -> calls in it.
    confs: BTreeMap<u64, Vec<u64>>,
    out: Vec<ImsOut>,
}

impl Ims {
    pub fn new(cfg: ImsConfig, seed: u64) -> Ims {
        let next_relay = cfg.relay_ports.0;
        Ims {
            cfg,
            rng: Rng::new(seed),
            tx: Txns::new(),
            regs: BTreeMap::new(),
            vectors: BTreeMap::new(),
            calls: BTreeMap::new(),
            by_call_id: BTreeMap::new(),
            relays: BTreeMap::new(),
            mixers: BTreeMap::new(),
            next_relay,
            pending: BTreeMap::new(),
            next_id: 1,
            proxied: BTreeMap::new(),
            confs: BTreeMap::new(),
            out: Vec::new(),
        }
    }

    pub fn take_out(&mut self) -> Vec<ImsOut> {
        for (to, data) in self.tx.take_sends() {
            self.out.push(ImsOut::Sip { to, data });
        }
        std::mem::take(&mut self.out)
    }

    fn log(&mut self, s: String) {
        self.out.push(ImsOut::Log(s));
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn hss(&mut self, p: Pending, line: String) {
        let id = self.id();
        self.pending.insert(id, p);
        self.out.push(ImsOut::Hss { id, line });
    }

    fn host(&self) -> String {
        Addr::new(self.cfg.ims_ip, self.cfg.sip_port).host()
    }

    fn is_phone(&self, a: Addr) -> bool {
        in_net(a.ip, self.cfg.ims_pool)
    }

    pub fn status(&self) -> ImsStatus {
        ImsStatus {
            registrations: self.regs.values().cloned().collect(),
            calls: self
                .calls
                .values()
                .map(|c| {
                    let st = match c.st {
                        CallSt::Routing => "routing",
                        CallSt::Calling => "ringing",
                        CallSt::Active => {
                            if c.conf.is_some() {
                                "conference"
                            } else {
                                "active"
                            }
                        }
                        CallSt::Ended => "ended",
                    };
                    (c.caller.clone(), c.target.clone(), st, c.started_ms)
                })
                .collect(),
            relays: self.relays.len(),
            retransmits: self.tx.retransmits,
        }
    }

    pub fn registration(&self, msisdn: &str) -> Option<&Registration> {
        self.regs.get(msisdn)
    }

    fn reg_at(&self, a: Addr) -> Option<&Registration> {
        self.regs.values().find(|r| r.contact.ip == a.ip)
    }

    // ------------------------------------------------------------ input

    /// A datagram on the SIP port.
    pub fn on_sip(&mut self, from: Addr, data: &[u8], now: u64) {
        let Ok(m) = sip::parse(data) else {
            self.log(format!("unparsable SIP from {}", from));
            return;
        };
        if m.is_request() {
            if !self.tx.on_request(from, &m, now) {
                return; // a retransmission, answered again
            }
            self.on_request(from, m, now);
        } else {
            self.on_response(from, m, now);
        }
    }

    fn respond(&mut self, req: &Message, from: Addr, code: u16, tag: Option<&str>, now: u64) {
        if let Ok(r) = req.response_to(code, tag) {
            let to = response_target(req, from);
            self.tx.respond(to, &r, now);
        }
    }

    fn respond_with(&mut self, req: &Message, from: Addr, r: Message, now: u64) {
        let to = response_target(req, from);
        self.tx.respond(to, &r, now);
    }

    fn on_request(&mut self, from: Addr, m: Message, now: u64) {
        let method = m.method().cloned().unwrap_or(Method::Other(String::new()));
        // in-dialog requests of a call
        if let Ok(cid) = m.headers().call_id() {
            if let Some(&call) = self.by_call_id.get(cid) {
                if method != Method::Invite || m.headers().to().ok().and_then(|t| t.tag().map(String::from)).is_some() {
                    return self.in_call(call, from, m, now);
                }
            }
        }
        match method {
            Method::Register => self.register(from, m, now),
            Method::Invite => self.invite(from, m, now),
            Method::Ack => {}
            Method::Cancel => self.respond(&m, from, 481, None, now),
            Method::Options => {
                let mut r = match m.response_to(200, Some(&new_tag(self.rng.next()))) {
                    Ok(r) => r,
                    Err(_) => return,
                };
                r.headers_mut().push("Allow", "INVITE, ACK, BYE, CANCEL, OPTIONS, MESSAGE, NOTIFY, REFER, REGISTER");
                self.respond_with(&m, from, r, now);
            }
            Method::Message | Method::Notify | Method::Subscribe => self.route_request(from, m, now),
            _ => self.respond(&m, from, 405, None, now),
        }
    }

    // ------------------------------------------------------------ registration

    fn register(&mut self, from: Addr, m: Message, now: u64) {
        if !self.is_phone(from) {
            return self.respond(&m, from, 403, None, now);
        }
        let h = m.headers();
        let auth = h.authorization().ok().flatten();
        let to = match h.to() {
            Ok(t) => t,
            Err(_) => return self.respond(&m, from, 400, None, now),
        };
        // The private identity: the username (IMSI@domain), else the To user.
        let impi = auth.as_ref().and_then(|a| a.username().map(String::from)).unwrap_or_else(|| to.uri.user().unwrap_or("").to_string());
        let imsi = impi.split('@').next().unwrap_or("").to_string();
        if imsi.is_empty() {
            return self.respond(&m, from, 400, None, now);
        }
        match auth {
            None => self.hss(Pending::Mar { req: m, from, imsi: imsi.clone() }, format!("MAR {}", imsi)),
            Some(a) => {
                let Some((nonce, xres, _)) = self.vectors.get(&imsi).cloned() else {
                    // stale nonce: challenge again
                    return self.hss(Pending::Mar { req: m, from, imsi: imsi.clone() }, format!("MAR {}", imsi));
                };
                if a.nonce() != Some(nonce.as_str()) || !sip::verify_authorization(&a, "REGISTER", &xres) {
                    self.vectors.remove(&imsi);
                    self.log(format!("REGISTER {} from {}: IMS AKA failed", imsi, from));
                    return self.respond(&m, from, 403, None, now);
                }
                self.vectors.remove(&imsi);
                // the address it sends from must be its own IMS address
                self.hss(Pending::RegCheck { req: m, from, imsi }, format!("IPR {}", from.host()));
            }
        }
    }

    fn register_answer(&mut self, req: Message, from: Addr, profile: &Profile, expires: u32, now: u64) {
        let msisdn = profile.msisdn.clone();
        let mut r = match req.response_to(200, Some(&new_tag(self.rng.next()))) {
            Ok(r) => r,
            Err(_) => return,
        };
        let contact = match req.headers().contact() {
            Ok(sip::Contact::Addrs(a)) if !a.is_empty() => a[0].uri.clone(),
            _ => Uri::sip(Some(&msisdn), &from.host(), Some(from.port)),
        };
        let c_addr = Addr::new(from.ip, contact.port().unwrap_or(from.port));
        let h = r.headers_mut();
        if expires > 0 {
            h.push("Contact", format!("<{}>;expires={}", contact, expires));
            h.push("P-Associated-URI", format!("<{}>, <tel:{}>", number_uri(&msisdn, &self.cfg.domain), msisdn));
            h.push("Service-Route", format!("<sip:scscf.{};lr>", self.cfg.domain));
        }
        h.push("Expires", expires.to_string());
        self.respond_with(&req, from, r, now);
        if expires == 0 {
            self.regs.remove(&msisdn);
            self.log(format!("{} deregistered", msisdn));
            return;
        }
        let fresh = !self.regs.contains_key(&msisdn);
        let calls = self.regs.get(&msisdn).map(|r| r.calls).unwrap_or(0);
        self.regs.insert(msisdn.clone(), Registration { imsi: profile.imsi.clone(), msisdn: msisdn.clone(), contact: c_addr, expires_ms: now + expires as u64 * 1000, calls });
        self.log(format!("{} registered at {} ({} s){}", msisdn, c_addr, expires, if fresh { "" } else { ", refreshed" }));
        // stored texts, MMS notifications and voicemail indications go out now
        let mut targets = vec![self.cfg.smsc, self.cfg.mrf];
        if let Some(m) = self.cfg.mmsc {
            targets.push(m);
        }
        for t in targets {
            self.notify_reg(t, &msisdn, now);
        }
    }

    fn notify_reg(&mut self, to: Addr, msisdn: &str, now: u64) {
        let mut m = Message::request(Method::Notify, Uri::sip(Some("reg"), &to.host(), Some(to.port)));
        let host = self.host();
        let branch = new_branch(self.rng.next());
        let tag = new_tag(self.rng.next());
        let h = m.headers_mut();
        h.push("Via", Via::new("UDP", &host, Some(self.cfg.sip_port), &branch).to_string());
        h.push("Max-Forwards", "70");
        h.push("From", format!("<sip:scscf@{}>;tag={}", self.cfg.domain, tag));
        h.push("To", format!("<{}>", number_uri(msisdn, &self.cfg.domain)));
        h.push("Call-ID", sip::new_call_id(self.rng.next(), &host));
        h.push("CSeq", "1 NOTIFY");
        h.push("Event", "reg");
        h.push("Subscription-State", "active");
        m.set_body("text/plain", format!("registered {}\r\n", msisdn).into_bytes());
        self.tx.request(0, to, &m, now);
    }

    // ------------------------------------------------------------ HSS / ENUM answers

    /// The HSS answered request `id`: the reply body, or the error.
    pub fn on_hss(&mut self, id: u64, reply: Result<String, String>, now: u64) {
        let Some(p) = self.pending.remove(&id) else { return };
        match p {
            Pending::Mar { req, from, imsi } => match reply {
                Ok(b) => {
                    let rand = crate::util::unhex(&kv(&b, "rand")).unwrap_or_default();
                    let xres = crate::util::unhex(&kv(&b, "xres")).unwrap_or_default();
                    if rand.len() != 16 || xres.len() != 16 {
                        return self.respond(&req, from, 500, None, now);
                    }
                    let mut r16 = [0u8; 16];
                    r16.copy_from_slice(&rand);
                    let ch = sip::aka_challenge(&self.cfg.domain, &r16);
                    let nonce = ch.nonce().unwrap_or("").to_string();
                    self.vectors.insert(imsi, (nonce, xres, now));
                    if let Ok(mut r) = req.response_to(401, Some(&new_tag(self.rng.next()))) {
                        r.headers_mut().push("WWW-Authenticate", ch.to_string());
                        self.respond_with(&req, from, r, now);
                    }
                }
                Err(e) => {
                    self.log(format!("REGISTER {}: {}", imsi, e));
                    self.respond(&req, from, 403, None, now);
                }
            },
            Pending::RegCheck { req, from, imsi } => {
                let ok = reply.as_ref().map(|b| kv(b, "imsi") == imsi).unwrap_or(false);
                if !ok {
                    self.log(format!("REGISTER {} from {}: not its IMS address", imsi, from));
                    return self.respond(&req, from, 403, None, now);
                }
                let h = req.headers();
                let asked = h
                    .contact()
                    .ok()
                    .and_then(|c| match c {
                        sip::Contact::Addrs(a) => a.first().and_then(|x| x.expires()),
                        _ => Some(0),
                    })
                    .or_else(|| h.expires().ok().flatten())
                    .unwrap_or(self.cfg.max_expires);
                let expires = if asked == 0 { 0 } else { asked.clamp(60, self.cfg.max_expires) };
                let line = format!("SAR {} {} scscf.{}", imsi, if expires == 0 { "deregister" } else { "register" }, self.cfg.domain);
                self.hss(Pending::Sar { req, from, imsi, expires }, line);
            }
            Pending::Sar { req, from, imsi, expires } => match reply {
                Ok(b) => {
                    let p = Profile::parse(&b);
                    self.register_answer(req, from, &p, expires, now);
                }
                Err(e) => {
                    self.log(format!("REGISTER {}: {}", imsi, e));
                    self.respond(&req, from, 403, None, now);
                }
            },
            Pending::CallLir { call } => self.call_located(call, reply, now),
            Pending::CallUdr { call, imsi } => {
                let p = reply.map(|b| Profile::parse(&b)).unwrap_or_else(|_| Profile { imsi, vm: true, cw: true, ..Default::default() });
                self.call_terminate(call, p, now);
            }
            Pending::CallEnum { .. } | Pending::MsgEnum { .. } => {}
            Pending::MsgLir { req, from, number, from_peer } => self.msg_located(req, from, number, from_peer, reply, now),
            Pending::Aar { call, a_side } => {
                let id: Option<u8> = reply.as_ref().ok().and_then(|b| kv(b, "bearer").parse().ok());
                match (id, self.calls.get_mut(&call)) {
                    (Some(id), Some(c)) => {
                        let l = if a_side { Some(&mut c.a) } else { c.b.as_mut() };
                        if let Some(l) = l {
                            l.bearer = Some(id);
                        }
                    }
                    (Some(_), None) => {}
                    (None, _) => self.log(format!("voice bearer for call {}: {}", call, reply.err().unwrap_or_default())),
                }
            }
            Pending::Feature | Pending::Bearer => {
                if let Err(e) = reply {
                    self.log(format!("hss: {}", e));
                }
            }
        }
    }

    /// ENUM (and SRV) answered request `id`.
    pub fn on_enum(&mut self, id: u64, answer: Option<EnumAnswer>, now: u64) {
        let Some(p) = self.pending.remove(&id) else { return };
        match p {
            Pending::CallEnum { call } => self.call_enum(call, answer, now),
            Pending::MsgEnum { req, from, number } => {
                match answer.and_then(|a| self.border_for(&a).map(|b| (a, b))) {
                    Some((a, (to, secret))) if a.domain != self.cfg.domain => self.proxy_to_peer(req, from, &number, &a.domain, to, secret, now),
                    _ => {
                        self.log(format!("text to {}: no carrier serves it", number));
                        self.respond(&req, from, 404, None, now);
                    }
                }
            }
            _ => {}
        }
    }

    /// The border address and secret for a peer domain (configured, or public peering).
    fn border_for(&self, a: &EnumAnswer) -> Option<(Addr, Option<String>)> {
        if let Some(p) = self.cfg.peers.iter().find(|p| p.domain == a.domain) {
            return p.addr.or(a.border).map(|b| (b, Some(p.secret.clone())));
        }
        if self.cfg.public_peering {
            return a.border.map(|b| (b, None));
        }
        None
    }

    /// Does a request from `from` come from a peer we accept? Returns its domain.
    fn peer_ok(&self, m: &Message, from: Addr) -> Option<String> {
        let h = m.headers();
        let cid = h.call_id().ok()?;
        let hdr = h.get("P-ECM-Peer");
        if let Some(v) = hdr {
            let ph = sip::ParamHeader::parse(v).ok()?;
            let domain = ph.value.trim().to_string();
            let proof = ph.params.get("proof").unwrap_or("");
            if let Some(p) = self.cfg.peers.iter().find(|p| p.domain == domain) {
                if crate::util::ct_eq_str(&peer_proof(&p.secret, &domain, cid), proof) && p.addr.is_none_or(|a| a.ip == from.ip) {
                    return Some(domain);
                }
                return None;
            }
            if self.cfg.public_peering {
                return Some(domain);
            }
        } else if self.cfg.public_peering {
            return Some(String::from("public"));
        }
        None
    }

    // ------------------------------------------------------------ texts and notices (proxy)

    fn route_request(&mut self, from: Addr, m: Message, now: u64) {
        let Some(uri) = m.uri().cloned() else { return };
        let user = uri.user().unwrap_or("").to_string();
        let from_phone = self.is_phone(from);
        let from_core = from == self.cfg.smsc || Some(from) == self.cfg.mmsc || from == self.cfg.mrf || (from.ip == self.cfg.ims_ip && !from_phone);
        if from_phone {
            // the P-CSCF: only registered phones, only as themselves
            let Some(reg) = self.reg_at(from).cloned() else {
                return self.respond(&m, from, 403, None, now);
            };
            let mut m = m;
            m.headers_mut().set("P-Asserted-Identity", format!("<{}>", number_uri(&reg.msisdn, &self.cfg.domain)));
            match user.as_str() {
                "smsc" => return self.proxy(m, from, self.cfg.smsc, now),
                "mrf" | "voicemail" => return self.proxy(m, from, self.cfg.mrf, now),
                _ => {}
            }
            if m.method() == Some(&Method::Subscribe) {
                // message-summary (voicemail): the media server keeps the boxes
                return self.proxy(m, from, self.cfg.mrf, now);
            }
            return match uri_number(&uri) {
                // a text straight to a number goes through the SMSC (store and forward)
                Some(_) => self.proxy(m, from, self.cfg.smsc, now),
                None => self.respond(&m, from, 404, None, now),
            };
        }
        if from_core {
            match uri_number(&uri) {
                Some(n) => {
                    let id_line = format!("LIR {}", n);
                    self.hss(Pending::MsgLir { req: m, from, number: n, from_peer: false }, id_line);
                }
                None => self.respond(&m, from, 404, None, now),
            }
            return;
        }
        // from another carrier
        let Some(_peer) = self.peer_ok(&m, from) else {
            self.log(format!("{} from unlisted peer {}: 403", m.method().map(|x| x.as_str().to_string()).unwrap_or_default(), from));
            return self.respond(&m, from, 403, None, now);
        };
        match uri_number(&uri) {
            Some(n) => self.hss(Pending::MsgLir { req: m, from, number: n.clone(), from_peer: true }, format!("LIR {}", n)),
            None => self.respond(&m, from, 404, None, now),
        }
    }

    fn msg_located(&mut self, req: Message, from: Addr, number: String, from_peer: bool, reply: Result<String, String>, now: u64) {
        let b = reply.unwrap_or_default();
        match kv(&b, "result").as_str() {
            "local" => {
                if from_peer {
                    // texts from other carriers go through our SMSC (it stores and delivers)
                    let mut m = req;
                    m.headers_mut().set("P-Called-Party-ID", format!("<{}>", number_uri(&number, &self.cfg.domain)));
                    if m.method() == Some(&Method::Message) {
                        return self.proxy(m, from, self.cfg.smsc, now);
                    }
                    return self.deliver_to_phone(m, from, &number, now);
                }
                self.deliver_to_phone(req, from, &number, now)
            }
            "unassigned" | "foreign" if !from_peer => {
                let id = self.id();
                self.pending.insert(id, Pending::MsgEnum { req, from, number: number.clone() });
                self.out.push(ImsOut::Enum { id, number });
            }
            _ => self.respond(&req, from, 404, None, now),
        }
    }

    fn deliver_to_phone(&mut self, m: Message, from: Addr, number: &str, now: u64) {
        match self.regs.get(number).cloned() {
            Some(r) => {
                let mut m = m;
                // the phone sees its own AOR in the Request-URI
                if let Message::Request { uri, .. } = &mut m {
                    *uri = Uri::sip(Some(number), &r.contact.host(), Some(r.contact.port));
                }
                self.proxy(m, from, r.contact, now)
            }
            None => self.respond(&m, from, 480, None, now),
        }
    }

    /// Forward a request statelessly: our Via on top, the response comes back the same way.
    fn proxy(&mut self, mut m: Message, from: Addr, to: Addr, now: u64) {
        stamp_via(&mut m, from);
        let branch = new_branch(self.rng.next());
        let host = self.host();
        let mf = m.headers().max_forwards().ok().flatten().unwrap_or(70);
        if mf == 0 {
            return self.respond(&m, from, 483, None, now);
        }
        let original = m.clone();
        let h = m.headers_mut();
        h.prepend("Via", Via::new("UDP", &host, Some(self.cfg.sip_port), &branch).to_string());
        h.set("Max-Forwards", (mf - 1).to_string());
        self.proxied.insert(branch, (original, from));
        let id = self.id();
        self.tx.request(id, to, &m, now);
    }

    #[allow(clippy::too_many_arguments)]
    fn proxy_to_peer(&mut self, mut m: Message, from: Addr, number: &str, domain: &str, to: Addr, secret: Option<String>, now: u64) {
        if let Message::Request { uri, .. } = &mut m {
            *uri = number_uri(number, domain);
        }
        let cid = m.headers().call_id().unwrap_or("").to_string();
        if let Some(s) = secret {
            let proof = peer_proof(&s, &self.cfg.domain, &cid);
            m.headers_mut().set("P-ECM-Peer", format!("{};proof={}", self.cfg.domain, proof));
        } else {
            m.headers_mut().set("P-ECM-Peer", self.cfg.domain.clone());
        }
        self.proxy(m, from, to, now)
    }

    fn on_response(&mut self, from: Addr, m: Message, now: u64) {
        self.tx.on_response(&m);
        let Some(branch) = crate::txn::branch_of(&m) else { return };
        if let Some((orig, back)) = self.proxied.get(&branch).cloned() {
            if m.code().unwrap_or(0) >= 200 {
                self.proxied.remove(&branch);
            }
            // strip our Via and send it to the next one
            let mut r = m;
            let h = r.headers_mut();
            let vias: Vec<String> = h.get_all("Via").iter().map(|s| s.to_string()).collect();
            h.remove("Via");
            let mut first = true;
            for v in vias {
                let vals: Vec<String> = sip::split_list(&v).into_iter().map(String::from).collect();
                for val in vals {
                    if first {
                        first = false;
                        continue;
                    }
                    h.push("Via", val);
                }
            }
            let to = response_target(&orig, back);
            self.out.push(ImsOut::Sip { to, data: r.to_bytes() });
            let _ = from;
            return;
        }
        // a response on a call leg
        let Ok(cid) = m.headers().call_id() else { return };
        if let Some(&call) = self.by_call_id.get(cid) {
            self.leg_response(call, m, now);
        }
    }

    // ------------------------------------------------------------ calls

    fn alloc_relay(&mut self, call: u64, a_side: bool) -> Option<u16> {
        self.alloc_port(RelayRole::Leg { call, a_side })
    }

    fn alloc_port(&mut self, role: RelayRole) -> Option<u16> {
        let (lo, hi) = self.cfg.relay_ports;
        for _ in lo..hi {
            let p = self.next_relay;
            self.next_relay = if p + 2 >= hi { lo } else { p + 2 };
            if !self.relays.contains_key(&p) {
                self.relays.insert(p, role);
                self.out.push(ImsOut::OpenRelay(p));
                return Some(p);
            }
        }
        None
    }

    fn free_relay(&mut self, p: u16) {
        if self.relays.remove(&p).is_some() {
            self.out.push(ImsOut::CloseRelay(p));
        }
    }

    fn media_of(sdp: &SessionDescription) -> Option<Addr> {
        let m = sdp.media.first()?;
        let host = m.connection.clone().or_else(|| sdp.connection.clone())?;
        Some(Addr::new(Addr::parse_ip(&host)?, m.port))
    }

    fn invite(&mut self, from: Addr, m: Message, now: u64) {
        self.respond(&m, from, 100, None, now);
        let from_phone = self.is_phone(from);
        let (kind, caller, hide) = if from_phone {
            let Some(reg) = self.reg_at(from).cloned() else {
                return self.respond(&m, from, 403, None, now);
            };
            let privacy = m.headers().privacy().iter().any(|p| p.eq_ignore_ascii_case("id") || p.eq_ignore_ascii_case("header"));
            (LegKind::Phone, reg.msisdn, privacy)
        } else if from == self.cfg.mrf {
            (LegKind::Mrf, String::from("mrf"), false)
        } else {
            match self.peer_ok(&m, from) {
                Some(_) => {
                    let pai = m.headers().p_asserted_identity().ok().and_then(|v| v.into_iter().next());
                    let who = pai.as_ref().and_then(|p| uri_number(&p.uri)).unwrap_or_else(|| "anonymous".into());
                    let hide = m.headers().privacy().iter().any(|p| p.eq_ignore_ascii_case("id"));
                    (LegKind::Peer, who, hide)
                }
                None => {
                    self.log(format!("INVITE from unlisted peer {}: 403", from));
                    return self.respond(&m, from, 403, None, now);
                }
            }
        };
        let Some(uri) = m.uri().cloned() else { return };
        let Some(target) = uri_number(&uri) else {
            return self.respond(&m, from, 404, None, now);
        };
        let Some(sdp) = SessionDescription::parse(m.body()).ok() else {
            return self.respond(&m, from, 488, None, now);
        };
        let our_tag = new_tag(self.rng.next());
        let Some(dialog) = Dialog::incoming(&m, &our_tag, from) else {
            return self.respond(&m, from, 400, None, now);
        };
        let id = self.id();
        let Some(relay) = self.alloc_relay(id, true) else {
            return self.respond(&m, from, 503, None, now);
        };
        let reply_to = response_target(&m, from);
        let a = Leg {
            kind,
            dialog,
            relay,
            media: Self::media_of(&sdp),
            ims_ip: if kind == LegKind::Phone { Some(from.ip) } else { None },
            number: caller.clone(),
            bearer: None,
            invite: m.clone(),
            reply_to,
            sdp: Some(sdp),
            answered: false,
            mix: None,
            muted: false,
        };
        let cid = a.dialog.call_id.clone();
        let call = Call {
            id,
            st: CallSt::Routing,
            a,
            b: None,
            caller: caller.clone(),
            hide_id: hide,
            target: target.clone(),
            dialled: target.clone(),
            forwards: 0,
            to_service: false,
            answer_deadline: None,
            started_ms: now,
            answered_ms: None,
            outcome: None,
            callee: None,
            conf: None,
        };
        self.by_call_id.insert(cid, id);
        self.calls.insert(id, call);
        if let Some(r) = self.regs.get_mut(&caller) {
            r.calls += 1;
        }
        self.log(format!("call {}: {} -> {}{}", id, caller, target, if hide { " (number hidden)" } else { "" }));
        // originating services: codes a phone dials
        match target.as_str() {
            "*86" => return self.to_service(id, "voicemail-retrieve", &caller, now),
            "611" => return self.to_service(id, "support", &caller, now),
            "*73" => {
                self.hss(Pending::Feature, format!("PUR {} cfu off", caller));
                return self.end_with(id, 200, Outcome::Answered, now);
            }
            _ => {}
        }
        if let Some(fwd) = target.strip_prefix("*72") {
            if fwd.starts_with('+') {
                self.hss(Pending::Feature, format!("PUR {} cfu {}", caller, fwd));
                return self.end_with(id, 200, Outcome::Answered, now);
            }
        }
        if !target.starts_with('+') {
            return self.end_with(id, 404, Outcome::NotInService, now);
        }
        self.locate(id, now);
    }

    fn locate(&mut self, call: u64, _now: u64) {
        let Some(c) = self.calls.get(&call) else { return };
        let t = c.target.clone();
        self.hss(Pending::CallLir { call }, format!("LIR {}", t));
    }

    fn call_located(&mut self, call: u64, reply: Result<String, String>, now: u64) {
        let Some(c) = self.calls.get(&call) else { return };
        if c.st == CallSt::Ended {
            return;
        }
        let from_peer = c.a.kind == LegKind::Peer;
        let b = reply.unwrap_or_default();
        match kv(&b, "result").as_str() {
            "local" => {
                let imsi = kv(&b, "imsi");
                self.hss(Pending::CallUdr { call, imsi: imsi.clone() }, format!("UDR {}", imsi));
            }
            "unassigned" | "foreign" if !from_peer => {
                // maybe ported, maybe another carrier's: ask ENUM
                let n = c.target.clone();
                let id = self.id();
                self.pending.insert(id, Pending::CallEnum { call });
                self.out.push(ImsOut::Enum { id, number: n });
            }
            _ => self.fail(call, 404, Outcome::NotInService, now),
        }
    }

    fn call_enum(&mut self, call: u64, answer: Option<EnumAnswer>, now: u64) {
        let peer = answer.and_then(|a| self.border_for(&a).map(|b| (a, b)));
        match peer {
            Some((a, (border, secret))) if a.domain != self.cfg.domain => self.call_peer(call, &a.domain, border, secret, now),
            _ => self.fail(call, 404, Outcome::NotInService, now),
        }
    }

    /// Terminating services for one of our numbers, then ring it.
    fn call_terminate(&mut self, call: u64, p: Profile, now: u64) {
        let Some(c) = self.calls.get_mut(&call) else { return };
        if c.st == CallSt::Ended {
            return;
        }
        let caller = c.caller.clone();
        c.callee = Some(p.clone());
        if p.blocked.iter().any(|b| *b == caller) {
            self.log(format!("call {}: {} blocked by {}", call, caller, p.msisdn));
            return self.end_with(call, 603, Outcome::Rejected, now);
        }
        if !p.cfu.is_empty() {
            return self.forward(call, &p.cfu.clone(), "unconditional", now);
        }
        if p.dnd {
            return self.no_reach(call, &p, Outcome::Busy, now);
        }
        let reg = self.regs.get(&p.msisdn).cloned();
        let Some(reg) = reg else {
            if !p.cfnrc.is_empty() {
                return self.forward(call, &p.cfnrc.clone(), "unreachable", now);
            }
            return self.no_reach(call, &p, Outcome::Unreachable, now);
        };
        // busy: in a call already (waiting allows one more)
        if reg.calls >= 2 || (reg.calls >= 1 && !p.cw) {
            if !p.cfb.is_empty() {
                return self.forward(call, &p.cfb.clone(), "busy", now);
            }
            return self.no_reach(call, &p, Outcome::Busy, now);
        }
        self.call_phone(call, &reg, now);
    }

    /// The callee can't take it: voicemail when it is on, else a busy / unreachable answer.
    fn no_reach(&mut self, call: u64, p: &Profile, why: Outcome, now: u64) {
        if p.vm {
            return self.to_service(call, "voicemail", &p.msisdn.clone(), now);
        }
        match why {
            Outcome::Busy => self.fail(call, 486, why, now),
            _ => self.fail(call, 480, why, now),
        }
    }

    fn forward(&mut self, call: u64, to: &str, why: &str, now: u64) {
        let Some(c) = self.calls.get_mut(&call) else { return };
        c.forwards += 1;
        if c.forwards > 5 {
            return self.fail(call, 482, Outcome::Failed, now);
        }
        let from = std::mem::replace(&mut c.target, to.to_string());
        c.st = CallSt::Routing;
        self.log(format!("call {}: {} forwards to {} ({})", call, from, to, why));
        if let Some(b) = self.calls.get_mut(&call).and_then(|c| c.b.take()) {
            self.drop_leg(call, b, now);
        }
        self.locate(call, now);
    }

    /// The INVITE a B leg starts with (caller ID per the TAS, our relay in the SDP).
    fn b_invite(&mut self, call: u64, kind: LegKind, request_uri: Uri, next_hop: Addr, relay: u16) -> Option<(Message, Dialog)> {
        let c = self.calls.get(&call)?;
        let sdp = c.a.sdp.clone()?;
        let hide = c.hide_id || c.callee.as_ref().is_some_and(|_| false);
        let caller = c.caller.clone();
        let dialled = c.dialled.clone();
        let target = c.target.clone();
        let caller_clir = false;
        let local_uri = if hide || caller_clir { Uri::sip(Some("anonymous"), "anonymous.invalid", None) } else { number_uri(&caller, &self.cfg.domain) };
        let display = if hide { Some("Anonymous") } else { None };
        let host = match kind {
            LegKind::Peer => Addr::new(self.cfg.public_ip, 0).host(),
            _ => self.host(),
        };
        let mut d = Dialog::outgoing(&mut self.rng, local_uri, display, request_uri, next_hop, &host);
        let mut m = d.request(&mut self.rng, Method::Invite, &host, self.cfg.sip_port);
        let sdp_out = relay_sdp(&self.cfg, &sdp, kind, relay);
        let h = m.headers_mut();
        h.push("Contact", format!("<sip:b2bua@{}:{}>", host, self.cfg.sip_port));
        if caller.starts_with('+') {
            h.push("P-Asserted-Identity", format!("<{}>", number_uri(&caller, &self.cfg.domain)));
        }
        if hide {
            h.push("Privacy", "id");
        }
        if dialled != target {
            h.push("History-Info", format!("<{}>;index=1", number_uri(&dialled, &self.cfg.domain)));
            h.push("Diversion", format!("<tel:{}>;reason=unconditional", dialled));
        }
        h.push("Allow", "INVITE, ACK, BYE, CANCEL, OPTIONS, MESSAGE, NOTIFY, REFER, UPDATE");
        m.set_body("application/sdp", sdp_out.to_bytes());
        Some((m, d))
    }

    /// Start the B leg. `sign`: a peer leg, signed with the peering secret (or unsigned public peering).
    #[allow(clippy::too_many_arguments)]
    fn start_b(&mut self, call: u64, kind: LegKind, uri: Uri, next_hop: Addr, number: &str, ims_ip: Option<[u8; 4]>, extra: &[(&str, String)], sign: Option<Option<String>>, now: u64) {
        let Some(relay) = self.alloc_relay(call, false) else {
            return self.fail(call, 503, Outcome::Failed, now);
        };
        let Some((mut inv, dialog)) = self.b_invite(call, kind, uri, next_hop, relay) else {
            self.free_relay(relay);
            return self.fail(call, 500, Outcome::Failed, now);
        };
        for (k, v) in extra {
            inv.headers_mut().set(k, v.clone());
        }
        let cid = dialog.call_id.clone();
        if let Some(secret) = sign {
            let me = self.cfg.domain.clone();
            let v = match secret {
                Some(s) => format!("{};proof={}", me, peer_proof(&s, &me, &dialog.call_id)),
                None => me,
            };
            inv.headers_mut().set("P-ECM-Peer", v);
        }
        let leg = Leg { kind, dialog, relay, media: None, ims_ip, number: number.into(), bearer: None, invite: inv.clone(), reply_to: next_hop, sdp: None, answered: false, mix: None, muted: false };
        self.by_call_id.insert(cid, call);
        let no_answer = self.cfg.no_answer_ms;
        let c = self.calls.get_mut(&call).expect("live call");
        c.b = Some(leg);
        c.st = CallSt::Calling;
        c.answer_deadline = if kind == LegKind::Phone { Some(now + no_answer) } else { None };
        self.tx.request(call, next_hop, &inv, now);
    }

    fn call_phone(&mut self, call: u64, reg: &Registration, now: u64) {
        let uri = Uri::sip(Some(&reg.msisdn), &reg.contact.host(), Some(reg.contact.port));
        if let Some(r) = self.regs.get_mut(&reg.msisdn) {
            r.calls += 1;
        }
        self.start_b(call, LegKind::Phone, uri, reg.contact, &reg.msisdn.clone(), Some(reg.contact.ip), &[], None, now);
    }

    fn call_peer(&mut self, call: u64, domain: &str, border: Addr, secret: Option<String>, now: u64) {
        let target = self.calls.get(&call).map(|c| c.target.clone()).unwrap_or_default();
        let uri = number_uri(&target, domain);
        self.start_b(call, LegKind::Peer, uri, border, &target, None, &[], Some(secret), now);
        self.log(format!("call {}: {} via the border to {} ({})", call, target, domain, border));
    }

    /// Send the call to a service on mrfd (`voicemail`, `voicemail-retrieve`, `support`, `announcement`).
    fn to_service(&mut self, call: u64, service: &str, mailbox: &str, now: u64) {
        let mrf = self.cfg.mrf;
        if let Some(b) = self.calls.get_mut(&call).and_then(|c| c.b.take()) {
            self.drop_leg(call, b, now);
        }
        if let Some(c) = self.calls.get_mut(&call) {
            c.to_service = true;
        }
        let uri = Uri::sip(Some(service), &mrf.host(), Some(mrf.port));
        let caller = self.calls.get(&call).map(|c| c.caller.clone()).unwrap_or_default();
        let extra = vec![("P-Called-Party-ID", format!("<tel:{}>", mailbox)), ("P-Asserted-Identity", format!("<tel:{}>", caller))];
        self.log(format!("call {}: to {} for {}", call, service, mailbox));
        self.start_b(call, LegKind::Mrf, uri, mrf, mailbox, None, &extra, None, now);
    }

    /// End a call that never reached anyone with `code` (the caller's phone plays
    /// busy for 486, the announcement otherwise).
    fn fail(&mut self, call: u64, code: u16, why: Outcome, now: u64) {
        let Some(c) = self.calls.get(&call) else { return };
        let to_service = c.to_service;
        if matches!(code, 404 | 480 | 604) && !to_service && c.a.kind != LegKind::Mrf {
            // early media: the media server's intercept tones, then the final answer
            let code_s = code.to_string();
            if let Some(c) = self.calls.get_mut(&call) {
                c.outcome = Some(why);
            }
            self.to_service(call, "announcement", &code_s, now);
            if let Some(c) = self.calls.get_mut(&call) {
                c.outcome = Some(why);
            }
            return;
        }
        self.end_with(call, code, why, now);
    }

    fn end_with(&mut self, call: u64, code: u16, why: Outcome, now: u64) {
        let Some(c) = self.calls.get_mut(&call) else { return };
        c.outcome.get_or_insert(why);
        if !c.a.answered {
            let inv = c.a.invite.clone();
            let to = c.a.reply_to;
            let tag = c.a.dialog.local.tag().unwrap_or("x").to_string();
            if let Ok(r) = inv.response_to(code, Some(&tag)) {
                self.tx.respond(to, &r, now);
            }
        }
        self.finish(call, now);
    }

    fn drop_leg(&mut self, call: u64, mut b: Leg, now: u64) {
        let host = self.host();
        if b.answered {
            let bye = b.dialog.request(&mut self.rng, Method::Bye, &host, self.cfg.sip_port);
            self.tx.request(call, b.dialog.next_hop, &bye, now);
        } else if let Some(cancel) = cancel_for(&b.invite) {
            self.tx.request(call, b.dialog.next_hop, &cancel, now);
        }
        if b.kind == LegKind::Phone {
            if let Some(r) = self.regs.get_mut(&b.number) {
                r.calls = r.calls.saturating_sub(1);
            }
        }
        self.release_bearer(&b);
        self.free_relay(b.relay);
        self.by_call_id.remove(&b.dialog.call_id);
    }

    fn release_bearer(&mut self, l: &Leg) {
        if let (Some(ip), Some(id)) = (l.ims_ip, l.bearer) {
            self.hss(Pending::Bearer, format!("STR {} {}", Addr::new(ip, 0).host(), id));
        }
    }

    fn setup_bearer(&mut self, call: u64, a_side: bool) {
        let Some(c) = self.calls.get(&call) else { return };
        let l = if a_side { Some(&c.a) } else { c.b.as_ref() };
        let Some(l) = l else { return };
        let (Some(ip), Some(media)) = (l.ims_ip, l.media) else { return };
        if l.bearer.is_some() {
            return;
        }
        let line = format!("AAR {} {} {} {}", Addr::new(ip, 0).host(), media.port, Addr::new(self.cfg.ims_ip, 0).host(), l.relay);
        self.hss(Pending::Aar { call, a_side }, line);
    }

    /// The call is over: release both legs, write the CDR.
    fn finish(&mut self, call: u64, now: u64) {
        if let Some(cf) = self.calls.get(&call).and_then(|c| c.conf) {
            self.unmix(call, true, now);
            self.unmix(call, false, now);
            self.leave_conference(cf, call, now);
        }
        let Some(mut c) = self.calls.remove(&call) else { return };
        c.st = CallSt::Ended;
        if let Some(b) = c.b.take() {
            self.drop_leg(call, b, now);
        }
        if c.a.kind == LegKind::Phone {
            if let Some(r) = self.regs.get_mut(&c.a.number) {
                r.calls = r.calls.saturating_sub(1);
            }
        }
        self.release_bearer(&c.a);
        self.free_relay(c.a.relay);
        self.by_call_id.remove(&c.a.dialog.call_id);
        let dur = c.answered_ms.map(|a| (now.saturating_sub(a)) / 1000).unwrap_or(0);
        let outcome = match c.outcome.unwrap_or(Outcome::Failed) {
            Outcome::Answered => "answered",
            Outcome::Busy => "busy",
            Outcome::Declined => "declined",
            Outcome::NoAnswer => "no-answer",
            Outcome::Unreachable => "unreachable",
            Outcome::NotInService => "not-in-service",
            Outcome::Cancelled => "cancelled",
            Outcome::Rejected => "rejected",
            Outcome::Failed => "failed",
        };
        let via = if c.to_service { "service" } else { "direct" };
        self.out.push(ImsOut::Cdr(format!(
            "call start={} answer={} end={} dur={} from={} to={} dialled={} result={} route={} hidden={}",
            c.started_ms / 1000,
            c.answered_ms.map(|a| (a / 1000).to_string()).unwrap_or_else(|| "-".into()),
            now / 1000,
            dur,
            c.caller,
            c.target,
            c.dialled,
            outcome,
            via,
            c.hide_id as u8
        )));
        self.log(format!("call {} ended: {}", call, outcome));
    }

    /// A response on a B leg (or on an A-leg request we sent, like BYE).
    fn leg_response(&mut self, call: u64, m: Message, now: u64) {
        let Some(code) = m.code() else { return };
        let cseq = m.headers().cseq().ok();
        let method = cseq.as_ref().map(|c| c.method.clone()).unwrap_or(Method::Other(String::new()));
        let cid = m.headers().call_id().unwrap_or("").to_string();
        if let Some(&(mc, side)) = self.mixers.get(&cid) {
            if method == Method::Invite {
                return self.mixer_response(mc, side, m, now);
            }
            return;
        }
        let Some(c) = self.calls.get(&call) else { return };
        let on_b = c.b.as_ref().is_some_and(|b| b.dialog.call_id == cid);
        if !on_b || method != Method::Invite {
            return; // BYE/CANCEL answers, re-INVITE answers on A, ...
        }
        let host = self.host();
        if (100..200).contains(&code) {
            if code == 100 {
                return;
            }
            // ringing / early media: tell the caller
            let c = self.calls.get_mut(&call).expect("live");
            let b = c.b.as_mut().expect("checked");
            b.dialog.learn(&m);
            let mut r = match c.a.invite.response_to(code, c.a.dialog.local.tag()) {
                Ok(r) => r,
                Err(_) => return,
            };
            if !m.body().is_empty() {
                if let Ok(sdp) = SessionDescription::parse(m.body()) {
                    b.media = Self::media_of(&sdp);
                    b.sdp = Some(sdp.clone());
                    let s = relay_sdp(&self.cfg, &sdp, c.a.kind, c.a.relay);
                    r.set_body("application/sdp", s.to_bytes());
                }
            }
            let to = c.a.reply_to;
            let inv = c.a.invite.clone();
            self.tx.respond(to, &r, now);
            let _ = inv;
            return;
        }
        if (200..300).contains(&code) {
            let c = self.calls.get_mut(&call).expect("live");
            let b = c.b.as_mut().expect("checked");
            b.dialog.learn(&m);
            let first = !b.answered;
            b.answered = true;
            if let Ok(sdp) = SessionDescription::parse(m.body()) {
                b.media = Self::media_of(&sdp);
                b.sdp = Some(sdp);
            }
            let ack = ack_for(&b.dialog, &mut self.rng, &host, self.cfg.sip_port);
            let hop = b.dialog.next_hop;
            self.tx.request(call, hop, &ack, now);
            if !first {
                return;
            }
            let c = self.calls.get_mut(&call).expect("live");
            let service = c.to_service;
            let announcement = c.outcome.is_some() && service && c.b.as_ref().is_some_and(|b| b.kind == LegKind::Mrf);
            if !c.a.answered {
                if announcement {
                    // early media from the announcement, final answer after it (mrfd hangs up)
                    let bsdp = c.b.as_ref().and_then(|b| b.sdp.clone());
                    if let (Some(s), Ok(mut r)) = (bsdp, c.a.invite.response_to(183, c.a.dialog.local.tag())) {
                        let s = relay_sdp(&self.cfg, &s, c.a.kind, c.a.relay);
                        r.set_body("application/sdp", s.to_bytes());
                        let to = c.a.reply_to;
                        self.tx.respond(to, &r, now);
                    }
                    c.st = CallSt::Active;
                    return;
                }
                c.a.answered = true;
                c.st = CallSt::Active;
                c.answered_ms = Some(now);
                c.answer_deadline = None;
                c.outcome = Some(Outcome::Answered);
                let bsdp = c.b.as_ref().and_then(|b| b.sdp.clone());
                let mut r = match c.a.invite.response_to(200, c.a.dialog.local.tag()) {
                    Ok(r) => r,
                    Err(_) => return,
                };
                r.headers_mut().push("Contact", format!("<sip:b2bua@{}:{}>", host, self.cfg.sip_port));
                if let Some(s) = bsdp {
                    let s = relay_sdp(&self.cfg, &s, c.a.kind, c.a.relay);
                    r.set_body("application/sdp", s.to_bytes());
                }
                let to = c.a.reply_to;
                self.tx.respond(to, &r, now);
                let (caller, target) = (c.caller.clone(), c.target.clone());
                self.log(format!("call {}: {} answered {}", call, target, caller));
                self.setup_bearer(call, true);
                self.setup_bearer(call, false);
            }
            return;
        }
        // a final failure on B
        let c = self.calls.get_mut(&call).expect("live");
        let b = c.b.take().expect("checked");
        if let Some(ack) = ack_non2xx(&b.invite, &m) {
            self.tx.request(call, b.dialog.next_hop, &ack, now);
        }
        if b.kind == LegKind::Phone {
            if let Some(r) = self.regs.get_mut(&b.number) {
                r.calls = r.calls.saturating_sub(1);
            }
        }
        self.free_relay(b.relay);
        self.by_call_id.remove(&b.dialog.call_id);
        let c = self.calls.get(&call).expect("live");
        if c.to_service {
            // the service itself failed (or an announcement ended without answering)
            let oc = c.outcome.unwrap_or(Outcome::Failed);
            let code2 = match oc {
                Outcome::NotInService => 404,
                Outcome::Unreachable => 480,
                Outcome::Busy => 486,
                _ => code,
            };
            return self.end_with(call, code2, oc, now);
        }
        let p = c.callee.clone().unwrap_or_default();
        match code {
            486 | 600 => {
                if !p.cfb.is_empty() {
                    return self.forward(call, &p.cfb, "busy", now);
                }
                self.no_reach(call, &p, Outcome::Busy, now)
            }
            603 => self.end_with(call, 486, Outcome::Declined, now),
            480 | 408 | 503 => {
                if b.kind == LegKind::Phone && !p.cfnrc.is_empty() {
                    return self.forward(call, &p.cfnrc, "unreachable", now);
                }
                if b.kind == LegKind::Phone {
                    return self.no_reach(call, &p, Outcome::Unreachable, now);
                }
                self.end_with(call, code, Outcome::Unreachable, now)
            }
            487 => self.end_with(call, 487, Outcome::Cancelled, now),
            404 | 604 => self.end_with(call, 404, Outcome::NotInService, now),
            403 => self.end_with(call, 403, Outcome::Rejected, now),
            _ => self.end_with(call, code, Outcome::Failed, now),
        }
    }

    /// A request inside a call's dialog (either leg).
    fn in_call(&mut self, call: u64, from: Addr, m: Message, now: u64) {
        let method = m.method().cloned().unwrap_or(Method::Other(String::new()));
        let cid = m.headers().call_id().unwrap_or("").to_string();
        if let Some(&(mc, side)) = self.mixers.get(&cid) {
            // mrfd ended a conference leg
            self.respond(&m, from, 200, None, now);
            if method == Method::Bye {
                self.unmix(mc, side, now);
            }
            return;
        }
        let Some(c) = self.calls.get(&call) else { return };
        let on_a = c.a.dialog.call_id == cid;
        match method {
            Method::Ack => {}
            Method::Cancel => {
                if !on_a {
                    return self.respond(&m, from, 481, None, now);
                }
                self.respond(&m, from, 200, None, now);
                let c = self.calls.get_mut(&call).expect("live");
                c.outcome = Some(Outcome::Cancelled);
                if !c.a.answered {
                    return self.end_with(call, 487, Outcome::Cancelled, now);
                }
            }
            Method::Bye => {
                self.respond(&m, from, 200, None, now);
                let c = self.calls.get_mut(&call).expect("live");
                if on_a {
                    c.a.answered = true; // nothing more to answer on A
                    self.finish(call, now);
                } else {
                    // B hung up: tell A, forget B
                    let b = c.b.take();
                    let host = self.host();
                    let c = self.calls.get_mut(&call).expect("live");
                    if c.a.answered || c.st == CallSt::Active {
                        let bye = c.a.dialog.request(&mut self.rng, Method::Bye, &host, self.cfg.sip_port);
                        let hop = c.a.dialog.next_hop;
                        c.a.answered = true;
                        self.tx.request(call, hop, &bye, now);
                    }
                    if let Some(mut b) = b {
                        b.answered = false;
                        if b.kind == LegKind::Phone {
                            if let Some(r) = self.regs.get_mut(&b.number) {
                                r.calls = r.calls.saturating_sub(1);
                            }
                        }
                        self.release_bearer(&b);
                        self.free_relay(b.relay);
                        self.by_call_id.remove(&b.dialog.call_id);
                    }
                    // an announcement ending: now the final answer
                    let c = self.calls.get(&call).expect("live");
                    if !c.a.answered {
                        let code = match c.outcome {
                            Some(Outcome::NotInService) => 404,
                            Some(Outcome::Unreachable) => 480,
                            _ => 480,
                        };
                        let oc = c.outcome.unwrap_or(Outcome::Failed);
                        return self.end_with(call, code, oc, now);
                    }
                    self.finish(call, now);
                }
            }
            Method::Invite | Method::Update => {
                // re-INVITE (hold / resume): relay it to the other leg's dialog with our SDP
                let sdp = SessionDescription::parse(m.body()).ok();
                let host = self.host();
                let c = self.calls.get_mut(&call).expect("live");
                let (this, other) = if on_a { (&mut c.a, c.b.as_mut()) } else { (c.b.as_mut().expect("on b"), Some(&mut c.a)) };
                if let Some(s) = &sdp {
                    this.media = Self::media_of(s).or(this.media);
                    this.sdp = Some(s.clone());
                }
                let this_kind = this.kind;
                let this_relay = this.relay;
                let reply_sdp = other.as_ref().and_then(|o| o.sdp.clone());
                if let Some(o) = other {
                    if let Some(s) = &sdp {
                        let kind = o.kind;
                        let relay = o.relay;
                        let mut re = o.dialog.request(&mut self.rng, method.clone(), &host, self.cfg.sip_port);
                        let out = {
                            let ip = if kind == LegKind::Peer { self.cfg.public_ip } else { self.cfg.ims_ip };
                            let mut x = s.clone();
                            x.origin.addr = Addr::new(ip, 0).host();
                            x.connection = Some(x.origin.addr.clone());
                            for md in x.media.iter_mut() {
                                md.port = relay;
                                md.connection = None;
                            }
                            x
                        };
                        re.headers_mut().push("Contact", format!("<sip:b2bua@{}:{}>", host, self.cfg.sip_port));
                        re.set_body("application/sdp", out.to_bytes());
                        let hop = o.dialog.next_hop;
                        self.tx.request(call, hop, &re, now);
                    }
                }
                // answer this side at once with the other side's media, direction mirrored
                let mut r = match m.response_to(200, None) {
                    Ok(r) => r,
                    Err(_) => return,
                };
                if let (Some(other_sdp), Some(s)) = (reply_sdp, sdp) {
                    let dir = s.media.first().map(|x| x.direction).unwrap_or_default();
                    let ans = relay_sdp(&self.cfg, &other_sdp, this_kind, this_relay).with_direction(dir.reverse());
                    r.set_body("application/sdp", ans.to_bytes());
                }
                r.headers_mut().push("Contact", format!("<sip:b2bua@{}:{}>", self.host(), self.cfg.sip_port));
                self.respond_with(&m, from, r, now);
            }
            Method::Refer => self.refer(call, on_a, from, m, now),
            Method::Info | Method::Notify | Method::Message | Method::Options => self.respond(&m, from, 200, None, now),
            _ => self.respond(&m, from, 405, None, now),
        }
    }

    /// Three-way calling: the phone in two calls sends `REFER` in one of them with
    /// `Refer-To: <sip:conference@domain;merge=<other Call-ID>>`. Both calls move
    /// into a conference on mrfd, which mixes the three voices: the far party of
    /// each call and the merging phone (on the first call's stream; its second
    /// stream goes quiet).
    fn refer(&mut self, call: u64, on_a: bool, from: Addr, m: Message, now: u64) {
        let rt = m.headers().get("Refer-To").and_then(|v| NameAddr::parse(v).ok());
        let other_cid = rt.as_ref().and_then(|n| n.uri.param("merge").map(String::from));
        let Some(other_cid) = other_cid else {
            return self.respond(&m, from, 501, None, now);
        };
        let Some(&other) = self.by_call_id.get(&other_cid) else {
            return self.respond(&m, from, 481, None, now);
        };
        if other == call || self.calls.get(&call).is_none_or(|c| c.st != CallSt::Active) || self.calls.get(&other).is_none_or(|c| c.st != CallSt::Active) {
            return self.respond(&m, from, 403, None, now);
        }
        self.respond(&m, from, 202, None, now);
        let conf = self.confs.keys().last().copied().unwrap_or(0) + 1;
        self.confs.insert(conf, vec![call, other]);
        // the merging phone: the leg that sent the REFER in `call`, the same phone in `other`
        let merger_ip = from.ip;
        let mut joins: Vec<(u64, bool)> = Vec::new();
        for id in [call, other] {
            let Some(c) = self.calls.get_mut(&id) else { continue };
            c.conf = Some(conf);
            let a_is_merger = if id == call { on_a } else { c.a.ims_ip == Some(merger_ip) };
            // the far party of each call joins; the merger joins once, on `call`
            joins.push((id, !a_is_merger));
            if id == call {
                joins.push((id, a_is_merger));
            } else if let Some(l) = if a_is_merger { Some(&mut c.a) } else { c.b.as_mut() } {
                l.muted = true;
            }
        }
        self.log(format!("calls {} and {} merged into conference {}", call, other, conf));
        let mrf = self.cfg.mrf;
        let host = self.host();
        for (id, a_side) in joins {
            let Some(port) = self.alloc_port(RelayRole::Mix { call: id, a_side }) else { continue };
            let Some(c) = self.calls.get(&id) else { continue };
            let leg = if a_side { Some(&c.a) } else { c.b.as_ref() };
            let Some(sdp) = leg.and_then(|l| l.sdp.clone()) else { continue };
            let mut d = Dialog::outgoing(&mut self.rng, Uri::sip(Some("b2bua"), &self.cfg.domain, None), None, Uri::sip(Some("conference"), &mrf.host(), Some(mrf.port)), mrf, &host);
            let mut inv = d.request(&mut self.rng, Method::Invite, &host, self.cfg.sip_port);
            inv.headers_mut().push("X-Conference", conf.to_string());
            inv.headers_mut().push("Contact", format!("<sip:b2bua@{}:{}>", host, self.cfg.sip_port));
            inv.set_body("application/sdp", relay_sdp(&self.cfg, &sdp, LegKind::Mrf, port).to_bytes());
            self.mixers.insert(d.call_id.clone(), (id, a_side));
            self.by_call_id.insert(d.call_id.clone(), id);
            let c = self.calls.get_mut(&id).expect("live");
            let leg = if a_side { &mut c.a } else { c.b.as_mut().expect("leg") };
            leg.mix = Some(Mix { port, mrf_media: None, dialog: d });
            self.tx.request(id, mrf, &inv, now);
        }
    }

    /// mrfd answered a mixer INVITE.
    fn mixer_response(&mut self, call: u64, a_side: bool, m: Message, now: u64) {
        let Some(code) = m.code() else { return };
        if code < 200 {
            return;
        }
        let host = self.host();
        let Some(c) = self.calls.get_mut(&call) else { return };
        let leg = if a_side { Some(&mut c.a) } else { c.b.as_mut() };
        let Some(leg) = leg else { return };
        let Some(mix) = leg.mix.as_mut() else { return };
        if (200..300).contains(&code) {
            mix.dialog.learn(&m);
            if let Ok(sdp) = SessionDescription::parse(m.body()) {
                mix.mrf_media = Self::media_of(&sdp);
            }
            let ack = ack_for(&mix.dialog, &mut self.rng, &host, self.cfg.sip_port);
            let hop = mix.dialog.next_hop;
            self.tx.request(call, hop, &ack, now);
        } else {
            let port = mix.port;
            leg.mix = None;
            self.free_relay(port);
            self.log(format!("conference join for call {} failed: {}", call, code));
        }
    }

    /// Leave the conference: hang up a leg's mixer dialog.
    fn unmix(&mut self, call: u64, a_side: bool, now: u64) {
        let host = self.host();
        let Some(c) = self.calls.get_mut(&call) else { return };
        let leg = if a_side { Some(&mut c.a) } else { c.b.as_mut() };
        let Some(leg) = leg else { return };
        leg.muted = false;
        let Some(mut mix) = leg.mix.take() else { return };
        let bye = mix.dialog.request(&mut self.rng, Method::Bye, &host, self.cfg.sip_port);
        self.tx.request(call, mix.dialog.next_hop, &bye, now);
        self.mixers.remove(&mix.dialog.call_id);
        self.by_call_id.remove(&mix.dialog.call_id);
        self.free_relay(mix.port);
    }

    /// A call of a conference ended: the other call goes back to a plain call.
    fn leave_conference(&mut self, conf: u64, gone: u64, now: u64) {
        let rest: Vec<u64> = self.confs.remove(&conf).unwrap_or_default().into_iter().filter(|c| *c != gone).collect();
        for id in rest {
            self.unmix(id, true, now);
            self.unmix(id, false, now);
            if let Some(c) = self.calls.get_mut(&id) {
                c.conf = None;
            }
        }
    }

    // ------------------------------------------------------------ media relay

    /// RTP (or RTCP) arrived on relay port `port`.
    pub fn on_relay(&mut self, port: u16, _from: Addr, data: &[u8], _now: u64) {
        let Some(role) = self.relays.get(&port).copied() else { return };
        match role {
            RelayRole::Leg { call, a_side } => {
                let Some(c) = self.calls.get(&call) else { return };
                let (this, other) = if a_side { (Some(&c.a), c.b.as_ref()) } else { (c.b.as_ref(), Some(&c.a)) };
                let Some(this) = this else { return };
                if this.muted {
                    return;
                }
                if let Some(mix) = &this.mix {
                    if let Some(to) = mix.mrf_media {
                        self.out.push(ImsOut::Relay { port: mix.port, to, data: data.to_vec() });
                    }
                    return;
                }
                let Some(other) = other else { return };
                if other.mix.is_some() || other.muted {
                    return;
                }
                if let Some(to) = other.media {
                    self.out.push(ImsOut::Relay { port: other.relay, to, data: data.to_vec() });
                }
            }
            RelayRole::Mix { call, a_side } => {
                // the mixer's output for this participant
                let Some(c) = self.calls.get(&call) else { return };
                let leg = if a_side { Some(&c.a) } else { c.b.as_ref() };
                if let Some(l) = leg {
                    if let Some(to) = l.media {
                        self.out.push(ImsOut::Relay { port: l.relay, to, data: data.to_vec() });
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------ time

    pub fn tick(&mut self, now: u64) {
        self.tx.tick(now);
        for e in self.tx.take_events() {
            let TxEvent::Timeout { owner, method, .. } = e;
            if method == Method::Invite && self.calls.contains_key(&owner) {
                // the callee never answered the INVITE at all
                let fake = Message::response(408, None);
                let _ = fake;
                self.b_timeout(owner, now);
            }
        }
        let due: Vec<u64> = self.calls.values().filter(|c| c.answer_deadline.is_some_and(|d| now >= d)).map(|c| c.id).collect();
        for id in due {
            self.no_answer(id, now);
        }
        let expired: Vec<String> = self.regs.values().filter(|r| now >= r.expires_ms).map(|r| r.msisdn.clone()).collect();
        for m in expired {
            if let Some(r) = self.regs.remove(&m) {
                self.log(format!("{} registration expired", m));
                self.hss(Pending::Bearer, format!("SAR {} deregister scscf.{}", r.imsi, self.cfg.domain));
            }
        }
        self.vectors.retain(|_, v| now.saturating_sub(v.2) < 60_000);
    }

    fn b_timeout(&mut self, call: u64, now: u64) {
        let Some(c) = self.calls.get_mut(&call) else { return };
        let Some(b) = c.b.take() else { return };
        if b.kind == LegKind::Phone {
            if let Some(r) = self.regs.get_mut(&b.number) {
                r.calls = r.calls.saturating_sub(1);
            }
        }
        self.free_relay(b.relay);
        self.by_call_id.remove(&b.dialog.call_id);
        let p = self.calls.get(&call).and_then(|c| c.callee.clone()).unwrap_or_default();
        if b.kind == LegKind::Phone {
            if !p.cfnrc.is_empty() {
                return self.forward(call, &p.cfnrc, "unreachable", now);
            }
            return self.no_reach(call, &p, Outcome::Unreachable, now);
        }
        self.end_with(call, 408, Outcome::Unreachable, now);
    }

    fn no_answer(&mut self, call: u64, now: u64) {
        let Some(c) = self.calls.get_mut(&call) else { return };
        c.answer_deadline = None;
        let p = c.callee.clone().unwrap_or_default();
        self.log(format!("call {}: no answer", call));
        if !p.cfnr.is_empty() {
            return self.forward(call, &p.cfnr, "no answer", now);
        }
        if p.vm {
            return self.to_service(call, "voicemail", &p.msisdn, now);
        }
        if let Some(b) = self.calls.get_mut(&call).and_then(|c| c.b.take()) {
            self.drop_leg(call, b, now);
        }
        self.end_with(call, 480, Outcome::NoAnswer, now);
    }
}
