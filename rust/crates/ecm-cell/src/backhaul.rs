//! Site <-> core protocol.
//!
//! **Security** (version 2): the connection starts with two plain messages,
//! `CoreHello` (the core's nonce) and `SiteAuth` (the site's nonce and its proof
//! of the site credential). From then on every control message both ways is
//! sealed with ChaCha20-Poly1305 under a key derived from the credential and both
//! nonces, and so is every tunnel datagram (another key): see
//! [`crate::crypto::BhChannel`]. The first sealed message, `SiteAccept`, proves
//! the core knows the credential too. Only `SiteReject` (the core has no key for
//! an unknown site) is plain after `SiteAuth`.
//!
//! **Control** runs over one TCP connection per site (the site connects to the
//! core, default port [`CONTROL_PORT`]): each message is `[u32 len][u8 type][body]`
//! (`len` counts type + body, at most [`MAX_CONTROL`]). [`Framer`] splits a
//! byte stream into messages.
//!
//! **User plane** is UDP (default port [`TUNNEL_PORT`] at the core; the site
//! sends from its own `tunnel_port`), sealed (see above): one datagram per IP packet, or two when
//! the packet doesn't fit [`MAX_TUNNEL_PAYLOAD`] (MTU handling: the tunnel
//! fragments, the phone never needs a smaller MTU):
//!
//! ```text
//! [u8 0xE1][u8 flags: 1 = first of two, 2 = second of two][u16 reserved 0][u32 session][u32 seq][IP bytes]
//! ```
//! `seq` counts per session and direction; the core's downlink `seq` also
//! rides the air (Data items), so a phone drops the duplicates a
//! make-before-break handover sends both ways.

use alloc::string::String;
use alloc::vec::Vec;

use crate::air::{Bearer, Cause, ImsInfo};
use crate::codec::{DResult, DecodeError, R, W};

pub const CONTROL_PORT: u16 = 7700;
pub const TUNNEL_PORT: u16 = 7701;
pub const VERSION: u8 = 2;
pub const MAX_CONTROL: usize = 65_536;

/// A cell a site serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellInfo {
    pub cell_id: u32,
    pub profile: u8,
    pub channel: u8,
    pub area: u16,
    /// The cell's radio address.
    pub addr: u32,
}

/// Addressing a phone's session gets at attach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct IpConfig {
    pub ip: [u8; 4],
    pub prefix: u8,
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub mtu: u16,
}

impl IpConfig {
    fn write(&self, w: &mut W) {
        w.raw(&self.ip).u8(self.prefix).raw(&self.gateway).raw(&self.dns).u16(self.mtu);
    }
    fn read(r: &mut R) -> DResult<IpConfig> {
        let c = IpConfig { ip: r.arr()?, prefix: r.u8()?, gateway: r.arr()?, dns: r.arr()?, mtu: r.u16()? };
        if c.prefix > 32 {
            return Err(DecodeError::Value("prefix"));
        }
        Ok(c)
    }
}

/// Keys and identifiers the core gives a site for one phone in one cell.
#[derive(Clone, PartialEq, Eq)]
pub struct AsContext {
    pub epoch: u32,
    pub k_as: [u8; 32],
}

impl core::fmt::Debug for AsContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AsContext {{ epoch: {}, k_as: <redacted> }}", self.epoch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ctl {
    /// 0x01 core -> site, first on a new connection.
    CoreHello { version: u8, carrier: String, nonce: [u8; 16] },
    /// 0x02 site -> core: `proof = HMAC(site_secret, "ECM-SITE-AUTH" || core_nonce || site_nonce || site_id)`.
    SiteAuth { version: u8, site_id: String, cells: Vec<CellInfo>, tunnel_port: u16, nonce: [u8; 16], proof: [u8; 32] },
    /// 0x03 core -> site.
    SiteAccept { heartbeat_ms: u16, revoked: Vec<String> },
    /// 0x04 core -> site, then the core closes.
    SiteReject { cause: Cause },
    /// 0x05 either way, every `heartbeat_ms`; three missed = link down.
    Heartbeat { seq: u32 },
    /// 0x10 site -> core: a phone asks to attach (`ue` = its radio address).
    UeAttach { ue: u32, cell_id: u32, imsi: String, carrier: String },
    /// 0x11 site -> core: an idle phone resumes.
    UeService { ue: u32, cell_id: u32, tmsi: u32, count: u32, mac: [u8; 16] },
    /// 0x12 core -> site.
    UeChallenge { ue: u32, rand: [u8; 16] },
    /// 0x13 site -> core.
    UeAuthResponse { ue: u32, res: [u8; 16] },
    /// 0x14 core -> site: authenticated; protect the air with `ctx` after the SMC.
    /// `ims`: the IMS bearer, when the carrier offers calls and the SIM has a number
    /// (a flag byte, then the fields when 1).
    UeAccept { ue: u32, session: u32, ip: IpConfig, tmsi: u32, ctx: AsContext, smc_mac: [u8; 16], ims: Option<ImsInfo> },
    /// 0x15 core -> site.
    UeReject { ue: u32, cause: Cause },
    /// 0x16 site -> core: attach complete, the phone is up.
    UeActive { session: u32 },
    /// 0x17 either way: end the phone's radio context at this site (core ->
    /// site: tell the phone `cause`; site -> core: the phone left / was lost).
    UeRelease { session: u32, cause: Cause },
    /// 0x18 site -> core: the phone went idle at this site.
    UeIdle { session: u32 },
    /// 0x20 source site -> core.
    HoRequired { session: u32, target_cell: u32 },
    /// 0x21 core -> target site.
    /// `bearers`: the phone's dedicated bearers (a call's voice flow), restarted at the target.
    HoRequest { ho: u32, session: u32, ue: u32, target_cell: u32, ip: IpConfig, tmsi: u32, ctx: AsContext, bearers: Vec<Bearer> },
    /// 0x22 target -> core.
    HoRequestAck { ho: u32, ue_id: u16, grant: crate::air::Grant },
    /// 0x23 target -> core or core -> source.
    HoFailure { ho: u32, session: u32, cause: Cause },
    /// 0x24 core -> source: send this to the phone.
    HoCommand { ho: u32, session: u32, cell_id: u32, addr: u32, profile: u8, channel: u8, epoch: u32, ue_id: u16, grant: crate::air::Grant },
    /// 0x25 target -> core: the phone arrived.
    PathSwitch { ho: u32, session: u32 },
    /// 0x26 core -> target.
    PathSwitchAck { session: u32 },
    /// 0x30 core -> site: page an idle phone.
    Page { tmsi: u32 },
    /// 0x31 core -> site: usage of a session.
    Usage { session: u32, used: u64, quota: u64 },
    /// 0x32 core -> site: refuse this IMSI locally from now on.
    Revoked { imsi: String },
    /// 0x40 core -> site: a dedicated bearer for a session (PCRF decision, e.g. a call's voice).
    BearerSetup { session: u32, bearer: Bearer },
    /// 0x41 core -> site.
    BearerRelease { session: u32, id: u8 },
    /// 0x42 site -> core: admitted (`ok`) or not (the cell's voice capacity is full:
    /// the flow stays best effort on the default scheduling).
    BearerResult { session: u32, id: u8, ok: bool },
}

const C_CORE_HELLO: u8 = 0x01;
const C_SITE_AUTH: u8 = 0x02;
const C_SITE_ACCEPT: u8 = 0x03;
const C_SITE_REJECT: u8 = 0x04;
const C_HEARTBEAT: u8 = 0x05;
const C_UE_ATTACH: u8 = 0x10;
const C_UE_SERVICE: u8 = 0x11;
const C_UE_CHALLENGE: u8 = 0x12;
const C_UE_AUTH_RESP: u8 = 0x13;
const C_UE_ACCEPT: u8 = 0x14;
const C_UE_REJECT: u8 = 0x15;
const C_UE_ACTIVE: u8 = 0x16;
const C_UE_RELEASE: u8 = 0x17;
const C_UE_IDLE: u8 = 0x18;
const C_HO_REQUIRED: u8 = 0x20;
const C_HO_REQUEST: u8 = 0x21;
const C_HO_ACK: u8 = 0x22;
const C_HO_FAILURE: u8 = 0x23;
const C_HO_COMMAND: u8 = 0x24;
const C_PATH_SWITCH: u8 = 0x25;
const C_PATH_SWITCH_ACK: u8 = 0x26;
const C_PAGE: u8 = 0x30;
const C_USAGE: u8 = 0x31;
const C_REVOKED: u8 = 0x32;
const C_BEARER_SETUP: u8 = 0x40;
const C_BEARER_RELEASE: u8 = 0x41;
const C_BEARER_RESULT: u8 = 0x42;

impl Ctl {
    /// The message with its `u32` length prefix.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::new();
        w.u32(0);
        match self {
            Ctl::CoreHello { version, carrier, nonce } => {
                w.u8(C_CORE_HELLO).u8(*version).str8(carrier).raw(nonce);
            }
            Ctl::SiteAuth { version, site_id, cells, tunnel_port, nonce, proof } => {
                w.u8(C_SITE_AUTH).u8(*version).str8(site_id).u8(cells.len().min(16) as u8);
                for c in cells.iter().take(16) {
                    w.u32(c.cell_id).u8(c.profile).u8(c.channel).u16(c.area).u32(c.addr);
                }
                w.u16(*tunnel_port).raw(nonce).raw(proof);
            }
            Ctl::SiteAccept { heartbeat_ms, revoked } => {
                w.u8(C_SITE_ACCEPT).u16(*heartbeat_ms).u16(revoked.len().min(4096) as u16);
                for i in revoked.iter().take(4096) {
                    w.str8(i);
                }
            }
            Ctl::SiteReject { cause } => {
                w.u8(C_SITE_REJECT).u8(*cause as u8);
            }
            Ctl::Heartbeat { seq } => {
                w.u8(C_HEARTBEAT).u32(*seq);
            }
            Ctl::UeAttach { ue, cell_id, imsi, carrier } => {
                w.u8(C_UE_ATTACH).u32(*ue).u32(*cell_id).str8(imsi).str8(carrier);
            }
            Ctl::UeService { ue, cell_id, tmsi, count, mac } => {
                w.u8(C_UE_SERVICE).u32(*ue).u32(*cell_id).u32(*tmsi).u32(*count).raw(mac);
            }
            Ctl::UeChallenge { ue, rand } => {
                w.u8(C_UE_CHALLENGE).u32(*ue).raw(rand);
            }
            Ctl::UeAuthResponse { ue, res } => {
                w.u8(C_UE_AUTH_RESP).u32(*ue).raw(res);
            }
            Ctl::UeAccept { ue, session, ip, tmsi, ctx, smc_mac, ims } => {
                w.u8(C_UE_ACCEPT).u32(*ue).u32(*session);
                ip.write(&mut w);
                w.u32(*tmsi).u32(ctx.epoch).raw(&ctx.k_as).raw(smc_mac);
                match ims {
                    Some(i) => {
                        w.u8(1);
                        i.write(&mut w);
                    }
                    None => {
                        w.u8(0);
                    }
                }
            }
            Ctl::UeReject { ue, cause } => {
                w.u8(C_UE_REJECT).u32(*ue).u8(*cause as u8);
            }
            Ctl::UeActive { session } => {
                w.u8(C_UE_ACTIVE).u32(*session);
            }
            Ctl::UeRelease { session, cause } => {
                w.u8(C_UE_RELEASE).u32(*session).u8(*cause as u8);
            }
            Ctl::UeIdle { session } => {
                w.u8(C_UE_IDLE).u32(*session);
            }
            Ctl::HoRequired { session, target_cell } => {
                w.u8(C_HO_REQUIRED).u32(*session).u32(*target_cell);
            }
            Ctl::HoRequest { ho, session, ue, target_cell, ip, tmsi, ctx, bearers } => {
                w.u8(C_HO_REQUEST).u32(*ho).u32(*session).u32(*ue).u32(*target_cell);
                ip.write(&mut w);
                w.u32(*tmsi).u32(ctx.epoch).raw(&ctx.k_as);
                w.u8(bearers.len().min(8) as u8);
                for b in bearers.iter().take(8) {
                    b.write(&mut w);
                }
            }
            Ctl::HoRequestAck { ho, ue_id, grant } => {
                w.u8(C_HO_ACK).u32(*ho).u16(*ue_id);
                grant.write(&mut w);
            }
            Ctl::HoFailure { ho, session, cause } => {
                w.u8(C_HO_FAILURE).u32(*ho).u32(*session).u8(*cause as u8);
            }
            Ctl::HoCommand { ho, session, cell_id, addr, profile, channel, epoch, ue_id, grant } => {
                w.u8(C_HO_COMMAND).u32(*ho).u32(*session).u32(*cell_id).u32(*addr).u8(*profile).u8(*channel).u32(*epoch).u16(*ue_id);
                grant.write(&mut w);
            }
            Ctl::PathSwitch { ho, session } => {
                w.u8(C_PATH_SWITCH).u32(*ho).u32(*session);
            }
            Ctl::PathSwitchAck { session } => {
                w.u8(C_PATH_SWITCH_ACK).u32(*session);
            }
            Ctl::Page { tmsi } => {
                w.u8(C_PAGE).u32(*tmsi);
            }
            Ctl::Usage { session, used, quota } => {
                w.u8(C_USAGE).u32(*session).u64(*used).u64(*quota);
            }
            Ctl::Revoked { imsi } => {
                w.u8(C_REVOKED).str8(imsi);
            }
            Ctl::BearerSetup { session, bearer } => {
                w.u8(C_BEARER_SETUP).u32(*session);
                bearer.write(&mut w);
            }
            Ctl::BearerRelease { session, id } => {
                w.u8(C_BEARER_RELEASE).u32(*session).u8(*id);
            }
            Ctl::BearerResult { session, id, ok } => {
                w.u8(C_BEARER_RESULT).u32(*session).u8(*id).u8(*ok as u8);
            }
        }
        let mut v = w.into_vec();
        let n = (v.len() - 4) as u32;
        v[..4].copy_from_slice(&n.to_be_bytes());
        v
    }

    /// Decode one message body (type + fields, no length prefix).
    pub fn decode(b: &[u8]) -> DResult<Ctl> {
        use crate::air::Grant;
        let mut r = R::new(b);
        let t = r.u8()?;
        let ctx = |r: &mut R| -> DResult<AsContext> { Ok(AsContext { epoch: r.u32()?, k_as: r.arr()? }) };
        let m = match t {
            C_CORE_HELLO => Ctl::CoreHello { version: r.u8()?, carrier: r.str8()?, nonce: r.arr()? },
            C_SITE_AUTH => {
                let version = r.u8()?;
                let site_id = r.str8()?;
                let n = r.count8(12)?;
                let mut cells = Vec::with_capacity(n);
                for _ in 0..n {
                    cells.push(CellInfo { cell_id: r.u32()?, profile: r.u8()?, channel: r.u8()?, area: r.u16()?, addr: r.u32()? });
                }
                Ctl::SiteAuth { version, site_id, cells, tunnel_port: r.u16()?, nonce: r.arr()?, proof: r.arr()? }
            }
            C_SITE_ACCEPT => {
                let heartbeat_ms = r.u16()?;
                let n = r.count16(1)?;
                let mut revoked = Vec::with_capacity(n);
                for _ in 0..n {
                    revoked.push(r.str8()?);
                }
                Ctl::SiteAccept { heartbeat_ms, revoked }
            }
            C_SITE_REJECT => Ctl::SiteReject { cause: Cause::from_code(r.u8()?)? },
            C_HEARTBEAT => Ctl::Heartbeat { seq: r.u32()? },
            C_UE_ATTACH => Ctl::UeAttach { ue: r.u32()?, cell_id: r.u32()?, imsi: r.str8()?, carrier: r.str8()? },
            C_UE_SERVICE => Ctl::UeService { ue: r.u32()?, cell_id: r.u32()?, tmsi: r.u32()?, count: r.u32()?, mac: r.arr()? },
            C_UE_CHALLENGE => Ctl::UeChallenge { ue: r.u32()?, rand: r.arr()? },
            C_UE_AUTH_RESP => Ctl::UeAuthResponse { ue: r.u32()?, res: r.arr()? },
            C_UE_ACCEPT => Ctl::UeAccept {
                ue: r.u32()?,
                session: r.u32()?,
                ip: IpConfig::read(&mut r)?,
                tmsi: r.u32()?,
                ctx: ctx(&mut r)?,
                smc_mac: r.arr()?,
                ims: match r.u8()? {
                    0 => None,
                    1 => Some(ImsInfo::read(&mut r)?),
                    _ => return Err(DecodeError::Value("ims flag")),
                },
            },
            C_UE_REJECT => Ctl::UeReject { ue: r.u32()?, cause: Cause::from_code(r.u8()?)? },
            C_UE_ACTIVE => Ctl::UeActive { session: r.u32()? },
            C_UE_RELEASE => Ctl::UeRelease { session: r.u32()?, cause: Cause::from_code(r.u8()?)? },
            C_UE_IDLE => Ctl::UeIdle { session: r.u32()? },
            C_HO_REQUIRED => Ctl::HoRequired { session: r.u32()?, target_cell: r.u32()? },
            C_HO_REQUEST => {
                let (ho, session, ue, target_cell) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?);
                let ip = IpConfig::read(&mut r)?;
                let tmsi = r.u32()?;
                let ctx = ctx(&mut r)?;
                let n = r.count8(10)?;
                let mut bearers = Vec::with_capacity(n);
                for _ in 0..n {
                    bearers.push(Bearer::read(&mut r)?);
                }
                Ctl::HoRequest { ho, session, ue, target_cell, ip, tmsi, ctx, bearers }
            }
            C_HO_ACK => Ctl::HoRequestAck { ho: r.u32()?, ue_id: r.u16()?, grant: Grant::read(&mut r)? },
            C_HO_FAILURE => Ctl::HoFailure { ho: r.u32()?, session: r.u32()?, cause: Cause::from_code(r.u8()?)? },
            C_HO_COMMAND => Ctl::HoCommand {
                ho: r.u32()?,
                session: r.u32()?,
                cell_id: r.u32()?,
                addr: r.u32()?,
                profile: r.u8()?,
                channel: r.u8()?,
                epoch: r.u32()?,
                ue_id: r.u16()?,
                grant: Grant::read(&mut r)?,
            },
            C_PATH_SWITCH => Ctl::PathSwitch { ho: r.u32()?, session: r.u32()? },
            C_PATH_SWITCH_ACK => Ctl::PathSwitchAck { session: r.u32()? },
            C_PAGE => Ctl::Page { tmsi: r.u32()? },
            C_USAGE => Ctl::Usage { session: r.u32()?, used: r.u64()?, quota: r.u64()? },
            C_REVOKED => Ctl::Revoked { imsi: r.str8()? },
            C_BEARER_SETUP => Ctl::BearerSetup { session: r.u32()?, bearer: Bearer::read(&mut r)? },
            C_BEARER_RELEASE => Ctl::BearerRelease { session: r.u32()?, id: r.u8()? },
            C_BEARER_RESULT => Ctl::BearerResult {
                session: r.u32()?,
                id: r.u8()?,
                ok: match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(DecodeError::Value("bearer result")),
                },
            },
            other => return Err(DecodeError::Type(other)),
        };
        r.end()?;
        Ok(m)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Ctl::CoreHello { .. } => "CoreHello",
            Ctl::SiteAuth { .. } => "SiteAuth",
            Ctl::SiteAccept { .. } => "SiteAccept",
            Ctl::SiteReject { .. } => "SiteReject",
            Ctl::Heartbeat { .. } => "Heartbeat",
            Ctl::UeAttach { .. } => "UeAttach",
            Ctl::UeService { .. } => "UeService",
            Ctl::UeChallenge { .. } => "UeChallenge",
            Ctl::UeAuthResponse { .. } => "UeAuthResponse",
            Ctl::UeAccept { .. } => "UeAccept",
            Ctl::UeReject { .. } => "UeReject",
            Ctl::UeActive { .. } => "UeActive",
            Ctl::UeRelease { .. } => "UeRelease",
            Ctl::UeIdle { .. } => "UeIdle",
            Ctl::HoRequired { .. } => "HoRequired",
            Ctl::HoRequest { .. } => "HoRequest",
            Ctl::HoRequestAck { .. } => "HoRequestAck",
            Ctl::HoFailure { .. } => "HoFailure",
            Ctl::HoCommand { .. } => "HoCommand",
            Ctl::PathSwitch { .. } => "PathSwitch",
            Ctl::PathSwitchAck { .. } => "PathSwitchAck",
            Ctl::Page { .. } => "Page",
            Ctl::Usage { .. } => "Usage",
            Ctl::Revoked { .. } => "Revoked",
            Ctl::BearerSetup { .. } => "BearerSetup",
            Ctl::BearerRelease { .. } => "BearerRelease",
            Ctl::BearerResult { .. } => "BearerResult",
        }
    }
}

/// What the site's registration proof covers: the `CoreHello` body and the
/// `SiteAuth` body without its proof (both re-encoded: the encodings are canonical).
pub fn auth_transcript(hello: &Ctl, auth: &Ctl) -> Vec<u8> {
    let mut t = hello.encode()[4..].to_vec();
    let a = auth.encode();
    t.extend_from_slice(&a[4..a.len() - 32]);
    t
}

/// Splits a TCP byte stream into control messages.
#[derive(Default)]
pub struct Framer {
    buf: Vec<u8>,
    /// A framing error (bad length or undecodable message): close the connection.
    pub broken: bool,
}

impl Framer {
    pub fn new() -> Framer {
        Framer::default()
    }
    pub fn push(&mut self, bytes: &[u8]) {
        if !self.broken {
            self.buf.extend_from_slice(bytes);
        }
    }
    /// The next complete message body (type + fields, or a sealed body), without decoding it.
    pub fn pop_body(&mut self) -> Option<Vec<u8>> {
        if self.broken || self.buf.len() < 4 {
            return None;
        }
        let n = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        if n == 0 || n > MAX_CONTROL {
            self.broken = true;
            return None;
        }
        if self.buf.len() < 4 + n {
            return None;
        }
        let b = self.buf[4..4 + n].to_vec();
        self.buf.drain(..4 + n);
        Some(b)
    }

    /// The next complete plain message, `None` when more bytes are needed or the stream is broken.
    pub fn pop(&mut self) -> Option<Ctl> {
        let b = self.pop_body()?;
        match Ctl::decode(&b) {
            Ok(m) => Some(m),
            Err(_) => {
                self.broken = true;
                None
            }
        }
    }
}

pub const TUNNEL_MAGIC: u8 = 0xE1;
pub const TUNNEL_HEADER: usize = 12;
/// Largest IP payload one tunnel datagram carries: a 1500-byte link MTU
/// minus IPv4 (20), UDP (8), the seal (header and tag) and the tunnel header.
pub const MAX_TUNNEL_PAYLOAD: usize = 1500 - 20 - 8 - crate::crypto::SEALED_TUNNEL_OVERHEAD - TUNNEL_HEADER;
pub const TF_FIRST: u8 = 1;
pub const TF_SECOND: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelPacket {
    pub session: u32,
    pub seq: u32,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl TunnelPacket {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W::new();
        w.u8(TUNNEL_MAGIC).u8(self.flags).u16(0).u32(self.session).u32(self.seq).raw(&self.payload);
        w.into_vec()
    }
    pub fn decode(b: &[u8]) -> DResult<TunnelPacket> {
        let mut r = R::new(b);
        if r.u8()? != TUNNEL_MAGIC {
            return Err(DecodeError::Value("tunnel magic"));
        }
        let flags = r.u8()?;
        if flags & !(TF_FIRST | TF_SECOND) != 0 || flags == TF_FIRST | TF_SECOND {
            return Err(DecodeError::Value("tunnel flags"));
        }
        if r.u16()? != 0 {
            return Err(DecodeError::Value("tunnel reserved"));
        }
        let session = r.u32()?;
        let seq = r.u32()?;
        let payload = r.rest().to_vec();
        if payload.is_empty() {
            return Err(DecodeError::Short);
        }
        Ok(TunnelPacket { session, seq, flags, payload })
    }
}

/// One IP packet as 1 or 2 tunnel datagrams.
pub fn tunnel_split(session: u32, seq: u32, ip: &[u8]) -> Vec<Vec<u8>> {
    if ip.len() <= MAX_TUNNEL_PAYLOAD {
        return alloc::vec![TunnelPacket { session, seq, flags: 0, payload: ip.to_vec() }.encode()];
    }
    let h = ip.len().div_ceil(2);
    alloc::vec![
        TunnelPacket { session, seq, flags: TF_FIRST, payload: ip[..h].to_vec() }.encode(),
        TunnelPacket { session, seq, flags: TF_SECOND, payload: ip[h..].to_vec() }.encode(),
    ]
}

/// Rejoins two-part tunnel packets (one pending first half per session).
#[derive(Default)]
pub struct TunnelJoin {
    pending: alloc::collections::BTreeMap<u32, (u32, Vec<u8>)>,
}

impl TunnelJoin {
    /// A whole IP packet `(session, seq, bytes)` once complete.
    pub fn push(&mut self, p: TunnelPacket) -> Option<(u32, u32, Vec<u8>)> {
        match p.flags {
            0 => Some((p.session, p.seq, p.payload)),
            TF_FIRST => {
                self.pending.insert(p.session, (p.seq, p.payload));
                None
            }
            _ => match self.pending.remove(&p.session) {
                Some((seq, mut first)) if seq == p.seq => {
                    first.extend_from_slice(&p.payload);
                    Some((p.session, seq, first))
                }
                _ => None,
            },
        }
    }
    pub fn forget(&mut self, session: u32) {
        self.pending.remove(&session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air::Grant;
    use alloc::vec;

    pub fn all() -> Vec<Ctl> {
        let ip = IpConfig { ip: [100, 65, 2, 10], prefix: 24, gateway: [100, 65, 2, 1], dns: [1, 1, 1, 1], mtu: 1500 };
        let ctx = AsContext { epoch: 3, k_as: [5; 32] };
        let g = Grant { from_slot: 1, to_slot: 9, period: 1, offset: 0, len: 1, subchannel: 1, cqi: 4 };
        let bearer = Bearer { id: 1, qci: crate::air::QCI_VOICE, ue_port: 40000, remote: [100, 65, 2, 1], remote_port: 30000 };
        vec![
            Ctl::CoreHello { version: 1, carrier: "cell-red".into(), nonce: [1; 16] },
            Ctl::SiteAuth {
                version: 1,
                site_id: "site-a".into(),
                cells: vec![CellInfo { cell_id: 1, profile: 1, channel: 1, area: 1, addr: 0xA1 }],
                tunnel_port: 7701,
                nonce: [9; 16],
                proof: [2; 32],
            },
            Ctl::SiteAccept { heartbeat_ms: 1000, revoked: vec!["310150000000009".into()] },
            Ctl::SiteReject { cause: Cause::AuthFailed },
            Ctl::Heartbeat { seq: 4 },
            Ctl::UeAttach { ue: 0xB, cell_id: 1, imsi: "310150000000001".into(), carrier: "cell-red".into() },
            Ctl::UeService { ue: 0xB, cell_id: 1, tmsi: 9, count: 2, mac: [3; 16] },
            Ctl::UeChallenge { ue: 0xB, rand: [4; 16] },
            Ctl::UeAuthResponse { ue: 0xB, res: [5; 16] },
            Ctl::UeAccept { ue: 0xB, session: 7, ip, tmsi: 8, ctx: ctx.clone(), smc_mac: [6; 16], ims: None },
            Ctl::UeAccept {
                ue: 0xB,
                session: 7,
                ip,
                tmsi: 8,
                ctx: ctx.clone(),
                smc_mac: [6; 16],
                ims: Some(ImsInfo { ip: [10, 250, 0, 2], prefix: 16, pcscf: [100, 65, 2, 1], domain: "cell-red.ecm".into(), msisdn: "+12145550142".into() }),
            },
            Ctl::UeReject { ue: 0xB, cause: Cause::Revoked },
            Ctl::UeActive { session: 7 },
            Ctl::UeRelease { session: 7, cause: Cause::QuotaExhausted },
            Ctl::UeIdle { session: 7 },
            Ctl::HoRequired { session: 7, target_cell: 2 },
            Ctl::HoRequest { ho: 1, session: 7, ue: 0xB, target_cell: 2, ip, tmsi: 8, ctx: ctx.clone(), bearers: vec![] },
            Ctl::HoRequest { ho: 1, session: 7, ue: 0xB, target_cell: 2, ip, tmsi: 8, ctx, bearers: vec![bearer] },
            Ctl::HoRequestAck { ho: 1, ue_id: 3, grant: g },
            Ctl::HoFailure { ho: 1, session: 7, cause: Cause::Congestion },
            Ctl::HoCommand { ho: 1, session: 7, cell_id: 2, addr: 0xA2, profile: 1, channel: 3, epoch: 4, ue_id: 3, grant: g },
            Ctl::PathSwitch { ho: 1, session: 7 },
            Ctl::PathSwitchAck { session: 7 },
            Ctl::Page { tmsi: 8 },
            Ctl::Usage { session: 7, used: 100, quota: 1000 },
            Ctl::Revoked { imsi: "310150000000009".into() },
            Ctl::BearerSetup { session: 7, bearer },
            Ctl::BearerRelease { session: 7, id: 1 },
            Ctl::BearerResult { session: 7, id: 1, ok: true },
        ]
    }

    #[test]
    fn control_round_trip_through_the_framer() {
        let msgs = all();
        let mut stream = Vec::new();
        for m in &msgs {
            stream.extend_from_slice(&m.encode());
        }
        // feed byte by byte: framing never depends on read sizes
        let mut f = Framer::new();
        let mut got = Vec::new();
        for b in &stream {
            f.push(core::slice::from_ref(b));
            while let Some(m) = f.pop() {
                got.push(m);
            }
        }
        assert!(!f.broken);
        assert_eq!(got, msgs);
        for m in &msgs {
            let b = m.encode();
            for n in 4..b.len() {
                assert!(Ctl::decode(&b[4..n]).is_err(), "{} cut at {}", m.name(), n);
            }
        }
    }

    #[test]
    fn the_proof_covers_the_whole_handshake() {
        let hello = Ctl::CoreHello { version: VERSION, carrier: "cell-red".into(), nonce: [1; 16] };
        let auth = |cell: u32, port: u16, proof: [u8; 32]| Ctl::SiteAuth {
            version: VERSION,
            site_id: "site-a".into(),
            cells: vec![CellInfo { cell_id: cell, profile: 1, channel: 1, area: 1, addr: 0xA1 }],
            tunnel_port: port,
            nonce: [2; 16],
            proof,
        };
        let t = auth_transcript(&hello, &auth(1, 7701, [0; 32]));
        assert_eq!(t, auth_transcript(&hello, &auth(1, 7701, [9; 32])), "the proof itself is not covered");
        assert_ne!(t, auth_transcript(&hello, &auth(257, 7701, [0; 32])), "cells");
        assert_ne!(t, auth_transcript(&hello, &auth(1, 7702, [0; 32])), "tunnel port");
        let other = Ctl::CoreHello { version: VERSION, carrier: "cmll-red".into(), nonce: [1; 16] };
        assert_ne!(t, auth_transcript(&other, &auth(1, 7701, [0; 32])), "the core's hello");
    }

    #[test]
    fn framer_breaks_on_garbage() {
        let mut f = Framer::new();
        f.push(&[0, 0, 0, 2, 0x7f, 0]);
        assert_eq!(f.pop(), None);
        assert!(f.broken);
        let mut f = Framer::new();
        f.push(&[0xff, 0xff, 0xff, 0xff]);
        assert_eq!(f.pop(), None);
        assert!(f.broken, "oversized length");
        let mut x: u32 = 12345;
        for _ in 0..5000 {
            let mut f = Framer::new();
            let n = (x % 40) as usize;
            let b: Vec<u8> = (0..n)
                .map(|_| {
                    x = x.wrapping_mul(1103515245).wrapping_add(12345);
                    (x >> 16) as u8
                })
                .collect();
            f.push(&b);
            while f.pop().is_some() {}
            let _ = Ctl::decode(&b);
            let _ = TunnelPacket::decode(&b);
        }
    }

    #[test]
    fn tunnel_split_and_join() {
        let small = vec![0x45u8; 100];
        let parts = tunnel_split(7, 1, &small);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].len(), TUNNEL_HEADER + 100);
        let big: Vec<u8> = (0..1500u32).map(|i| i as u8).collect();
        let parts = tunnel_split(7, 2, &big);
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|p| p.len() + crate::crypto::SEALED_TUNNEL_OVERHEAD <= 1500 - 28));
        let mut j = TunnelJoin::default();
        assert_eq!(j.push(TunnelPacket::decode(&parts[0]).unwrap()), None);
        assert_eq!(j.push(TunnelPacket::decode(&parts[1]).unwrap()), Some((7, 2, big.clone())));
        // a second half without its first is dropped
        assert_eq!(j.push(TunnelPacket::decode(&parts[1]).unwrap()), None);
        assert!(TunnelPacket::decode(&[TUNNEL_MAGIC, 3, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 9]).is_err());
        assert!(TunnelPacket::decode(&[0xE2]).is_err());
    }
}
