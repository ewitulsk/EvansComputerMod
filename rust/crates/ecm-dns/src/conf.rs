//! `/etc/named.conf`: one statement per line, `#` comments.
//!
//! ```text
//! recursion yes                      # answer RD queries for clients in `allow`
//! allow 100.64.0.0/10 10.0.0.0/8     # who may use recursion (default: private ranges)
//! root-hints /etc/named/root.hints   # where iteration starts
//! fallback 1.1.1.1                   # names the in-game root does not know
//! zone v3.ring primary /etc/named/v3.ring.zone
//! zone ecm primary /etc/named/ecm.zone notify 100.71.0.55
//! zone ring secondary 100.68.0.55            # file /etc/named/sec/ring.zone
//! zone . secondary 100.66.0.57 file /etc/named/sec/root.zone
//! allow-transfer 100.64.0.0/10       # who may AXFR (default: any)
//! listen 100.67.0.53                 # address to answer on (default: all)
//! port 53
//! ```

use crate::name::{normalize, parse_cidr, parse_ip, Ip};

pub const PATH: &str = "/etc/named.conf";
/// UDP port of `named reload | cache | flush | status` on 127.0.0.1 (like rndc's 953).
pub const CONTROL_PORT: u16 = 953;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZoneKind {
    Primary { file: String, notify: Vec<Ip> },
    Secondary { primary: Ip, file: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneConf {
    pub name: String,
    pub kind: ZoneKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedConf {
    pub recursion: bool,
    pub allow: Vec<(Ip, u8)>,
    pub root_hints: Option<String>,
    pub fallback: Vec<Ip>,
    pub zones: Vec<ZoneConf>,
    /// Empty: anyone.
    pub allow_transfer: Vec<(Ip, u8)>,
    pub listen: Option<Ip>,
    pub port: u16,
}

pub fn default_allow() -> Vec<(Ip, u8)> {
    ["127.0.0.0/8", "10.0.0.0/8", "100.64.0.0/10", "172.16.0.0/12", "192.168.0.0/16"]
        .iter()
        .filter_map(|c| parse_cidr(c))
        .collect()
}

/// Where a secondary keeps its copy by default (`root` for the root zone).
pub fn secondary_file(zone: &str) -> String {
    format!("/etc/named/sec/{}.zone", if zone.is_empty() { "root" } else { zone })
}

impl NamedConf {
    pub fn parse(text: &str) -> Result<NamedConf, String> {
        let mut c = NamedConf {
            recursion: false,
            allow: Vec::new(),
            root_hints: None,
            fallback: Vec::new(),
            zones: Vec::new(),
            allow_transfer: Vec::new(),
            listen: None,
            port: 53,
        };
        let mut allow_set = false;
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let err = |m: String| format!("named.conf line {}: {}", n + 1, m);
            let t: Vec<&str> = line.split_whitespace().collect();
            let ips = |list: &[&str]| -> Result<Vec<Ip>, String> {
                list.iter().map(|s| parse_ip(s).ok_or_else(|| err(format!("bad address '{s}'")))).collect()
            };
            let cidrs = |list: &[&str]| -> Result<Vec<(Ip, u8)>, String> {
                list.iter()
                    .filter(|s| **s != "any")
                    .map(|s| parse_cidr(s).ok_or_else(|| err(format!("bad network '{s}'"))))
                    .collect()
            };
            match t[0] {
                "recursion" => {
                    c.recursion = match t.get(1).copied() {
                        Some("yes") | Some("on") => true,
                        Some("no") | Some("off") => false,
                        _ => return Err(err("recursion yes|no".into())),
                    }
                }
                "allow" | "allow-recursion" => {
                    allow_set = true;
                    if t[1..].contains(&"any") {
                        c.allow.push(([0, 0, 0, 0], 0));
                    }
                    c.allow.extend(cidrs(&t[1..])?);
                }
                "root-hints" => c.root_hints = Some(t.get(1).ok_or_else(|| err("root-hints FILE".into()))?.to_string()),
                "fallback" | "forwarders" => c.fallback.extend(ips(&t[1..])?),
                "allow-transfer" => c.allow_transfer.extend(cidrs(&t[1..])?),
                "listen" => c.listen = Some(ips(&t[1..2])?.first().copied().ok_or_else(|| err("listen ADDRESS".into()))?),
                "port" => c.port = t.get(1).and_then(|p| p.parse().ok()).ok_or_else(|| err("port NUMBER".into()))?,
                "zone" => {
                    if t.len() < 4 {
                        return Err(err("zone NAME primary FILE | zone NAME secondary ADDRESS".into()));
                    }
                    let name = normalize(t[1]).ok_or_else(|| err(format!("bad zone name '{}'", t[1])))?;
                    if c.zones.iter().any(|z| z.name == name) {
                        return Err(err(format!("zone {} twice", t[1])));
                    }
                    let kind = match t[2] {
                        "primary" | "master" => {
                            let mut notify = Vec::new();
                            match t.get(4) {
                                Some(&"notify") => notify = ips(&t[5..])?,
                                Some(o) => return Err(err(format!("unexpected '{o}'"))),
                                None => {}
                            }
                            ZoneKind::Primary { file: t[3].to_string(), notify }
                        }
                        "secondary" | "slave" => {
                            let primary = parse_ip(t[3]).ok_or_else(|| err(format!("bad primary address '{}'", t[3])))?;
                            let file = match (t.get(4), t.get(5)) {
                                (Some(&"file"), Some(f)) => f.to_string(),
                                (None, _) => secondary_file(&name),
                                _ => return Err(err("zone NAME secondary ADDRESS [file FILE]".into())),
                            };
                            ZoneKind::Secondary { primary, file }
                        }
                        k => return Err(err(format!("zone type '{k}' (primary or secondary)"))),
                    };
                    c.zones.push(ZoneConf { name, kind });
                }
                other => return Err(err(format!("unknown statement '{other}'"))),
            }
        }
        if !allow_set {
            c.allow = default_allow();
        }
        if c.recursion && c.root_hints.is_none() {
            return Err("named.conf: recursion needs root-hints".into());
        }
        Ok(c)
    }

    pub fn may_recurse(&self, client: Ip) -> bool {
        self.recursion && self.allow.iter().any(|n| crate::name::in_cidr(client, *n))
    }

    pub fn may_transfer(&self, client: Ip) -> bool {
        self.allow_transfer.is_empty() || self.allow_transfer.iter().any(|n| crate::name::in_cidr(client, *n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        let c = NamedConf::parse(
            "# resolver\nrecursion yes\nroot-hints /etc/named/root.hints\nfallback 1.1.1.1\nzone v3.ring primary /etc/named/v3.ring.zone\nzone ecm primary /etc/named/ecm.zone notify 100.71.0.55 100.72.0.55\nzone ring secondary 100.68.0.55\nzone . secondary 100.66.0.57 file /etc/r.zone\nallow-transfer 100.64.0.0/10\n",
        )
        .unwrap();
        assert!(c.may_recurse([100, 70, 0, 1]) && !c.may_recurse([8, 8, 8, 8]));
        assert_eq!(c.zones.len(), 4);
        assert_eq!(c.zones[1].kind, ZoneKind::Primary { file: "/etc/named/ecm.zone".into(), notify: vec![[100, 71, 0, 55], [100, 72, 0, 55]] });
        assert_eq!(c.zones[2].kind, ZoneKind::Secondary { primary: [100, 68, 0, 55], file: "/etc/named/sec/ring.zone".into() });
        assert_eq!(c.zones[3].name, "");
        assert!(c.may_transfer([100, 65, 0, 1]) && !c.may_transfer([1, 1, 1, 1]));
        assert_eq!(c.fallback, vec![[1, 1, 1, 1]]);
    }

    #[test]
    fn errors() {
        assert!(NamedConf::parse("recursion yes\n").is_err(), "no hints");
        assert!(NamedConf::parse("zone x primary\n").is_err());
        assert!(NamedConf::parse("zone x weird f\n").is_err());
        assert!(NamedConf::parse("zone x primary a\nzone X primary b\n").is_err());
        assert!(NamedConf::parse("bogus 1\n").unwrap_err().contains("line 1"));
        let c = NamedConf::parse("recursion no\n").unwrap();
        assert!(!c.may_recurse([127, 0, 0, 1]));
        assert!(c.may_transfer([1, 2, 3, 4]));
    }
}
