//! The resolver's cache: record sets and negative answers (NXDOMAIN for a name, NODATA
//! for a name and type) kept until their TTL runs out. Times are milliseconds.

use crate::name::fqdn;
use crate::wire::{type_name, Record};
use std::collections::HashMap;

/// Entries kept at most; the ones expiring first are dropped beyond it.
pub const MAX_ENTRIES: usize = 4096;
/// Negative answers without an SOA are kept this long (seconds).
pub const DEFAULT_NEGATIVE_TTL: u32 = 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The records, TTLs counted down to what is left.
    Records(Vec<Record>),
    /// The type does not exist at the name (SOA with its remaining TTL, if known).
    NoData(Option<Record>),
    /// The name does not exist.
    NxDomain(Option<Record>),
}

#[derive(Clone, Debug)]
enum Entry {
    Records(Vec<Record>),
    NoData(Option<Record>),
    NxDomain(Option<Record>),
}

#[derive(Clone, Debug)]
struct Slot {
    entry: Entry,
    stored: i64,
    expires: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Cache {
    map: HashMap<(String, u16), Slot>,
}

/// Key type used for "the whole name does not exist".
const NX: u16 = 0;

fn left(rec: &Record, stored: i64, now: i64) -> Record {
    let mut r = rec.clone();
    let gone = ((now - stored).max(0) / 1000) as u32;
    r.ttl = r.ttl.saturating_sub(gone);
    r
}

impl Cache {
    pub fn new() -> Cache {
        Cache::default()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn flush(&mut self) {
        self.map.clear();
    }

    /// Drop one name (all types).
    pub fn flush_name(&mut self, name: &str) {
        self.map.retain(|(n, _), _| n != name);
    }

    fn insert(&mut self, key: (String, u16), entry: Entry, ttl: u32, now: i64) {
        if ttl == 0 {
            return;
        }
        if self.map.len() >= MAX_ENTRIES && !self.map.contains_key(&key) {
            self.expire(now);
            if self.map.len() >= MAX_ENTRIES {
                if let Some(k) = self.map.iter().min_by_key(|(_, s)| s.expires).map(|(k, _)| k.clone()) {
                    self.map.remove(&k);
                }
            }
        }
        self.map.insert(key, Slot { entry, stored: now, expires: now + ttl as i64 * 1000 });
    }

    /// Store a record set (all of one name and type); its TTL is the lowest of them.
    pub fn put(&mut self, records: Vec<Record>, now: i64) {
        let Some(first) = records.first() else { return };
        let key = (first.name.clone(), first.rtype);
        let ttl = records.iter().map(|r| r.ttl).min().unwrap_or(0);
        self.insert(key, Entry::Records(records), ttl, now);
    }

    /// Store every record set of `records`, grouped by name and type.
    pub fn put_all(&mut self, records: &[Record], now: i64) {
        let mut groups: Vec<Vec<Record>> = Vec::new();
        for r in records {
            match groups.iter_mut().find(|g| g[0].name == r.name && g[0].rtype == r.rtype) {
                Some(g) => {
                    if !g.contains(r) {
                        g.push(r.clone())
                    }
                }
                None => groups.push(vec![r.clone()]),
            }
        }
        for g in groups {
            self.put(g, now);
        }
    }

    fn negative_ttl(soa: &Option<Record>) -> u32 {
        match soa {
            Some(r) => r.soa().map_or(r.ttl, |s| r.ttl.min(s.minimum)),
            None => DEFAULT_NEGATIVE_TTL,
        }
    }

    pub fn put_nodata(&mut self, name: &str, rtype: u16, soa: Option<Record>, now: i64) {
        let ttl = Self::negative_ttl(&soa);
        self.insert((name.to_string(), rtype), Entry::NoData(soa), ttl, now);
    }

    pub fn put_nxdomain(&mut self, name: &str, soa: Option<Record>, now: i64) {
        let ttl = Self::negative_ttl(&soa);
        self.insert((name.to_string(), NX), Entry::NxDomain(soa), ttl, now);
    }

    fn live(&self, key: &(String, u16), now: i64) -> Option<&Slot> {
        self.map.get(key).filter(|s| s.expires > now)
    }

    /// What the cache knows about `name`/`rtype`.
    pub fn get(&self, name: &str, rtype: u16, now: i64) -> Option<Hit> {
        if let Some(s) = self.live(&(name.to_string(), NX), now) {
            if let Entry::NxDomain(soa) = &s.entry {
                return Some(Hit::NxDomain(soa.as_ref().map(|r| left(r, s.stored, now))));
            }
        }
        let s = self.live(&(name.to_string(), rtype), now)?;
        Some(match &s.entry {
            Entry::Records(rs) => Hit::Records(rs.iter().map(|r| left(r, s.stored, now)).collect()),
            Entry::NoData(soa) => Hit::NoData(soa.as_ref().map(|r| left(r, s.stored, now))),
            Entry::NxDomain(soa) => Hit::NxDomain(soa.as_ref().map(|r| left(r, s.stored, now))),
        })
    }

    /// Records only (no negative entries).
    pub fn records(&self, name: &str, rtype: u16, now: i64) -> Option<Vec<Record>> {
        match self.get(name, rtype, now)? {
            Hit::Records(r) => Some(r),
            _ => None,
        }
    }

    /// Remove expired entries.
    pub fn expire(&mut self, now: i64) {
        self.map.retain(|_, s| s.expires > now);
    }

    /// `named cache`: one line per live entry, sorted.
    pub fn dump(&self, now: i64) -> Vec<String> {
        let mut out = Vec::new();
        for ((name, t), s) in &self.map {
            if s.expires <= now {
                continue;
            }
            match &s.entry {
                Entry::Records(rs) => out.extend(rs.iter().map(|r| left(r, s.stored, now).to_columns())),
                Entry::NoData(_) => out.push(format!(
                    "{:<23} {:>6} ; NODATA {}",
                    fqdn(name),
                    (s.expires - now + 999) / 1000,
                    type_name(*t)
                )),
                Entry::NxDomain(_) => {
                    out.push(format!("{:<23} {:>6} ; NXDOMAIN",fqdn(name), (s.expires - now + 999) / 1000))
                }
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::*;

    #[test]
    fn ttls_count_down_and_expire() {
        let mut c = Cache::new();
        c.put(vec![Record::new("web.v4.ring", 300, RData::A([100, 68, 0, 10]))], 0);
        let Some(Hit::Records(r)) = c.get("web.v4.ring", A, 10_500) else { panic!() };
        assert_eq!(r[0].ttl, 290);
        assert!(c.get("web.v4.ring", A, 300_000).is_none());
        assert!(c.get("web.v4.ring", MX, 0).is_none());
    }

    #[test]
    fn negatives() {
        let mut c = Cache::new();
        let soa = Record::new(
            "ring",
            3600,
            RData::Soa(Soa { mname: "a".into(), rname: "b".into(), serial: 1, refresh: 1, retry: 1, expire: 1, minimum: 30 }),
        );
        c.put_nxdomain("nosuch.ring", Some(soa.clone()), 0);
        assert!(matches!(c.get("nosuch.ring", A, 1000), Some(Hit::NxDomain(Some(_)))));
        assert!(matches!(c.get("nosuch.ring", MX, 1000), Some(Hit::NxDomain(_))));
        assert!(c.get("nosuch.ring", A, 30_000).is_none(), "the SOA minimum caps it");
        c.put_nodata("ring", MX, Some(soa), 0);
        assert!(matches!(c.get("ring", MX, 0), Some(Hit::NoData(_))));
        assert_eq!(c.dump(0).len(), 2);
        c.flush();
        assert!(c.is_empty());
    }

    #[test]
    fn bounded() {
        let mut c = Cache::new();
        for i in 0..(MAX_ENTRIES + 50) {
            c.put(vec![Record::new(&format!("h{i}.t"), 100 + i as u32, RData::A([1, 1, 1, 1]))], 0);
        }
        assert_eq!(c.len(), MAX_ENTRIES);
        assert!(c.get("h0.t", A, 0).is_none(), "the earliest expiry went first");
    }
}
