//! The running site without its I/O: the `ecm_cell` site machine, the settings,
//! the log, and the admin commands. `main.rs` owns the sockets and the radio.
//!
//! Admin commands (one per line on 127.0.0.1:7710, replies framed as in
//! `ecm_cell::admin`):
//!
//! | Command | Reply body |
//! |---|---|
//! | `status` | `key value` lines (see [`Daemon::status_text`]), then one `ue ...` line per phone and one `nb ...` line per neighbour |
//! | `config` | the config file |
//! | `set <key> <value>` | `saved <key>`: written to the file; `restart` applies it |
//! | `restart` | `restarting`: the settings are read again and the radio reconfigured |
//! | `stop` | `stopping` |
//! | `log [n]` | the last n (default 50) log lines |
//! | `radio` | the radio's framed status (`key value`) |

use crate::config::{Conf, Runtime};
use ecm_cell::codec::fmt_ip;
use ecm_cell::core::Entropy;
use ecm_cell::phy::Profile;
use ecm_cell::site::Site;
use std::collections::VecDeque;

pub const LOG_LINES: usize = 300;

/// What the main loop does after a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Restart,
    Stop,
}

pub struct Daemon {
    pub site: Site,
    pub conf: Conf,
    pub rt: Runtime,
    pub log: VecDeque<String>,
    pub started_ms: u64,
    /// The radio's framed status (`key value`), refreshed by the main loop.
    pub radio: Vec<(String, String)>,
}

impl Daemon {
    pub fn new(conf: Conf, entropy: Box<dyn Entropy + Send>, now_ms: u64) -> Result<Daemon, String> {
        let (cfg, rt) = conf.site_config()?;
        Ok(Daemon { site: Site::with_entropy(cfg, entropy), conf, rt, log: VecDeque::new(), started_ms: now_ms, radio: Vec::new() })
    }

    pub fn log(&mut self, now_ms: u64, line: &str) {
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        let t = now_ms.saturating_sub(self.started_ms);
        self.log.push_back(format!("[{:>6}.{:01}] {}", t / 1000, (t % 1000) / 100, line));
    }

    fn radio_value(&self, k: &str) -> Option<&str> {
        self.radio.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }

    /// The `status` body.
    pub fn status_text(&self, now_ms: u64) -> String {
        let c = &self.site.cfg;
        let st = self.site.status();
        let mut s = String::new();
        let mut kv = |k: &str, v: String| s.push_str(&format!("{k} {v}\n"));
        kv("site", c.site_id.clone());
        kv("carrier", c.carrier.clone());
        kv("cell", c.cell_id.to_string());
        kv("profile", c.profile.name().into());
        kv("channel", c.channel.to_string());
        kv("power", c.power_dbm.map(|p| format!("{p:.1}")).unwrap_or_else(|| format!("auto ({:.0})", c.profile.default_power_dbm(ecm_cell::phy::Role::Bs))));
        kv("address", format!("{:08x}", c.address));
        kv("radio", if self.rt.radio.is_empty() { "first SDR".into() } else { self.rt.radio.clone() });
        kv("core", format!("{}:{}", fmt_ip(self.rt.core), self.rt.core_port));
        kv("link", st.core.into());
        if !st.core_error.is_empty() && st.core != "up" {
            kv("link_error", st.core_error.clone());
        }
        kv("uptime_s", (now_ms.saturating_sub(self.started_ms) / 1000).to_string());
        kv("phones", st.ues.iter().filter(|u| u.state == "connected").count().to_string());
        kv("ul_bytes", st.ul_bytes.to_string());
        kv("dl_bytes", st.dl_bytes.to_string());
        kv("backhaul_rejects", st.backhaul_rejects.to_string());
        kv("ul_mask", format!("{:04b}", st.ul_mask));
        for (k, name) in [("tx_frames", "radio_tx_frames"), ("rx_frames", "radio_rx_frames"), ("beacons_sent", "radio_beacons"), ("harq_failed", "radio_harq_failed"), ("tx_freq", "radio_tx_freq")] {
            if let Some(v) = self.radio_value(k) {
                kv(name, v.to_string());
            }
        }
        for u in &st.ues {
            s.push_str(&format!(
                "ue {:08x} {} {} {} {:.1} {} {} {}\n",
                u.addr,
                if u.imsi.is_empty() { "-" } else { &u.imsi },
                u.state,
                u.ip.map(fmt_ip).unwrap_or_else(|| "-".into()),
                u.rssi_dbm,
                u.cqi_ul,
                u.subchannel,
                u.used
            ));
        }
        for n in &st.neighbours {
            let p = Profile::from_code(n.profile).map(|p| p.name()).unwrap_or("?");
            s.push_str(&format!("nb {} {} {} {:04b}\n", n.cell_id, p, n.channel, n.ul_mask));
        }
        s
    }

    /// Run one admin command. `save` writes the config file.
    pub fn handle(&mut self, line: &str, now_ms: u64, save: &mut dyn FnMut(&str) -> Result<(), String>) -> (Result<String, String>, Action) {
        let w: Vec<&str> = line.split_whitespace().collect();
        match w.as_slice() {
            ["status"] => (Ok(self.status_text(now_ms)), Action::None),
            ["config"] => (Ok(self.conf.to_text()), Action::None),
            ["set", key, rest @ ..] => {
                let value = rest.join(" ");
                let mut c = self.conf.clone();
                let r = c.set(key, &value).and_then(|_| save(&c.to_text()));
                match r {
                    Ok(()) => {
                        self.conf = c;
                        self.log(now_ms, &format!("config: {key} = {value} (restart to apply)"));
                        (Ok(format!("saved {key}")), Action::None)
                    }
                    Err(e) => (Err(e), Action::None),
                }
            }
            ["restart"] => match self.conf.site_config() {
                Ok(_) => (Ok("restarting".into()), Action::Restart),
                Err(e) => (Err(e), Action::None),
            },
            ["stop"] => (Ok("stopping".into()), Action::Stop),
            ["log"] | ["log", _] => {
                let n = w.get(1).and_then(|n| n.parse::<usize>().ok()).unwrap_or(50);
                let skip = self.log.len().saturating_sub(n);
                (Ok(self.log.iter().skip(skip).cloned().collect::<Vec<_>>().join("\n")), Action::None)
            }
            ["radio"] => (Ok(self.radio.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join("\n")), Action::None),
            [] => (Err("empty command".into()), Action::None),
            _ => (Err(format!("unknown command '{}' (status, config, set, restart, stop, log, radio)", line.trim())), Action::None),
        }
    }
}

/// A phone as `status` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct UeView {
    pub addr: String,
    pub imsi: String,
    pub state: String,
    pub ip: String,
    pub rssi_dbm: f64,
    pub cqi: u8,
    pub subchannel: u8,
    pub used: u64,
}

/// A neighbour as `status` lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct NbView {
    pub cell: u32,
    pub profile: String,
    pub channel: u8,
    pub ul_mask: String,
}

/// The `status` reply, parsed (for the Cell Site app).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusView {
    pub kv: Vec<(String, String)>,
    pub ues: Vec<UeView>,
    pub nbs: Vec<NbView>,
}

impl StatusView {
    pub fn parse(body: &[String]) -> StatusView {
        let mut v = StatusView::default();
        for l in body {
            let w: Vec<&str> = l.split_whitespace().collect();
            match w.as_slice() {
                ["ue", addr, imsi, state, ip, rssi, cqi, sub, used] => v.ues.push(UeView {
                    addr: addr.to_string(),
                    imsi: imsi.to_string(),
                    state: state.to_string(),
                    ip: ip.to_string(),
                    rssi_dbm: rssi.parse().unwrap_or(0.0),
                    cqi: cqi.parse().unwrap_or(0),
                    subchannel: sub.parse().unwrap_or(0),
                    used: used.parse().unwrap_or(0),
                }),
                ["nb", cell, profile, channel, mask] => v.nbs.push(NbView {
                    cell: cell.parse().unwrap_or(0),
                    profile: profile.to_string(),
                    channel: channel.parse().unwrap_or(0),
                    ul_mask: mask.to_string(),
                }),
                [k, ..] => {
                    let rest = l[k.len()..].trim().to_string();
                    v.kv.push((k.to_string(), rest));
                }
                [] => {}
            }
        }
        v
    }

    pub fn get(&self, k: &str) -> &str {
        self.kv.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str()).unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_cell::Now;

    struct Fixed;
    impl Entropy for Fixed {
        fn fill(&mut self, b: &mut [u8]) {
            b.fill(7);
        }
    }

    fn daemon() -> Daemon {
        let c = Conf::parse("site_id = site-a\nsecret = 5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e\ncarrier = cell-red\ncore = 10.0.1.1\ncell_id = 1\nchannel = 3\n").unwrap();
        Daemon::new(c, Box::new(Fixed), 1000).unwrap()
    }

    #[test]
    fn status_reads_back() {
        let mut d = daemon();
        d.site.start(Now { ms: 1000, slot: 0 });
        d.radio = vec![("tx_frames".into(), "12".into()), ("beacons_sent".into(), "40".into())];
        let text = d.status_text(4500);
        let v = StatusView::parse(&text.lines().map(String::from).collect::<Vec<_>>());
        assert_eq!(v.get("site"), "site-a");
        assert_eq!(v.get("channel"), "3");
        assert_eq!(v.get("link"), "down");
        assert_eq!(v.get("power"), "auto (43)");
        assert_eq!(v.get("uptime_s"), "3");
        assert_eq!(v.get("radio_beacons"), "40");
        assert!(v.ues.is_empty(), "{text}");
        // phone and neighbour lines
        let v = StatusView::parse(&["ue 000000b1 001010000000001 connected 100.65.2.2 -71.5 9 2 1500".to_string(), "nb 2 lte 3 0010".into()]);
        assert_eq!(v.ues[0].imsi, "001010000000001");
        assert_eq!((v.ues[0].rssi_dbm, v.ues[0].cqi, v.ues[0].subchannel, v.ues[0].used), (-71.5, 9, 2, 1500));
        assert_eq!(v.nbs[0], NbView { cell: 2, profile: "lte".into(), channel: 3, ul_mask: "0010".into() });
    }

    #[test]
    fn commands() {
        let mut d = daemon();
        let saved = std::cell::RefCell::new(String::new());
        let mut save = |t: &str| {
            *saved.borrow_mut() = t.to_string();
            Ok(())
        };
        let (r, a) = d.handle("set channel 5", 0, &mut save);
        assert_eq!((r, a), (Ok("saved channel".into()), Action::None));
        assert!(saved.borrow().contains("channel = 5"));
        assert_eq!(d.site.cfg.channel, 3, "applies on restart");
        assert_eq!(d.handle("restart", 0, &mut save).1, Action::Restart);
        assert!(d.handle("set channel 9", 0, &mut save).0.is_err());
        assert!(d.handle("set colour red", 0, &mut save).0.unwrap_err().contains("unknown key"));
        assert_eq!(d.handle("stop", 0, &mut save).1, Action::Stop);
        assert!(d.handle("frobnicate", 0, &mut save).0.unwrap_err().contains("unknown command"));
        let log = d.handle("log 1", 0, &mut save).0.unwrap();
        assert!(log.contains("config: channel = 5"), "{log}");
        assert!(d.handle("config", 0, &mut save).0.unwrap().contains("site_id = site-a"));
        let mut failing = |_: &str| Err("read-only".to_string());
        assert_eq!(d.handle("set channel 2", 0, &mut failing).0, Err("read-only".into()));
        assert_eq!(d.conf.get("channel"), "5", "not changed when the file can't be written");
    }
}
