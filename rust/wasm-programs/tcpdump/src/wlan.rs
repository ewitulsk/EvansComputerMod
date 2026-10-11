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

/// The data rate a radiotap header reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    /// None given.
    Unknown,
    /// Legacy (Rate field, bit 2): 500 kb/s units.
    Legacy(u8),
    /// HT (MCS field, bit 19): index, 40 MHz wide, short guard interval.
    Ht { mcs: u8, bw40: bool, short_gi: bool },
}

impl Rate {
    /// Data rate in kb/s, if known. HT: MCS 0-31 (1-4 streams) and MCS 32.
    pub fn kbps(self) -> Option<u32> {
        match self {
            Rate::Unknown => None,
            Rate::Legacy(r) => Some(r as u32 * 500),
            Rate::Ht { mcs, bw40, short_gi } => {
                // One stream, long GI: 20 MHz has 52 data subcarriers, 40 MHz 108.
                const BW20: [u32; 8] = [6_500, 13_000, 19_500, 26_000, 39_000, 52_000, 58_500, 65_000];
                const BW40: [u32; 8] = [13_500, 27_000, 40_500, 54_000, 81_000, 108_000, 121_500, 135_000];
                let long = match mcs {
                    0..=31 => (if bw40 { BW40 } else { BW20 })[(mcs % 8) as usize] * (mcs as u32 / 8 + 1),
                    32 => 6_000, // 40 MHz duplicate BPSK 1/2
                    _ => return None,
                };
                // Short GI: 3.6 us symbols instead of 4 us.
                Some(if short_gi { (long * 10 + 4) / 9 } else { long })
            }
        }
    }

    /// `54.0 Mb/s`, `65.0 Mb/s MCS 7 20 MHz long GI`, or `? Mb/s`.
    pub fn text(self) -> String {
        let mbps = match self.kbps() {
            // one decimal, rounded
            Some(k) => {
                let tenths = (k + 50) / 100;
                format!("{}.{}", tenths / 10, tenths % 10)
            }
            None => "?".into(),
        };
        match self {
            Rate::Ht { mcs, bw40, short_gi } => format!(
                "{mbps} Mb/s MCS {mcs} {} MHz {} GI",
                if bw40 { 40 } else { 20 },
                if short_gi { "short" } else { "long" }
            ),
            _ => format!("{mbps} Mb/s"),
        }
    }
}

/// (size, alignment) of radiotap fields 0..=19 (19 = MCS).
const FIELDS: [(usize, usize); 20] = [
    (8, 8), // 0 TSFT
    (1, 1), // 1 Flags
    (1, 1), // 2 Rate
    (4, 2), // 3 Channel
    (2, 2), // 4 FHSS
    (1, 1), // 5 dBm antenna signal
    (1, 1), // 6 dBm antenna noise
    (2, 2), // 7 Lock quality
    (2, 2), // 8 TX attenuation
    (2, 2), // 9 dB TX attenuation
    (1, 1), // 10 dBm TX power
    (1, 1), // 11 Antenna
    (1, 1), // 12 dB antenna signal
    (1, 1), // 13 dB antenna noise
    (2, 2), // 14 RX flags
    (2, 2), // 15 TX flags
    (1, 1), // 16 RTS retries
    (1, 1), // 17 data retries
    (8, 4), // 18 XChannel
    (3, 1), // 19 MCS: known, flags, index
];

/// Radiotap fields we print: (tsft_us, rate, freq_mhz, signal_dbm, header_len).
pub fn radiotap(b: &[u8]) -> Option<(u64, Rate, u16, i8, usize)> {
    if b.len() < 8 || b[0] != 0 {
        return None;
    }
    let len = u16::from_le_bytes([b[2], b[3]]) as usize;
    let present = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    if len > b.len() || present & 0x8000_0000 != 0 {
        return None;
    }
    let mut off = 8;
    let (mut tsft, mut rate, mut freq, mut sig) = (0u64, Rate::Unknown, 0u16, 0i8);
    let align = |off: usize, a: usize| (off + a - 1) / a * a;
    for (bit, &(size, a)) in FIELDS.iter().enumerate() {
        if present & (1 << bit) == 0 {
            continue;
        }
        off = align(off, a);
        let f = b.get(off..off + size)?;
        match bit {
            0 => tsft = u64::from_le_bytes(f.try_into().ok()?),
            2 => rate = Rate::Legacy(f[0]),
            3 => freq = u16::from_le_bytes([f[0], f[1]]),
            5 => sig = f[0] as i8,
            19 => {
                // known: 0x01 bandwidth, 0x02 MCS index, 0x04 guard interval;
                // flags: bits 0-1 bandwidth (1 = 40 MHz), bit 2 short GI.
                let (known, flags, mcs) = (f[0], f[1], f[2]);
                if known & 0x02 != 0 {
                    let bw40 = known & 0x01 != 0 && flags & 0x03 == 1;
                    let short_gi = known & 0x04 != 0 && flags & 0x04 != 0;
                    rate = Rate::Ht { mcs: mcs & 0x7f, bw40, short_gi };
                }
            }
            _ => {}
        }
        off += size;
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

/// The radio part of a capture line: `<rate> Mb/s [MCS ...] <freq> MHz <signal>dBm signal`.
pub fn radio_text(rate: Rate, freq: u16, sig: i8) -> String {
    format!("{} {} MHz {}dBm signal", rate.text(), freq, sig)
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
                    println!("{} {} {}", format_ts_us(tsft), radio_text(rate, freq, sig), describe(f));
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

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_wifi::radiotap::encapsulate;

    fn line(rt: &[u8]) -> String {
        let (_, rate, freq, sig, _) = radiotap(rt).unwrap();
        radio_text(rate, freq, sig)
    }

    /// HT frames carry the radiotap MCS field (bit 19: known, flags, index)
    /// instead of Rate; the shown rate comes from the MCS index, bandwidth
    /// and guard interval (one stream, 20 MHz, long GI: 6.5..65 Mb/s).
    #[test]
    fn ht_rate_comes_from_the_mcs_field() {
        let beacon = [0x80u8, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 2, 0xaa, 0, 0, 0, 1, 2, 0xaa, 0, 0, 0, 1, 0, 0];
        for (code, want) in [(0x80u8, "6.5 Mb/s"), (0x83, "26.0 Mb/s"), (0x87, "65.0 Mb/s MCS 7 20 MHz long GI")] {
            let rt = encapsulate(&beacon, 1_000_000, code, 6, -48, false);
            let l = line(&rt);
            assert!(l.starts_with(want), "code {code:#x}: {l}");
            assert!(l.ends_with("2437 MHz -48dBm signal"), "{l}");
            assert_eq!(&rt[radiotap(&rt).unwrap().4..], &beacon);
        }
        // Legacy rates are unchanged: 54 Mb/s, 5.5 Mb/s.
        assert_eq!(line(&encapsulate(&beacon, 0, 108, 6, -40, false)), "54.0 Mb/s 2437 MHz -40dBm signal");
        assert_eq!(line(&encapsulate(&beacon, 0, 11, 1, -40, false)), "5.5 Mb/s 2412 MHz -40dBm signal");
        // 40 MHz / short GI / two streams, from the MCS flags.
        let ht = |mcs, bw40, short_gi| Rate::Ht { mcs, bw40, short_gi }.text();
        assert_eq!(ht(7, false, true), "72.2 Mb/s MCS 7 20 MHz short GI");
        assert_eq!(ht(7, true, false), "135.0 Mb/s MCS 7 40 MHz long GI");
        assert_eq!(ht(15, true, true), "300.0 Mb/s MCS 15 40 MHz short GI");
        assert_eq!(ht(0, false, false), "6.5 Mb/s MCS 0 20 MHz long GI");
        assert_eq!(Rate::Ht { mcs: 77, bw40: false, short_gi: false }.text(), "? Mb/s MCS 77 20 MHz long GI");
    }
}
