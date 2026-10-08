//! Monitor-mode capture on `wlan0`: radiotap + 802.11 frames from the kernel's
//! Wi-Fi control channel (`mon_read`), printed like tcpdump's 802.11 decoder and
//! optionally written to a pcap with link type 127 (IEEE802_11_RADIO), which
//! Wireshark decodes.

use ecm_host_abi::wifi;

pub const LINKTYPE_ETHERNET: u32 = 1;
pub const LINKTYPE_IEEE802_11_RADIO: u32 = 127;

/// A pcap file being written.
pub struct Pcap {
    f: std::fs::File,
}

impl Pcap {
    pub fn create(path: &str, linktype: u32) -> std::io::Result<Pcap> {
        use std::io::Write;
        let mut f = std::fs::File::create(path)?;
        let mut h = Vec::with_capacity(24);
        h.extend_from_slice(&0xa1b2c3d4u32.to_le_bytes());
        h.extend_from_slice(&2u16.to_le_bytes());
        h.extend_from_slice(&4u16.to_le_bytes());
        h.extend_from_slice(&0i32.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h.extend_from_slice(&65535u32.to_le_bytes());
        h.extend_from_slice(&linktype.to_le_bytes());
        f.write_all(&h)?;
        Ok(Pcap { f })
    }

    pub fn write(&mut self, ts_us: i64, data: &[u8]) {
        use std::io::Write;
        let mut r = Vec::with_capacity(16 + data.len());
        r.extend_from_slice(&((ts_us / 1_000_000) as u32).to_le_bytes());
        r.extend_from_slice(&((ts_us % 1_000_000) as u32).to_le_bytes());
        r.extend_from_slice(&(data.len() as u32).to_le_bytes());
        r.extend_from_slice(&(data.len() as u32).to_le_bytes());
        r.extend_from_slice(data);
        let _ = self.f.write_all(&r);
        let _ = self.f.flush();
    }
}

fn mac(b: &[u8]) -> String {
    format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", b[0], b[1], b[2], b[3], b[4], b[5])
}

/// Radiotap fields we print: (tsft_us, rate_500k, freq_mhz, signal_dbm, header_len).
pub fn radiotap(b: &[u8]) -> Option<(u64, u8, u16, i8, usize)> {
    if b.len() < 8 || b[0] != 0 {
        return None;
    }
    let len = u16::from_le_bytes([b[2], b[3]]) as usize;
    let present = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    if len > b.len() || present & 0x8000_0000 != 0 {
        return None;
    }
    let mut off = 8;
    let (mut tsft, mut rate, mut freq, mut sig) = (0u64, 0u8, 0u16, 0i8);
    let align = |off: usize, a: usize| (off + a - 1) / a * a;
    for bit in 0..6 {
        if present & (1 << bit) == 0 {
            continue;
        }
        match bit {
            0 => {
                off = align(off, 8);
                tsft = u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?);
                off += 8;
            }
            1 => off += 1,
            2 => {
                rate = *b.get(off)?;
                off += 1;
            }
            3 => {
                off = align(off, 2);
                freq = u16::from_le_bytes([*b.get(off)?, *b.get(off + 1)?]);
                off += 4;
            }
            4 => off += 2,
            5 => {
                sig = *b.get(off)? as i8;
                off += 1;
            }
            _ => {}
        }
    }
    Some((tsft, rate, freq, sig, len))
}

fn ssid_of(body: &[u8]) -> Option<String> {
    let mut i = 0;
    while i + 2 <= body.len() {
        let (id, l) = (body[i], body[i + 1] as usize);
        if i + 2 + l > body.len() {
            return None;
        }
        if id == 0 {
            return Some(String::from_utf8_lossy(&body[i + 2..i + 2 + l]).into_owned());
        }
        i += 2 + l;
    }
    None
}

/// One-line description of an 802.11 frame (tcpdump's style).
pub fn describe(f: &[u8]) -> String {
    if f.len() < 10 {
        return format!("[|802.11] length {}", f.len());
    }
    let fc = f[0];
    let (ty, st) = ((fc >> 2) & 3, fc >> 4);
    let a1 = mac(&f[4..10]);
    match ty {
        0 if f.len() >= 24 => {
            let (sa, bssid) = (mac(&f[10..16]), mac(&f[16..22]));
            let body = &f[24..];
            let ies = |skip: usize| body.get(skip..).and_then(ssid_of).unwrap_or_default();
            match st {
                0 => format!("Assoc Request ({}) BSSID:{} SA:{}", ies(4), bssid, sa),
                1 => format!(
                    "Assoc Response AID({}) BSSID:{} DA:{}",
                    body.get(4..6).map(|x| u16::from_le_bytes([x[0], x[1]]) & 0x3fff).unwrap_or(0),
                    bssid,
                    a1
                ),
                4 => format!("Probe Request ({}) SA:{} DA:{}", ies(0), sa, a1),
                5 => format!("Probe Response ({}) BSSID:{} DA:{}", ies(12), bssid, a1),
                8 => format!("Beacon ({}) BSSID:{}", ies(12), bssid),
                10 => format!("Disassociation BSSID:{} DA:{}", bssid, a1),
                11 => format!("Authentication BSSID:{} SA:{} DA:{}", bssid, sa, a1),
                12 => format!("DeAuthentication BSSID:{} DA:{}", bssid, a1),
                _ => format!("Mgmt subtype {} BSSID:{}", st, bssid),
            }
        }
        1 => match st {
            13 => format!("Acknowledgment RA:{}", a1),
            11 if f.len() >= 16 => format!("Request-To-Send RA:{} TA:{}", a1, mac(&f[10..16])),
            12 => format!("Clear-To-Send RA:{}", a1),
            _ => format!("Control subtype {} RA:{}", st, a1),
        },
        2 if f.len() >= 24 => {
            let flags = f[1];
            let prot = if flags & 0x40 != 0 { " Data IV (CCMP)" } else { "" };
            let qos = if st & 8 != 0 { "QoS " } else { "" };
            format!("{}Data{} {}->{} BSSID:{} length {}", qos, prot, mac(&f[10..16]), a1, mac(&f[16..22]), f.len())
        }
        _ => format!("[|802.11] type {} length {}", ty, f.len()),
    }
}

pub fn format_ts_us(us: u64) -> String {
    let s = us / 1_000_000;
    format!("{:02}:{:02}:{:02}.{:06}", (s / 3600) % 24, (s / 60) % 60, s % 60, us % 1_000_000)
}

/// Capture until `count` frames (or forever). Returns frames captured.
pub fn capture(count: Option<u64>, mut pcap: Option<Pcap>, hex: bool, quiet: bool) -> u64 {
    let status = wifi::ctl("status").map(|(_, r)| String::from_utf8_lossy(&r).into_owned()).unwrap_or_default();
    if wifi::kv(&status, "present") != Some("1") {
        eprintln!("tcpdump: wlan0: No such device exists");
        std::process::exit(1);
    }
    if wifi::kv(&status, "mode") != Some("monitor") {
        eprintln!("tcpdump: wlan0: not in monitor mode (run: iw dev wlan0 set type monitor)");
        std::process::exit(1);
    }
    let ch = wifi::kv(&status, "channel").unwrap_or("?");
    eprintln!(
        "tcpdump: listening on wlan0, link-type IEEE802_11_RADIO (802.11 plus radiotap header), channel {}, snapshot length 65535 bytes",
        ch
    );
    let mut n = 0u64;
    loop {
        if count.is_some_and(|c| n >= c) {
            break;
        }
        match wifi::ctl("mon_read") {
            Some((1, rt)) => {
                n += 1;
                let Some((tsft, rate, freq, sig, hlen)) = radiotap(&rt) else { continue };
                if let Some(p) = pcap.as_mut() {
                    p.write(tsft as i64, &rt);
                }
                if !quiet {
                    let f = &rt[hlen..];
                    println!(
                        "{} {}.{} Mb/s {} MHz {}dBm signal {}",
                        format_ts_us(tsft),
                        rate / 2,
                        if rate & 1 != 0 { 5 } else { 0 },
                        freq,
                        sig,
                        describe(f)
                    );
                    if hex {
                        for (i, c) in f.chunks(16).enumerate() {
                            let h: Vec<String> = c.iter().map(|b| format!("{:02x}", b)).collect();
                            println!("\t0x{:04x}:  {}", i * 16, h.join(" "));
                        }
                    }
                }
            }
            Some(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
            None => {
                eprintln!("tcpdump: wlan0: capture failed");
                std::process::exit(1);
            }
        }
    }
    n
}
