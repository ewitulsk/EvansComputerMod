//! LLDP agent (IEEE 802.1AB-2016), nearest-bridge scope only.
//!
//! * Transmits every `timer` seconds on link-up L2 ports with transmit
//!   enabled, TTL = min(65535, timer x holdtime + 1).
//! * `txdelay` is the minimum gap between LLDPDUs on a port (applies to
//!   the fast first frame after enable and to triggered sends on local
//!   changes). When transmission is disabled on a link-up port, a shutdown
//!   LLDPDU (TTL 0) is sent and the port may not transmit again for
//!   `reinit` seconds.
//! * Receive: only frames to 01:80:c2:00:00:0e are consumed. TTL 0 deletes
//!   the neighbour; a neighbour expires at last update + TTL; link down
//!   purges the port's table. At most [`MAX_NEIGHBORS_PER_PORT`] per port:
//!   new neighbours are dropped when full (counted in `too_many`).

use crate::bridge::{Bridge, Deadline};
use crate::lldpdu::{self, Lldpdu, CAP_BRIDGE, CHASSIS_SUBTYPE_MAC, PORT_SUBTYPE_IFNAME};
use crate::log::Severity;
use crate::types::LldpTlv;
use ecm_net::types::MacAddr;

pub const MAX_NEIGHBORS_PER_PORT: usize = 64;

/// A learned LLDP neighbour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LldpNeighbor {
    pub port: usize,
    pub chassis_id: Vec<u8>,
    pub port_id: Vec<u8>,
    pub ttl: u16,
    pub first_seen_ms: i64,
    pub last_update_ms: i64,
    pub expires_ms: i64,
    pub port_desc: Option<String>,
    pub sys_name: Option<String>,
    pub sys_desc: Option<String>,
    pub caps: Option<(u16, u16)>,
    pub mgmt_ipv4: Option<[u8; 4]>,
}

impl LldpNeighbor {
    pub fn chassis_string(&self) -> String {
        lldpdu::id_to_string(&self.chassis_id)
    }
    pub fn port_string(&self) -> String {
        lldpdu::id_to_string(&self.port_id)
    }
}

/// Per-port LLDP counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LldpPortStats {
    pub tx: u64,
    pub rx: u64,
    pub rx_errors: u64,
    pub rx_discards: u64,
    pub ageouts: u64,
    pub too_many: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct LldpPort {
    pub tx_on: bool,
    pub next_tx: i64,
    pub last_tx: i64,
    pub reinit_until: i64,
    pub neighbors: Vec<LldpNeighbor>,
    pub stats: LldpPortStats,
}

impl LldpPort {
    pub fn new() -> Self {
        LldpPort { tx_on: false, next_tx: i64::MIN, last_tx: i64::MIN / 2, reinit_until: i64::MIN, neighbors: Vec::new(), stats: LldpPortStats::default() }
    }
}

impl Bridge {
    fn lldp_should_tx(&self, p: usize) -> bool {
        match self.ports.get(p) {
            Some(ep) => self.lldp_cfg.enabled && ep.cfg.lldp_tx && ep.link_up && ep.cfg.admin_up && self.is_l2_port(p),
            None => false,
        }
    }

    fn lldp_ttl(&self) -> u16 {
        (self.lldp_cfg.timer_secs.saturating_mul(self.lldp_cfg.holdtime).saturating_add(1)).min(65535) as u16
    }

    fn lldp_build(&self, p: usize, ttl: u16) -> Lldpdu {
        let c = &self.lldp_cfg;
        let mut chassis = vec![CHASSIS_SUBTYPE_MAC];
        chassis.extend_from_slice(&self.bridge_mac);
        let mut port_id = vec![PORT_SUBTYPE_IFNAME];
        port_id.extend_from_slice(format!("eth{}", p).as_bytes());
        let mgmt = c.mgmt_ipv4.or_else(|| self.svis.values().next().map(|(ip, _)| *ip));
        let full = ttl != 0;
        Lldpdu {
            chassis_id: chassis,
            port_id,
            ttl,
            port_desc: (full && c.tlv(LldpTlv::PortDesc)).then(|| format!("Port {}", p)),
            sys_name: (full && c.tlv(LldpTlv::SysName)).then(|| c.sys_name.clone()),
            sys_desc: (full && c.tlv(LldpTlv::SysDesc)).then(|| c.sys_desc.clone()),
            caps: (full && c.tlv(LldpTlv::SysCaps)).then_some((CAP_BRIDGE, CAP_BRIDGE)),
            mgmt_ipv4: if full && c.tlv(LldpTlv::MgmtAddr) { mgmt } else { None },
            mgmt_ifindex: (p as u32).saturating_add(1),
        }
    }

    fn lldp_send(&mut self, p: usize, ttl: u16) {
        let pdu = self.lldp_build(p, ttl);
        let mac = match self.ports.get_mut(p) {
            Some(ep) => {
                ep.lldp.stats.tx += 1;
                ep.lldp.last_tx = self.now;
                ep.mac
            }
            None => return,
        };
        self.tx(p, lldpdu::build_frame(mac, &pdu));
    }

    pub(crate) fn lldp_run(&mut self, now: i64) {
        let timer = self.lldp_cfg.timer_secs as i64 * 1000;
        let txdelay = self.lldp_cfg.txdelay_secs as i64 * 1000;
        let reinit = self.lldp_cfg.reinit_secs as i64 * 1000;
        for p in 0..self.ports.len() {
            let want = self.lldp_should_tx(p);
            let link_up = self.ports.get(p).map(|e| e.link_up).unwrap_or(false);
            let (tx_on, reinit_until) = match self.ports.get(p) {
                Some(e) => (e.lldp.tx_on, e.lldp.reinit_until),
                None => continue,
            };
            if want && !tx_on && now >= reinit_until {
                if let Some(e) = self.ports.get_mut(p) {
                    e.lldp.tx_on = true;
                    e.lldp.next_tx = now.max(e.lldp.last_tx.saturating_add(txdelay));
                }
            } else if !want && tx_on {
                if link_up {
                    self.lldp_send(p, 0); // shutdown LLDPDU
                }
                if let Some(e) = self.ports.get_mut(p) {
                    e.lldp.tx_on = false;
                    e.lldp.reinit_until = now.saturating_add(reinit);
                }
            }
            let due = self.ports.get(p).map(|e| e.lldp.tx_on && now >= e.lldp.next_tx).unwrap_or(false);
            if due {
                let ttl = self.lldp_ttl();
                self.lldp_send(p, ttl);
                if let Some(e) = self.ports.get_mut(p) {
                    e.lldp.next_tx = now.saturating_add(timer);
                }
            }
            // Neighbour ageing.
            let mut expired = Vec::new();
            if let Some(e) = self.ports.get_mut(p) {
                e.lldp.neighbors.retain(|n| {
                    if now >= n.expires_ms {
                        expired.push(format!("LLDP neighbor aged out on eth{} (chassis {}, port {})", p, n.chassis_string(), n.port_string()));
                        false
                    } else {
                        true
                    }
                });
                e.lldp.stats.ageouts += expired.len() as u64;
            }
            for m in expired {
                self.log(Severity::Info, m);
            }
        }
    }

    /// Local information changed: schedule a triggered transmission.
    pub(crate) fn lldp_local_change(&mut self) {
        let txdelay = self.lldp_cfg.txdelay_secs as i64 * 1000;
        let now = self.now;
        for e in self.ports.iter_mut() {
            if e.lldp.tx_on {
                e.lldp.next_tx = e.lldp.next_tx.min(now.max(e.lldp.last_tx.saturating_add(txdelay)));
            }
        }
    }

    pub(crate) fn lldp_purge_port(&mut self, p: usize) {
        let n = match self.ports.get_mut(p) {
            Some(e) => {
                let n = e.lldp.neighbors.len();
                e.lldp.neighbors.clear();
                e.lldp.tx_on = false;
                // A new link: the first LLDPDU after link-up goes out at once.
                e.lldp.last_tx = i64::MIN / 2;
                n
            }
            None => 0,
        };
        if n > 0 {
            self.log(Severity::Info, format!("LLDP {} neighbor(s) purged on eth{} (link down)", n, p));
        }
    }

    /// LLDPDU payload received on port `p`.
    pub(crate) fn lldp_rx(&mut self, p: usize, payload: &[u8], now: i64) {
        let enabled = self.lldp_cfg.enabled;
        let mut log = None;
        {
            let ep = match self.ports.get_mut(p) {
                Some(e) => e,
                None => return,
            };
            if !enabled || !ep.cfg.lldp_rx {
                ep.lldp.stats.rx_discards += 1;
                return;
            }
            let pdu = match lldpdu::decode(payload) {
                Ok(x) => x,
                Err(_) => {
                    ep.lldp.stats.rx_errors += 1;
                    ep.lldp.stats.rx_discards += 1;
                    return;
                }
            };
            ep.lldp.stats.rx += 1;
            let nb = &mut ep.lldp.neighbors;
            let idx = nb.iter().position(|n| n.chassis_id == pdu.chassis_id && n.port_id == pdu.port_id);
            if pdu.ttl == 0 {
                if let Some(i) = idx {
                    let n = nb.remove(i);
                    log = Some(format!("LLDP neighbor removed on eth{} (chassis {}, port {}, TTL 0)", p, n.chassis_string(), n.port_string()));
                }
            } else {
                let expires = now.saturating_add(pdu.ttl as i64 * 1000);
                let full = nb.len() >= MAX_NEIGHBORS_PER_PORT;
                match idx.and_then(|i| nb.get_mut(i)) {
                    Some(n) => {
                        n.ttl = pdu.ttl;
                        n.last_update_ms = now;
                        n.expires_ms = expires;
                        n.port_desc = pdu.port_desc;
                        n.sys_name = pdu.sys_name;
                        n.sys_desc = pdu.sys_desc;
                        n.caps = pdu.caps;
                        n.mgmt_ipv4 = pdu.mgmt_ipv4;
                    }
                    None if full => {
                        ep.lldp.stats.too_many += 1;
                    }
                    None => {
                        let n = LldpNeighbor {
                            port: p,
                            chassis_id: pdu.chassis_id,
                            port_id: pdu.port_id,
                            ttl: pdu.ttl,
                            first_seen_ms: now,
                            last_update_ms: now,
                            expires_ms: expires,
                            port_desc: pdu.port_desc,
                            sys_name: pdu.sys_name,
                            sys_desc: pdu.sys_desc,
                            caps: pdu.caps,
                            mgmt_ipv4: pdu.mgmt_ipv4,
                        };
                        log = Some(format!(
                            "LLDP neighbor added on eth{} (chassis {}, port {}, sys {})",
                            p,
                            n.chassis_string(),
                            n.port_string(),
                            n.sys_name.clone().unwrap_or_default()
                        ));
                        nb.push(n);
                    }
                }
            }
        }
        if let Some(m) = log {
            self.log(Severity::Info, m);
        }
    }

    pub(crate) fn lldp_deadline(&self, dl: &mut Deadline) {
        for (p, e) in self.ports.iter().enumerate() {
            if e.lldp.tx_on {
                dl.at(e.lldp.next_tx);
            } else if self.lldp_should_tx(p) {
                dl.at(e.lldp.reinit_until.max(self.now));
            }
            for n in &e.lldp.neighbors {
                dl.at(n.expires_ms);
            }
        }
    }

    /// All neighbours (optionally one port's).
    pub fn lldp_neighbors(&self, port: Option<usize>) -> Vec<LldpNeighbor> {
        self.ports
            .iter()
            .enumerate()
            .filter(|(i, _)| port.map(|p| p == *i).unwrap_or(true))
            .flat_map(|(_, e)| e.lldp.neighbors.iter().cloned())
            .collect()
    }

    pub fn lldp_port_stats(&self, port: usize) -> Option<LldpPortStats> {
        self.ports.get(port).map(|e| e.lldp.stats)
    }

    pub fn clear_lldp_neighbors(&mut self) {
        for e in self.ports.iter_mut() {
            e.lldp.neighbors.clear();
        }
    }

    pub fn clear_lldp_stats(&mut self) {
        for e in self.ports.iter_mut() {
            e.lldp.stats = LldpPortStats::default();
        }
    }

    /// Chassis ID advertised (the bridge MAC).
    pub fn lldp_chassis_mac(&self) -> MacAddr {
        MacAddr(self.bridge_mac)
    }
}
