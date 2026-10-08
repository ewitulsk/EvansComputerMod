//! Wi-Fi control channel to the kernel's `wlan0` (see the kernel's `net/wifi.rs`).
//!
//! One text request per call (e.g. `scan`, `scan_results`, `connect <ssid_hex> wpa2`,
//! `install_ptk <bssid> <tk_hex>`, `events <after_seq>`, `mon_read`); the kernel
//! answers with a status and a reply (text, or a binary frame for `eapol_rx` /
//! `mon_read`). Used by `iw`, `wpa_supplicant`, `wpa_cli` and `tcpdump -i wlan0`.

use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn wifi_ctl(req_ptr: i32, req_len: i32, reply_ptr: i32, reply_cap: i32, status_ptr: i32) -> i32;
}

/// Kernel status codes.
pub const ENODEV: i32 = -19;
pub const EBUSY: i32 = -16;
pub const EINVAL: i32 = -22;
pub const ENOTCONN: i32 = -107;

/// Largest reply the kernel produces (its IPC result region minus the status word).
pub const REPLY_CAP: usize = 8188;

/// Send one request. Returns `(status, reply)`; `None` if the call itself failed
/// (no kernel, not running on ECM).
pub fn ctl(req: &str) -> Option<(i32, Vec<u8>)> {
    ctl_bytes(req.as_bytes())
}

#[cfg(target_arch = "wasm32")]
pub fn ctl_bytes(req: &[u8]) -> Option<(i32, Vec<u8>)> {
    let mut reply = alloc::vec![0u8; REPLY_CAP];
    let mut status: i32 = 0;
    let n = unsafe {
        wifi_ctl(
            req.as_ptr() as i32,
            req.len() as i32,
            reply.as_mut_ptr() as i32,
            reply.len() as i32,
            &mut status as *mut i32 as i32,
        )
    };
    if n < 0 {
        return None;
    }
    reply.truncate(n as usize);
    Some((status, reply))
}

/// Off-target builds (host unit tests) have no kernel.
#[cfg(not(target_arch = "wasm32"))]
pub fn ctl_bytes(_req: &[u8]) -> Option<(i32, Vec<u8>)> {
    None
}

/// Lower-case hex of `b`.
pub fn hex(b: &[u8]) -> alloc::string::String {
    let mut s = alloc::string::String::with_capacity(b.len() * 2);
    for x in b {
        let d = b"0123456789abcdef";
        s.push(d[(x >> 4) as usize] as char);
        s.push(d[(x & 15) as usize] as char);
    }
    s
}

/// Decode hex (either case); `None` on odd length or a non-hex digit.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.as_bytes();
    if s.len() % 2 != 0 {
        return None;
    }
    let v = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(s.len() / 2);
    for c in s.chunks(2) {
        out.push(v(c[0])? << 4 | v(c[1])?);
    }
    Some(out)
}

/// One line of `scan_results`: `bssid freq signal_dbm security channel ssid_hex`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanEntry {
    pub bssid: alloc::string::String,
    pub freq: u32,
    pub signal_dbm: i32,
    /// `open`, `wpa2-psk` or `unsupported`.
    pub security: alloc::string::String,
    pub channel: u32,
    /// Raw SSID bytes (empty for a hidden network).
    pub ssid: Vec<u8>,
}

impl ScanEntry {
    pub fn parse_line(l: &str) -> Option<ScanEntry> {
        let t: Vec<&str> = l.split_whitespace().collect();
        if t.len() < 6 {
            return None;
        }
        Some(ScanEntry {
            bssid: t[0].into(),
            freq: t[1].parse().ok()?,
            signal_dbm: t[2].parse().ok()?,
            security: t[3].into(),
            channel: t[4].parse().ok()?,
            ssid: if t[5] == "-" { Vec::new() } else { unhex(t[5])? },
        })
    }

    pub fn parse_all(text: &str) -> Vec<ScanEntry> {
        text.lines().filter_map(ScanEntry::parse_line).collect()
    }

    /// SSID for display (lossy UTF-8).
    pub fn ssid_str(&self) -> alloc::string::String {
        alloc::string::String::from_utf8_lossy(&self.ssid).into_owned()
    }
}

/// One line of `events`: `<seq> <NAME> key=value ...`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiEvent {
    pub seq: u64,
    pub name: alloc::string::String,
    pub fields: Vec<(alloc::string::String, alloc::string::String)>,
}

impl WifiEvent {
    pub fn parse_line(l: &str) -> Option<WifiEvent> {
        let mut it = l.split_whitespace();
        let seq = it.next()?.parse().ok()?;
        let name = it.next()?.into();
        let fields = it
            .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
            .collect();
        Some(WifiEvent { seq, name, fields })
    }

    pub fn parse_all(text: &str) -> Vec<WifiEvent> {
        text.lines().filter_map(WifiEvent::parse_line).collect()
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.fields.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }

    /// A hex-valued field (`ssid`, `ap_rsn`, `sta_rsn`); `-` means absent.
    pub fn hex_field(&self, k: &str) -> Option<Vec<u8>> {
        self.get(k).filter(|v| *v != "-").and_then(unhex)
    }
}

/// `key=value` lines (`status`, `stats`) → lookup.
pub fn kv<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(hex(b"ecm-lab"), "65636d2d6c6162");
        assert_eq!(unhex("65636D2d6c6162").unwrap(), b"ecm-lab");
        assert!(unhex("abc").is_none());
        assert!(unhex("zz").is_none());
    }

    #[test]
    fn parses_scan_lines_events_and_kv() {
        let e = ScanEntry::parse_line("02:aa:00:00:00:01 2437 -48 wpa2-psk 6 65636d2d6c6162").unwrap();
        assert_eq!((e.freq, e.signal_dbm, e.channel), (2437, -48, 6));
        assert_eq!(e.ssid_str(), "ecm-lab");
        assert_eq!(ScanEntry::parse_line("02:aa:00:00:00:02 2412 -70 open 1 -").unwrap().ssid, b"");
        assert!(ScanEntry::parse_line("garbage").is_none());
        let ev = WifiEvent::parse_line("7 CONNECTED bssid=02:aa:00:00:00:01 ssid=65636d ap_rsn=- roamed=0").unwrap();
        assert_eq!((ev.seq, ev.name.as_str()), (7, "CONNECTED"));
        assert_eq!(ev.get("bssid"), Some("02:aa:00:00:00:01"));
        assert_eq!(ev.hex_field("ssid").unwrap(), b"ecm");
        assert_eq!(ev.hex_field("ap_rsn"), None);
        assert_eq!(kv("present=1
mode=monitor
", "mode"), Some("monitor"));
        assert_eq!(kv("present=1
", "mode"), None);
    }

    #[test]
    fn off_target_ctl_reports_no_kernel() {
        assert!(ctl("status").is_none());
    }
}
