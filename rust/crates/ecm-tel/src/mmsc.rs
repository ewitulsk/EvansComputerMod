//! `mmscd`, sans-IO: the carrier's MMSC (docs/TELEPHONY.md).
//!
//! Real MMS flow (OMA MMS encapsulation, [`crate::mms`]):
//!
//! 1. The phone POSTs an `M-Send.req` to `http://<mmsc>:8002/mms` over its data
//!    bearer (zero-rated by the core). Who sent it comes from the core
//!    (`IPD`: the address's subscriber, as a P-GW's header enrichment), not
//!    from the PDU. Over `max_bytes` gets `Error-content-not-accepted`.
//! 2. It is stored and answered `M-Send.conf` (**Sent**). Every recipient gets
//!    an `M-Notification.ind` as a WAP push (a binary SMS to port 2948, sent
//!    through `smscd`, so it pages an idle phone and waits for an off one).
//! 3. The recipient's phone GETs the content location and answers
//!    `M-NotifyResp.ind` (status Retrieved); the sender then gets an
//!    `M-Delivery.ind` (**Delivered**) the same way.
//! 4. Several recipients make a group: every phone gets the whole To list, so
//!    replies go to the whole group.
//! 5. Recipients on other carriers: MM4 ([`MmscOut::Mm4`]), an SMTP message to
//!    that carrier's MMSC (found by ENUM, then the domain's MX), carrying the
//!    message as a base64 `application/vnd.wap.mms-message` part; its delivery
//!    report comes back as `MM4_delivery_report.REQ`.
//! 6. A recipient whose phone doesn't fetch MMS (it registered without
//!    `+ecm.mms`, e.g. a Terminal) gets a text with a link instead; the link
//!    (`/v/<id>?r=<token>`) is a web page for the Browser.
//!
//! Content waits `expiry_ms` (7 in-game days). One CDR line per MMS.

use std::collections::BTreeMap;

use crate::ims::kv;
use crate::mms::{self, Body, MessageType, Pdu, ResponseStatus, Status, TimeSpec};
use crate::sip::{self, Method, Uri};
use crate::dialog::{number_uri, Dialog};
use crate::txn::{Addr, Rng, Txns};
use crate::util::{base64_decode, base64_encode, hex};

pub const SIP_PORT: u16 = 5063;
pub const HTTP_PORT: u16 = 8002;
pub const SPOOL: &str = "/var/spool/mms";

#[derive(Clone, Debug)]
pub struct MmscConfig {
    pub domain: String,
    pub carrier: String,
    pub ip: [u8; 4],
    pub sip_port: u16,
    pub http_port: u16,
    pub smsc: Addr,
    pub max_bytes: usize,
    pub expiry_ms: u64,
    /// Other carriers' domains we exchange MM4 with, and the shared secret.
    pub peers: Vec<(String, String)>,
    pub public_peering: bool,
    /// Signs Browser links.
    pub secret: String,
}

impl MmscConfig {
    pub fn new(domain: &str, carrier: &str, ip: [u8; 4]) -> MmscConfig {
        MmscConfig {
            domain: domain.into(),
            carrier: carrier.into(),
            ip,
            sip_port: SIP_PORT,
            http_port: HTTP_PORT,
            smsc: Addr::new(ip, 5062),
            max_bytes: 600 * 1024,
            expiry_ms: 7 * 20 * 60 * 1000,
            peers: Vec::new(),
            public_peering: false,
            secret: "mmsc".into(),
        }
    }
    pub fn url(&self) -> String {
        format!("http://{}:{}", Addr::new(self.ip, 0).host(), self.http_port)
    }
}

/// An HTTP answer.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpReply {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

impl HttpReply {
    fn new(status: u16, ct: &str, body: Vec<u8>) -> HttpReply {
        HttpReply { status, content_type: ct.into(), body }
    }
    fn text(status: u16, s: &str) -> HttpReply {
        HttpReply::new(status, "text/plain", s.as_bytes().to_vec())
    }
    fn pdu(p: &Pdu) -> HttpReply {
        HttpReply::new(200, mms::MMS_CONTENT_TYPE, p.encode())
    }
}

/// [`Mmsc::http`]: answered now, or once the core says who is asking.
#[derive(Debug, PartialEq)]
pub enum HttpResult {
    Now(HttpReply),
    Later(u64),
}

#[derive(Debug, PartialEq)]
pub enum MmscOut {
    Sip { to: Addr, data: Vec<u8> },
    Hss { id: u64, line: String },
    /// Find the carrier of a number (ENUM); answer with [`Mmsc::on_enum`].
    Enum { id: u64, number: String },
    /// Send an MM4 message (RFC 5322 text) to `domain`'s MMSC (its MX, SMTP);
    /// answer with [`Mmsc::on_mm4_sent`].
    Mm4 { id: u64, domain: String, rcpt: Vec<String>, from: String, message: String },
    /// The answer to a [`HttpResult::Later`] request.
    HttpReply { req: u64, reply: HttpReply },
    Save { path: String, data: Vec<u8> },
    Delete { path: String },
    Cdr(String),
    Log(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recipient {
    pub number: String,
    /// `local`, or another carrier's domain.
    pub route: String,
    pub notified: bool,
    pub fetched: bool,
    pub reported: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stored {
    pub id: String,
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub date_ms: u64,
    pub body: Body,
    pub size: usize,
    pub recipients: Vec<Recipient>,
    /// From another carrier (MM4): its domain, for the delivery report.
    pub origin: Option<String>,
    pub delivery_report: bool,
}

enum Pending {
    /// Who POSTed (IPD), then what.
    Send { req: u64, pdu: Pdu },
    Fetch { req: u64, id: String },
    Ack { req: u64, pdu: Pdu },
    /// Where a recipient is (LIR).
    Locate { id: String, number: String },
    Enum { id: String, number: String },
    Mm4 { id: String, number: String },
    Report,
}

pub struct Mmsc {
    pub cfg: MmscConfig,
    rng: Rng,
    tx: Txns,
    store: BTreeMap<String, Stored>,
    pending: BTreeMap<u64, Pending>,
    next: u64,
    /// Registered phones that don't fetch MMS (from imsd's notices).
    no_mms: BTreeMap<String, bool>,
    out: Vec<MmscOut>,
}

/// `+1214...` from `+1214.../TYPE=PLMN` or `+1214...@domain/TYPE=PLMN`.
fn bare(a: &str) -> String {
    let (n, _) = mms::parse_address(a);
    n.split('@').next().unwrap_or(n).to_string()
}

/// The MM4 proof: `HMAC-SHA256(secret, "ECM-MM4" || domain || message-id)`.
pub fn mm4_proof(secret: &str, domain: &str, message_id: &str) -> String {
    crate::ims::peer_proof(secret, &format!("MM4{}", domain), message_id)
}

impl Mmsc {
    pub fn new(cfg: MmscConfig, seed: u64) -> Mmsc {
        Mmsc { cfg, rng: Rng::new(seed), tx: Txns::new(), store: BTreeMap::new(), pending: BTreeMap::new(), next: 1, no_mms: BTreeMap::new(), out: Vec::new() }
    }

    pub fn take_out(&mut self) -> Vec<MmscOut> {
        for (to, data) in self.tx.take_sends() {
            self.out.push(MmscOut::Sip { to, data });
        }
        std::mem::take(&mut self.out)
    }

    fn log(&mut self, s: String) {
        self.out.push(MmscOut::Log(s));
    }

    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }

    pub fn stored(&self) -> Vec<&Stored> {
        self.store.values().collect()
    }

    fn hss(&mut self, p: Pending, line: String) -> u64 {
        let id = self.id();
        self.pending.insert(id, p);
        self.out.push(MmscOut::Hss { id, line });
        id
    }

    /// An HTTP request from `from`: `POST /mms` (M-Send.req, M-NotifyResp.ind,
    /// M-Acknowledge.ind), `GET /mms/<id>` (M-Retrieve.conf), `GET /v/<id>?r=<token>` (a web page).
    pub fn http(&mut self, method: &str, path: &str, from: [u8; 4], body: &[u8], now: u64) -> HttpResult {
        let (p, query) = path.split_once('?').unwrap_or((path, ""));
        let req = self.id();
        let who = format!("IPD {}", Addr::new(from, 0).host());
        match (method, p) {
            ("POST", "/mms") => {
                let Ok(pdu) = Pdu::decode(body) else {
                    return HttpResult::Now(HttpReply::text(400, "not an MMS PDU\n"));
                };
                match pdu.message_type {
                    MessageType::SendReq => {
                        if body.len() > self.cfg.max_bytes {
                            let conf = Pdu::send_conf(pdu.transaction_id.as_deref().unwrap_or(""), ResponseStatus::ErrorContentNotAccepted, None);
                            return HttpResult::Now(HttpReply::pdu(&conf));
                        }
                        self.hss(Pending::Send { req, pdu }, who);
                    }
                    MessageType::NotifyRespInd | MessageType::AcknowledgeInd => {
                        self.hss(Pending::Ack { req, pdu }, who);
                    }
                    _ => return HttpResult::Now(HttpReply::text(400, "unexpected PDU\n")),
                }
                HttpResult::Later(req)
            }
            ("GET", p) if p.starts_with("/mms/") => {
                let id = p.trim_start_matches("/mms/").to_string();
                self.hss(Pending::Fetch { req, id }, who);
                HttpResult::Later(req)
            }
            ("GET", p) if p.starts_with("/v/") => HttpResult::Now(self.web_view(p, query)),
            _ => {
                let _ = now;
                HttpResult::Now(HttpReply::text(404, "not found\n"))
            }
        }
    }

    fn token(&self, id: &str, number: &str) -> String {
        crate::ims::peer_proof(&self.cfg.secret, id, number)[..16].to_string()
    }

    /// The Browser fallback page.
    fn web_view(&self, p: &str, query: &str) -> HttpReply {
        let rest = p.trim_start_matches("/v/");
        let (id, part) = match rest.split_once('/') {
            Some((i, n)) => (i, n.parse::<usize>().ok()),
            None => (rest, None),
        };
        let r = query.split('&').find_map(|kv| kv.strip_prefix("r=")).unwrap_or("");
        let Some(m) = self.store.get(id) else { return HttpReply::text(404, "this message is gone (they expire after 7 days)\n") };
        let ok = m.recipients.iter().any(|x| self.token(id, &x.number) == r);
        if !ok {
            return HttpReply::text(403, "bad link\n");
        }
        if let Some(n) = part {
            return match m.body.attachments().get(n) {
                Some(a) => HttpReply::new(200, &a.content_type, a.data.clone()),
                None => HttpReply::text(404, "no such part\n"),
            };
        }
        let mut h = format!(
            "<!doctype html><html><head><title>Picture message</title></head><body><h1>Message from {}</h1><p>to {}</p>",
            esc(&m.from),
            esc(&m.to.join(", "))
        );
        if !m.subject.is_empty() {
            h += &format!("<h2>{}</h2>", esc(&m.subject));
        }
        if let Some(t) = m.body.text() {
            h += &format!("<p>{}</p>", esc(&t));
        }
        for (i, a) in m.body.attachments().iter().enumerate() {
            let src = format!("/v/{}/{}?r={}", id, i, r);
            if a.content_type.starts_with("image/") {
                h += &format!("<p><img src=\"{}\" alt=\"picture\"></p>", src);
            } else {
                h += &format!("<p><a href=\"{}\">{}</a></p>", src, esc(a.name.as_deref().unwrap_or("attachment")));
            }
        }
        h += "</body></html>";
        HttpReply::new(200, "text/html", h.into_bytes())
    }

    pub fn on_hss(&mut self, id: u64, reply: Result<String, String>, now: u64) {
        let Some(p) = self.pending.remove(&id) else { return };
        let who = reply.as_ref().ok().map(|b| kv(b, "msisdn")).filter(|m| m.starts_with('+'));
        match p {
            Pending::Send { req, pdu } => {
                let tid = pdu.transaction_id.clone().unwrap_or_default();
                let Some(from) = who else {
                    let conf = Pdu::send_conf(&tid, ResponseStatus::ErrorServiceDenied, None);
                    return self.out.push(MmscOut::HttpReply { req, reply: HttpReply::pdu(&conf) });
                };
                let to: Vec<String> = pdu.to.iter().map(|a| bare(a)).filter(|a| !a.is_empty()).collect();
                if to.is_empty() || to.len() > 20 {
                    let conf = Pdu::send_conf(&tid, ResponseStatus::ErrorSendingAddressUnresolved, None);
                    return self.out.push(MmscOut::HttpReply { req, reply: HttpReply::pdu(&conf) });
                }
                let mid = format!("{}{}", hex(&self.rng.next().to_be_bytes()), now % 100_000);
                let body = pdu.body.clone().unwrap_or_else(|| Body::mixed(Vec::new()));
                let size = body.encode().len();
                let m = Stored {
                    id: mid.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    subject: pdu.subject.clone().unwrap_or_default(),
                    date_ms: now,
                    body,
                    size,
                    recipients: to.iter().map(|n| Recipient { number: n.clone(), route: String::new(), notified: false, fetched: false, reported: false }).collect(),
                    origin: None,
                    delivery_report: pdu.delivery_report.unwrap_or(true),
                };
                self.out.push(MmscOut::Cdr(format!("mms time={} from={} to={} bytes={} parts={}", now / 1000, from, to.join(","), size, m.body.parts.len())));
                self.log(format!("MMS {} from {} to {} ({} bytes)", mid, from, to.join(","), size));
                self.save(&m);
                self.store.insert(mid.clone(), m);
                let conf = Pdu::send_conf(&tid, ResponseStatus::Ok, Some(&mid));
                self.out.push(MmscOut::HttpReply { req, reply: HttpReply::pdu(&conf) });
                for n in to {
                    self.hss(Pending::Locate { id: mid.clone(), number: n.clone() }, format!("LIR {}", n));
                }
            }
            Pending::Fetch { req, id } => {
                let reply = match (who, self.store.get(&id)) {
                    (Some(w), Some(m)) if m.recipients.iter().any(|r| r.number == w) || m.from == w => {
                        let p = Pdu::retrieve_conf(None, Some(&m.id), m.date_ms / 1000, Some(&mms::plmn_address(&m.from)), m.to.iter().map(|t| mms::plmn_address(t)).collect(), (!m.subject.is_empty()).then_some(m.subject.as_str()), m.body.clone());
                        let r = HttpReply::pdu(&p);
                        if let Some(x) = self.store.get_mut(&id).and_then(|m| m.recipients.iter_mut().find(|r| r.number == w)) {
                            x.fetched = true;
                        }
                        r
                    }
                    (_, None) => HttpReply::text(404, "expired or unknown\n"),
                    _ => HttpReply::text(403, "not yours\n"),
                };
                self.out.push(MmscOut::HttpReply { req, reply });
            }
            Pending::Ack { req, pdu } => {
                let tid = pdu.transaction_id.clone().unwrap_or_default();
                self.out.push(MmscOut::HttpReply { req, reply: HttpReply::text(204, "") });
                let Some(w) = who else { return };
                // the transaction id is "<message id>.<n>": which message this phone acknowledges
                let mid = tid.split('.').next().unwrap_or("").to_string();
                let retrieved = pdu.message_type == MessageType::AcknowledgeInd || pdu.status == Some(Status::Retrieved);
                if retrieved {
                    self.delivered(&mid, &w, now);
                }
            }
            Pending::Locate { id, number } => {
                let b = reply.unwrap_or_default();
                match kv(&b, "result").as_str() {
                    "local" => self.route(&id, &number, "local", now),
                    _ => {
                        let eid = self.id();
                        self.pending.insert(eid, Pending::Enum { id, number: number.clone() });
                        self.out.push(MmscOut::Enum { id: eid, number });
                    }
                }
            }
            Pending::Enum { .. } | Pending::Mm4 { .. } | Pending::Report => {}
        }
    }

    /// ENUM answered: the domain of a recipient's carrier (None: nobody).
    pub fn on_enum(&mut self, id: u64, domain: Option<String>, now: u64) {
        let Some(Pending::Enum { id: mid, number }) = self.pending.remove(&id) else { return };
        match domain {
            Some(d) if d != self.cfg.domain => self.route(&mid, &number, &d, now),
            _ => {
                self.log(format!("MMS {} to {}: no carrier has that number", mid, number));
                self.report_to_sender(&mid, &number, Status::Rejected, now);
            }
        }
    }

    fn route(&mut self, id: &str, number: &str, route: &str, now: u64) {
        let Some(m) = self.store.get_mut(id) else { return };
        if let Some(r) = m.recipients.iter_mut().find(|r| r.number == number) {
            r.route = route.to_string();
        }
        if route == "local" {
            self.notify(id, number, now);
        } else {
            self.mm4_forward(id, number, route, now);
        }
    }

    /// The recipient's phone gets the notification (or, if it doesn't do MMS, a link).
    fn notify(&mut self, id: &str, number: &str, now: u64) {
        let Some(m) = self.store.get(id).cloned() else { return };
        if self.no_mms.get(number).copied().unwrap_or(false) {
            let link = format!("{}/v/{}?r={}", self.cfg.url(), id, self.token(id, number));
            let text = format!("{} sent you a picture message: {}", m.from, link);
            self.service_text(number, &text, now);
        } else {
            let n = m.recipients.iter().position(|r| r.number == number).unwrap_or(0);
            let tid = format!("{}.{}", id, n);
            let location = format!("{}/mms/{}", self.cfg.url(), id);
            let pdu = Pdu::notification_ind(&tid, Some(&mms::plmn_address(&m.from)), (!m.subject.is_empty()).then_some(m.subject.as_str()), m.size as u64, TimeSpec::Relative(self.cfg.expiry_ms / 1000), &location);
            self.push(number, &pdu, now);
        }
        if let Some(r) = self.store.get_mut(id).and_then(|m| m.recipients.iter_mut().find(|r| r.number == number)) {
            r.notified = true;
        }
    }

    /// A WAP push (an MMS PDU) to a phone, through the SMSC.
    fn push(&mut self, number: &str, pdu: &Pdu, now: u64) {
        let tid = (self.rng.next() & 0xff) as u8;
        let body = mms::wap_push_encode(&pdu.encode(), tid);
        let host = Addr::new(self.cfg.ip, 0).host();
        let smsc = self.cfg.smsc;
        let mut d = Dialog::outgoing(&mut self.rng, Uri::sip(Some("mmsc"), &self.cfg.domain, None), None, Uri::sip(Some("wap-push"), &smsc.host(), Some(smsc.port)), smsc, &host);
        let mut m = d.request(&mut self.rng, Method::Message, &host, self.cfg.sip_port);
        m.headers_mut().set("To", format!("<{}>", number_uri(number, &self.cfg.domain)));
        m.set_body(mms::MMS_CONTENT_TYPE, body);
        self.tx.request(0, smsc, &m, now);
    }

    fn service_text(&mut self, number: &str, text: &str, now: u64) {
        let host = Addr::new(self.cfg.ip, 0).host();
        let smsc = self.cfg.smsc;
        let mut d = Dialog::outgoing(&mut self.rng, Uri::sip(Some("mmsc"), &self.cfg.domain, None), None, Uri::sip(Some("service-text"), &smsc.host(), Some(smsc.port)), smsc, &host);
        let mut m = d.request(&mut self.rng, Method::Message, &host, self.cfg.sip_port);
        m.headers_mut().set("To", format!("<{}>", number_uri(number, &self.cfg.domain)));
        m.headers_mut().push("X-Service-From", "MMS");
        m.set_body("text/plain;charset=utf-8", text.as_bytes().to_vec());
        self.tx.request(0, smsc, &m, now);
    }

    fn delivered(&mut self, id: &str, number: &str, now: u64) {
        let Some(m) = self.store.get_mut(id) else { return };
        let Some(r) = m.recipients.iter_mut().find(|r| r.number == number) else { return };
        if r.reported {
            return;
        }
        r.fetched = true;
        r.reported = true;
        let m = m.clone();
        self.log(format!("MMS {} retrieved by {}", id, number));
        if let Some(origin) = &m.origin {
            self.mm4_report(&m, number, origin, now);
        } else if m.delivery_report {
            self.report_to_sender(id, number, Status::Retrieved, now);
        }
        self.save(&m);
    }

    fn report_to_sender(&mut self, id: &str, number: &str, status: Status, now: u64) {
        let Some(m) = self.store.get(id).cloned() else { return };
        if m.origin.is_some() {
            return;
        }
        let p = Pdu::delivery_ind(id, &mms::plmn_address(number), now / 1000, status);
        let from = m.from.clone();
        self.push(&from, &p, now);
    }

    /// `registered <number> <ip> mms|nomms` from imsd: notifications that wait go out.
    pub fn on_sip(&mut self, from: Addr, data: &[u8], now: u64) {
        let Ok(m) = sip::parse(data) else { return };
        if !m.is_request() {
            self.tx.on_response(&m);
            return;
        }
        if !self.tx.on_request(from, &m, now) {
            return;
        }
        if let Ok(r) = m.response_to(200, Some("mmsc")) {
            self.tx.respond(crate::dialog::response_target(&m, from), &r, now);
        }
        if m.method() == Some(&Method::Notify) {
            let body = String::from_utf8_lossy(m.body()).to_string();
            for l in body.lines() {
                let w: Vec<&str> = l.split_whitespace().collect();
                if let ["registered", n, _ip, cap] = w.as_slice() {
                    self.no_mms.insert(n.to_string(), *cap == "nomms");
                }
            }
        }
    }

    // ------------------------------------------------------------ MM4

    fn mm4_forward(&mut self, id: &str, number: &str, domain: &str, now: u64) {
        let Some(m) = self.store.get(id).cloned() else { return };
        let peer = self.cfg.peers.iter().find(|(d, _)| d == domain).cloned();
        if peer.is_none() && !self.cfg.public_peering {
            self.log(format!("MMS {} to {}: {} is not a peer", id, number, domain));
            return self.report_to_sender(id, number, Status::Rejected, now);
        }
        let pdu = Pdu::retrieve_conf(None, Some(&m.id), m.date_ms / 1000, Some(&mms::plmn_address(&m.from)), m.to.iter().map(|t| mms::plmn_address(t)).collect(), (!m.subject.is_empty()).then_some(m.subject.as_str()), m.body.clone());
        let proof = peer.map(|(_, s)| format!("{};proof={}", self.cfg.domain, mm4_proof(&s, &self.cfg.domain, &m.id))).unwrap_or_else(|| self.cfg.domain.clone());
        let msg = format!(
            "From: {from}@{dom}/TYPE=PLMN\r\nTo: {to}@{peer}/TYPE=PLMN\r\nSubject: {subj}\r\nDate: {date}\r\nMIME-Version: 1.0\r\nX-Mms-3GPP-MMS-Version: 6.10.0\r\nX-Mms-Message-Type: MM4_forward.REQ\r\nX-Mms-Transaction-ID: {id}.{to}\r\nX-Mms-Message-ID: {id}\r\nX-Mms-Ack-Request: Yes\r\nX-Mms-Originator-System: system-user@mms.{dom}\r\nX-ECM-Peer: {proof}\r\nContent-Type: application/vnd.wap.mms-message\r\nContent-Transfer-Encoding: base64\r\n\r\n{body}\r\n",
            from = m.from,
            dom = self.cfg.domain,
            to = number,
            peer = domain,
            subj = m.subject.replace(['\r', '\n'], " "),
            date = m.date_ms / 1000,
            id = m.id,
            proof = proof,
            body = wrap76(&base64_encode(&pdu.encode()))
        );
        let eid = self.id();
        self.pending.insert(eid, Pending::Mm4 { id: id.into(), number: number.into() });
        self.out.push(MmscOut::Mm4 { id: eid, domain: domain.into(), rcpt: vec![format!("{}@{}/TYPE=PLMN", number, domain)], from: format!("system-user@mms.{}", self.cfg.domain), message: msg });
    }

    /// The SMTP exchange for an `Mm4` ended: accepted (`true`) or not.
    pub fn on_mm4_sent(&mut self, id: u64, ok: bool, now: u64) {
        let Some(Pending::Mm4 { id: mid, number }) = self.pending.remove(&id) else { return };
        if let Some(r) = self.store.get_mut(&mid).and_then(|m| m.recipients.iter_mut().find(|r| r.number == number)) {
            r.notified = ok;
        }
        if !ok {
            self.log(format!("MM4 for {} ({}) refused", mid, number));
            self.report_to_sender(&mid, &number, Status::Unreachable, now);
        }
    }

    fn mm4_report(&mut self, m: &Stored, number: &str, origin: &str, _now: u64) {
        let peer = self.cfg.peers.iter().find(|(d, _)| d == origin).cloned();
        let proof = peer.map(|(_, s)| format!("{};proof={}", self.cfg.domain, mm4_proof(&s, &self.cfg.domain, &m.id))).unwrap_or_else(|| self.cfg.domain.clone());
        let msg = format!(
            "From: {to}@{dom}/TYPE=PLMN\r\nTo: {from}@{origin}/TYPE=PLMN\r\nMIME-Version: 1.0\r\nX-Mms-3GPP-MMS-Version: 6.10.0\r\nX-Mms-Message-Type: MM4_delivery_report.REQ\r\nX-Mms-Message-ID: {id}\r\nX-Mms-MM-Status-Code: Retrieved\r\nX-ECM-Peer: {proof}\r\n\r\n",
            to = number,
            dom = self.cfg.domain,
            from = m.from,
            origin = origin,
            id = m.id,
            proof = proof
        );
        let eid = self.id();
        self.pending.insert(eid, Pending::Report);
        self.out.push(MmscOut::Mm4 { id: eid, domain: origin.into(), rcpt: vec![format!("{}@{}/TYPE=PLMN", m.from, origin)], from: format!("system-user@mms.{}", self.cfg.domain), message: msg });
    }

    /// An MM4 message arrived over SMTP (after `DATA`). `Err`: the SMTP error to answer.
    pub fn on_mm4(&mut self, text: &str, now: u64) -> Result<(), String> {
        let (head, body) = text.split_once("\r\n\r\n").or_else(|| text.split_once("\n\n")).unwrap_or((text, ""));
        let mut hdr: BTreeMap<String, String> = BTreeMap::new();
        for l in head.lines() {
            if let Some((k, v)) = l.split_once(':') {
                hdr.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            }
        }
        let g = |k: &str| hdr.get(k).cloned().unwrap_or_default();
        let mid = g("x-mms-message-id");
        // the peer
        let ph = g("x-ecm-peer");
        let (pdom, proof) = match ph.split_once(";proof=") {
            Some((d, p)) => (d.trim().to_string(), p.trim().to_string()),
            None => (ph.trim().to_string(), String::new()),
        };
        let ok = match self.cfg.peers.iter().find(|(d, _)| *d == pdom) {
            Some((_, s)) => crate::util::ct_eq_str(&mm4_proof(s, &pdom, &mid), &proof),
            None => self.cfg.public_peering && !pdom.is_empty(),
        };
        if !ok {
            self.log(format!("MM4 from unlisted peer '{}' refused", pdom));
            return Err("550 not a peer of this MMSC".into());
        }
        match g("x-mms-message-type").as_str() {
            "MM4_forward.REQ" => {
                let raw = base64_decode(&body.split_whitespace().collect::<String>()).ok_or("554 bad base64")?;
                let p = Pdu::decode(&raw).map_err(|e| format!("554 bad MMS PDU: {}", e))?;
                let from = bare(&g("from"));
                let to_hdr = bare(&g("to"));
                let all_to: Vec<String> = p.to.iter().map(|a| bare(a)).collect();
                let body = p.body.clone().unwrap_or_else(|| Body::mixed(Vec::new()));
                let size = body.encode().len();
                // keep the originator's message id: its delivery report names it
                let id = if self.store.contains_key(&mid) || mid.is_empty() { format!("{}-{}", mid, &hex(&self.rng.next().to_be_bytes())[..6]) } else { mid.clone() };
                let m = Stored {
                    id: id.clone(),
                    from: from.clone(),
                    to: if all_to.is_empty() { vec![to_hdr.clone()] } else { all_to },
                    subject: p.subject.clone().unwrap_or_default(),
                    date_ms: now,
                    body,
                    size,
                    recipients: vec![Recipient { number: to_hdr.clone(), route: "local".into(), notified: false, fetched: false, reported: false }],
                    origin: Some(pdom.clone()),
                    delivery_report: true,
                };
                self.log(format!("MM4 from {}: MMS {} from {} for {}", pdom, id, from, to_hdr));
                self.out.push(MmscOut::Cdr(format!("mms time={} from={} to={} bytes={} via={}", now / 1000, from, to_hdr, size, pdom)));
                self.save(&m);
                self.store.insert(id.clone(), m);
                self.notify(&id, &to_hdr, now);
                Ok(())
            }
            "MM4_delivery_report.REQ" => {
                let to = bare(&g("from"));
                if self.store.contains_key(&mid) {
                    self.report_to_sender(&mid, &to, Status::Retrieved, now);
                }
                Ok(())
            }
            other => Err(format!("554 unknown MM4 type {}", other)),
        }
    }

    fn save(&mut self, m: &Stored) {
        let pdu = Pdu::retrieve_conf(None, Some(&m.id), m.date_ms / 1000, Some(&mms::plmn_address(&m.from)), m.to.iter().map(|t| mms::plmn_address(t)).collect(), (!m.subject.is_empty()).then_some(m.subject.as_str()), m.body.clone());
        self.out.push(MmscOut::Save { path: format!("{}/{}.mms", SPOOL, m.id), data: pdu.encode() });
    }

    pub fn tick(&mut self, now: u64) {
        self.tx.tick(now);
        let _ = self.tx.take_events();
        let old: Vec<String> = self.store.values().filter(|m| now.saturating_sub(m.date_ms) >= self.cfg.expiry_ms).map(|m| m.id.clone()).collect();
        for id in old {
            self.store.remove(&id);
            self.out.push(MmscOut::Delete { path: format!("{}/{}.mms", SPOOL, id) });
        }
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn wrap76(s: &str) -> String {
    s.as_bytes().chunks(76).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\r\n")
}

