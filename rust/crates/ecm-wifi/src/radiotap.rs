//! Radiotap header writer and parser (pcap link type 127, `LINKTYPE_IEEE802_11_RADIOTAP`).
//!
//! The writer emits the fields Wireshark needs to show a monitor-mode capture: TSFT,
//! Flags, Rate (legacy) or MCS (HT), Channel and antenna signal. The parser walks any
//! radiotap header with the standard field alignments for bits 0..=22, honours extended
//! presence bitmaps, and skips unknown trailing fields using the header length.

use crate::frame;

pub const LINKTYPE_IEEE802_11_RADIOTAP: u32 = 127;

const P_TSFT: u32 = 1 << 0;
const P_FLAGS: u32 = 1 << 1;
const P_RATE: u32 = 1 << 2;
const P_CHANNEL: u32 = 1 << 3;
const P_DBM_SIGNAL: u32 = 1 << 5;
const P_DBM_TX_POWER: u32 = 1 << 10;
const P_MCS: u32 = 1 << 19;
const P_EXT: u32 = 1 << 31;

/// Flags field: frame includes FCS at the end.
pub const F_FCS: u8 = 0x10;
/// Flags field: frame failed FCS check.
pub const F_BADFCS: u8 = 0x40;

/// Channel flags.
const CH_CCK: u16 = 0x0020;
const CH_OFDM: u16 = 0x0040;
const CH_2GHZ: u16 = 0x0080;
const CH_5GHZ: u16 = 0x0100;

/// Metadata carried in (or recovered from) a radiotap header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RadiotapInfo {
    pub tsft: Option<u64>,
    pub flags: Option<u8>,
    /// Rate code (500 kb/s units, or 0x80|MCS for HT; see [`crate::rate`]).
    pub rate: Option<u8>,
    pub freq: Option<u16>,
    pub channel_flags: Option<u16>,
    pub signal_dbm: Option<i8>,
    pub tx_power_dbm: Option<i8>,
}

/// (size, alignment) of radiotap fields 0..=22.
const FIELDS: [(usize, usize); 23] = [
    (8, 8),  // 0 TSFT
    (1, 1),  // 1 Flags
    (1, 1),  // 2 Rate
    (4, 2),  // 3 Channel
    (2, 1),  // 4 FHSS
    (1, 1),  // 5 dBm antenna signal
    (1, 1),  // 6 dBm antenna noise
    (2, 2),  // 7 Lock quality
    (2, 2),  // 8 TX attenuation
    (2, 2),  // 9 dB TX attenuation
    (1, 1),  // 10 dBm TX power
    (1, 1),  // 11 Antenna
    (1, 1),  // 12 dB antenna signal
    (1, 1),  // 13 dB antenna noise
    (2, 2),  // 14 RX flags
    (2, 2),  // 15 TX flags
    (1, 1),  // 16 RTS retries
    (1, 1),  // 17 data retries
    (8, 4),  // 18 XChannel
    (3, 1),  // 19 MCS
    (8, 4),  // 20 A-MPDU status
    (12, 2), // 21 VHT
    (12, 8), // 22 timestamp
];

fn align(off: usize, a: usize) -> usize {
    (off + a - 1) / a * a
}

/// Build a radiotap header for a frame. `rate` is a rate code, `channel` an 802.11
/// channel number; `fcs` says whether the frame that follows ends with an FCS.
pub fn write(tsft: u64, rate: u8, channel: u8, signal_dbm: i8, fcs: bool) -> Vec<u8> {
    let ht = rate & 0x80 != 0;
    let mut present = P_TSFT | P_FLAGS | P_CHANNEL | P_DBM_SIGNAL;
    present |= if ht { P_MCS } else { P_RATE };
    let mut o = vec![0u8, 0, 0, 0];
    o.extend_from_slice(&present.to_le_bytes());
    // TSFT at offset 8 (already 8-aligned)
    o.extend_from_slice(&tsft.to_le_bytes());
    o.push(if fcs { F_FCS } else { 0 });
    if !ht {
        o.push(rate);
    }
    while o.len() % 2 != 0 {
        o.push(0);
    }
    let freq = frame::channel_to_freq(channel);
    let mut cf = if channel <= 14 { CH_2GHZ } else { CH_5GHZ };
    cf |= if !ht && rate <= 22 && channel <= 14 && rate != 12 && rate != 18 { CH_CCK } else { CH_OFDM };
    o.extend_from_slice(&freq.to_le_bytes());
    o.extend_from_slice(&cf.to_le_bytes());
    o.push(signal_dbm as u8);
    if ht {
        // known: bandwidth | MCS index | guard interval; flags: 20 MHz, long GI
        o.push(0x07);
        o.push(0x00);
        o.push(rate & 0x7f);
    }
    let len = o.len() as u16;
    o[2..4].copy_from_slice(&len.to_le_bytes());
    o
}

/// Prepend a radiotap header to an 802.11 frame.
pub fn encapsulate(frame_80211: &[u8], tsft: u64, rate: u8, channel: u8, signal_dbm: i8, fcs: bool) -> Vec<u8> {
    let mut o = write(tsft, rate, channel, signal_dbm, fcs);
    o.extend_from_slice(frame_80211);
    o
}

/// Parse a radiotap header; returns the metadata and the header length (the 802.11
/// frame starts there).
pub fn parse(b: &[u8]) -> Option<(RadiotapInfo, usize)> {
    if b.len() < 8 || b[0] != 0 {
        return None;
    }
    let hlen = u16::from_le_bytes([b[2], b[3]]) as usize;
    if hlen < 8 || hlen > b.len() {
        return None;
    }
    // Collect presence words.
    let mut words = Vec::new();
    let mut p = 4;
    loop {
        if p + 4 > hlen {
            return None;
        }
        let w = u32::from_le_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]);
        words.push(w);
        p += 4;
        if w & P_EXT == 0 {
            break;
        }
    }
    let present = words[0];
    let mut info = RadiotapInfo::default();
    let mut off = p;
    for (bit, &(size, al)) in FIELDS.iter().enumerate() {
        if present & (1 << bit) == 0 {
            continue;
        }
        off = align(off, al);
        if off + size > hlen {
            return None;
        }
        let f = &b[off..off + size];
        match 1u32 << bit {
            P_TSFT => info.tsft = Some(u64::from_le_bytes(f.try_into().ok()?)),
            P_FLAGS => info.flags = Some(f[0]),
            P_RATE => info.rate = Some(f[0]),
            P_CHANNEL => {
                info.freq = Some(u16::from_le_bytes([f[0], f[1]]));
                info.channel_flags = Some(u16::from_le_bytes([f[2], f[3]]));
            }
            P_DBM_SIGNAL => info.signal_dbm = Some(f[0] as i8),
            P_DBM_TX_POWER => info.tx_power_dbm = Some(f[0] as i8),
            P_MCS => {
                if f[0] & 0x02 != 0 {
                    info.rate = Some(0x80 | (f[2] & 0x7f));
                }
            }
            _ => {}
        }
        off += size;
    }
    // Bits 23..=28 (and vendor namespaces) are not decoded; the header length still
    // tells us where the frame starts.
    Some((info, hlen))
}
