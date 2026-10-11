//! The router service's `dhcp-server`: named pools from `router.cfg`, each
//! served on the interface whose address is the pool's default router.
//! Lease logic is shared with the `dhcpd` program (`ecm_net::dhcp_server`).

use ecm_net::dhcp::Message;
use ecm_net::dhcp_server::{LeaseDb, PoolSpec};
use ecm_net::Ipv4Addr;
use std::collections::BTreeMap;

pub use ecm_net::dhcp_server::Lease;

#[derive(Clone, Debug)]
pub struct Pool {
    pub start: Ipv4Addr,
    pub end: Ipv4Addr,
    pub router: Ipv4Addr,
    pub dns: Ipv4Addr,
    pub lease_secs: u32,
    pub enabled: bool,
}
impl Default for Pool {
    fn default() -> Self {
        Self {
            start: Ipv4Addr::ZERO,
            end: Ipv4Addr::ZERO,
            router: Ipv4Addr::ZERO,
            dns: Ipv4Addr::ZERO,
            lease_secs: 86400,
            enabled: false,
        }
    }
}

/// Router pools always hand out a /24.
const ROUTER_POOL_PREFIX: u8 = 24;

pub struct Server {
    pub leases: Vec<Lease>,
    pub dirty: bool,
}
impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}
impl Server {
    pub fn new() -> Self {
        Self {
            leases: Vec::new(),
            dirty: false,
        }
    }
    fn with_db<R>(&mut self, f: impl FnOnce(&mut LeaseDb) -> R) -> R {
        let mut db = LeaseDb {
            leases: core::mem::take(&mut self.leases),
            dirty: false,
        };
        let r = f(&mut db);
        self.leases = db.leases;
        self.dirty |= db.dirty;
        r
    }
    pub fn receive(
        &mut self,
        pools: &BTreeMap<String, Pool>,
        m: &Message,
        server: Ipv4Addr,
        now: i64,
    ) -> Option<Message> {
        if server.is_unspecified() {
            return None;
        }
        let (name, p) = pools.iter().find(|(_, p)| {
            p.enabled && p.router == server && p.start != Ipv4Addr::ZERO && p.lease_secs >= 4
        })?;
        let spec = PoolSpec {
            name: name.clone(),
            start: p.start,
            end: p.end,
            prefix: ROUTER_POOL_PREFIX,
            router: Some(p.router),
            dns: Some(p.dns),
            lease_secs: p.lease_secs,
        };
        self.with_db(|db| db.handle(&spec, &[], m, server, now))
    }
    pub fn render(&self) -> String {
        LeaseDb {
            leases: self.leases.clone(),
            dirty: false,
        }
        .render()
    }
    pub fn restore(&mut self, text: &str, now: i64) {
        self.with_db(|db| db.restore(text, now));
        self.dirty = false;
    }
}
