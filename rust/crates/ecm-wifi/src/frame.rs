//! IEEE 802.11 frame codec: MAC header, information elements, management frames,
//! data frames with LLC/SNAP ↔ Ethernet II conversion, and the FCS.
//!
//! Frames handled here never carry an FCS unless stated: the SoftMAC low MAC strips and
//! appends it ([`fcs`], [`append_fcs`], [`strip_fcs`] are provided for pcaps and tests).

pub type MacAddr = [u8; 6];

pub const BROADCAST: MacAddr = [0xff; 6];

pub const TYPE_MGMT: u8 = 0;
pub const TYPE_CTRL: u8 = 1;
pub const TYPE_DATA: u8 = 2;

// Management subtypes.
pub const ST_ASSOC_REQ: u8 = 0;
pub const ST_ASSOC_RESP: u8 = 1;
pub const ST_REASSOC_REQ: u8 = 2;
pub const ST_REASSOC_RESP: u8 = 3;
pub const ST_PROBE_REQ: u8 = 4;
pub const ST_PROBE_RESP: u8 = 5;
pub const ST_BEACON: u8 = 8;
pub const ST_DISASSOC: u8 = 10;
pub const ST_AUTH: u8 = 11;
pub const ST_DEAUTH: u8 = 12;
// Data subtypes.
pub const ST_DATA: u8 = 0;
pub const ST_NULL: u8 = 4;
pub const ST_QOS_DATA: u8 = 8;
pub const ST_QOS_NULL: u8 = 12;

// Frame-control flag byte (second octet).
pub const FL_TO_DS: u8 = 0x01;
pub const FL_FROM_DS: u8 = 0x02;
pub const FL_MORE_FRAG: u8 = 0x04;
pub const FL_RETRY: u8 = 0x08;
pub const FL_PWR_MGT: u8 = 0x10;
pub const FL_MORE_DATA: u8 = 0x20;
pub const FL_PROTECTED: u8 = 0x40;
pub const FL_ORDER: u8 = 0x80;

// Information element ids.
pub const IE_SSID: u8 = 0;
pub const IE_SUPP_RATES: u8 = 1;
pub const IE_DS_PARAMS: u8 = 3;
pub const IE_TIM: u8 = 5;
pub const IE_HT_CAP: u8 = 45;
pub const IE_RSN: u8 = 48;
pub const IE_EXT_RATES: u8 = 50;
pub const IE_HT_OP: u8 = 61;
pub const IE_VENDOR: u8 = 221;

// Capability information bits.
pub const CAP_ESS: u16 = 0x0001;
pub const CAP_PRIVACY: u16 = 0x0010;
pub const CAP_SHORT_PREAMBLE: u16 = 0x0020;
pub const CAP_SHORT_SLOT: u16 = 0x0400;

// Status / reason codes used by the stack.
pub const STATUS_SUCCESS: u16 = 0;
pub const STATUS_UNSPECIFIED: u16 = 1;
pub const REASON_UNSPECIFIED: u16 = 1;
pub const REASON_PREV_AUTH_INVALID: u16 = 2;
pub const REASON_DEAUTH_LEAVING: u16 = 3;
pub const REASON_INACTIVITY: u16 = 4;
pub const REASON_CLASS3_NONASSOC: u16 = 7;
pub const REASON_DISASSOC_LEAVING: u16 = 8;
pub const REASON_MIC_FAILURE: u16 = 14;
pub const REASON_4WAY_TIMEOUT: u16 = 15;
pub const REASON_GROUP_KEY_TIMEOUT: u16 = 16;
pub const REASON_IE_MISMATCH: u16 = 17;

pub const ETHERTYPE_EAPOL: u16 = 0x888e;
pub const LLC_SNAP: [u8; 6] = [0xaa, 0xaa, 0x03, 0x00, 0x00, 0x00];

pub fn is_multicast(a: &MacAddr) -> bool {
    a[0] & 1 != 0
}

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

pub fn frame_type(f: &[u8]) -> u8 {
    (f[0] >> 2) & 3
}

pub fn frame_subtype(f: &[u8]) -> u8 {
    f[0] >> 4
}

/// True for a data frame with both ToDS and FromDS (four-address WDS/mesh).
pub fn has_addr4(f: &[u8]) -> bool {
    f.len() >= 2 && frame_type(f) == TYPE_DATA && f[1] & 3 == 3
}

/// Length of the MAC header of a management or data frame (`None` for control frames
/// or truncated input).
pub fn header_len(f: &[u8]) -> Option<usize> {
    if f.len() < 2 {
        return None;
    }
    if f[0] & 3 != 0 {
        return None; // protocol version must be 0
    }
    let len = match frame_type(f) {
        TYPE_MGMT => 24 + if f[1] & FL_ORDER != 0 { 4 } else { 0 },
        TYPE_DATA => {
            let mut l = 24;
            if has_addr4(f) {
                l += 6;
            }
            if frame_subtype(f) & 0x08 != 0 {
                l += 2;
                if f[1] & FL_ORDER != 0 {
                    l += 4;
                }
            }
            l
        }
        _ => return None,
    };
    if f.len() < len {
        None
    } else {
        Some(len)
    }
}

/// QoS Control field of a QoS data frame.
pub fn qos_control(f: &[u8]) -> Option<u16> {
    if f.len() < 26 || frame_type(f) != TYPE_DATA || frame_subtype(f) & 0x08 == 0 {
        return None;
    }
    let off = if has_addr4(f) { 30 } else { 24 };
    if f.len() < off + 2 {
        return None;
    }
    Some(rd16(f, off))
}

/// TID of a frame for replay counters: QoS TID, else 0.
pub fn tid(f: &[u8]) -> usize {
    qos_control(f).map(|q| (q & 0x0f) as usize).unwrap_or(0)
}

/// Parsed generic MAC header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub ftype: u8,
    pub subtype: u8,
    pub flags: u8,
    pub duration: u16,
    pub addr1: MacAddr,
    pub addr2: MacAddr,
    pub addr3: MacAddr,
    /// Sequence number (12 bits).
    pub seq: u16,
    /// Fragment number (4 bits).
    pub frag: u8,
    pub addr4: Option<MacAddr>,
    pub qos: Option<u16>,
}

impl Header {
    pub fn new(ftype: u8, subtype: u8, flags: u8, a1: MacAddr, a2: MacAddr, a3: MacAddr) -> Self {
        Header {
            ftype,
            subtype,
            flags,
            duration: 0,
            addr1: a1,
            addr2: a2,
            addr3: a3,
            seq: 0,
            frag: 0,
            addr4: None,
            qos: None,
        }
    }

    pub fn parse(f: &[u8]) -> Option<(Header, usize)> {
        let hlen = header_len(f)?;
        let mut a = [[0u8; 6]; 3];
        for (i, slot) in a.iter_mut().enumerate() {
            slot.copy_from_slice(&f[4 + 6 * i..10 + 6 * i]);
        }
        let sc = rd16(f, 22);
        let addr4 = if has_addr4(f) {
            let mut m = [0u8; 6];
            m.copy_from_slice(&f[24..30]);
            Some(m)
        } else {
            None
        };
        Some((
            Header {
                ftype: frame_type(f),
                subtype: frame_subtype(f),
                flags: f[1],
                duration: rd16(f, 2),
                addr1: a[0],
                addr2: a[1],
                addr3: a[2],
                seq: sc >> 4,
                frag: (sc & 0x0f) as u8,
                addr4,
                qos: qos_control(f),
            },
            hlen,
        ))
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.push((self.ftype & 3) << 2 | (self.subtype & 0x0f) << 4);
        let mut flags = self.flags;
        if self.addr4.is_some() {
            flags |= FL_TO_DS | FL_FROM_DS;
        }
        out.push(flags);
        out.extend_from_slice(&self.duration.to_le_bytes());
        out.extend_from_slice(&self.addr1);
        out.extend_from_slice(&self.addr2);
        out.extend_from_slice(&self.addr3);
        out.extend_from_slice(&((self.seq & 0x0fff) << 4 | (self.frag as u16 & 0x0f)).to_le_bytes());
        if let Some(a4) = self.addr4 {
            out.extend_from_slice(&a4);
        }
        if let Some(q) = self.qos {
            out.extend_from_slice(&q.to_le_bytes());
        }
    }

    pub fn to_ds(&self) -> bool {
        self.flags & FL_TO_DS != 0
    }
    pub fn from_ds(&self) -> bool {
        self.flags & FL_FROM_DS != 0
    }
    pub fn protected(&self) -> bool {
        self.flags & FL_PROTECTED != 0
    }
    pub fn retry(&self) -> bool {
        self.flags & FL_RETRY != 0
    }

    /// BSSID of the frame according to the DS bits (None for WDS).
    pub fn bssid(&self) -> Option<MacAddr> {
        match (self.to_ds(), self.from_ds()) {
            (false, false) => Some(self.addr3),
            (true, false) => Some(self.addr1),
            (false, true) => Some(self.addr2),
            (true, true) => None,
        }
    }
}

/// Patch the sequence-control field of a serialized frame.
pub fn set_seq(f: &mut [u8], seq: u16) {
    if f.len() >= 24 {
        let frag = f[22] & 0x0f;
        let sc = (seq & 0x0fff) << 4 | frag as u16;
        f[22..24].copy_from_slice(&sc.to_le_bytes());
    }
}

// ---------------------------------------------------------------------------
// Information elements
// ---------------------------------------------------------------------------

/// Iterate the (id, body) pairs of an IE list, stopping at the first truncated element.
pub fn ies(b: &[u8]) -> IeIter<'_> {
    IeIter { b, pos: 0 }
}

pub struct IeIter<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for IeIter<'a> {
    type Item = (u8, &'a [u8]);
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos + 2 > self.b.len() {
            return None;
        }
        let id = self.b[self.pos];
        let len = self.b[self.pos + 1] as usize;
        let start = self.pos + 2;
        if start + len > self.b.len() {
            return None;
        }
        self.pos = start + len;
        Some((id, &self.b[start..start + len]))
    }
}

/// Find the first IE with `id`.
pub fn find_ie(b: &[u8], id: u8) -> Option<&[u8]> {
    ies(b).find(|(i, _)| *i == id).map(|(_, d)| d)
}

/// Find the first IE with `id` and return it including its 2-byte header.
pub fn find_ie_raw(b: &[u8], id: u8) -> Option<&[u8]> {
    let mut pos = 0;
    while pos + 2 <= b.len() {
        let len = b[pos + 1] as usize;
        if pos + 2 + len > b.len() {
            return None;
        }
        if b[pos] == id {
            return Some(&b[pos..pos + 2 + len]);
        }
        pos += 2 + len;
    }
    None
}

pub fn push_ie(out: &mut Vec<u8>, id: u8, data: &[u8]) {
    out.push(id);
    out.push(data.len().min(255) as u8);
    out.extend_from_slice(&data[..data.len().min(255)]);
}

/// Write Supported Rates (first 8) and, if needed, Extended Supported Rates.
pub fn push_rates(out: &mut Vec<u8>, rates: &[u8]) {
    let n = rates.len().min(8);
    push_ie(out, IE_SUPP_RATES, &rates[..n]);
    if rates.len() > 8 {
        push_ie(out, IE_EXT_RATES, &rates[8..]);
    }
}

/// Cipher / AKM suite selectors (OUI 00-0F-AC).
pub const SUITE_CCMP: u32 = 0x000f_ac04;
pub const SUITE_TKIP: u32 = 0x000f_ac02;
pub const SUITE_AKM_8021X: u32 = 0x000f_ac01;
pub const SUITE_AKM_PSK: u32 = 0x000f_ac02;

/// RSN element (§9.4.2.24) for the suites this stack understands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rsn {
    pub version: u16,
    pub group: u32,
    pub pairwise: Vec<u32>,
    pub akm: Vec<u32>,
    pub caps: u16,
}

impl Rsn {
    /// WPA2-Personal: group and pairwise CCMP, AKM PSK.
    pub fn wpa2_psk() -> Rsn {
        Rsn { version: 1, group: SUITE_CCMP, pairwise: vec![SUITE_CCMP], akm: vec![SUITE_AKM_PSK], caps: 0 }
    }

    /// Parse the body of an RSN IE (without id/len). Missing optional fields default
    /// to CCMP / 802.1X as in the standard.
    pub fn parse(b: &[u8]) -> Option<Rsn> {
        if b.len() < 2 {
            return None;
        }
        let version = rd16(b, 0);
        if version != 1 {
            return None;
        }
        let mut p = 2;
        let suite = |p: usize| -> u32 { u32::from_be_bytes([b[p], b[p + 1], b[p + 2], b[p + 3]]) };
        let group = if b.len() >= p + 4 {
            p += 4;
            suite(p - 4)
        } else {
            return Some(Rsn { version, group: SUITE_CCMP, pairwise: vec![SUITE_CCMP], akm: vec![SUITE_AKM_8021X], caps: 0 });
        };
        let list = |p: &mut usize| -> Option<Option<Vec<u32>>> {
            if b.len() < *p + 2 {
                return Some(None);
            }
            let n = rd16(b, *p) as usize;
            *p += 2;
            if b.len() < *p + 4 * n {
                return None;
            }
            let v = (0..n).map(|i| suite(*p + 4 * i)).collect();
            *p += 4 * n;
            Some(Some(v))
        };
        let pairwise = list(&mut p)?.unwrap_or_else(|| vec![SUITE_CCMP]);
        let akm = list(&mut p)?.unwrap_or_else(|| vec![SUITE_AKM_8021X]);
        let caps = if b.len() >= p + 2 { rd16(b, p) } else { 0 };
        Some(Rsn { version, group, pairwise, akm, caps })
    }

    /// Body bytes (without id/len).
    pub fn body(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(20);
        o.extend_from_slice(&self.version.to_le_bytes());
        o.extend_from_slice(&self.group.to_be_bytes());
        o.extend_from_slice(&(self.pairwise.len() as u16).to_le_bytes());
        for s in &self.pairwise {
            o.extend_from_slice(&s.to_be_bytes());
        }
        o.extend_from_slice(&(self.akm.len() as u16).to_le_bytes());
        for s in &self.akm {
            o.extend_from_slice(&s.to_be_bytes());
        }
        o.extend_from_slice(&self.caps.to_le_bytes());
        o
    }

    /// Full element including id/len.
    pub fn to_ie(&self) -> Vec<u8> {
        let mut o = Vec::new();
        push_ie(&mut o, IE_RSN, &self.body());
        o
    }

    /// Does this AP offer WPA2-PSK with CCMP?
    pub fn supports_psk_ccmp(&self) -> bool {
        self.group == SUITE_CCMP && self.pairwise.contains(&SUITE_CCMP) && self.akm.contains(&SUITE_AKM_PSK)
    }
}

/// Our HT Capabilities element body: 20 MHz, short GI, RX MCS 0-7.
pub fn ht_cap_body() -> [u8; 26] {
    let mut b = [0u8; 26];
    b[0] = 0x20; // short GI for 20 MHz
    b[3] = 0xff; // RX MCS bitmask: MCS 0-7
    b
}

/// RX MCS bitmask (MCS 0..7) from an HT Capabilities body.
pub fn ht_mcs_mask(ht_cap: &[u8]) -> u8 {
    if ht_cap.len() >= 4 {
        ht_cap[3]
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Management frames
// ---------------------------------------------------------------------------

/// Beacon / probe response body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BeaconBody {
    pub timestamp: u64,
    /// Beacon interval in TU (1024 µs).
    pub interval: u16,
    pub cap: u16,
    pub ies: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MgmtBody {
    Beacon(BeaconBody),
    ProbeResp(BeaconBody),
    ProbeReq { ies: Vec<u8> },
    Auth { algo: u16, seq: u16, status: u16, ies: Vec<u8> },
    AssocReq { cap: u16, listen: u16, ies: Vec<u8> },
    ReassocReq { cap: u16, listen: u16, current_ap: MacAddr, ies: Vec<u8> },
    AssocResp { cap: u16, status: u16, aid: u16, ies: Vec<u8> },
    ReassocResp { cap: u16, status: u16, aid: u16, ies: Vec<u8> },
    Disassoc { reason: u16 },
    Deauth { reason: u16 },
    /// Any other subtype (action frames, ...), kept raw.
    Other { body: Vec<u8> },
}

/// A management frame (header + typed body).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mgmt {
    pub hdr: Header,
    pub body: MgmtBody,
}

impl Mgmt {
    /// Build a management frame from addresses: DA, SA, BSSID.
    pub fn new(da: MacAddr, sa: MacAddr, bssid: MacAddr, body: MgmtBody) -> Mgmt {
        let st = match &body {
            MgmtBody::Beacon(_) => ST_BEACON,
            MgmtBody::ProbeResp(_) => ST_PROBE_RESP,
            MgmtBody::ProbeReq { .. } => ST_PROBE_REQ,
            MgmtBody::Auth { .. } => ST_AUTH,
            MgmtBody::AssocReq { .. } => ST_ASSOC_REQ,
            MgmtBody::ReassocReq { .. } => ST_REASSOC_REQ,
            MgmtBody::AssocResp { .. } => ST_ASSOC_RESP,
            MgmtBody::ReassocResp { .. } => ST_REASSOC_RESP,
            MgmtBody::Disassoc { .. } => ST_DISASSOC,
            MgmtBody::Deauth { .. } => ST_DEAUTH,
            MgmtBody::Other { .. } => 13, // action
        };
        Mgmt { hdr: Header::new(TYPE_MGMT, st, 0, da, sa, bssid), body }
    }

    pub fn parse(f: &[u8]) -> Option<Mgmt> {
        let (hdr, hlen) = Header::parse(f)?;
        if hdr.ftype != TYPE_MGMT || hdr.protected() {
            return None;
        }
        let b = &f[hlen..];
        let need = |n: usize| if b.len() >= n { Some(()) } else { None };
        let body = match hdr.subtype {
            ST_BEACON | ST_PROBE_RESP => {
                need(12)?;
                let bb = BeaconBody {
                    timestamp: u64::from_le_bytes(b[0..8].try_into().ok()?),
                    interval: rd16(b, 8),
                    cap: rd16(b, 10),
                    ies: b[12..].to_vec(),
                };
                if hdr.subtype == ST_BEACON {
                    MgmtBody::Beacon(bb)
                } else {
                    MgmtBody::ProbeResp(bb)
                }
            }
            ST_PROBE_REQ => MgmtBody::ProbeReq { ies: b.to_vec() },
            ST_AUTH => {
                need(6)?;
                MgmtBody::Auth { algo: rd16(b, 0), seq: rd16(b, 2), status: rd16(b, 4), ies: b[6..].to_vec() }
            }
            ST_ASSOC_REQ => {
                need(4)?;
                MgmtBody::AssocReq { cap: rd16(b, 0), listen: rd16(b, 2), ies: b[4..].to_vec() }
            }
            ST_REASSOC_REQ => {
                need(10)?;
                let mut ap = [0u8; 6];
                ap.copy_from_slice(&b[4..10]);
                MgmtBody::ReassocReq { cap: rd16(b, 0), listen: rd16(b, 2), current_ap: ap, ies: b[10..].to_vec() }
            }
            ST_ASSOC_RESP | ST_REASSOC_RESP => {
                need(6)?;
                let (cap, status, aid, ies) = (rd16(b, 0), rd16(b, 2), rd16(b, 4) & 0x3fff, b[6..].to_vec());
                if hdr.subtype == ST_ASSOC_RESP {
                    MgmtBody::AssocResp { cap, status, aid, ies }
                } else {
                    MgmtBody::ReassocResp { cap, status, aid, ies }
                }
            }
            ST_DISASSOC => {
                need(2)?;
                MgmtBody::Disassoc { reason: rd16(b, 0) }
            }
            ST_DEAUTH => {
                need(2)?;
                MgmtBody::Deauth { reason: rd16(b, 0) }
            }
            _ => MgmtBody::Other { body: b.to_vec() },
        };
        Some(Mgmt { hdr, body })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity(64);
        self.hdr.write(&mut o);
        match &self.body {
            MgmtBody::Beacon(bb) | MgmtBody::ProbeResp(bb) => {
                o.extend_from_slice(&bb.timestamp.to_le_bytes());
                o.extend_from_slice(&bb.interval.to_le_bytes());
                o.extend_from_slice(&bb.cap.to_le_bytes());
                o.extend_from_slice(&bb.ies);
            }
            MgmtBody::ProbeReq { ies } => o.extend_from_slice(ies),
            MgmtBody::Auth { algo, seq, status, ies } => {
                o.extend_from_slice(&algo.to_le_bytes());
                o.extend_from_slice(&seq.to_le_bytes());
                o.extend_from_slice(&status.to_le_bytes());
                o.extend_from_slice(ies);
            }
            MgmtBody::AssocReq { cap, listen, ies } => {
                o.extend_from_slice(&cap.to_le_bytes());
                o.extend_from_slice(&listen.to_le_bytes());
                o.extend_from_slice(ies);
            }
            MgmtBody::ReassocReq { cap, listen, current_ap, ies } => {
                o.extend_from_slice(&cap.to_le_bytes());
                o.extend_from_slice(&listen.to_le_bytes());
                o.extend_from_slice(current_ap);
                o.extend_from_slice(ies);
            }
            MgmtBody::AssocResp { cap, status, aid, ies } | MgmtBody::ReassocResp { cap, status, aid, ies } => {
                o.extend_from_slice(&cap.to_le_bytes());
                o.extend_from_slice(&status.to_le_bytes());
                o.extend_from_slice(&(aid | 0xc000).to_le_bytes());
                o.extend_from_slice(ies);
            }
            MgmtBody::Disassoc { reason } | MgmtBody::Deauth { reason } => {
                o.extend_from_slice(&reason.to_le_bytes())
            }
            MgmtBody::Other { body } => o.extend_from_slice(body),
        }
        o
    }
}

/// Everything a station learns about a BSS from a beacon or probe response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BssInfo {
    pub bssid: MacAddr,
    pub ssid: Vec<u8>,
    /// Channel from the DS Parameter Set, else the channel it was heard on.
    pub channel: u8,
    pub cap: u16,
    pub beacon_interval: u16,
    /// Supported + extended rates (500 kb/s units with the basic bit 0x80).
    pub rates: Vec<u8>,
    /// RX MCS mask from the HT Capabilities element (0 = no HT).
    pub ht_mcs: u8,
    /// RSN element body (without id/len), if present.
    pub rsn_ie: Option<Vec<u8>>,
    pub rsn: Option<Rsn>,
    /// Last RSSI in dBm.
    pub rssi: i32,
    /// Time (ms) the BSS was last heard.
    pub last_seen: u64,
}

impl BssInfo {
    /// Build from a beacon/probe-response body, the transmitter and the rx metadata.
    pub fn from_beacon(bssid: MacAddr, bb: &BeaconBody, rx_channel: u8, rssi: i32, now: u64) -> BssInfo {
        let ssid = find_ie(&bb.ies, IE_SSID).map(|s| s.to_vec()).unwrap_or_default();
        let channel = find_ie(&bb.ies, IE_DS_PARAMS).and_then(|d| d.first().copied()).unwrap_or(rx_channel);
        let mut rates = find_ie(&bb.ies, IE_SUPP_RATES).map(|s| s.to_vec()).unwrap_or_default();
        if let Some(x) = find_ie(&bb.ies, IE_EXT_RATES) {
            rates.extend_from_slice(x);
        }
        let ht_mcs = find_ie(&bb.ies, IE_HT_CAP).map(ht_mcs_mask).unwrap_or(0);
        let rsn_ie = find_ie(&bb.ies, IE_RSN).map(|s| s.to_vec());
        let rsn = rsn_ie.as_deref().and_then(Rsn::parse);
        BssInfo {
            bssid,
            ssid,
            channel,
            cap: bb.cap,
            beacon_interval: bb.interval,
            rates,
            ht_mcs,
            rsn_ie,
            rsn,
            rssi,
            last_seen: now,
        }
    }

    /// Security summary for scan listings.
    pub fn security(&self) -> Security {
        match &self.rsn {
            Some(r) if r.supports_psk_ccmp() => Security::Wpa2Psk,
            Some(_) => Security::Unsupported,
            None if self.cap & CAP_PRIVACY != 0 => Security::Unsupported,
            None => Security::Open,
        }
    }

    /// Hidden SSID (empty or all-zero SSID element).
    pub fn hidden(&self) -> bool {
        self.ssid.iter().all(|&b| b == 0)
    }

    /// SSID as text for display (lossy).
    pub fn ssid_str(&self) -> String {
        String::from_utf8_lossy(&self.ssid).into_owned()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    Open,
    Wpa2Psk,
    /// WEP, TKIP-only, 802.1X/EAP, ...: listed but not joinable.
    Unsupported,
}

// ---------------------------------------------------------------------------
// Data frames ↔ Ethernet II
// ---------------------------------------------------------------------------

/// Wrap an Ethernet II frame (dst ‖ src ‖ type ‖ payload) into an unprotected
/// non-QoS data MPDU from a station to its AP (ToDS: A1 = BSSID, A2 = SA, A3 = DA).
pub fn ethernet_to_data_tods(eth: &[u8], bssid: &MacAddr) -> Option<Vec<u8>> {
    if eth.len() < 14 {
        return None;
    }
    let mut da = [0u8; 6];
    let mut sa = [0u8; 6];
    da.copy_from_slice(&eth[0..6]);
    sa.copy_from_slice(&eth[6..12]);
    let hdr = Header::new(TYPE_DATA, ST_DATA, FL_TO_DS, *bssid, sa, da);
    let mut o = Vec::with_capacity(eth.len() + 32);
    hdr.write(&mut o);
    o.extend_from_slice(&LLC_SNAP);
    o.extend_from_slice(&eth[12..]);
    Some(o)
}

/// Wrap an Ethernet II frame into a data MPDU from an AP to a station
/// (FromDS: A1 = DA, A2 = BSSID, A3 = SA). Used by test APs and the Java mirror.
pub fn ethernet_to_data_fromds(eth: &[u8], bssid: &MacAddr) -> Option<Vec<u8>> {
    if eth.len() < 14 {
        return None;
    }
    let mut da = [0u8; 6];
    let mut sa = [0u8; 6];
    da.copy_from_slice(&eth[0..6]);
    sa.copy_from_slice(&eth[6..12]);
    let hdr = Header::new(TYPE_DATA, ST_DATA, FL_FROM_DS, da, *bssid, sa);
    let mut o = Vec::with_capacity(eth.len() + 32);
    hdr.write(&mut o);
    o.extend_from_slice(&LLC_SNAP);
    o.extend_from_slice(&eth[12..]);
    Some(o)
}

/// Convert an unprotected (or already decrypted) data MPDU into Ethernet II.
/// Handles ToDS / FromDS / IBSS addressing and LLC/SNAP (and bridge-tunnel 00-00-F8);
/// non-SNAP payloads become 802.3 length-framed. Null-function frames yield `None`.
pub fn data_to_ethernet(mpdu: &[u8]) -> Option<Vec<u8>> {
    let (h, hlen) = Header::parse(mpdu)?;
    if h.ftype != TYPE_DATA || h.subtype & 0x04 != 0 {
        return None; // not data, or (QoS) Null
    }
    let (da, sa) = match (h.to_ds(), h.from_ds()) {
        (false, false) => (h.addr1, h.addr2),
        (false, true) => (h.addr1, h.addr3),
        (true, false) => (h.addr3, h.addr2),
        (true, true) => (h.addr3, h.addr4?),
    };
    let body = &mpdu[hlen..];
    let mut o = Vec::with_capacity(body.len() + 14);
    o.extend_from_slice(&da);
    o.extend_from_slice(&sa);
    if body.len() >= 8 && body[0..3] == LLC_SNAP[0..3] && (body[3..6] == [0, 0, 0] || body[3..6] == [0, 0, 0xf8]) {
        o.extend_from_slice(&body[6..]);
    } else {
        o.extend_from_slice(&(body.len() as u16).to_be_bytes());
        o.extend_from_slice(body);
    }
    Some(o)
}

/// EtherType of an Ethernet II frame.
pub fn ethertype(eth: &[u8]) -> Option<u16> {
    if eth.len() < 14 {
        return None;
    }
    Some(u16::from_be_bytes([eth[12], eth[13]]))
}

/// Build an Ethernet II frame.
pub fn ethernet(dst: &MacAddr, src: &MacAddr, ethertype: u16, payload: &[u8]) -> Vec<u8> {
    let mut o = Vec::with_capacity(14 + payload.len());
    o.extend_from_slice(dst);
    o.extend_from_slice(src);
    o.extend_from_slice(&ethertype.to_be_bytes());
    o.extend_from_slice(payload);
    o
}

// ---------------------------------------------------------------------------
// FCS (CRC-32, IEEE 802.3 polynomial, reflected)
// ---------------------------------------------------------------------------

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC_TABLE: [u32; 256] = crc_table();

/// FCS of a frame (CRC-32 over header and body).
pub fn fcs(b: &[u8]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for &x in b {
        c = CRC_TABLE[((c ^ x as u32) & 0xff) as usize] ^ (c >> 8);
    }
    !c
}

/// Append the FCS (little endian, as transmitted).
pub fn append_fcs(f: &mut Vec<u8>) {
    let c = fcs(f);
    f.extend_from_slice(&c.to_le_bytes());
}

/// Verify and remove a trailing FCS; `None` if it does not match.
pub fn strip_fcs(f: &[u8]) -> Option<&[u8]> {
    if f.len() < 4 {
        return None;
    }
    let (body, tail) = f.split_at(f.len() - 4);
    if fcs(body).to_le_bytes() == tail {
        Some(body)
    } else {
        None
    }
}

/// 2.4 / 5 GHz channel number → centre frequency in MHz.
pub fn channel_to_freq(ch: u8) -> u16 {
    match ch {
        14 => 2484,
        1..=13 => 2407 + 5 * ch as u16,
        _ => 5000 + 5 * ch as u16,
    }
}

/// Centre frequency (MHz) → channel number.
pub fn freq_to_channel(freq: u16) -> Option<u8> {
    match freq {
        2484 => Some(14),
        2412..=2472 if (freq - 2407) % 5 == 0 => Some(((freq - 2407) / 5) as u8),
        5000..=5900 if (freq - 5000) % 5 == 0 => Some(((freq - 5000) / 5) as u8),
        _ => None,
    }
}

/// Format a MAC as aa:bb:cc:dd:ee:ff.
pub fn mac_str(m: &MacAddr) -> String {
    format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
}
