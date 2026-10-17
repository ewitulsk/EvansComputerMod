//! `/etc/cored.toml`: the carrier core's program settings. The network itself
//! (carrier, pool, sites, subscribers) is the subscriber database, `db`.
//!
//! ```toml
//! db = /etc/cored.db          # the subscriber database (cored init writes it)
//! listen = 7700               # sites' control connections (TCP)
//! tunnel_port = 7701          # sites' tunnels (UDP)
//! tun = cell0                 # the interface towards the phones (address = the pool's gateway)
//! admin_port = 7711           # the admin socket, 127.0.0.1 only
//! imsi_prefix = 00101         # new SIMs' IMSIs start with this (MCC + MNC; 001 01 = test network)
//! reader = ""                 # the Card Reader/Writer to write SIMs with (empty = the first)
//! hss_port = 3868             # the HSS/PCRF port imsd, smscd and mmscd ask (docs/TELEPHONY.md)
//! hss_secret = ""             # their shared secret; empty = only programs on this computer may ask
//! ```
//! Every key is optional: without the file `cored` uses these defaults.

use ecm_cell::admin::{conf_value, parse_conf};
use std::collections::BTreeMap;

pub const PATH: &str = "/etc/cored.toml";
pub const ADMIN_PORT: u16 = 7711;

pub struct Key {
    pub name: &'static str,
    pub default: &'static str,
    pub help: &'static str,
}

pub const KEYS: &[Key] = &[
    Key { name: "db", default: "/etc/cored.db", help: "the subscriber database file (cored init writes it)" },
    Key { name: "listen", default: "7700", help: "the port sites connect to (TCP)" },
    Key { name: "tunnel_port", default: "7701", help: "the port of the sites' tunnels (UDP)" },
    Key { name: "tun", default: "cell0", help: "the interface towards the phones (its address is the pool's gateway)" },
    Key { name: "admin_port", default: "7711", help: "the admin socket's port (TCP on 127.0.0.1)" },
    Key { name: "imsi_prefix", default: "00101", help: "new SIMs' IMSIs start with these digits (MCC + MNC, 5-6 digits)" },
    Key { name: "reader", default: "", help: "the Card Reader/Writer for new SIMs (empty = the first one)" },
    Key { name: "hss_port", default: "3868", help: "the HSS/PCRF port imsd, smscd and mmscd ask (TCP)" },
    Key { name: "hss_secret", default: "", help: "the shared secret of IMS programs on other computers (empty: only this computer may ask)" },
];

fn check(name: &str, v: &str) -> Result<(), String> {
    let ok = match name {
        "db" => v.starts_with('/') && !v.ends_with('/'),
        "listen" | "tunnel_port" | "admin_port" | "hss_port" => v.parse::<u16>().is_ok_and(|p| p > 0),
        "hss_secret" => v.len() <= 64 && !v.contains(char::is_whitespace),
        "tun" => (1..=11).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric()),
        "imsi_prefix" => (5..=6).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit()),
        "reader" => !v.contains(char::is_whitespace),
        _ => return Err(format!("unknown key '{name}' (cored config keys lists them)")),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("bad {name} '{v}': {}", KEYS.iter().find(|k| k.name == name).map(|k| k.help).unwrap_or("")))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conf {
    values: BTreeMap<String, String>,
}

impl Conf {
    pub fn parse(text: &str) -> Result<Conf, String> {
        let mut c = Conf::default();
        for l in parse_conf(text)? {
            check(&l.key, &l.value).map_err(|e| format!("line {}: {}", l.line, e))?;
            c.values.insert(l.key, l.value);
        }
        Ok(c)
    }

    pub fn get(&self, name: &str) -> String {
        self.values.get(name).cloned().unwrap_or_else(|| KEYS.iter().find(|k| k.name == name).map(|k| k.default.to_string()).unwrap_or_default())
    }

    pub fn port(&self, name: &str) -> u16 {
        self.get(name).parse().unwrap_or(0)
    }

    pub fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
        if !KEYS.iter().any(|k| k.name == name) {
            return Err(format!("unknown key '{name}' (cored config keys lists them)"));
        }
        if value.is_empty() {
            self.values.remove(name);
            return Ok(());
        }
        check(name, value)?;
        self.values.insert(name.into(), value.into());
        Ok(())
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# cored: this computer's carrier core (docs/gui/CELLULAR_V2.md). `cored config set <key> <value>`,\n# then restart cored. The network itself (sites, subscribers) is the database `db`.\n");
        for k in KEYS {
            if let Some(v) = self.values.get(k.name) {
                s.push_str(&format!("{} = {}\n", k.name, conf_value(v)));
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_checks() {
        let c = Conf::parse("").unwrap();
        assert_eq!(c.get("db"), "/etc/cored.db");
        assert_eq!(c.port("admin_port"), 7711);
        let c = Conf::parse("listen = 7800\nimsi_prefix = 310150\n").unwrap();
        assert_eq!(c.port("listen"), 7800);
        assert_eq!(Conf::parse(&c.to_text()).unwrap(), c);
        assert!(Conf::parse("imsi_prefix = 3101\n").unwrap_err().starts_with("line 1: bad imsi_prefix"));
        assert!(Conf::parse("listen = 7700\nport = 1\n").unwrap_err().starts_with("line 2: unknown key 'port'"));
        assert!(Conf::parse("tun = \"cell 0\"\n").is_err());
        let mut c = Conf::default();
        c.set("tun", "cell1").unwrap();
        assert!(c.set("db", "relative.db").is_err());
        c.set("tun", "").unwrap();
        assert_eq!(c.get("tun"), "cell0");
    }
}
