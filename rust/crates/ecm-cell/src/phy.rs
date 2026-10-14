//! The radio below the protocol: a mirror of the SDR framed PHY record API
//! (docs/radio/SDR_PROGRAMS.md "Framed PHY mode"; Rust `ecm_radio::framed`).
//!
//! The state machines never touch a radio. They emit [`PhyCmd`]s and take
//! [`PhyEvent`]s; a driver applies them to anything implementing
//! [`FramedPhy`]: the in-game device file (`ecm_radio::framed::Framed`, with
//! the `framed` feature) or the simulator's medium.

use alloc::vec::Vec;

pub const BROADCAST: u32 = 0xFFFF_FFFF;
/// Largest payload of one TX record.
pub const MAX_PAYLOAD: usize = 1500;

pub const FLAG_HARQ: u8 = 0x01;
pub const FLAG_ACK: u8 = 0x02;
pub const FLAG_BEACON: u8 = 0x04;
pub const FLAG_RETX: u8 = 0x08;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Lte,
    Mmwave,
}

impl Profile {
    pub fn code(self) -> u8 {
        match self {
            Profile::Lte => 1,
            Profile::Mmwave => 2,
        }
    }
    pub fn from_code(c: u8) -> Option<Profile> {
        match c {
            1 => Some(Profile::Lte),
            2 => Some(Profile::Mmwave),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Profile::Lte => "lte",
            Profile::Mmwave => "mmwave",
        }
    }
    pub fn parse(s: &str) -> Option<Profile> {
        match s {
            "lte" => Some(Profile::Lte),
            "mmwave" => Some(Profile::Mmwave),
            _ => None,
        }
    }
    /// Channel numbers 1..=channels().
    pub fn channels(self) -> u8 {
        match self {
            Profile::Lte => 6,
            Profile::Mmwave => 4,
        }
    }
    /// Slot length on the airtime clock, µs.
    pub fn slot_us(self) -> u64 {
        match self {
            Profile::Lte => 1000,
            Profile::Mmwave => 125,
        }
    }
    pub fn max_cqi(self) -> u8 {
        match self {
            Profile::Lte => 15,
            Profile::Mmwave => 18,
        }
    }
    /// Downlink / uplink centre frequency of a channel, Hz.
    pub fn dl_hz(self, ch: u8) -> f64 {
        match self {
            Profile::Lte => 791e6 + (ch.max(1) - 1) as f64 * 10e6,
            Profile::Mmwave => 28.05e9 + (ch.max(1) - 1) as f64 * 100e6,
        }
    }
    pub fn ul_hz(self, ch: u8) -> f64 {
        match self {
            Profile::Lte => 703e6 + (ch.max(1) - 1) as f64 * 10e6,
            Profile::Mmwave => 27.55e9 + (ch.max(1) - 1) as f64 * 100e6,
        }
    }
    pub fn bandwidth_hz(self) -> f64 {
        match self {
            Profile::Lte => 10e6,
            Profile::Mmwave => 100e6,
        }
    }
    /// Default transmit power for a role, dBm.
    pub fn default_power_dbm(self, role: Role) -> f64 {
        match (self, role) {
            (Profile::Lte, Role::Bs) => 43.0,
            (Profile::Mmwave, Role::Bs) => 30.0,
            (_, Role::Ue) => 23.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Transmits on the downlink, hears the uplink.
    Bs,
    /// Transmits on the uplink, hears the downlink.
    Ue,
}

/// Radio configuration (`set_phy`, `set_role`, `set_channel`, `set_power`, `set_address`).
#[derive(Clone, Debug, PartialEq)]
pub struct PhyConfig {
    pub profile: Profile,
    pub role: Role,
    pub channel: u8,
    /// None = the role's default.
    pub power_dbm: Option<f64>,
    pub address: u32,
}

/// One TX record.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TxReq {
    pub dst: u32,
    pub harq: bool,
    /// Absolute slot; 0 = the next slot.
    pub slot: u64,
    /// 0 = automatic.
    pub cqi: u8,
    /// 0 = whole carrier, 1..=4 a quarter.
    pub subchannel: u8,
    /// Echoed in the STATUS record.
    pub tag: u16,
    pub payload: Vec<u8>,
}

/// One RX record.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RxFrame {
    pub flags: u8,
    pub src: u32,
    pub dst: u32,
    pub seq: u16,
    /// CQI this frame's SINR supports.
    pub cqi: u8,
    pub cqi_sent: u8,
    pub rssi_dbm: f64,
    pub sinr_db: f64,
    pub timestamp_us: u64,
    pub slot: u64,
    pub subchannel: u8,
    pub cqi_report: u8,
    pub payload: Vec<u8>,
}

impl RxFrame {
    pub fn is_beacon(&self) -> bool {
        self.flags & FLAG_BEACON != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TxResult {
    Sent,
    Acked,
    Failed,
    Rejected,
}

impl TxResult {
    pub fn from_code(c: u8) -> TxResult {
        match c {
            0 => TxResult::Sent,
            1 => TxResult::Acked,
            2 => TxResult::Failed,
            _ => TxResult::Rejected,
        }
    }
}

/// One STATUS record.
#[derive(Clone, Debug, PartialEq)]
pub struct TxStatus {
    pub result: TxResult,
    pub dst: u32,
    pub tag: u16,
    pub attempts: u8,
    pub cqi: u8,
    pub slot: u64,
    pub airtime_us: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PhyEvent {
    Rx(RxFrame),
    Status(TxStatus),
}

/// What a state machine asks of its radio.
#[derive(Clone, Debug, PartialEq)]
pub enum PhyCmd {
    Configure(PhyConfig),
    Tx(TxReq),
    /// Beacon offload: `period_ms` 0 stops beacon `index`.
    Beacon { index: u8, period_ms: u32, payload: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhyError(pub alloc::string::String);

/// A framed radio.
pub trait FramedPhy {
    fn configure(&mut self, c: &PhyConfig) -> Result<(), PhyError>;
    fn send(&mut self, tx: &TxReq) -> Result<(), PhyError>;
    fn beacon(&mut self, index: u8, period_ms: u32, payload: &[u8]) -> Result<(), PhyError>;
    /// Next waiting record, without blocking.
    fn poll(&mut self) -> Option<PhyEvent>;
    /// Current slot on the airtime clock.
    fn slot(&mut self) -> u64;
}

/// Apply commands to a radio; the first error stops and is returned.
pub fn apply(phy: &mut dyn FramedPhy, cmds: impl IntoIterator<Item = PhyCmd>) -> Result<(), PhyError> {
    for c in cmds {
        match c {
            PhyCmd::Configure(cfg) => phy.configure(&cfg)?,
            PhyCmd::Tx(t) => phy.send(&t)?,
            PhyCmd::Beacon { index, period_ms, payload } => phy.beacon(index, period_ms, &payload)?,
        }
    }
    Ok(())
}

/// Spectral efficiency (bits/s/Hz) per CQI 1..=18, as `CqiTable.EFFICIENCY`.
pub const EFFICIENCY: [f64; 19] = [
    0.0, 0.1523, 0.2344, 0.3770, 0.6016, 0.8770, 1.1758, 1.4766, 1.9141, 2.4063, 2.7305, 3.3223, 3.9023, 4.5234,
    5.1152, 5.5547, 6.2266, 6.9141, 7.4063,
];
/// SINR for 10 % block error per CQI, dB (`CqiTable.REQUIRED_SINR_DB`).
pub const REQUIRED_SINR_DB: [f64; 19] = [
    0.0, -6.7, -4.7, -2.3, 0.2, 2.4, 4.3, 5.9, 8.1, 10.3, 11.7, 14.1, 16.3, 18.7, 21.0, 22.7, 24.5, 26.5, 28.5,
];
/// Link adaptation margin, dB.
pub const CQI_MARGIN_DB: f64 = 3.0;

/// The fastest CQI whose threshold + margin `sinr_db` clears (1 if none).
pub fn cqi_for(profile: Profile, sinr_db: f64) -> u8 {
    let mut best = 1;
    for c in 1..=profile.max_cqi() {
        if sinr_db >= REQUIRED_SINR_DB[c as usize] + CQI_MARGIN_DB {
            best = c;
        }
    }
    best
}

/// Air overhead of the PHY (control channels, reference signals).
pub fn overhead(profile: Profile) -> f64 {
    match profile {
        Profile::Lte => 0.25,
        Profile::Mmwave => 0.18,
    }
}

/// One OFDM symbol, µs.
pub fn symbol_us(profile: Profile) -> u64 {
    match profile {
        Profile::Lte => 71,
        Profile::Mmwave => 9,
    }
}

/// Airtime of `bytes` at `cqi` on a whole carrier (`subchannel` 0) or a quarter, µs.
pub fn airtime_us(profile: Profile, cqi: u8, subchannel: u8, bytes: usize) -> u64 {
    let c = cqi.clamp(1, profile.max_cqi()) as usize;
    let bw = profile.bandwidth_hz() / if subchannel == 0 { 1.0 } else { 4.0 };
    let rate = EFFICIENCY[c] * bw * (1.0 - overhead(profile));
    let us = (bytes as f64 * 8.0 / rate * 1e6) as u64 + 1;
    symbol_us(profile) + us
}

/// Most payload bytes that fit `us` of airtime.
pub fn bytes_in(profile: Profile, cqi: u8, subchannel: u8, us: u64) -> usize {
    let c = cqi.clamp(1, profile.max_cqi()) as usize;
    let bw = profile.bandwidth_hz() / if subchannel == 0 { 1.0 } else { 4.0 };
    let rate = EFFICIENCY[c] * bw * (1.0 - overhead(profile));
    let us = us.saturating_sub(symbol_us(profile) + 1);
    (rate * us as f64 / 8e6) as usize
}

#[cfg(feature = "framed")]
mod framed_device {
    //! The in-game radio: `/dev/frames.<bay>` / `/dev/sdrN.frames` through `ecm_radio::framed`.
    use super::*;
    use alloc::format;
    use ecm_radio::framed as f;

    impl FramedPhy for f::Framed {
        fn configure(&mut self, c: &PhyConfig) -> Result<(), PhyError> {
            let cfg = f::Config {
                phy: Some(c.profile.name().into()),
                role: Some(match c.role {
                    Role::Bs => "bs".into(),
                    Role::Ue => "ue".into(),
                }),
                channel: Some(c.channel as u32),
                bandwidth_hz: None,
                power_dbm: c.power_dbm,
                address: Some(c.address),
                promiscuous: Some(false),
            };
            f::Framed::configure(self, &cfg).map_err(|e| PhyError(format!("{}", e)))
        }
        fn send(&mut self, tx: &TxReq) -> Result<(), PhyError> {
            let o = f::TxOpts { harq: tx.harq, slot: tx.slot, cqi: tx.cqi, subchannel: tx.subchannel, tag: tx.tag };
            f::Framed::send(self, tx.dst, &tx.payload, &o).map_err(|e| PhyError(format!("{}", e)))
        }
        fn beacon(&mut self, index: u8, period_ms: u32, payload: &[u8]) -> Result<(), PhyError> {
            f::Framed::beacon(self, index, period_ms, payload).map_err(|e| PhyError(format!("{}", e)))
        }
        fn poll(&mut self) -> Option<PhyEvent> {
            match self.recv(std::time::Duration::ZERO).ok()?? {
                f::Record::Rx(r) => Some(PhyEvent::Rx(RxFrame {
                    flags: r.flags,
                    src: r.src,
                    dst: r.dst,
                    seq: r.seq,
                    cqi: r.cqi,
                    cqi_sent: r.cqi_sent,
                    rssi_dbm: r.rssi_dbm,
                    sinr_db: r.sinr_db,
                    timestamp_us: r.timestamp_us,
                    slot: r.slot,
                    subchannel: r.subchannel,
                    cqi_report: r.cqi_report,
                    payload: r.payload,
                })),
                f::Record::Status(s) => Some(PhyEvent::Status(TxStatus {
                    result: TxResult::from_code(s.result),
                    dst: s.dst,
                    tag: s.tag,
                    attempts: s.attempts,
                    cqi: s.cqi,
                    slot: s.slot,
                    airtime_us: s.airtime_us,
                })),
            }
        }
        fn slot(&mut self) -> u64 {
            f::Framed::slot(self).unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_match_the_framed_phy_tables() {
        assert_eq!(Profile::Lte.dl_hz(1), 791e6);
        assert_eq!(Profile::Lte.ul_hz(6), 753e6);
        assert_eq!(Profile::Mmwave.dl_hz(4), 28.35e9);
        assert_eq!(Profile::Lte.slot_us(), 1000);
        assert_eq!(Profile::Mmwave.slot_us(), 125);
        assert_eq!(cqi_for(Profile::Lte, 100.0), 15);
        assert_eq!(cqi_for(Profile::Mmwave, 100.0), 18);
        assert_eq!(cqi_for(Profile::Lte, -20.0), 1);
        assert_eq!(cqi_for(Profile::Lte, 8.1 + 3.0), 8);
        for p in [Profile::Lte, Profile::Mmwave] {
            assert_eq!(Profile::from_code(p.code()), Some(p));
            assert_eq!(Profile::parse(p.name()), Some(p));
        }
    }

    #[test]
    fn airtime_and_capacity_agree() {
        let t = airtime_us(Profile::Lte, 15, 0, 1500);
        assert!(t > 71 && t < 400, "{}", t);
        let q = airtime_us(Profile::Lte, 15, 1, 1500);
        assert!(q > 3 * (t - 71), "a quarter carrier is ~4x slower");
        let n = bytes_in(Profile::Lte, 10, 2, 1000);
        assert!(airtime_us(Profile::Lte, 10, 2, n) <= 1000);
        assert!(airtime_us(Profile::Lte, 10, 2, n + 8) > 1000 - 8);
    }
}
