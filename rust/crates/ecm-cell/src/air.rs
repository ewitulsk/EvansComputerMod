//! Air-interface messages: the payload of framed PHY frames between a phone
//! and a cell site.
//!
//! A frame payload is either one plain message (`type < 0x10`) or a protected
//! frame (`0x10`, see [`crate::crypto`]) whose plaintext is a list of
//! [`Item`]s, each `[type u8][len u16][body]`, so a frame can carry data and
//! control together (a buffer report riding on data, a grant with downlink).
//! Everything is big-endian; decoding is strict.

use alloc::string::String;
use alloc::vec::Vec;

use crate::codec::{DResult, DecodeError, R, W};

/// Air protocol version (in the system information).
pub const VERSION: u8 = 1;

/// Uplink resources granted to one phone: the frames it starts in slots
/// `s` with `from <= s < to` and `(s - offset) % period < len` (`len == period`
/// = every slot), on `subchannel`, at `cqi` (the site's measurement of this
/// phone, forced on the TX record so frames fit the granted run).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Grant {
    pub from_slot: u64,
    pub to_slot: u64,
    pub period: u16,
    pub offset: u16,
    pub len: u16,
    pub subchannel: u8,
    pub cqi: u8,
}

impl Grant {
    pub const SIZE: usize = 8 + 8 + 2 + 2 + 2 + 1 + 1;

    pub fn write(&self, w: &mut W) {
        w.u64(self.from_slot).u64(self.to_slot).u16(self.period).u16(self.offset).u16(self.len).u8(self.subchannel).u8(self.cqi);
    }
    pub fn read(r: &mut R) -> DResult<Grant> {
        let g = Grant {
            from_slot: r.u64()?,
            to_slot: r.u64()?,
            period: r.u16()?,
            offset: r.u16()?,
            len: r.u16()?,
            subchannel: r.u8()?,
            cqi: r.u8()?,
        };
        if g.period == 0 || g.len == 0 || g.len > g.period || g.offset >= g.period || g.subchannel > 4 || g.to_slot < g.from_slot {
            return Err(DecodeError::Value("grant"));
        }
        Ok(g)
    }
    /// May a frame start in `slot`?
    pub fn allows(&self, slot: u64) -> bool {
        slot >= self.from_slot && slot < self.to_slot && (slot + self.period as u64 - self.offset as u64) % (self.period as u64) < self.len as u64
    }
    /// The first slot >= `slot` this grant allows, with how many consecutive
    /// slots the run has left from there.
    pub fn next_run(&self, slot: u64) -> Option<(u64, u64)> {
        let p = self.period as u64;
        let mut s = slot.max(self.from_slot);
        for _ in 0..=p {
            if s >= self.to_slot {
                return None;
            }
            let ph = (s + p - self.offset as u64) % p;
            if ph < self.len as u64 {
                let run = (self.len as u64 - ph).min(self.to_slot - s);
                return Some((s, run));
            }
            s += 1;
        }
        None
    }
}

/// A neighbour cell in the system information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Neighbour {
    pub cell_id: u32,
    pub profile: u8,
    pub channel: u8,
    /// Uplink sub-channels that cell uses (bit 0 = sub-channel 1), as last heard.
    pub ul_mask: u8,
}

pub const SIB_CORE_UP: u8 = 0x01;
pub const SIB_BARRED: u8 = 0x02;

/// System information, broadcast with beacon offload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysInfo {
    pub version: u8,
    pub carrier: String,
    pub cell_id: u32,
    pub area: u16,
    /// [`SIB_CORE_UP`]: the site has a working link to its core; [`SIB_BARRED`]: no new attaches.
    pub flags: u8,
    pub profile: u8,
    pub channel: u8,
    pub tx_power_dbm: i8,
    /// Uplink data sub-channels in use here (bit 0 = sub-channel 1).
    pub ul_mask: u8,
    /// The sub-channel random access uses (1..=4).
    pub rach_sub: u8,
    pub neighbours: Vec<Neighbour>,
}

impl SysInfo {
    pub fn core_up(&self) -> bool {
        self.flags & SIB_CORE_UP != 0
    }
    pub fn barred(&self) -> bool {
        self.flags & SIB_BARRED != 0
    }
}

/// Why something was refused or torn down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    UnknownSubscriber = 1,
    WrongCarrier = 2,
    AuthFailed = 3,
    Revoked = 4,
    QuotaExhausted = 5,
    CoreUnavailable = 6,
    Congestion = 7,
    UserDetach = 8,
    RadioLost = 9,
    SiteShutdown = 10,
    HandoverDone = 11,
    Admin = 12,
    SessionLost = 13,
    Inactivity = 14,
    HandoverFailed = 15,
    Protocol = 16,
}

impl Cause {
    pub fn from_code(c: u8) -> DResult<Cause> {
        use Cause::*;
        Ok(match c {
            1 => UnknownSubscriber,
            2 => WrongCarrier,
            3 => AuthFailed,
            4 => Revoked,
            5 => QuotaExhausted,
            6 => CoreUnavailable,
            7 => Congestion,
            8 => UserDetach,
            9 => RadioLost,
            10 => SiteShutdown,
            11 => HandoverDone,
            12 => Admin,
            13 => SessionLost,
            14 => Inactivity,
            15 => HandoverFailed,
            16 => Protocol,
            _ => return Err(DecodeError::Value("cause")),
        })
    }
    pub fn name(self) -> &'static str {
        use Cause::*;
        match self {
            UnknownSubscriber => "unknown subscriber",
            WrongCarrier => "wrong carrier",
            AuthFailed => "authentication failed",
            Revoked => "SIM revoked",
            QuotaExhausted => "quota exhausted",
            CoreUnavailable => "core unavailable",
            Congestion => "congestion",
            UserDetach => "detached",
            RadioLost => "radio link lost",
            SiteShutdown => "site shut down",
            HandoverDone => "handed over",
            Admin => "released by the operator",
            SessionLost => "session lost",
            Inactivity => "inactivity",
            HandoverFailed => "handover failed",
            Protocol => "protocol error",
        }
    }
    /// A cause after which the phone must not try again on its own
    /// (until the SIM changes or the user asks).
    pub fn is_permanent(self) -> bool {
        matches!(self, Cause::UnknownSubscriber | Cause::AuthFailed | Cause::Revoked | Cause::WrongCarrier)
    }
}

impl core::fmt::Display for Cause {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RachCause {
    Attach = 0,
    Service = 1,
}

/// Plain (unprotected) messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plain {
    /// 0x01, broadcast by beacon offload.
    SysInfo(SysInfo),
    /// 0x02 UE -> site on the random access sub-channel.
    RachRequest { nonce: u32, cause: RachCause },
    /// 0x03 site -> UE.
    RachResponse { nonce: u32, ue_id: u16, grant: Grant },
    /// 0x04 UE -> site.
    AttachRequest { imsi: String, carrier: String },
    /// 0x05 site -> UE (RAND from the core).
    AuthChallenge { rand: [u8; 16] },
    /// 0x06 UE -> site (RES from the SIM).
    AuthResponse { res: [u8; 16] },
    /// 0x07 site -> UE.
    AttachReject { cause: Cause },
    /// 0x08 site -> UE: start protection with `K_AS(cell, epoch)`; the MAC
    /// (computed by the core) proves the network knows KS.
    SecurityModeCommand { epoch: u32, mac: [u8; 16] },
    /// 0x09 site -> broadcast: idle phones with these TMSIs have downlink waiting.
    Paging { tmsis: Vec<u32> },
    /// 0x0A UE -> site: resume an idle session.
    ServiceRequest { tmsi: u32, count: u32, mac: [u8; 16] },
}

const T_SYSINFO: u8 = 0x01;
const T_RACH_REQ: u8 = 0x02;
const T_RACH_RESP: u8 = 0x03;
const T_ATTACH_REQ: u8 = 0x04;
const T_AUTH_CHAL: u8 = 0x05;
const T_AUTH_RESP: u8 = 0x06;
const T_ATTACH_REJ: u8 = 0x07;
const T_SMC: u8 = 0x08;
const T_PAGING: u8 = 0x09;
const T_SERVICE: u8 = 0x0A;

impl Plain {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::new();
        match self {
            Plain::SysInfo(s) => {
                w.u8(T_SYSINFO).u8(s.version).str8(&s.carrier).u32(s.cell_id).u16(s.area).u8(s.flags).u8(s.profile).u8(s.channel);
                w.i8(s.tx_power_dbm).u8(s.ul_mask).u8(s.rach_sub).u8(s.neighbours.len().min(16) as u8);
                for n in s.neighbours.iter().take(16) {
                    w.u32(n.cell_id).u8(n.profile).u8(n.channel).u8(n.ul_mask);
                }
            }
            Plain::RachRequest { nonce, cause } => {
                w.u8(T_RACH_REQ).u32(*nonce).u8(*cause as u8);
            }
            Plain::RachResponse { nonce, ue_id, grant } => {
                w.u8(T_RACH_RESP).u32(*nonce).u16(*ue_id);
                grant.write(&mut w);
            }
            Plain::AttachRequest { imsi, carrier } => {
                w.u8(T_ATTACH_REQ).str8(imsi).str8(carrier);
            }
            Plain::AuthChallenge { rand } => {
                w.u8(T_AUTH_CHAL).raw(rand);
            }
            Plain::AuthResponse { res } => {
                w.u8(T_AUTH_RESP).raw(res);
            }
            Plain::AttachReject { cause } => {
                w.u8(T_ATTACH_REJ).u8(*cause as u8);
            }
            Plain::SecurityModeCommand { epoch, mac } => {
                w.u8(T_SMC).u32(*epoch).raw(mac);
            }
            Plain::Paging { tmsis } => {
                w.u8(T_PAGING).u8(tmsis.len().min(32) as u8);
                for t in tmsis.iter().take(32) {
                    w.u32(*t);
                }
            }
            Plain::ServiceRequest { tmsi, count, mac } => {
                w.u8(T_SERVICE).u32(*tmsi).u32(*count).raw(mac);
            }
        }
        w.into_vec()
    }

    pub fn decode(b: &[u8]) -> DResult<Plain> {
        let mut r = R::new(b);
        let t = r.u8()?;
        let m = match t {
            T_SYSINFO => {
                let version = r.u8()?;
                if version != VERSION {
                    return Err(DecodeError::Value("version"));
                }
                let carrier = r.str8()?;
                let cell_id = r.u32()?;
                let area = r.u16()?;
                let flags = r.u8()?;
                let profile = r.u8()?;
                let channel = r.u8()?;
                let tx_power_dbm = r.i8()?;
                let ul_mask = r.u8()?;
                let rach_sub = r.u8()?;
                if !(1..=4).contains(&rach_sub) {
                    return Err(DecodeError::Value("rach sub-channel"));
                }
                let n = r.count8(7)?;
                let mut neighbours = Vec::with_capacity(n);
                for _ in 0..n {
                    neighbours.push(Neighbour { cell_id: r.u32()?, profile: r.u8()?, channel: r.u8()?, ul_mask: r.u8()? });
                }
                Plain::SysInfo(SysInfo { version, carrier, cell_id, area, flags, profile, channel, tx_power_dbm, ul_mask, rach_sub, neighbours })
            }
            T_RACH_REQ => {
                let nonce = r.u32()?;
                let cause = match r.u8()? {
                    0 => RachCause::Attach,
                    1 => RachCause::Service,
                    _ => return Err(DecodeError::Value("rach cause")),
                };
                Plain::RachRequest { nonce, cause }
            }
            T_RACH_RESP => Plain::RachResponse { nonce: r.u32()?, ue_id: r.u16()?, grant: Grant::read(&mut r)? },
            T_ATTACH_REQ => Plain::AttachRequest { imsi: r.str8()?, carrier: r.str8()? },
            T_AUTH_CHAL => Plain::AuthChallenge { rand: r.arr()? },
            T_AUTH_RESP => Plain::AuthResponse { res: r.arr()? },
            T_ATTACH_REJ => Plain::AttachReject { cause: Cause::from_code(r.u8()?)? },
            T_SMC => Plain::SecurityModeCommand { epoch: r.u32()?, mac: r.arr()? },
            T_PAGING => {
                let n = r.count8(4)?;
                let mut tmsis = Vec::with_capacity(n);
                for _ in 0..n {
                    tmsis.push(r.u32()?);
                }
                Plain::Paging { tmsis }
            }
            T_SERVICE => Plain::ServiceRequest { tmsi: r.u32()?, count: r.u32()?, mac: r.arr()? },
            other => return Err(DecodeError::Type(other)),
        };
        r.end()?;
        Ok(m)
    }
}

/// A measured cell in a measurement report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measurement {
    pub cell_id: u32,
    pub profile: u8,
    pub channel: u8,
    /// dBm x10.
    pub rssi_x10: i16,
    /// dB x10.
    pub sinr_x10: i16,
    pub ul_mask: u8,
}

/// Items inside a protected frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// 0x20 UE -> site.
    SecurityModeComplete,
    /// 0x21 site -> UE.
    AttachAccept { ip: [u8; 4], prefix: u8, gateway: [u8; 4], dns: [u8; 4], mtu: u16, tmsi: u32, session: u32 },
    /// 0x22 UE -> site.
    AttachComplete,
    /// 0x23 site -> UE: uplink resources (replaces the previous grant).
    Grant(Grant),
    /// 0x24 UE -> site: bytes waiting to go up.
    BufferStatus { bytes: u32 },
    /// 0x25 UE -> site: the serving cell first, then neighbours.
    MeasurementReport { cells: Vec<Measurement> },
    /// 0x26 site -> UE.
    HandoverCommand { cell_id: u32, addr: u32, profile: u8, channel: u8, epoch: u32, ue_id: u16, grant: Grant },
    /// 0x27 UE -> target site (protected with the target's key).
    HandoverConfirm,
    /// 0x28 UE -> site.
    Detach { cause: Cause },
    /// 0x29 site -> UE.
    DetachAccept,
    /// 0x2A site -> UE: the network ends the session.
    Release { cause: Cause },
    /// 0x2B either way: one segment of an IP packet. `seq` is the tunnel
    /// sequence number (downlink duplicates are dropped by it); a packet is
    /// `count` segments `0..count` with the same `seq`.
    Data { seq: u32, idx: u8, count: u8, bytes: Vec<u8> },
    /// 0x2C UE -> site: the phone is away measuring other channels for these slots.
    MeasGap { from_slot: u64, to_slot: u64 },
    /// 0x2D either way: still here.
    Keepalive,
}

const I_SMC_COMPLETE: u8 = 0x20;
const I_ATTACH_ACCEPT: u8 = 0x21;
const I_ATTACH_COMPLETE: u8 = 0x22;
const I_GRANT: u8 = 0x23;
const I_BSR: u8 = 0x24;
const I_MEAS: u8 = 0x25;
const I_HO_CMD: u8 = 0x26;
const I_HO_CONFIRM: u8 = 0x27;
const I_DETACH: u8 = 0x28;
const I_DETACH_ACCEPT: u8 = 0x29;
const I_RELEASE: u8 = 0x2A;
const I_DATA: u8 = 0x2B;
const I_GAP: u8 = 0x2C;
const I_KEEPALIVE: u8 = 0x2D;

/// Item framing overhead.
pub const ITEM_HEADER: usize = 3;
/// Bytes a Data item adds to its segment.
pub const DATA_OVERHEAD: usize = ITEM_HEADER + 4 + 1 + 1;

impl Item {
    fn body(&self, w: &mut W) -> u8 {
        match self {
            Item::SecurityModeComplete => I_SMC_COMPLETE,
            Item::AttachAccept { ip, prefix, gateway, dns, mtu, tmsi, session } => {
                w.raw(ip).u8(*prefix).raw(gateway).raw(dns).u16(*mtu).u32(*tmsi).u32(*session);
                I_ATTACH_ACCEPT
            }
            Item::AttachComplete => I_ATTACH_COMPLETE,
            Item::Grant(g) => {
                g.write(w);
                I_GRANT
            }
            Item::BufferStatus { bytes } => {
                w.u32(*bytes);
                I_BSR
            }
            Item::MeasurementReport { cells } => {
                w.u8(cells.len().min(16) as u8);
                for m in cells.iter().take(16) {
                    w.u32(m.cell_id).u8(m.profile).u8(m.channel).i16(m.rssi_x10).i16(m.sinr_x10).u8(m.ul_mask);
                }
                I_MEAS
            }
            Item::HandoverCommand { cell_id, addr, profile, channel, epoch, ue_id, grant } => {
                w.u32(*cell_id).u32(*addr).u8(*profile).u8(*channel).u32(*epoch).u16(*ue_id);
                grant.write(w);
                I_HO_CMD
            }
            Item::HandoverConfirm => I_HO_CONFIRM,
            Item::Detach { cause } => {
                w.u8(*cause as u8);
                I_DETACH
            }
            Item::DetachAccept => I_DETACH_ACCEPT,
            Item::Release { cause } => {
                w.u8(*cause as u8);
                I_RELEASE
            }
            Item::Data { seq, idx, count, bytes } => {
                w.u32(*seq).u8(*idx).u8(*count).raw(bytes);
                I_DATA
            }
            Item::MeasGap { from_slot, to_slot } => {
                w.u64(*from_slot).u64(*to_slot);
                I_GAP
            }
            Item::Keepalive => I_KEEPALIVE,
        }
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        let mut w = W::new();
        let t = self.body(&mut w);
        let b = w.into_vec();
        out.push(t);
        out.extend_from_slice(&(b.len() as u16).to_be_bytes());
        out.extend_from_slice(&b);
    }

    pub fn encoded_len(&self) -> usize {
        let mut v = Vec::new();
        self.write(&mut v);
        v.len()
    }

    fn read(t: u8, b: &[u8]) -> DResult<Item> {
        let mut r = R::new(b);
        let it = match t {
            I_SMC_COMPLETE => Item::SecurityModeComplete,
            I_ATTACH_ACCEPT => {
                let ip = r.arr()?;
                let prefix = r.u8()?;
                if prefix > 32 {
                    return Err(DecodeError::Value("prefix"));
                }
                Item::AttachAccept { ip, prefix, gateway: r.arr()?, dns: r.arr()?, mtu: r.u16()?, tmsi: r.u32()?, session: r.u32()? }
            }
            I_ATTACH_COMPLETE => Item::AttachComplete,
            I_GRANT => Item::Grant(Grant::read(&mut r)?),
            I_BSR => Item::BufferStatus { bytes: r.u32()? },
            I_MEAS => {
                let n = r.count8(11)?;
                let mut cells = Vec::with_capacity(n);
                for _ in 0..n {
                    cells.push(Measurement {
                        cell_id: r.u32()?,
                        profile: r.u8()?,
                        channel: r.u8()?,
                        rssi_x10: r.i16()?,
                        sinr_x10: r.i16()?,
                        ul_mask: r.u8()?,
                    });
                }
                Item::MeasurementReport { cells }
            }
            I_HO_CMD => Item::HandoverCommand {
                cell_id: r.u32()?,
                addr: r.u32()?,
                profile: r.u8()?,
                channel: r.u8()?,
                epoch: r.u32()?,
                ue_id: r.u16()?,
                grant: Grant::read(&mut r)?,
            },
            I_HO_CONFIRM => Item::HandoverConfirm,
            I_DETACH => Item::Detach { cause: Cause::from_code(r.u8()?)? },
            I_DETACH_ACCEPT => Item::DetachAccept,
            I_RELEASE => Item::Release { cause: Cause::from_code(r.u8()?)? },
            I_DATA => {
                let seq = r.u32()?;
                let idx = r.u8()?;
                let count = r.u8()?;
                if count == 0 || idx >= count {
                    return Err(DecodeError::Value("segment"));
                }
                Item::Data { seq, idx, count, bytes: r.rest().to_vec() }
            }
            I_GAP => {
                let (from_slot, to_slot) = (r.u64()?, r.u64()?);
                if to_slot < from_slot {
                    return Err(DecodeError::Value("gap"));
                }
                Item::MeasGap { from_slot, to_slot }
            }
            I_KEEPALIVE => Item::Keepalive,
            other => return Err(DecodeError::Type(other)),
        };
        r.end()?;
        Ok(it)
    }
}

pub fn encode_items(items: &[Item]) -> Vec<u8> {
    let mut v = Vec::new();
    for i in items {
        i.write(&mut v);
    }
    v
}

/// Strict: any malformed item fails the whole list.
pub fn decode_items(b: &[u8]) -> DResult<Vec<Item>> {
    let mut r = R::new(b);
    let mut out = Vec::new();
    while r.left() > 0 {
        let t = r.u8()?;
        let body = r.blob16()?;
        out.push(Item::read(t, body)?);
    }
    if out.is_empty() {
        return Err(DecodeError::Value("empty item list"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn grant() -> Grant {
        Grant { from_slot: 100, to_slot: 300, period: 4, offset: 1, len: 2, subchannel: 2, cqi: 9 }
    }

    pub fn all_plain() -> Vec<Plain> {
        vec![
            Plain::SysInfo(SysInfo {
                version: VERSION,
                carrier: "cell-red".into(),
                cell_id: 7,
                area: 3,
                flags: SIB_CORE_UP,
                profile: 1,
                channel: 3,
                tx_power_dbm: 43,
                ul_mask: 0b0101,
                rach_sub: 4,
                neighbours: vec![Neighbour { cell_id: 8, profile: 1, channel: 1, ul_mask: 2 }],
            }),
            Plain::RachRequest { nonce: 0xdead, cause: RachCause::Service },
            Plain::RachResponse { nonce: 1, ue_id: 2, grant: grant() },
            Plain::AttachRequest { imsi: "310150000000001".into(), carrier: "cell-red".into() },
            Plain::AuthChallenge { rand: [7; 16] },
            Plain::AuthResponse { res: [8; 16] },
            Plain::AttachReject { cause: Cause::WrongCarrier },
            Plain::SecurityModeCommand { epoch: 9, mac: [1; 16] },
            Plain::Paging { tmsis: vec![1, 2, 3] },
            Plain::ServiceRequest { tmsi: 5, count: 6, mac: [2; 16] },
        ]
    }

    pub fn all_items() -> Vec<Item> {
        vec![
            Item::SecurityModeComplete,
            Item::AttachAccept { ip: [100, 65, 2, 10], prefix: 24, gateway: [100, 65, 2, 1], dns: [0; 4], mtu: 1500, tmsi: 77, session: 5 },
            Item::AttachComplete,
            Item::Grant(grant()),
            Item::BufferStatus { bytes: 4000 },
            Item::MeasurementReport {
                cells: vec![Measurement { cell_id: 1, profile: 1, channel: 1, rssi_x10: -653, sinr_x10: 211, ul_mask: 1 }],
            },
            Item::HandoverCommand { cell_id: 2, addr: 0xB2, profile: 1, channel: 3, epoch: 4, ue_id: 9, grant: grant() },
            Item::HandoverConfirm,
            Item::Detach { cause: Cause::UserDetach },
            Item::DetachAccept,
            Item::Release { cause: Cause::QuotaExhausted },
            Item::Data { seq: 42, idx: 1, count: 2, bytes: vec![1, 2, 3] },
            Item::MeasGap { from_slot: 5, to_slot: 9 },
            Item::Keepalive,
        ]
    }

    #[test]
    fn plain_round_trip() {
        for m in all_plain() {
            let b = m.encode();
            assert_eq!(Plain::decode(&b).as_ref(), Ok(&m), "{:?}", m);
            // every truncation fails cleanly; one extra byte is trailing
            for n in 0..b.len() {
                assert!(Plain::decode(&b[..n]).is_err(), "{:?} cut at {}", m, n);
            }
            let mut long = b.clone();
            long.push(0);
            assert!(Plain::decode(&long).is_err());
        }
    }

    #[test]
    fn items_round_trip() {
        let items = all_items();
        let b = encode_items(&items);
        assert_eq!(decode_items(&b).unwrap(), items);
        for i in &items {
            assert_eq!(i.encoded_len(), encode_items(core::slice::from_ref(i)).len());
        }
        for n in 0..b.len() {
            let _ = decode_items(&b[..n]); // must not panic
        }
        assert!(decode_items(&[]).is_err());
    }

    #[test]
    fn bad_values_rejected() {
        let mut g = grant();
        g.len = 5; // longer than the period
        let mut w = W::new();
        w.u8(T_RACH_RESP).u32(1).u16(2);
        g.write(&mut w);
        assert_eq!(Plain::decode(&w.into_vec()), Err(DecodeError::Value("grant")));
        assert_eq!(Plain::decode(&[T_ATTACH_REJ, 99]), Err(DecodeError::Value("cause")));
        assert_eq!(Plain::decode(&[0x7f]), Err(DecodeError::Type(0x7f)));
        assert!(decode_items(&[I_DATA, 0, 6, 0, 0, 0, 1, 2, 2]).is_err(), "idx >= count");
    }

    #[test]
    fn grant_runs() {
        let g = grant(); // period 4, offset 1, len 2 -> slots 1,2 mod 4
        assert!(!g.allows(100) && g.allows(101) && g.allows(102) && !g.allows(103));
        assert_eq!(g.next_run(100), Some((101, 2)));
        assert_eq!(g.next_run(102), Some((102, 1)));
        assert_eq!(g.next_run(299), None);
        let c = Grant { from_slot: 0, to_slot: 10, period: 1, offset: 0, len: 1, subchannel: 1, cqi: 1 };
        assert_eq!(c.next_run(3), Some((3, 1)));
        assert_eq!(c.next_run(10), None);
    }

    /// Fuzz-ish: random bytes and bit flips of valid messages never panic.
    #[test]
    fn garbage_never_panics() {
        let mut x: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let seeds: Vec<Vec<u8>> = all_plain().iter().map(|m| m.encode()).chain([encode_items(&all_items())]).collect();
        for round in 0..20_000 {
            let mut b = if round % 2 == 0 {
                let n = (next() % 64) as usize;
                (0..n).map(|_| next() as u8).collect::<Vec<u8>>()
            } else {
                seeds[(next() % seeds.len() as u64) as usize].clone()
            };
            if !b.is_empty() {
                for _ in 0..1 + next() % 4 {
                    let i = (next() % b.len() as u64) as usize;
                    b[i] ^= 1 << (next() % 8);
                }
            }
            let _ = Plain::decode(&b);
            let _ = decode_items(&b);
        }
    }
}
