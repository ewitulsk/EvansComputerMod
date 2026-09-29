//! Filtering database keyed by (VLAN, MAC).
//!
//! Bounded to [`FDB_CAPACITY`] entries. Drop policy: learning a new address
//! into a full table evicts the least-recently-seen dynamic entry; if every
//! entry is static the new address is not learned (and a new static entry is
//! refused).

use crate::types::PortRef;
use std::collections::{BTreeMap, BTreeSet};

pub const FDB_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FdbEntry {
    pub port: PortRef,
    pub is_static: bool,
    pub last_seen_ms: i64,
    pub prev_port: Option<PortRef>,
    pub move_count: u32,
    pub last_move_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Learn {
    New,
    Refreshed,
    Moved { from: PortRef },
    /// A static entry exists for this (MAC, VLAN).
    Static,
    /// Table full of static entries.
    Dropped,
}

type Key = (u16, [u8; 6]);

#[derive(Clone, Debug, Default)]
pub(crate) struct Fdb {
    map: BTreeMap<Key, FdbEntry>,
    /// Dynamic entries ordered by (last_seen, key) for O(log n) eviction/ageing.
    by_age: BTreeSet<(i64, Key)>,
}

impl Fdb {
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[cfg(test)]
    pub fn dynamic_len(&self) -> usize {
        self.by_age.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u16, [u8; 6], &FdbEntry)> {
        self.map.iter().map(|((v, m), e)| (*v, *m, e))
    }

    pub fn lookup(&self, mac: &[u8; 6], vlan: u16) -> Option<PortRef> {
        self.map.get(&(vlan, *mac)).map(|e| e.port)
    }

    #[cfg(test)]
    pub fn get(&self, mac: &[u8; 6], vlan: u16) -> Option<&FdbEntry> {
        self.map.get(&(vlan, *mac))
    }

    fn evict_oldest_dynamic(&mut self) -> bool {
        let first = self.by_age.iter().next().copied();
        match first {
            Some((t, k)) => {
                self.by_age.remove(&(t, k));
                self.map.remove(&k);
                true
            }
            None => false,
        }
    }

    pub fn learn(&mut self, mac: [u8; 6], vlan: u16, port: PortRef, now: i64) -> Learn {
        let key = (vlan, mac);
        if let Some(e) = self.map.get_mut(&key) {
            if e.is_static {
                return Learn::Static;
            }
            self.by_age.remove(&(e.last_seen_ms, key));
            e.last_seen_ms = now;
            self.by_age.insert((now, key));
            if e.port == port {
                return Learn::Refreshed;
            }
            let from = e.port;
            e.prev_port = Some(from);
            e.port = port;
            e.move_count = e.move_count.saturating_add(1);
            e.last_move_ms = now;
            return Learn::Moved { from };
        }
        if self.map.len() >= FDB_CAPACITY && !self.evict_oldest_dynamic() {
            return Learn::Dropped;
        }
        self.map.insert(key, FdbEntry { port, is_static: false, last_seen_ms: now, prev_port: None, move_count: 0, last_move_ms: -1 });
        self.by_age.insert((now, key));
        Learn::New
    }

    /// Insert or convert an entry to static. Returns Ok(true) if an existing
    /// entry was updated.
    pub fn add_static(&mut self, mac: [u8; 6], vlan: u16, port: PortRef, now: i64) -> Result<bool, ()> {
        let key = (vlan, mac);
        if let Some(e) = self.map.get_mut(&key) {
            if !e.is_static {
                self.by_age.remove(&(e.last_seen_ms, key));
            }
            *e = FdbEntry { port, is_static: true, last_seen_ms: now, prev_port: None, move_count: 0, last_move_ms: -1 };
            return Ok(true);
        }
        if self.map.len() >= FDB_CAPACITY && !self.evict_oldest_dynamic() {
            return Err(());
        }
        self.map.insert(key, FdbEntry { port, is_static: true, last_seen_ms: now, prev_port: None, move_count: 0, last_move_ms: -1 });
        Ok(false)
    }

    pub fn remove_static(&mut self, mac: [u8; 6], vlan: u16, port: PortRef) -> bool {
        let key = (vlan, mac);
        match self.map.get(&key) {
            Some(e) if e.is_static && e.port == port => {
                self.map.remove(&key);
                true
            }
            _ => false,
        }
    }

    /// Remove dynamic entries matching `pred`; returns how many.
    pub fn remove_dynamic_where(&mut self, mut pred: impl FnMut(u16, &[u8; 6], &FdbEntry) -> bool) -> usize {
        let doomed: Vec<(i64, Key)> = self
            .map
            .iter()
            .filter(|(k, e)| !e.is_static && pred(k.0, &k.1, e))
            .map(|(k, e)| (e.last_seen_ms, *k))
            .collect();
        for (t, k) in &doomed {
            self.by_age.remove(&(*t, *k));
            self.map.remove(k);
        }
        doomed.len()
    }

    /// Remove every entry (static too) matching `pred`.
    pub fn remove_all_where(&mut self, mut pred: impl FnMut(u16, &[u8; 6], &FdbEntry) -> bool) -> usize {
        let doomed: Vec<(Key, FdbEntry)> = self.map.iter().filter(|(k, e)| pred(k.0, &k.1, e)).map(|(k, e)| (*k, *e)).collect();
        for (k, e) in &doomed {
            if !e.is_static {
                self.by_age.remove(&(e.last_seen_ms, *k));
            }
            self.map.remove(k);
        }
        doomed.len()
    }

    pub fn flush_port(&mut self, port: PortRef) -> usize {
        self.remove_dynamic_where(|_, _, e| e.port == port)
    }

    pub fn flush_except(&mut self, port: PortRef) -> usize {
        self.remove_dynamic_where(|_, _, e| e.port != port)
    }

    /// Drop dynamic entries not seen for `age_ms`.
    pub fn age(&mut self, now: i64, age_ms: i64) -> usize {
        let mut n = 0;
        while let Some(&(t, k)) = self.by_age.iter().next() {
            if now.saturating_sub(t) <= age_ms {
                break;
            }
            self.by_age.remove(&(t, k));
            self.map.remove(&k);
            n += 1;
        }
        n
    }

    /// Time the oldest dynamic entry was last seen.
    pub fn oldest_dynamic(&self) -> Option<i64> {
        self.by_age.iter().next().map(|(t, _)| *t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(n: u16) -> [u8; 6] {
        [2, 0, 0, 0, (n >> 8) as u8, n as u8]
    }

    #[test]
    fn learn_refresh_move() {
        let mut f = Fdb::default();
        assert_eq!(f.learn(mac(1), 1, PortRef::Eth(0), 0), Learn::New);
        assert_eq!(f.learn(mac(1), 1, PortRef::Eth(0), 10), Learn::Refreshed);
        assert_eq!(f.learn(mac(1), 1, PortRef::Eth(2), 20), Learn::Moved { from: PortRef::Eth(0) });
        let e = f.get(&mac(1), 1).unwrap();
        assert_eq!((e.port, e.prev_port, e.move_count, e.last_move_ms), (PortRef::Eth(2), Some(PortRef::Eth(0)), 1, 20));
        // Same MAC in another VLAN is independent.
        assert_eq!(f.learn(mac(1), 2, PortRef::Eth(3), 20), Learn::New);
        assert_eq!(f.lookup(&mac(1), 1), Some(PortRef::Eth(2)));
        assert_eq!(f.lookup(&mac(1), 2), Some(PortRef::Eth(3)));
    }

    #[test]
    fn statics_are_sticky() {
        let mut f = Fdb::default();
        f.learn(mac(1), 1, PortRef::Eth(0), 0);
        assert_eq!(f.add_static(mac(1), 1, PortRef::Eth(4), 0), Ok(true));
        assert_eq!(f.learn(mac(1), 1, PortRef::Eth(0), 5), Learn::Static);
        assert_eq!(f.lookup(&mac(1), 1), Some(PortRef::Eth(4)));
        assert_eq!(f.age(1_000_000, 1), 0);
        assert!(!f.remove_static(mac(1), 1, PortRef::Eth(0)));
        assert!(f.remove_static(mac(1), 1, PortRef::Eth(4)));
        assert_eq!(f.len(), 0);
    }

    #[test]
    fn bounded_with_lru_eviction() {
        let mut f = Fdb::default();
        for i in 0..FDB_CAPACITY as u16 {
            f.learn(mac(i), 1, PortRef::Eth(0), i as i64);
        }
        // Refresh the oldest so the second-oldest is evicted.
        f.learn(mac(0), 1, PortRef::Eth(0), 5000);
        assert_eq!(f.learn(mac(5000), 1, PortRef::Eth(1), 5001), Learn::New);
        assert_eq!(f.len(), FDB_CAPACITY);
        assert!(f.lookup(&mac(0), 1).is_some());
        assert!(f.lookup(&mac(1), 1).is_none());
    }

    #[test]
    fn full_of_statics_drops() {
        let mut f = Fdb::default();
        for i in 0..FDB_CAPACITY as u16 {
            assert!(f.add_static(mac(i), 1, PortRef::Eth(0), 0).is_ok());
        }
        assert_eq!(f.learn(mac(9999), 1, PortRef::Eth(0), 0), Learn::Dropped);
        assert_eq!(f.add_static(mac(9999), 1, PortRef::Eth(0), 0), Err(()));
    }

    #[test]
    fn ageing_and_flush() {
        let mut f = Fdb::default();
        f.learn(mac(1), 1, PortRef::Eth(0), 0);
        f.learn(mac(2), 1, PortRef::Eth(1), 100);
        f.learn(mac(3), 1, PortRef::Lag(1), 200);
        assert_eq!(f.age(300, 250), 1);
        assert_eq!(f.flush_except(PortRef::Lag(1)), 1);
        assert_eq!(f.flush_port(PortRef::Lag(1)), 1);
        assert_eq!(f.len(), 0);
        assert_eq!(f.dynamic_len(), 0);
    }
}
