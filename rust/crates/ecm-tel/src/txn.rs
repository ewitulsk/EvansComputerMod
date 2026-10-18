//! SIP transactions over UDP (RFC 3261 section 17, trimmed): what every
//! telephony program needs so a lost datagram doesn't lose a call or a text.
//!
//! * **Client side**: a request is resent at 500 ms, 1 s, 2 s, 4 s ... (Timer A)
//!   until a response arrives; an INVITE stops at its first response (a
//!   provisional one), anything else at its final one. After 32 s (Timer B / F) the
//!   transaction times out and the owner gets [`TxEvent::Timeout`].
//! * **Server side**: a retransmitted request (same branch and method) is
//!   answered with the last response sent, never handed up twice.
//! * **2xx to INVITE**: resent (500 ms doubling) until the ACK comes, as the
//!   transaction user must (RFC 3261 13.3.1.4).
//!
//! Addresses are [`Addr`]; the machine owning the transactions sends what
//! [`Txns::take_sends`] returns.

use std::collections::BTreeMap;

use crate::sip::{Message, Method};

/// IPv4 address and port.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Addr {
    pub ip: [u8; 4],
    pub port: u16,
}

impl Addr {
    pub fn new(ip: [u8; 4], port: u16) -> Addr {
        Addr { ip, port }
    }
    pub fn host(&self) -> String {
        format!("{}.{}.{}.{}", self.ip[0], self.ip[1], self.ip[2], self.ip[3])
    }
    pub fn parse_ip(s: &str) -> Option<[u8; 4]> {
        let mut ip = [0u8; 4];
        let mut it = s.split('.');
        for b in ip.iter_mut() {
            *b = it.next()?.parse().ok()?;
        }
        if it.next().is_some() {
            return None;
        }
        Some(ip)
    }
    /// `a.b.c.d:port` (port defaults to `default`).
    pub fn parse(s: &str, default: u16) -> Option<Addr> {
        let (h, p) = match s.rsplit_once(':') {
            Some((h, p)) => (h, p.parse().ok()?),
            None => (s, default),
        };
        Some(Addr { ip: Addr::parse_ip(h)?, port: p })
    }
}

impl std::fmt::Display for Addr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host(), self.port)
    }
}

/// What happened to a client transaction.
#[derive(Clone, Debug, PartialEq)]
pub enum TxEvent {
    /// No response in time: `owner` should treat it as a 408.
    Timeout { owner: u64, branch: String, method: Method },
}

struct Client {
    owner: u64,
    method: Method,
    to: Addr,
    bytes: Vec<u8>,
    next_ms: u64,
    interval: u64,
    deadline: u64,
    provisional: bool,
}

struct Server {
    last: Option<(Addr, Vec<u8>)>,
    at_ms: u64,
}

struct Ok2xx {
    to: Addr,
    bytes: Vec<u8>,
    next_ms: u64,
    interval: u64,
    deadline: u64,
}

/// The transaction layer of one SIP endpoint.
#[derive(Default)]
pub struct Txns {
    client: BTreeMap<String, Client>,
    /// (branch, method, sent-by): RFC 3261 17.2.3 matches on the Via's sent-by too.
    server: BTreeMap<(String, String, String), Server>,
    ok2xx: BTreeMap<String, Ok2xx>,
    sends: Vec<(Addr, Vec<u8>)>,
    events: Vec<TxEvent>,
    /// Requests resent (for status and tests).
    pub retransmits: u64,
}

pub const T1_MS: u64 = 500;
pub const TIMEOUT_MS: u64 = 64 * T1_MS;

fn method_of(m: &Message) -> Option<Method> {
    match m {
        Message::Request { method, .. } => Some(method.clone()),
        Message::Response { headers, .. } => headers.cseq().ok().map(|c| c.method),
    }
}

/// The top Via's sent-by (`host:port`) as written by the client, which a response
/// carries unchanged (a proxy's `received` is not part of it).
fn sent_by(m: &Message, from: Addr) -> String {
    match m.headers().top_via() {
        Ok(v) => format!("{}:{}", v.host, v.port.unwrap_or(5060)),
        Err(_) => from.to_string(),
    }
}

/// The top Via's branch.
pub fn branch_of(m: &Message) -> Option<String> {
    m.headers().top_via().ok().and_then(|v| v.branch().map(String::from))
}

impl Txns {
    pub fn new() -> Txns {
        Txns::default()
    }

    /// Send request `m` (its top Via carries a fresh branch) to `to` and keep
    /// resending it until a response arrives. ACK is sent once (no transaction).
    pub fn request(&mut self, owner: u64, to: Addr, m: &Message, now: u64) {
        let bytes = m.to_bytes();
        self.sends.push((to, bytes.clone()));
        let method = method_of(m).unwrap_or(Method::Other("?".into()));
        if method == Method::Ack {
            return;
        }
        let Some(branch) = branch_of(m) else { return };
        self.client.insert(branch, Client { owner, method, to, bytes, next_ms: now + T1_MS, interval: T1_MS, deadline: now + TIMEOUT_MS, provisional: false });
    }

    /// A response arrived: stops the matching client transaction's resends.
    /// Returns the owner, or None when it matches nothing (a stray or a
    /// retransmitted 2xx after the transaction ended, which the caller may still want).
    pub fn on_response(&mut self, m: &Message) -> Option<u64> {
        let code = m.code()?;
        let branch = branch_of(m)?;
        let c = self.client.get_mut(&branch)?;
        let owner = c.owner;
        if code >= 200 {
            self.client.remove(&branch);
        } else if c.method == Method::Invite {
            c.provisional = true;
        }
        Some(owner)
    }

    /// Forget a client transaction (e.g. the owner gave up).
    pub fn cancel(&mut self, branch: &str) {
        self.client.remove(branch);
    }

    /// A request arrived. False when it is a retransmission (its last response was resent).
    pub fn on_request(&mut self, from: Addr, m: &Message, now: u64) -> bool {
        let Some(method) = method_of(m) else { return false };
        if method == Method::Ack {
            // an ACK ends 2xx resending for that dialog
            if let Ok(cid) = m.headers().call_id() {
                self.ok2xx.remove(cid);
            }
            return true;
        }
        let Some(branch) = branch_of(m) else { return true };
        let key = (branch, method.as_str().to_string(), sent_by(m, from));
        if let Some(s) = self.server.get(&key) {
            if let Some((to, b)) = s.last.clone() {
                self.sends.push((to, b));
            }
            return false;
        }
        self.server.insert(key, Server { last: None, at_ms: now });
        true
    }

    /// Send response `m` (to the request whose branch/method it carries) to `to`.
    pub fn respond(&mut self, to: Addr, m: &Message, now: u64) {
        let bytes = m.to_bytes();
        self.sends.push((to, bytes.clone()));
        if let (Some(branch), Some(method)) = (branch_of(m), method_of(m)) {
            let key = (branch, method.as_str().to_string(), sent_by(m, to));
            if let Some(s) = self.server.get_mut(&key) {
                s.last = Some((to, bytes.clone()));
            }
            if method == Method::Invite && m.code().is_some_and(|c| (200..300).contains(&c)) {
                if let Ok(cid) = m.headers().call_id() {
                    self.ok2xx.insert(cid.to_string(), Ok2xx { to, bytes, next_ms: now + T1_MS, interval: T1_MS, deadline: now + TIMEOUT_MS });
                }
            }
        }
    }

    /// Resend what is due; expire old state.
    pub fn tick(&mut self, now: u64) {
        let mut dead = Vec::new();
        for (b, c) in self.client.iter_mut() {
            if now >= c.deadline {
                dead.push(b.clone());
                continue;
            }
            if !c.provisional && now >= c.next_ms {
                self.sends.push((c.to, c.bytes.clone()));
                self.retransmits += 1;
                c.interval = (c.interval * 2).min(4000);
                c.next_ms = now + c.interval;
            }
        }
        for b in dead {
            let c = self.client.remove(&b).expect("listed");
            // an INVITE that got a provisional response waits for its final one (the dialog's own timers end it)
            if !(c.provisional && c.method == Method::Invite) {
                self.events.push(TxEvent::Timeout { owner: c.owner, branch: b, method: c.method });
            }
        }
        self.server.retain(|_, s| now.saturating_sub(s.at_ms) < TIMEOUT_MS);
        let mut gone = Vec::new();
        for (cid, o) in self.ok2xx.iter_mut() {
            if now >= o.deadline {
                gone.push(cid.clone());
            } else if now >= o.next_ms {
                self.sends.push((o.to, o.bytes.clone()));
                o.interval = (o.interval * 2).min(4000);
                o.next_ms = now + o.interval;
            }
        }
        for g in gone {
            self.ok2xx.remove(&g);
        }
    }

    pub fn take_sends(&mut self) -> Vec<(Addr, Vec<u8>)> {
        std::mem::take(&mut self.sends)
    }

    pub fn take_events(&mut self) -> Vec<TxEvent> {
        std::mem::take(&mut self.events)
    }

    /// Client transactions in flight.
    pub fn pending(&self) -> usize {
        self.client.len()
    }
}

/// A small deterministic generator for tags, branches and Call-IDs (xorshift64*).
#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sip::{new_branch, Uri, Via};

    fn req(branch: &str, method: Method) -> Message {
        let mut m = Message::request(method.clone(), Uri::sip(Some("+12145550142"), "cell-red.ecm", None));
        let h = m.headers_mut();
        h.push("Via", Via::new("UDP", "10.250.0.2", Some(5060), branch).to_string());
        h.push("From", "<sip:+12145550100@cell-red.ecm>;tag=a");
        h.push("To", "<sip:+12145550142@cell-red.ecm>");
        h.push("Call-ID", "c1");
        h.push("CSeq", format!("1 {}", method.as_str()));
        m
    }

    #[test]
    fn requests_are_resent_until_answered_then_time_out() {
        let mut t = Txns::new();
        let to = Addr::new([10, 0, 0, 1], 5060);
        let m = req(&new_branch(1), Method::Message);
        t.request(7, to, &m, 0);
        assert_eq!(t.take_sends().len(), 1);
        t.tick(499);
        assert!(t.take_sends().is_empty());
        t.tick(500);
        assert_eq!(t.take_sends().len(), 1);
        t.tick(1500);
        assert_eq!(t.take_sends().len(), 1, "1 s later");
        let ok = m.response_to(200, Some("b")).unwrap();
        assert_eq!(t.on_response(&ok), Some(7));
        t.tick(10_000);
        assert!(t.take_sends().is_empty());
        assert_eq!(t.pending(), 0);
        // no answer at all: a timeout event
        let m2 = req(&new_branch(2), Method::Message);
        t.request(8, to, &m2, 0);
        for ms in (0..40_000).step_by(100) {
            t.tick(ms);
        }
        assert!(matches!(t.take_events().as_slice(), [TxEvent::Timeout { owner: 8, .. }]));
        assert!(t.retransmits >= 6);
    }

    #[test]
    fn an_invite_stops_at_a_provisional_and_duplicates_get_the_last_response() {
        let mut t = Txns::new();
        let to = Addr::new([10, 0, 0, 1], 5060);
        let inv = req(&new_branch(3), Method::Invite);
        t.request(1, to, &inv, 0);
        t.take_sends();
        assert_eq!(t.on_response(&inv.response_to(180, Some("x")).unwrap()), Some(1));
        t.tick(5000);
        assert!(t.take_sends().is_empty(), "no resends after 180");
        // server side
        let mut s = Txns::new();
        let from = Addr::new([10, 250, 0, 2], 5060);
        assert!(s.on_request(from, &inv, 0));
        let ok = inv.response_to(200, Some("y")).unwrap();
        s.respond(from, &ok, 0);
        s.take_sends();
        assert!(!s.on_request(from, &inv, 100), "retransmission");
        assert_eq!(s.take_sends().len(), 1, "the 200 again");
        // the 2xx is resent until the ACK
        s.tick(600);
        assert_eq!(s.take_sends().len(), 1);
        let mut ack = req(&new_branch(4), Method::Ack);
        ack.headers_mut().set("CSeq", "1 ACK");
        s.on_request(from, &ack, 700);
        s.tick(5000);
        assert!(s.take_sends().is_empty());
    }

    #[test]
    fn addr_parse() {
        assert_eq!(Addr::parse("10.0.0.1:5062", 5060), Some(Addr::new([10, 0, 0, 1], 5062)));
        assert_eq!(Addr::parse("10.0.0.1", 5060), Some(Addr::new([10, 0, 0, 1], 5060)));
        assert_eq!(Addr::parse("10.0.0", 5060), None);
        assert_eq!(Addr::new([1, 2, 3, 4], 5).to_string(), "1.2.3.4:5");
    }
}
