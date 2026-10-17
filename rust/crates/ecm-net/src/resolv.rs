//! The stub resolver's configuration files, as on Linux:
//!
//! * `/etc/hosts`: `address name [aliases...]`, looked at first (`localhost` too);
//! * `/etc/resolv.conf`: `nameserver` (up to 3, tried in order), `search` (domains
//!   appended to short names), `domain` (a one-entry search list), `options ndots:N`.
//!
//! The kernel writes `resolv.conf` from DHCP and `resolvectl dns` (marked with
//! [`KERNEL_MARK`]); a file without the mark was written by the player and is left alone.

use crate::types::Ipv4Addr;

pub const RESOLV_CONF: &str = "etc/resolv.conf";
pub const HOSTS: &str = "etc/hosts";
pub const MAX_NAMESERVERS: usize = 3;
pub const MAX_SEARCH: usize = 6;
pub const KERNEL_MARK: &str = "# Written by the kernel";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvConf {
    pub nameservers: Vec<Ipv4Addr>,
    pub search: Vec<String>,
    pub ndots: u8,
}

impl ResolvConf {
    pub fn parse(text: &str) -> ResolvConf {
        let mut c = ResolvConf { ndots: 1, ..Default::default() };
        for line in text.lines() {
            let line = line.split(['#', ';']).next().unwrap_or("").trim();
            let mut w = line.split_whitespace();
            match w.next() {
                Some("nameserver") => {
                    if let Some(ip) = w.next().and_then(Ipv4Addr::parse) {
                        if c.nameservers.len() < MAX_NAMESERVERS && !ip.is_unspecified() {
                            c.nameservers.push(ip);
                        }
                    }
                }
                Some("search") => {
                    c.search = w.filter_map(clean_domain).take(MAX_SEARCH).collect();
                }
                Some("domain") => {
                    c.search = w.next().and_then(clean_domain).into_iter().collect();
                }
                Some("options") => {
                    for o in w {
                        if let Some(n) = o.strip_prefix("ndots:").and_then(|n| n.parse::<u8>().ok()) {
                            c.ndots = n.min(15);
                        }
                    }
                }
                _ => {}
            }
        }
        c
    }

    /// What the kernel writes after DHCP or `resolvectl dns`.
    pub fn render_kernel(server: Ipv4Addr, domain: &str) -> String {
        let mut s = format!("{KERNEL_MARK} from DHCP / resolvectl. Edit it and it is yours.\n");
        if !server.is_unspecified() {
            s.push_str(&format!("nameserver {server}\n"));
        }
        if !domain.is_empty() {
            s.push_str(&format!("search {domain}\n"));
        }
        s
    }

    /// The names to try for `name`, in order (`man resolv.conf`): a name with a
    /// trailing dot only as it is; one with at least `ndots` dots as it is first, then
    /// with each search domain; a shorter one with the search domains first.
    pub fn candidates(&self, name: &str) -> Vec<String> {
        if let Some(abs) = name.strip_suffix('.') {
            return vec![abs.to_string()];
        }
        let dots = name.bytes().filter(|&b| b == b'.').count();
        let searched: Vec<String> = self.search.iter().map(|d| format!("{name}.{d}")).filter(|n| n.len() <= 253).collect();
        let mut out = Vec::new();
        if dots >= self.ndots as usize {
            out.push(name.to_string());
            out.extend(searched);
        } else {
            out.extend(searched);
            out.push(name.to_string());
        }
        out
    }
}

fn clean_domain(d: &str) -> Option<String> {
    let d = d.trim_end_matches('.').to_ascii_lowercase();
    (!d.is_empty() && d.len() <= 253 && d.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.' || c == b'_'))
        .then_some(d)
}

/// `/etc/hosts` lookup (case-insensitive, a trailing dot ignored).
pub fn hosts_lookup(text: &str, name: &str) -> Option<Ipv4Addr> {
    let name = name.strip_suffix('.').unwrap_or(name);
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let mut w = line.split_whitespace();
        let Some(ip) = w.next().and_then(Ipv4Addr::parse) else { continue };
        if w.any(|n| n.trim_end_matches('.').eq_ignore_ascii_case(name)) {
            return Some(ip);
        }
    }
    None
}

/// The default `/etc/hosts`.
pub const DEFAULT_HOSTS: &str = "127.0.0.1 localhost\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolv_conf() {
        let c = ResolvConf::parse("# x\nnameserver 100.67.0.53\nnameserver 1.1.1.1\nnameserver 9.9.9.9\nnameserver 8.8.8.8\nsearch v3.ring ring.\noptions ndots:2 rotate\n");
        assert_eq!(c.nameservers.len(), 3);
        assert_eq!(c.search, vec!["v3.ring".to_string(), "ring".to_string()]);
        assert_eq!(c.ndots, 2);
        let c = ResolvConf::parse("nameserver 100.67.0.53\nsearch v3.ring\n");
        assert_eq!(c.candidates("web"), vec!["web.v3.ring", "web"]);
        assert_eq!(c.candidates("web.v4.ring"), vec!["web.v4.ring", "web.v4.ring.v3.ring"]);
        assert_eq!(c.candidates("web."), vec!["web"]);
        let k = ResolvConf::parse(&ResolvConf::render_kernel(Ipv4Addr::new(100, 67, 0, 53), "v3.ring"));
        assert_eq!(k, ResolvConf { nameservers: vec![Ipv4Addr::new(100, 67, 0, 53)], search: vec!["v3.ring".into()], ndots: 1 });
        assert!(ResolvConf::render_kernel(Ipv4Addr::ZERO, "").starts_with(KERNEL_MARK));
    }

    #[test]
    fn hosts() {
        let h = "127.0.0.1 localhost\n10.0.0.5 nas nas.home # my box\nbad line\n";
        assert_eq!(hosts_lookup(h, "NAS.home."), Some(Ipv4Addr::new(10, 0, 0, 5)));
        assert_eq!(hosts_lookup(h, "localhost"), Some(Ipv4Addr::LOCALHOST));
        assert_eq!(hosts_lookup(h, "box"), None);
    }
}
