//! `wpa_supplicant.conf`: global `key=value` lines and `network={ ... }` blocks.
//!
//! ```text
//! ctrl_interface=/run/wpa_supplicant
//! network={
//!     ssid="ecm-lab"           # or ssid=65636d2d6c6162 (hex)
//!     psk="correct horse"      # passphrase (8..63 chars), or psk=<64 hex digits>
//!     key_mgmt=WPA-PSK         # NONE for an open network (default: WPA-PSK if psk is set)
//!     bssid=02:aa:00:00:00:01  # optional: lock to one AP
//!     priority=5               # optional: higher wins
//!     disabled=1               # optional
//! }
//! ```
//! Parsed into [`ConfNetwork`]s (kept as written, so `wpa_cli` can edit and save the
//! file) and turned into `ecm_wifi` [`NetworkConfig`]s (the PSK is derived here).

use ecm_wifi::frame::MacAddr;
use ecm_wifi::supplicant::NetworkConfig;

/// The pre-shared key as written in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Psk {
    Passphrase(String),
    Hex(String),
}

/// One `network={}` block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfNetwork {
    pub ssid: Vec<u8>,
    pub psk: Option<Psk>,
    /// `key_mgmt=NONE`.
    pub open: bool,
    pub bssid: Option<MacAddr>,
    pub priority: i32,
    pub disabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conf {
    /// Global lines (`ctrl_interface=...`, `country=...`), kept verbatim.
    pub globals: Vec<String>,
    pub networks: Vec<ConfNetwork>,
}

/// A parse error with its line number (1-based).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfError {
    pub line: usize,
    pub msg: String,
}

impl core::fmt::Display for ConfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

fn err(line: usize, msg: impl Into<String>) -> ConfError {
    ConfError { line, msg: msg.into() }
}

/// A quoted string (`"..."`) or hex bytes.
pub fn parse_string_value(v: &str) -> Option<Vec<u8>> {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        return Some(v[1..v.len() - 1].as_bytes().to_vec());
    }
    ecm_host_abi::wifi::unhex(v)
}

fn strip_comment(l: &str) -> &str {
    // '#' starts a comment unless inside quotes.
    let mut q = false;
    for (i, c) in l.char_indices() {
        match c {
            '"' => q = !q,
            '#' if !q => return &l[..i],
            _ => {}
        }
    }
    l
}

/// Set one network variable (`wpa_cli set_network <id> <var> <value>` uses this too).
pub fn set_var(n: &mut ConfNetwork, key: &str, value: &str) -> Result<(), String> {
    let value = value.trim();
    match key {
        "ssid" => {
            let s = parse_string_value(value).ok_or("ssid must be \"text\" or hex")?;
            if s.is_empty() || s.len() > 32 {
                return Err("ssid must be 1..32 bytes".into());
            }
            n.ssid = s;
        }
        "psk" => {
            if value.starts_with('"') {
                let p = parse_string_value(value).ok_or("bad psk")?;
                let p = String::from_utf8(p).map_err(|_| "psk must be ASCII")?;
                if p.len() < 8 || p.len() > 63 || !p.bytes().all(|c| (0x20..=0x7e).contains(&c)) {
                    return Err("passphrase must be 8..63 printable ASCII characters".into());
                }
                n.psk = Some(Psk::Passphrase(p));
            } else if value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit()) {
                n.psk = Some(Psk::Hex(value.to_ascii_lowercase()));
            } else {
                return Err("psk must be \"passphrase\" or 64 hex digits".into());
            }
        }
        "key_mgmt" => match value {
            "NONE" => n.open = true,
            "WPA-PSK" => n.open = false,
            _ => return Err(format!("unsupported key_mgmt {}", value)),
        },
        "bssid" => {
            n.bssid = Some(crate::parse_mac(value).ok_or("bad bssid")?);
        }
        "priority" => n.priority = value.parse().map_err(|_| "priority must be a number")?,
        "disabled" => n.disabled = value == "1",
        // Accepted for compatibility; WPA2/CCMP is all there is.
        "proto" | "pairwise" | "group" | "scan_ssid" | "id_str" | "auth_alg" => {}
        _ => return Err(format!("unknown network variable '{}'", key)),
    }
    Ok(())
}

pub fn parse(text: &str) -> Result<Conf, ConfError> {
    let mut conf = Conf::default();
    let mut cur: Option<(ConfNetwork, usize)> = None;
    for (i, raw) in text.lines().enumerate() {
        let ln = i + 1;
        let l = strip_comment(raw).trim();
        if l.is_empty() {
            continue;
        }
        if let Some((n, start)) = cur.as_mut() {
            if l == "}" {
                let (n, start) = (n.clone(), *start);
                // A disabled block may be incomplete (`wpa_cli add_network` creates one).
                if !n.disabled {
                    if n.ssid.is_empty() {
                        return Err(err(start, "network block without ssid"));
                    }
                    if !n.open && n.psk.is_none() {
                        return Err(err(start, "network block needs psk= or key_mgmt=NONE"));
                    }
                }
                conf.networks.push(n);
                cur = None;
                continue;
            }
            let (k, v) = l.split_once('=').ok_or_else(|| err(ln, format!("expected key=value, got '{}'", l)))?;
            set_var(n, k.trim(), v).map_err(|m| err(ln, m))?;
            continue;
        }
        if l.replace(' ', "") == "network={" {
            cur = Some((ConfNetwork::default(), ln));
            continue;
        }
        if l.contains('=') {
            conf.globals.push(l.to_string());
            continue;
        }
        return Err(err(ln, format!("unexpected '{}'", l)));
    }
    if let Some((_, start)) = cur {
        return Err(err(start, "network block not closed"));
    }
    Ok(conf)
}

fn quote_or_hex(b: &[u8]) -> String {
    if !b.is_empty() && b.iter().all(|&c| (0x20..=0x7e).contains(&c) && c != b'"') {
        format!("\"{}\"", String::from_utf8_lossy(b))
    } else {
        ecm_host_abi::wifi::hex(b)
    }
}

/// Write the file back (comments are not preserved).
pub fn render(c: &Conf) -> String {
    let mut s = String::new();
    for g in &c.globals {
        s += g;
        s += "\n";
    }
    for n in &c.networks {
        s += "\nnetwork={\n";
        if !n.ssid.is_empty() {
            s += &format!("\tssid={}\n", quote_or_hex(&n.ssid));
        }
        match &n.psk {
            Some(Psk::Passphrase(p)) => s += &format!("\tpsk=\"{}\"\n", p),
            Some(Psk::Hex(h)) => s += &format!("\tpsk={}\n", h),
            None => {}
        }
        if n.open {
            s += "\tkey_mgmt=NONE\n";
        }
        if let Some(b) = n.bssid {
            s += &format!("\tbssid={}\n", ecm_wifi::frame::mac_str(&b));
        }
        if n.priority != 0 {
            s += &format!("\tpriority={}\n", n.priority);
        }
        if n.disabled {
            s += "\tdisabled=1\n";
        }
        s += "}\n";
    }
    s
}

impl ConfNetwork {
    /// The supplicant's view (derives the PMK from a passphrase: 4096 PBKDF2 rounds).
    pub fn to_network_config(&self) -> Result<NetworkConfig, String> {
        if self.ssid.is_empty() {
            return Err("no ssid".into());
        }
        let mut n = if self.open {
            NetworkConfig::open(&self.ssid)
        } else {
            match &self.psk {
                Some(Psk::Passphrase(p)) => NetworkConfig::wpa2_passphrase(&self.ssid, p),
                Some(Psk::Hex(h)) => NetworkConfig::wpa2_psk_hex(&self.ssid, h),
                None => return Err("no psk".into()),
            }
        }
        .map_err(|e| format!("{:?}", e))?;
        n.bssid = self.bssid;
        n.priority = self.priority;
        n.disabled = self.disabled;
        Ok(n)
    }

    /// `wpa_cli list_networks` flags column.
    pub fn flags(&self) -> &'static str {
        if self.disabled {
            "[DISABLED]"
        } else {
            ""
        }
    }
}
