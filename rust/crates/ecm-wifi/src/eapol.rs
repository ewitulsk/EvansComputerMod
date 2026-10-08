//! EAPOL (IEEE 802.1X) framing and the RSN EAPOL-Key descriptor (§12.7.2), plus
//! key-data KDE helpers (GTK KDE, padding for AES key wrap).

use crate::crypto;

pub const EAPOL_TYPE_EAP: u8 = 0;
pub const EAPOL_TYPE_START: u8 = 1;
pub const EAPOL_TYPE_LOGOFF: u8 = 2;
pub const EAPOL_TYPE_KEY: u8 = 3;

/// Descriptor type for RSN (IEEE 802.11) keys.
pub const DESC_RSN: u8 = 2;

// Key Information bits.
pub const KI_VERSION_MASK: u16 = 0x0007;
/// Key descriptor version 2: HMAC-SHA1-128 MIC, AES key wrap.
pub const KI_VERSION_AES_SHA1: u16 = 0x0002;
pub const KI_PAIRWISE: u16 = 0x0008;
pub const KI_INSTALL: u16 = 0x0040;
pub const KI_ACK: u16 = 0x0080;
pub const KI_MIC: u16 = 0x0100;
pub const KI_SECURE: u16 = 0x0200;
pub const KI_ERROR: u16 = 0x0400;
pub const KI_REQUEST: u16 = 0x0800;
pub const KI_ENC_KEY_DATA: u16 = 0x1000;

/// Offset of the MIC field inside a whole EAPOL-Key frame (4-byte EAPOL header +
/// 1 desc type + 2 key info + 2 key len + 8 replay + 32 nonce + 16 IV + 8 RSC + 8 reserved).
pub const MIC_OFFSET: usize = 81;
/// Minimum EAPOL-Key frame length (key data length field included).
pub const KEY_FRAME_MIN: usize = 99;

/// An EAPOL-Key frame with the RSN descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyFrame {
    /// EAPOL protocol version (1 or 2).
    pub version: u8,
    pub key_info: u16,
    pub key_len: u16,
    pub replay_counter: u64,
    pub nonce: [u8; 32],
    pub iv: [u8; 16],
    pub rsc: u64,
    pub mic: [u8; 16],
    pub key_data: Vec<u8>,
}

impl KeyFrame {
    pub fn new(version: u8, key_info: u16) -> KeyFrame {
        KeyFrame {
            version,
            key_info,
            key_len: 0,
            replay_counter: 0,
            nonce: [0; 32],
            iv: [0; 16],
            rsc: 0,
            mic: [0; 16],
            key_data: Vec::new(),
        }
    }

    pub fn has(&self, bit: u16) -> bool {
        self.key_info & bit != 0
    }

    pub fn descriptor_version(&self) -> u16 {
        self.key_info & KI_VERSION_MASK
    }

    /// Parse a whole EAPOL frame (starting at the EAPOL version byte). Trailing bytes
    /// beyond the EAPOL body length (Ethernet padding) are ignored.
    pub fn parse(b: &[u8]) -> Option<KeyFrame> {
        if b.len() < KEY_FRAME_MIN || b[1] != EAPOL_TYPE_KEY || b[4] != DESC_RSN {
            return None;
        }
        let body_len = u16::from_be_bytes([b[2], b[3]]) as usize;
        if b.len() < 4 + body_len || body_len < KEY_FRAME_MIN - 4 {
            return None;
        }
        let kd_len = u16::from_be_bytes([b[97], b[98]]) as usize;
        if 99 + kd_len > 4 + body_len {
            return None;
        }
        let mut nonce = [0u8; 32];
        nonce.copy_from_slice(&b[17..49]);
        let mut iv = [0u8; 16];
        iv.copy_from_slice(&b[49..65]);
        let mut mic = [0u8; 16];
        mic.copy_from_slice(&b[81..97]);
        Some(KeyFrame {
            version: b[0],
            key_info: u16::from_be_bytes([b[5], b[6]]),
            key_len: u16::from_be_bytes([b[7], b[8]]),
            replay_counter: u64::from_be_bytes(b[9..17].try_into().ok()?),
            nonce,
            iv,
            rsc: u64::from_le_bytes(b[65..73].try_into().ok()?),
            mic,
            key_data: b[99..99 + kd_len].to_vec(),
        })
    }

    /// Serialize the whole EAPOL frame.
    pub fn to_bytes(&self) -> Vec<u8> {
        let body_len = 95 + self.key_data.len();
        let mut o = Vec::with_capacity(4 + body_len);
        o.push(self.version);
        o.push(EAPOL_TYPE_KEY);
        o.extend_from_slice(&(body_len as u16).to_be_bytes());
        o.push(DESC_RSN);
        o.extend_from_slice(&self.key_info.to_be_bytes());
        o.extend_from_slice(&self.key_len.to_be_bytes());
        o.extend_from_slice(&self.replay_counter.to_be_bytes());
        o.extend_from_slice(&self.nonce);
        o.extend_from_slice(&self.iv);
        o.extend_from_slice(&self.rsc.to_le_bytes());
        o.extend_from_slice(&[0u8; 8]);
        o.extend_from_slice(&self.mic);
        o.extend_from_slice(&(self.key_data.len() as u16).to_be_bytes());
        o.extend_from_slice(&self.key_data);
        o
    }

    /// Compute the MIC with `kck` and store it (sets nothing else).
    pub fn sign(&mut self, kck: &[u8; 16]) {
        self.mic = [0; 16];
        let bytes = self.to_bytes();
        self.mic = crypto::eapol_mic(kck, &bytes);
    }

    /// Serialize with the MIC computed over the frame.
    pub fn to_signed_bytes(&mut self, kck: &[u8; 16]) -> Vec<u8> {
        self.sign(kck);
        self.to_bytes()
    }

    /// Verify the MIC of this parsed frame against `kck`.
    pub fn verify_mic(&self, kck: &[u8; 16]) -> bool {
        let mut z = self.clone();
        z.mic = [0; 16];
        let expect = crypto::eapol_mic(kck, &z.to_bytes());
        crypto::ct_eq(&expect, &self.mic)
    }
}

/// Is this EAPOL frame a pairwise (4-way handshake) EAPOL-Key message?
///
/// [`crate::Wlan`] always sends these in the clear, even once a PTK is installed: a
/// retransmitted M3 (lost M4) is otherwise answered with an M4 protected by a key the
/// authenticator only installs after receiving M4. Group-key messages are protected.
/// Authenticators must therefore accept unprotected EAPOL at any time.
pub fn is_pairwise_key_frame(eapol: &[u8]) -> bool {
    eapol.len() >= 7 && eapol[1] == EAPOL_TYPE_KEY && u16::from_be_bytes([eapol[5], eapol[6]]) & KI_PAIRWISE != 0
}

/// Verify a MIC directly over raw EAPOL bytes (as received, padding stripped to the body
/// length). Equivalent to [`KeyFrame::verify_mic`] but byte-exact on the received frame.
pub fn verify_mic_raw(kck: &[u8; 16], eapol: &[u8]) -> bool {
    if eapol.len() < KEY_FRAME_MIN {
        return false;
    }
    let body_len = u16::from_be_bytes([eapol[2], eapol[3]]) as usize;
    let end = (4 + body_len).min(eapol.len());
    let mut z = eapol[..end].to_vec();
    let mut mic = [0u8; 16];
    mic.copy_from_slice(&z[MIC_OFFSET..MIC_OFFSET + 16]);
    z[MIC_OFFSET..MIC_OFFSET + 16].fill(0);
    crypto::ct_eq(&crypto::eapol_mic(kck, &z), &mic)
}

/// GTK key data encapsulation (KDE type 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GtkKde {
    pub key_id: u8,
    pub tx: bool,
    pub gtk: Vec<u8>,
}

pub const KDE_OUI: [u8; 3] = [0x00, 0x0f, 0xac];
pub const KDE_GTK: u8 = 1;

/// Build a GTK KDE element.
pub fn gtk_kde(key_id: u8, tx: bool, gtk: &[u8]) -> Vec<u8> {
    let mut o = vec![0xdd, (6 + gtk.len()) as u8];
    o.extend_from_slice(&KDE_OUI);
    o.push(KDE_GTK);
    o.push((key_id & 3) | if tx { 4 } else { 0 });
    o.push(0);
    o.extend_from_slice(gtk);
    o
}

/// Parsed key data: the RSN element (whole, with id/len) and the GTK KDE, if present.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyData {
    pub rsn_ie: Option<Vec<u8>>,
    pub gtk: Option<GtkKde>,
}

/// Parse key data (plaintext). Stops at the 0xDD 0x00 padding marker.
pub fn parse_key_data(b: &[u8]) -> KeyData {
    let mut kd = KeyData::default();
    let mut p = 0;
    while p + 2 <= b.len() {
        let id = b[p];
        let len = b[p + 1] as usize;
        if id == 0xdd && len == 0 {
            break; // padding
        }
        if p + 2 + len > b.len() {
            break;
        }
        let d = &b[p + 2..p + 2 + len];
        match id {
            0x30 if kd.rsn_ie.is_none() => kd.rsn_ie = Some(b[p..p + 2 + len].to_vec()),
            0xdd if len >= 6 && d[0..3] == KDE_OUI && d[3] == KDE_GTK => {
                kd.gtk = Some(GtkKde { key_id: d[4] & 3, tx: d[4] & 4 != 0, gtk: d[6..].to_vec() });
            }
            _ => {}
        }
        p += 2 + len;
    }
    kd
}

/// Pad key data for AES key wrap: append 0xDD then zeros up to a multiple of 8, at
/// least 16 bytes (§12.7.2).
pub fn pad_key_data(mut b: Vec<u8>) -> Vec<u8> {
    if b.len() < 16 || b.len() % 8 != 0 {
        b.push(0xdd);
        while b.len() % 8 != 0 || b.len() < 16 {
            b.push(0);
        }
    }
    b
}
