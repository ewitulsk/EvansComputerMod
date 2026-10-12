//! `/etc/celld-site.toml`: the cell site's settings.
//!
//! ```toml
//! # celld-site (Cellular v2 cell site)
//! site_id = "site-a"                              # the id the core knows this site by
//! secret = "5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e"     # its credential (cored site add prints one)
//! carrier = "cell-red"
//! core = 10.0.1.1                                 # the core computer's address
//! cell_id = 1
//! profile = lte                                   # lte | mmwave
//! channel = 3
//! power = auto                                    # dBm or auto (the profile's base-station power)
//! ```
//! Optional, with defaults: `area = 1`, `radio = ""` (the first SDR; or `sdr_1`, an
//! attachment name, a `/dev/...frames` path), `address = auto` (the radio frame address,
//! hex; auto = `a0000000` + cell id), `core_port = 7700`, `core_tunnel_port = 7701`,
//! `tunnel_port = 7701`, `neighbours = ""` (`cell:channel[:profile],...`; more are
//! learned from phones), `idle_after = 0` (seconds before a quiet phone goes idle;
//! 0 = never), `admin_port = 7710`.
//!
//! Every key is checked when the file is read and when it is set ([`Conf::set`]):
//! a typo is an error naming the line, never a silently ignored setting.

use ecm_cell::admin::{conf_value, parse_conf};
use ecm_cell::air::Neighbour;
use ecm_cell::backhaul::{CONTROL_PORT, TUNNEL_PORT};
use ecm_cell::codec::{parse_ip, unhex};
use ecm_cell::db::valid_site_id;
use ecm_cell::phy::Profile;
use ecm_cell::simauth::valid_carrier;
use ecm_cell::site::SiteConfig;
use std::collections::BTreeMap;

pub const PATH: &str = "/etc/celld-site.toml";
pub const ADMIN_PORT: u16 = 7710;

/// One key: name, default ("" = required, no default), help.
pub struct Key {
    pub name: &'static str,
    pub default: &'static str,
    pub required: bool,
    pub help: &'static str,
}

pub const KEYS: &[Key] = &[
    Key { name: "site_id", default: "", required: true, help: "the id the core knows this site by (1-32 of a-z A-Z 0-9 . _ -)" },
    Key { name: "secret", default: "", required: true, help: "the site credential, 32 hex digits (cored site add <id> prints one)" },
    Key { name: "carrier", default: "", required: true, help: "the carrier (network) name, as the core's database says" },
    Key { name: "core", default: "", required: true, help: "the core computer's IPv4 address" },
    Key { name: "cell_id", default: "", required: true, help: "this cell's number, unique in the carrier (1-4294967295)" },
    Key { name: "profile", default: "lte", required: false, help: "lte or mmwave (the radio profile)" },
    Key { name: "channel", default: "1", required: false, help: "the channel (lte 1-6, mmwave 1-4)" },
    Key { name: "power", default: "auto", required: false, help: "transmit power in dBm, or auto (the profile's base-station power)" },
    Key { name: "area", default: "1", required: false, help: "the tracking area (1-65535)" },
    Key { name: "radio", default: "", required: false, help: "the framed radio: empty = the first SDR; sdr_1, an attachment name or a /dev path" },
    Key { name: "address", default: "auto", required: false, help: "the radio frame address in hex, or auto (a0000000 + cell id)" },
    Key { name: "core_port", default: "7700", required: false, help: "the core's control port (TCP)" },
    Key { name: "core_tunnel_port", default: "7701", required: false, help: "the core's tunnel port (UDP)" },
    Key { name: "tunnel_port", default: "7701", required: false, help: "this site's tunnel port (UDP)" },
    Key { name: "neighbours", default: "", required: false, help: "neighbour cells, cell:channel[:profile] separated by commas (more are learned)" },
    Key { name: "idle_after", default: "0", required: false, help: "seconds before a quiet phone goes idle (0 = never)" },
    Key { name: "admin_port", default: "7710", required: false, help: "the admin socket's port (TCP on 127.0.0.1)" },
];

fn key(name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.name == name)
}

fn port(v: &str) -> Result<u16, String> {
    v.parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| format!("'{v}' is not a port (1-65535)"))
}

/// Check one value; `profile` is the profile in force (for the channel's range).
pub fn check(name: &str, v: &str, profile: Profile) -> Result<(), String> {
    let ok = match name {
        "site_id" => valid_site_id(v),
        "secret" => unhex::<16>(v).is_some(),
        "carrier" => valid_carrier(v),
        "core" => parse_ip(v).is_some(),
        "cell_id" => v.parse::<u32>().is_ok_and(|c| c > 0),
        "profile" => Profile::parse(v).is_some(),
        "channel" => v.parse::<u8>().is_ok_and(|c| c >= 1 && c <= profile.channels()),
        "power" => v == "auto" || v.parse::<f64>().is_ok_and(|p| (-30.0..=60.0).contains(&p)),
        "area" => v.parse::<u16>().is_ok_and(|a| a > 0),
        "radio" => !v.contains(char::is_whitespace),
        "address" => v == "auto" || (v.len() <= 8 && u32::from_str_radix(v, 16).is_ok_and(|a| a != 0 && a != u32::MAX)),
        "core_port" | "core_tunnel_port" | "tunnel_port" | "admin_port" => port(v).is_ok(),
        "neighbours" => parse_neighbours(v).is_ok(),
        "idle_after" => v.parse::<u32>().is_ok(),
        _ => return Err(format!("unknown key '{name}' (celld-site config keys lists them)")),
    };
    if ok {
        Ok(())
    } else {
        Err(format!("bad {name} '{v}': {}", key(name).map(|k| k.help).unwrap_or("")))
    }
}

/// `cell:channel[:profile],...`
pub fn parse_neighbours(v: &str) -> Result<Vec<Neighbour>, String> {
    let mut out = Vec::new();
    for part in v.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let w: Vec<&str> = part.split(':').collect();
        let bad = || format!("bad neighbour '{part}' (cell:channel[:profile])");
        if !(2..=3).contains(&w.len()) {
            return Err(bad());
        }
        let cell_id = w[0].parse::<u32>().map_err(|_| bad())?;
        let channel = w[1].parse::<u8>().map_err(|_| bad())?;
        let profile = match w.get(2) {
            Some(p) => Profile::parse(p).ok_or_else(bad)?,
            None => Profile::Lte,
        };
        out.push(Neighbour { cell_id, profile: profile.code(), channel, ul_mask: 0 });
    }
    Ok(out)
}

/// The settings, as written in the file (every value checked).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conf {
    values: BTreeMap<String, String>,
}

/// What the program needs beyond the `ecm_cell` site machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Runtime {
    pub radio: String,
    pub core: [u8; 4],
    pub core_port: u16,
    pub core_tunnel_port: u16,
    pub admin_port: u16,
}

impl Conf {
    pub fn parse(text: &str) -> Result<Conf, String> {
        let mut c = Conf::default();
        let lines = parse_conf(text)?;
        let profile = lines.iter().find(|l| l.key == "profile").and_then(|l| Profile::parse(&l.value)).unwrap_or(Profile::Lte);
        for l in lines {
            check(&l.key, &l.value, profile).map_err(|e| format!("line {}: {}", l.line, e))?;
            c.values.insert(l.key, l.value);
        }
        Ok(c)
    }

    pub fn get(&self, name: &str) -> String {
        self.values.get(name).cloned().unwrap_or_else(|| key(name).map(|k| k.default.to_string()).unwrap_or_default())
    }

    pub fn is_set(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    fn profile(&self) -> Profile {
        Profile::parse(&self.get("profile")).unwrap_or(Profile::Lte)
    }

    /// Set (or, with an empty value for an optional key, reset) one key.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
        let k = key(name).ok_or_else(|| format!("unknown key '{name}' (celld-site config keys lists them)"))?;
        if value.is_empty() && !k.required {
            self.values.remove(name);
            return Ok(());
        }
        let mut profile = self.profile();
        if name == "profile" {
            profile = Profile::parse(value).ok_or_else(|| format!("bad profile '{value}': lte or mmwave"))?;
        }
        check(name, value, profile)?;
        self.values.insert(name.to_string(), value.to_string());
        // A profile change can leave the channel out of range: back to channel 1.
        if name == "profile" && self.get("channel").parse::<u8>().is_ok_and(|c| c > profile.channels()) {
            self.values.insert("channel".into(), "1".into());
        }
        Ok(())
    }

    /// The keys still missing before the site can start.
    pub fn missing(&self) -> Vec<&'static str> {
        KEYS.iter().filter(|k| k.required && !self.values.contains_key(k.name)).map(|k| k.name).collect()
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("# celld-site: this computer's cell site (docs/gui/CELLULAR_V2.md). Edit with\n# `celld-site config set <key> <value>` or the Cell Site app, then `celld-site restart`.\n");
        for k in KEYS {
            if let Some(v) = self.values.get(k.name) {
                s.push_str(&format!("{} = {}\n", k.name, conf_value(v)));
            }
        }
        s
    }

    /// The site machine's settings and the program's own.
    pub fn site_config(&self) -> Result<(SiteConfig, Runtime), String> {
        let missing = self.missing();
        if !missing.is_empty() {
            return Err(format!("not configured: set {} (celld-site config set <key> <value>)", missing.join(", ")));
        }
        let cell_id: u32 = self.get("cell_id").parse().map_err(|_| "bad cell_id")?;
        let address = match self.get("address").as_str() {
            "auto" => 0xA000_0000 | (cell_id & 0x0FFF_FFFF),
            a => u32::from_str_radix(a, 16).map_err(|_| "bad address")?,
        };
        let secret = unhex::<16>(&self.get("secret")).ok_or("bad secret")?;
        let mut cfg = SiteConfig::new(&self.get("site_id"), secret, &self.get("carrier"), cell_id, address);
        cfg.profile = self.profile();
        cfg.channel = self.get("channel").parse().map_err(|_| "bad channel")?;
        cfg.area = self.get("area").parse().map_err(|_| "bad area")?;
        cfg.power_dbm = match self.get("power").as_str() {
            "auto" => None,
            p => Some(p.parse().map_err(|_| "bad power")?),
        };
        cfg.tunnel_port = port(&self.get("tunnel_port"))?;
        cfg.neighbours = parse_neighbours(&self.get("neighbours"))?;
        cfg.idle_after_ms = self.get("idle_after").parse::<u64>().map_err(|_| "bad idle_after")? * 1000;
        let rt = Runtime {
            radio: self.get("radio"),
            core: parse_ip(&self.get("core")).ok_or("bad core address")?,
            core_port: port(&self.get("core_port")).unwrap_or(CONTROL_PORT),
            core_tunnel_port: port(&self.get("core_tunnel_port")).unwrap_or(TUNNEL_PORT),
            admin_port: port(&self.get("admin_port")).unwrap_or(ADMIN_PORT),
        };
        Ok((cfg, rt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "site_id = \"site-a\"\nsecret = 5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e\ncarrier = cell-red\ncore = 10.0.1.1\ncell_id = 7\nchannel = 3\n";

    #[test]
    fn a_full_config_makes_a_site() {
        let c = Conf::parse(GOOD).unwrap();
        let (cfg, rt) = c.site_config().unwrap();
        assert_eq!(cfg.site_id, "site-a");
        assert_eq!(cfg.cell_id, 7);
        assert_eq!(cfg.channel, 3);
        assert_eq!(cfg.address, 0xA000_0007);
        assert_eq!(cfg.power_dbm, None);
        assert_eq!(rt.core, [10, 0, 1, 1]);
        assert_eq!((rt.core_port, rt.core_tunnel_port, rt.admin_port), (7700, 7701, 7710));
        assert_eq!(Conf::parse(&c.to_text()).unwrap(), c, "round trip");
    }

    #[test]
    fn errors_name_the_line_and_the_key() {
        for (bad, want) in [
            ("site_id = a\nchanel = 3\n", "line 2: unknown key 'chanel'"),
            ("channel = 9\n", "line 1: bad channel '9'"),
            ("profile = mmwave\nchannel = 5\n", "line 2: bad channel '5'"),
            ("secret = 1234\n", "line 1: bad secret"),
            ("core = 10.0.1\n", "line 1: bad core"),
            ("power = loud\n", "line 1: bad power"),
            ("neighbours = 2:3:lte,x\n", "line 1: bad neighbours"),
        ] {
            let e = Conf::parse(bad).unwrap_err();
            assert!(e.starts_with(want), "{bad:?}: {e}");
        }
        let c = Conf::parse("site_id = a\n").unwrap();
        assert_eq!(c.missing(), vec!["secret", "carrier", "core", "cell_id"]);
        assert!(c.site_config().unwrap_err().contains("set secret, carrier, core, cell_id"));
    }

    #[test]
    fn set_checks_and_resets() {
        let mut c = Conf::parse(GOOD).unwrap();
        c.set("power", "30").unwrap();
        assert_eq!(c.site_config().unwrap().0.power_dbm, Some(30.0));
        assert!(c.set("power", "999").is_err());
        assert!(c.set("colour", "red").unwrap_err().contains("unknown key"));
        c.set("power", "").unwrap();
        assert_eq!(c.get("power"), "auto");
        assert!(c.set("site_id", "").is_err(), "a required key can't be emptied");
        c.set("channel", "6").unwrap();
        c.set("profile", "mmwave").unwrap();
        assert_eq!(c.get("channel"), "1", "out of the new profile's range");
        c.set("neighbours", "2:3, 5:1:mmwave").unwrap();
        assert_eq!(c.site_config().unwrap().0.neighbours.len(), 2);
    }
}
