//! Authoritative answers from loaded zones: data, CNAMEs inside the zone, referrals
//! (NS + glue) at zone cuts, and NXDOMAIN / NODATA with the SOA for negative caching.

use crate::name::{is_sub, parent};
use crate::wire::{Message, RData, Record, A, ANY, CNAME, MX, NOERROR, NS, NXDOMAIN, REFUSED, SOA, SRV};
use crate::zone::Zone;

pub const MAX_CNAME: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    /// Data (after any CNAMEs in `records` first).
    Answer(Vec<Record>),
    /// The name is below a delegation: NS records of the cut plus glue addresses.
    Referral { chain: Vec<Record>, ns: Vec<Record>, glue: Vec<Record> },
    NxDomain { chain: Vec<Record>, soa: Record },
    NoData { chain: Vec<Record>, soa: Record },
}

impl Zone {
    /// Answer `qname`/`qtype` from this zone (`qname` must be at or below the origin).
    pub fn lookup(&self, qname: &str, qtype: u16) -> Lookup {
        let mut chain: Vec<Record> = Vec::new();
        let mut name = qname.to_string();
        loop {
            if let Some(cut) = self.cut_above(&name, qtype) {
                let ns: Vec<Record> = self.get(&cut, NS).into_iter().cloned().collect();
                let glue = self.glue(&ns);
                return Lookup::Referral { chain, ns, glue };
            }
            let here: Vec<&Record> = self.records.iter().filter(|r| r.name == name).collect();
            if qtype != CNAME && qtype != ANY {
                if let Some(c) = here.iter().find(|r| r.rtype == CNAME) {
                    chain.push((*c).clone());
                    let target = c.data.target().unwrap_or("").to_string();
                    if chain.len() >= MAX_CNAME || !is_sub(&target, &self.origin) {
                        return Lookup::Answer(chain);
                    }
                    name = target;
                    continue;
                }
            }
            let hits: Vec<Record> =
                here.iter().filter(|r| qtype == ANY || r.rtype == qtype).map(|r| (*r).clone()).collect();
            if !hits.is_empty() {
                chain.extend(hits);
                return Lookup::Answer(chain);
            }
            let soa = self.negative_soa();
            let exists = !here.is_empty() || self.records.iter().any(|r| is_sub(&r.name, &name));
            return if exists { Lookup::NoData { chain, soa } } else { Lookup::NxDomain { chain, soa } };
        }
    }

    /// The SOA as sent with a negative answer: its TTL capped by the minimum (RFC 2308).
    pub fn negative_soa(&self) -> Record {
        let mut soa = self.soa_record().clone();
        soa.ttl = soa.ttl.min(self.soa().minimum);
        soa
    }

    /// The highest delegation point strictly below the origin on the way to `name`
    /// (`name` itself counts, except for its own NS query at the apex).
    fn cut_above(&self, name: &str, _qtype: u16) -> Option<String> {
        let mut path = Vec::new();
        let mut n = name;
        while n != self.origin {
            path.push(n);
            n = parent(n)?;
        }
        path.into_iter().rev().find(|c| self.records.iter().any(|r| r.name == *c && r.rtype == NS)).map(String::from)
    }

    /// Addresses in this zone for the targets of `ns` (glue and in-zone hosts).
    fn glue(&self, ns: &[Record]) -> Vec<Record> {
        let mut out = Vec::new();
        for r in ns {
            if let Some(t) = r.data.target() {
                for a in self.get(t, A) {
                    if !out.contains(a) {
                        out.push(a.clone());
                    }
                }
            }
        }
        out
    }
}

/// The zones one server is authoritative for.
#[derive(Clone, Debug, Default)]
pub struct Authority {
    pub zones: Vec<Zone>,
}

impl Authority {
    /// The closest enclosing zone for `name`.
    pub fn find(&self, name: &str) -> Option<&Zone> {
        self.zones.iter().filter(|z| is_sub(name, &z.origin)).max_by_key(|z| z.origin.len() + !z.origin.is_empty() as usize)
    }

    pub fn find_mut(&mut self, origin: &str) -> Option<&mut Zone> {
        self.zones.iter_mut().find(|z| z.origin == origin)
    }

    /// Replace (or add) a zone.
    pub fn put(&mut self, zone: Zone) {
        match self.zones.iter_mut().find(|z| z.origin == zone.origin) {
            Some(z) => *z = zone,
            None => self.zones.push(zone),
        }
    }

    /// Authoritative response to a standard query, or `None` when no zone covers it.
    pub fn respond(&self, q: &Message) -> Option<Message> {
        let question = q.question()?;
        let zone = self.find(&question.name)?;
        let mut r = Message::response_to(q, NOERROR);
        match zone.lookup(&question.name, question.qtype) {
            Lookup::Answer(records) => {
                r.header.aa = true;
                r.additional = additional_for(self, &records);
                r.answers = records;
            }
            Lookup::Referral { chain, ns, glue } => {
                r.header.aa = !chain.is_empty();
                r.answers = chain;
                r.authority = ns;
                r.additional = glue;
            }
            Lookup::NxDomain { chain, soa } => {
                r.header.aa = true;
                r.header.rcode = NXDOMAIN;
                r.answers = chain;
                r.authority = vec![soa];
            }
            Lookup::NoData { chain, soa } => {
                r.header.aa = true;
                r.answers = chain;
                r.authority = vec![soa];
            }
        }
        Some(r)
    }
}

/// Addresses of NS, MX and SRV targets we have, for the additional section.
fn additional_for(auth: &Authority, records: &[Record]) -> Vec<Record> {
    let mut out: Vec<Record> = Vec::new();
    for r in records {
        if !matches!(r.rtype, NS | MX | SRV) {
            continue;
        }
        let Some(t) = r.data.target() else { continue };
        if let Some(z) = auth.find(t) {
            for a in z.get(t, A) {
                if !out.contains(a) {
                    out.push(a.clone());
                }
            }
        }
    }
    out
}

/// The REFUSED response for a query no zone covers (and no recursion).
pub fn refused(q: &Message) -> Message {
    Message::response_to(q, REFUSED)
}

/// Is this record set a delegation (NS records not at the apex of `zone`)?
pub fn is_delegation(r: &Record, zone: &str) -> bool {
    r.rtype == NS && r.name != zone
}

/// SOA serial of a response's answer, if it carries one.
pub fn answer_serial(m: &Message) -> Option<u32> {
    m.answers.iter().find_map(|r| match &r.data {
        RData::Soa(s) if r.rtype == SOA => Some(s.serial),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::*;

    pub const TEST_ZONE: &str = "$ORIGIN test.\n$TTL 300\n@ SOA ns1 admin 1 60 30 600 30\n@ NS ns1\nns1 A 10.0.0.2\nlab NS ns1.lab\nns1.lab A 10.0.0.3\nother NS ns.elsewhere.\nalias CNAME lab-www.lab\nhere CNAME x\nx A 10.0.0.7\nempty.deep TXT \"x\"\n";

    fn zone() -> Zone {
        Zone::parse(TEST_ZONE, "test").unwrap()
    }

    #[test]
    fn answers_and_cnames() {
        let z = zone();
        assert_eq!(z.lookup("x.test", A), Lookup::Answer(z.get("x.test", A).into_iter().cloned().collect()));
        match z.lookup("here.test", A) {
            Lookup::Answer(r) => {
                assert_eq!(r.len(), 2);
                assert_eq!(r[0].rtype, CNAME);
                assert_eq!(r[1].data, RData::A([10, 0, 0, 7]));
            }
            o => panic!("{o:?}"),
        }
        // CNAME query returns the CNAME itself
        assert!(matches!(z.lookup("here.test", CNAME), Lookup::Answer(r) if r.len() == 1));
    }

    #[test]
    fn referral_with_glue() {
        let z = zone();
        match z.lookup("www.lab.test", A) {
            Lookup::Referral { ns, glue, .. } => {
                assert_eq!(ns[0].data, RData::Ns("ns1.lab.test".into()));
                assert_eq!(glue[0].data, RData::A([10, 0, 0, 3]));
            }
            o => panic!("{o:?}"),
        }
        // the cut itself is a referral too
        assert!(matches!(z.lookup("lab.test", NS), Lookup::Referral { .. }));
        // out-of-zone NS: no glue
        assert!(matches!(z.lookup("a.other.test", A), Lookup::Referral { glue, .. } if glue.is_empty()));
        // CNAME into a delegated name: the chain, then the referral
        assert!(matches!(z.lookup("alias.test", A), Lookup::Referral { chain, .. } if chain.len() == 1));
    }

    #[test]
    fn negatives() {
        let z = zone();
        assert!(matches!(z.lookup("nosuch.test", A), Lookup::NxDomain { soa, .. } if soa.ttl == 30));
        assert!(matches!(z.lookup("x.test", MX), Lookup::NoData { .. }));
        // empty non-terminal
        assert!(matches!(z.lookup("deep.test", A), Lookup::NoData { .. }));
    }

    #[test]
    fn naptr_and_srv_answers_with_srv_target_addresses() {
        let mut a = Authority::default();
        a.put(Zone::parse(crate::zone::tests::ENUM_ZONE, "5.5.5.4.1.2.1.e164.arpa").unwrap());
        a.put(Zone::parse("$TTL 60\n@ SOA ns1 a 1 1 1 1 1\n@ NS ns1\nns1 A 10.0.0.3\nsip A 10.0.0.5\n", "carrier.ecm").unwrap());
        let r = a.respond(&Message::query(1, "2.4.1.0.5.5.5.4.1.2.1.e164.arpa", NAPTR, true)).unwrap();
        assert!(r.header.aa && r.answers.len() == 2 && r.answers.iter().all(|x| x.rtype == NAPTR));
        assert!(r.additional.is_empty(), "NAPTR has no address to add");
        let uri = crate::enum_dns::enum_sip_uri(crate::enum_dns::naptrs(&r.answers), "+12145550142");
        assert_eq!(uri.as_deref(), Some("sip:+12145550142@carrier.ecm"));
        let r = a.respond(&Message::query(2, "_sip._udp.5.5.5.4.1.2.1.e164.arpa", SRV, true)).unwrap();
        assert_eq!(r.answers.len(), 2);
        // the SRV target's address from the other zone; the apex (no A) adds nothing
        assert_eq!(r.additional, vec![Record::new("sip.carrier.ecm", 60, RData::A([10, 0, 0, 5]))]);
        // the right name, another type: NODATA; NAPTR at a name without any: NXDOMAIN
        assert!(a.respond(&Message::query(3, "2.4.1.0.5.5.5.4.1.2.1.e164.arpa", SRV, true)).unwrap().answers.is_empty());
        let r = a.respond(&Message::query(4, "9.9.9.9.5.5.5.4.1.2.1.e164.arpa", NAPTR, true)).unwrap();
        assert_eq!(r.header.rcode, NXDOMAIN);
    }

    #[test]
    fn server_picks_the_closest_zone() {
        let mut a = Authority::default();
        a.put(zone());
        a.put(Zone::parse("$TTL 60\n@ SOA ns1 a 1 1 1 1 1\n@ NS ns1\nns1 A 10.0.0.3\nwww A 10.0.0.4\n", "lab.test").unwrap());
        a.put(Zone::parse("$TTL 60\n@ SOA a b 1 1 1 1 1\n@ NS a.root-servers.test.\n", "").unwrap());
        let q = Message::query(5, "www.lab.test", A, true);
        let r = a.respond(&q).unwrap();
        assert!(r.header.aa && r.header.qr && r.header.rd && r.answers[0].data == RData::A([10, 0, 0, 4]));
        let r = a.respond(&Message::query(6, "nope.example", A, false)).unwrap();
        assert_eq!(r.header.rcode, NXDOMAIN);
        assert_eq!(r.authority[0].name, "");
        let b = Authority::default();
        assert!(b.respond(&q).is_none());
    }
}
