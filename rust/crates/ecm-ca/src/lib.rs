//! The in-game certificate authority (docs/TLS.md).
//!
//! An authority is a computer running `cad`. Another computer gets a
//! certificate for its name by proving it answers at that name:
//!
//! 1. `cert` makes a key and sends an **order**: `POST /order` with the
//!    public key and the names.
//! 2. `cad` answers with a **token**. The computer must serve
//!    `<token>.<public key hex>` at
//!    `http://<name>/.well-known/ecm-challenge/<token>` (port 80) for every name.
//! 3. `cert` sends `POST /finalize` with the order and a signature made with
//!    the key (so the key is really its own). `cad` fetches the challenge from
//!    every name and, if all answer, signs a certificate.
//!
//! Requests and answers are lines of `word value`, over plain HTTP: nothing
//! in them is secret, and the certificate that comes back is checked against
//! the authority the computer already trusts.
//!
//! An authority only signs for names that lead to addresses inside the game
//! (see `ecm_tls::is_public`): it has no say about the real internet.

use ecm_ssh_crypto as c;
use ecm_tls::x509::{self, SubjectName};

/// The port `cad` listens on.
pub const PORT: u16 = 8555;
pub const CHALLENGE_PREFIX: &str = "/.well-known/ecm-challenge/";
/// Certificates last this long.
pub const LIFETIME_DAYS: u64 = 90;
/// `cert renew` replaces a certificate with fewer days left than this.
pub const RENEW_DAYS_LEFT: u64 = 30;
/// An order must be finalized within this many seconds.
pub const ORDER_SECONDS: u64 = 600;
pub const MAX_NAMES: usize = 8;
/// A certificate is valid from this long before it was signed (clocks differ a little).
pub const BACKDATE_SECONDS: u64 = 3600;

pub fn hex(b: &[u8]) -> String {
    ecm_tls::hex(b)
}

fn key32(s: &str) -> Option<[u8; 32]> {
    ecm_tls::unhex(s)?.try_into().ok()
}

/// The lines of a message as (word, rest).
fn fields(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|l| {
        let l = l.trim();
        (!l.is_empty()).then(|| l.split_once(' ').map(|(k, v)| (k, v.trim())).unwrap_or((l, "")))
    })
}

fn field<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    fields(text).find(|(k, _)| *k == word).map(|(_, v)| v)
}

/// `POST /order`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRequest {
    pub public: [u8; 32],
    pub names: Vec<SubjectName>,
}

impl OrderRequest {
    pub fn encode(&self) -> String {
        let mut s = format!("key {}\n", hex(&self.public));
        for n in &self.names {
            s.push_str(&format!("name {n}\n"));
        }
        s
    }

    pub fn parse(text: &str) -> Result<OrderRequest, String> {
        let public = field(text, "key").and_then(key32).ok_or("the order has no key (64 hex digits)")?;
        let mut names = Vec::new();
        for (_, v) in fields(text).filter(|(k, _)| *k == "name") {
            let n = SubjectName::parse(v).ok_or_else(|| format!("'{v}' is not a host name or an address"))?;
            if !names.contains(&n) {
                names.push(n);
            }
        }
        if names.is_empty() {
            return Err("the order names nothing".into());
        }
        if names.len() > MAX_NAMES {
            return Err(format!("more than {MAX_NAMES} names in one certificate"));
        }
        Ok(OrderRequest { public, names })
    }
}

/// The answer to an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order {
    pub id: String,
    pub token: String,
}

impl Order {
    pub fn encode(&self) -> String {
        format!("order {}\ntoken {}\n", self.id, self.token)
    }

    pub fn parse(text: &str) -> Result<Order, String> {
        let id = field(text, "order").filter(|s| is_token(s)).ok_or("bad answer from the authority (no order)")?;
        let token = field(text, "token").filter(|s| is_token(s)).ok_or("bad answer from the authority (no token)")?;
        Ok(Order { id: id.into(), token: token.into() })
    }

    /// The path the challenge is served at.
    pub fn path(&self) -> String {
        format!("{CHALLENGE_PREFIX}{}", self.token)
    }
}

/// What the challenge file holds: the token and the key it was ordered for.
pub fn challenge_content(token: &str, public: &[u8; 32]) -> String {
    format!("{token}.{}", hex(public))
}

/// Tokens and order ids: 8..64 lower-case hex digits (safe as a file name).
pub fn is_token(s: &str) -> bool {
    (8..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn proof_message(order: &str) -> Vec<u8> {
    format!("ecm-cert-order-v1\n{order}\n").into_bytes()
}

/// `POST /finalize`: the order and the key's signature over it.
pub fn finalize_request(order: &Order, seed: &[u8; 32]) -> String {
    let sig = c::ed25519_sign(&c::ed25519_private_key_from_bytes(seed), &proof_message(&order.id));
    format!("order {}\nproof {}\n", order.id, hex(&sig))
}

/// (order id, proof) of a finalize request.
pub fn parse_finalize(text: &str) -> Result<(String, [u8; 64]), String> {
    let id = field(text, "order").filter(|s| is_token(s)).ok_or("no order")?;
    let proof: [u8; 64] = field(text, "proof").and_then(ecm_tls::unhex).and_then(|v| v.try_into().ok()).ok_or("no proof")?;
    Ok((id.to_string(), proof))
}

/// Why an authority will not sign for a name.
pub fn refuse_name(name: &SubjectName, resolved: Option<[u8; 4]>) -> Option<String> {
    match (name, resolved) {
        (SubjectName::Ip(ip), _) if ecm_tls::is_public(*ip) => Some(format!("{name} is an address on the real internet; an in-game authority only signs for the game's networks")),
        (SubjectName::Ip(_), _) => None,
        (SubjectName::Dns(_), None) => Some(format!("{name} has no address (the authority could not resolve it)")),
        (SubjectName::Dns(_), Some(ip)) if ecm_tls::is_public(ip) => Some(format!("{name} is on the real internet; an in-game authority only signs for the game's networks")),
        (SubjectName::Dns(d), _) if d == "localhost" => Some("localhost is every computer; ask for this computer's own name".into()),
        _ => None,
    }
}

/// An open order on the authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub order: Order,
    pub request: OrderRequest,
    pub opened: u64,
}

/// A certificate authority: its key and name, and the orders in progress.
pub struct Authority {
    seed: [u8; 32],
    pub common_name: String,
    pub pending: Vec<Pending>,
}

/// What an authority did with a finalize request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issued {
    /// The certificate, PEM.
    pub pem: String,
    pub names: Vec<SubjectName>,
    pub not_after: u64,
    pub serial: String,
}

impl Authority {
    pub fn new(seed: [u8; 32], common_name: &str) -> Authority {
        Authority { seed, common_name: common_name.to_string(), pending: Vec::new() }
    }

    pub fn public(&self) -> [u8; 32] {
        x509::public_of(&self.seed)
    }

    /// The line another computer saves in `/etc/ssl/certs/<name>.anchor` to trust this authority.
    pub fn anchor_line(&self) -> String {
        ecm_tls::Authority::anchor_line(&self.public(), &self.common_name)
    }

    /// The authority's own (self-signed) certificate, for tools that want one.
    pub fn root_pem(&self) -> String {
        let mut serial = [0u8; 16];
        serial.copy_from_slice(&self.public()[..16]);
        // Fixed dates: the same key always gives the same certificate.
        let der = x509::root_certificate(&self.seed, &self.common_name, &serial, x509::unix_of(2020, 1, 1, 0, 0, 0), x509::unix_of(2120, 1, 1, 0, 0, 0));
        x509::pem(x509::CERTIFICATE, &der)
    }

    /// Open an order. `random` is 32 fresh random bytes (the id and the token).
    pub fn open(&mut self, request: OrderRequest, now: u64, random: &[u8; 32]) -> Order {
        self.pending.retain(|p| now.saturating_sub(p.opened) < ORDER_SECONDS);
        // An authority is small: the oldest orders make room.
        while self.pending.len() >= 64 {
            self.pending.remove(0);
        }
        let order = Order { id: hex(&random[..12]), token: hex(&random[12..]) };
        self.pending.push(Pending { order: order.clone(), request, opened: now });
        order
    }

    /// The order a finalize request is for, if it is open and the proof is the key's.
    pub fn claim(&mut self, finalize: &str, now: u64) -> Result<Pending, String> {
        let (id, proof) = parse_finalize(finalize)?;
        let i = self.pending.iter().position(|p| p.order.id == id).ok_or("no such order (they last ten minutes)")?;
        let p = self.pending.remove(i);
        if now.saturating_sub(p.opened) >= ORDER_SECONDS {
            return Err("the order is too old; start again".into());
        }
        let key = c::ed25519_public_key_from_bytes(&p.request.public).map_err(|_| "the order's key is not an ed25519 key")?;
        if !c::ed25519_verify(&key, &proof_message(&id), &proof) {
            return Err("the proof was not made with the order's key".into());
        }
        Ok(p)
    }

    /// Sign the certificate for a claimed order whose names all answered the challenge.
    pub fn issue(&self, p: &Pending, now: u64, random: &[u8; 16]) -> Issued {
        let not_after = now + LIFETIME_DAYS * 86_400;
        let der = x509::leaf_certificate(&self.seed, &self.common_name, &p.request.public, &p.request.names, random, now.saturating_sub(BACKDATE_SECONDS), not_after);
        Issued { pem: x509::pem(x509::CERTIFICATE, &der), names: p.request.names.clone(), not_after, serial: hex(random) }
    }
}

/// What `cert` remembers about a certificate so `cert renew` can ask again: `/etc/ssl/server.req`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Renewal {
    pub names: Vec<String>,
    /// `host:port` of the authority.
    pub authority: String,
    /// The folder a web server serves, or empty when `cert` answers the challenge itself.
    pub webroot: String,
}

impl Renewal {
    pub fn encode(&self) -> String {
        let mut s = format!("authority {}\n", self.authority);
        if !self.webroot.is_empty() {
            s.push_str(&format!("webroot {}\n", self.webroot));
        }
        for n in &self.names {
            s.push_str(&format!("name {n}\n"));
        }
        s
    }

    pub fn parse(text: &str) -> Option<Renewal> {
        let r = Renewal {
            names: fields(text).filter(|(k, _)| *k == "name").map(|(_, v)| v.to_string()).collect(),
            authority: field(text, "authority")?.to_string(),
            webroot: field(text, "webroot").unwrap_or("").to_string(),
        };
        (!r.names.is_empty()).then_some(r)
    }
}

/// Whole days left before `not_after` (0 when past).
pub fn days_left(not_after: u64, now: u64) -> u64 {
    not_after.saturating_sub(now) / 86_400
}

/// `host[:port]` with the authority's port as the default.
pub fn authority_address(s: &str) -> Option<(String, u16)> {
    let s = s.trim().trim_start_matches("http://").trim_end_matches('/');
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (s, PORT),
    };
    (!host.is_empty() && host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-')).then(|| (host.to_ascii_lowercase(), port))
}

/// A file name for an authority's anchor: its name in lower case, other characters as `-`.
pub fn anchor_file(common_name: &str) -> String {
    let mut s: String = common_name.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-');
    format!("{}.anchor", if s.is_empty() { "authority" } else { s })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_tls::{Check, Identity, Trust};
    use std::io::{Read, Write};

    const NOW: u64 = 1_795_000_000;

    fn names(n: &[&str]) -> Vec<SubjectName> {
        n.iter().map(|s| SubjectName::parse(s).unwrap()).collect()
    }

    #[test]
    fn messages_round_trip() {
        let req = OrderRequest { public: [5; 32], names: names(&["shop.ecm", "100.65.0.21"]) };
        assert_eq!(OrderRequest::parse(&req.encode()).unwrap(), req);
        assert!(OrderRequest::parse("name shop.ecm\n").unwrap_err().contains("no key"));
        assert!(OrderRequest::parse(&format!("key {}\n", hex(&[5; 32]))).unwrap_err().contains("names nothing"));
        assert!(OrderRequest::parse(&format!("key {}\nname a b\n", hex(&[5; 32]))).unwrap_err().contains("not a host name"));
        let many: String = (0..9).map(|i| format!("name h{i}.ecm\n")).collect();
        assert!(OrderRequest::parse(&format!("key {}\n{many}", hex(&[5; 32]))).unwrap_err().contains("more than 8"));
        let o = Order { id: "00112233445566778899aabb".into(), token: "ffeeddccbbaa99887766554433221100ffeeddcc".into() };
        assert_eq!(Order::parse(&o.encode()).unwrap(), o);
        assert_eq!(o.path(), "/.well-known/ecm-challenge/ffeeddccbbaa99887766554433221100ffeeddcc");
        assert!(Order::parse("order ../x\ntoken abc\n").is_err());
        assert!(is_token("0123abcd") && !is_token("0123ABCD") && !is_token("abc") && !is_token("../../etc"));
        let r = Renewal { names: vec!["shop.ecm".into()], authority: "ca.v1.ring:8555".into(), webroot: "/srv/www".into() };
        assert_eq!(Renewal::parse(&r.encode()), Some(r));
        assert_eq!(Renewal::parse("authority x:1\n"), None);
    }

    #[test]
    fn which_names_an_authority_signs() {
        let n = |s: &str| SubjectName::parse(s).unwrap();
        assert_eq!(refuse_name(&n("shop.ecm"), Some([100, 65, 0, 21])), None);
        assert_eq!(refuse_name(&n("10.0.7.1"), None), None);
        assert!(refuse_name(&n("shop.ecm"), None).unwrap().contains("no address"));
        assert!(refuse_name(&n("repo.evanwitulski.com"), Some([203, 0, 113, 9])).unwrap().contains("real internet"));
        assert!(refuse_name(&n("8.8.8.8"), None).unwrap().contains("real internet"));
        assert!(refuse_name(&n("localhost"), Some([127, 0, 0, 1])).unwrap().contains("every computer"));
    }

    #[test]
    fn order_challenge_issue_and_use() {
        let mut ca = Authority::new([7; 32], "Test Village CA");
        let seed = [3u8; 32];
        let req = OrderRequest { public: x509::public_of(&seed), names: names(&["shop.ecm", "10.0.7.1"]) };
        let order = ca.open(req.clone(), NOW, &[0xAB; 32]);
        assert_eq!(challenge_content(&order.token, &req.public), format!("{}.{}", order.token, hex(&req.public)));

        // A proof made with another key is refused, and uses the order up.
        let mut ca2 = Authority::new([7; 32], "Test Village CA");
        let o2 = ca2.open(req.clone(), NOW, &[0xCD; 32]);
        assert!(ca2.claim(&finalize_request(&o2, &[4u8; 32]), NOW + 5).unwrap_err().contains("not made with the order's key"));
        assert!(ca2.claim(&finalize_request(&o2, &seed), NOW + 5).unwrap_err().contains("no such order"));
        // Too late.
        let o3 = ca2.open(req.clone(), NOW, &[0xEF; 32]);
        assert!(ca2.claim(&finalize_request(&o3, &seed), NOW + ORDER_SECONDS).is_err());

        let pending = ca.claim(&finalize_request(&order, &seed), NOW + 5).unwrap();
        assert_eq!(pending.request, req);
        // The real clock, so the handshake below accepts the dates.
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let issued = ca.issue(&pending, now, &[9; 16]);
        assert_eq!(issued.not_after, now + 90 * 86_400);
        assert_eq!(days_left(issued.not_after, now), 90);
        assert_eq!(days_left(issued.not_after, issued.not_after + 1), 0);
        let der = x509::pem_blocks(&issued.pem, x509::CERTIFICATE).remove(0);
        let info = x509::describe(&der).unwrap();
        assert_eq!((info.issuer.as_str(), info.names.clone(), info.not_before), ("Test Village CA", req.names.clone(), now - BACKDATE_SECONDS));

        // A computer that saved the anchor line trusts a server using the certificate.
        let anchor = ecm_tls::Authority::parse_anchor(&ca.anchor_line(), "village.anchor").unwrap();
        let id = Identity::from_pem(&issued.pem, &x509::pem(x509::PRIVATE_KEY, &x509::pkcs8(&seed))).unwrap();
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        let server = std::thread::spawn(move || {
            let mut t = id.accept(b).unwrap();
            t.write_all(b"hello").unwrap();
            t.close();
        });
        let mut t = Trust::with(vec![anchor]).connect(a, "shop.ecm", [10, 0, 7, 1], Check::Full).unwrap();
        let mut s = String::new();
        t.read_to_string(&mut s).unwrap();
        assert_eq!(s, "hello");
        server.join().unwrap();

        // The root certificate is the same every time and names the authority.
        assert_eq!(ca.root_pem(), Authority::new([7; 32], "Test Village CA").root_pem());
        let root = x509::describe(&x509::pem_blocks(&ca.root_pem(), x509::CERTIFICATE)[0]).unwrap();
        assert!(root.is_ca && root.subject == "Test Village CA");
    }

    #[test]
    fn addresses_and_file_names() {
        assert_eq!(authority_address("ca.v1.ring"), Some(("ca.v1.ring".into(), PORT)));
        assert_eq!(authority_address("http://100.65.0.58:9000/"), Some(("100.65.0.58".into(), 9000)));
        assert_eq!(authority_address("a b"), None);
        assert_eq!(authority_address(":8555"), None);
        assert_eq!(anchor_file("Tech Village CA"), "tech-village-ca.anchor");
        assert_eq!(anchor_file("Evan's  CA!"), "evan-s-ca.anchor");
        assert_eq!(anchor_file("!!"), "authority.anchor");
    }
}
