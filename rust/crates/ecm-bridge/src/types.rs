//! Configuration and introspection types shared by the bridge, the CLI and
//! callers.

use crate::frame::HashMode;
use crate::log::Severity;
use std::collections::BTreeSet;
use std::fmt;

pub const MIN_VID: u16 = 1;
pub const MAX_VID: u16 = 4094;
pub const DEFAULT_VID: u16 = 1;
pub const MIN_LAG_ID: u16 = 1;
pub const MAX_LAG_ID: u16 = 256;
/// Maximum physical members per LAG.
pub const MAX_LAG_MEMBERS: usize = 8;
/// Maximum physical ports a bridge accepts (extra MACs are ignored).
pub const MAX_PORTS: usize = 256;

/// Identifies an interface / bridge port.
///
/// `Eth(n)` is physical port n (`ethN`), `Lag(id)` the aggregated port
/// `lagID`, and `Cpu` the switch's own internal port (the SVIs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PortRef {
    Eth(usize),
    Lag(u16),
    Cpu,
}

impl fmt::Display for PortRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PortRef::Eth(n) => write!(f, "eth{}", n),
            PortRef::Lag(n) => write!(f, "lag{}", n),
            PortRef::Cpu => write!(f, "cpu"),
        }
    }
}

/// Allowed-VLAN list on a trunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AllowedList {
    /// Every VLAN in the database, including ones created later.
    All,
    Some(BTreeSet<u16>),
}

impl AllowedList {
    pub fn allows(&self, vid: u16) -> bool {
        match self {
            AllowedList::All => true,
            AllowedList::Some(s) => s.contains(&vid),
        }
    }
}

/// Port forwarding mode (same semantics as the original AOS-CX style switch).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortMode {
    /// L3 (the default): not a bridge member; the kernel hands its frames to
    /// the host stack.
    Routed,
    /// One untagged VLAN; tagged ingress (other than priority tags) is dropped.
    Access { vid: u16 },
    /// 802.1Q trunk.
    Trunk { native: u16, native_tag: bool, allowed: AllowedList },
}

impl PortMode {
    pub fn is_routed(&self) -> bool {
        matches!(self, PortMode::Routed)
    }
}

pub const DEFAULT_PORT_PRIORITY: u8 = 128;
pub const DEFAULT_PORT_COST: u32 = 20_000;

/// Per-bridge-port spanning-tree configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StpPortConfig {
    /// `no spanning-tree` on the interface: the port forwards without STP.
    pub enabled: bool,
    pub priority: u8,
    pub cost: u32,
    pub admin_edge: bool,
    pub bpdu_guard: bool,
    pub root_guard: bool,
    pub tcn_guard: bool,
}

impl Default for StpPortConfig {
    fn default() -> Self {
        StpPortConfig {
            enabled: true,
            priority: DEFAULT_PORT_PRIORITY,
            cost: DEFAULT_PORT_COST,
            admin_edge: false,
            bpdu_guard: false,
            root_guard: false,
            tcn_guard: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VlanConfig {
    pub vid: u16,
    pub name: Option<String>,
    pub description: Option<String>,
    pub active: bool,
}

/// Configuration of a physical port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EthConfig {
    pub mode: PortMode,
    pub admin_up: bool,
    pub stp: StpPortConfig,
    pub lag: Option<u16>,
    pub lldp_tx: bool,
    pub lldp_rx: bool,
}

impl Default for EthConfig {
    fn default() -> Self {
        EthConfig { mode: PortMode::Routed, admin_up: true, stp: StpPortConfig::default(), lag: None, lldp_tx: true, lldp_rx: true }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LacpMode {
    Active,
    Passive,
    /// Static LAG: no LACPDUs, every link-up member distributes.
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LacpRate {
    /// 1 s transmit / 3 s timeout.
    Fast,
    /// 30 s transmit / 90 s timeout.
    Slow,
}

/// Configuration of a LAG (aggregated bridge port). Members are the
/// physical ports whose [`EthConfig::lag`] names this LAG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LagConfig {
    pub id: u16,
    pub mode: PortMode,
    pub admin_up: bool,
    pub stp: StpPortConfig,
    pub lacp_mode: LacpMode,
    pub lacp_rate: LacpRate,
    pub hash: HashMode,
    pub fallback: bool,
}

impl LagConfig {
    pub fn new(id: u16) -> Self {
        LagConfig {
            id,
            mode: PortMode::Routed,
            admin_up: true,
            stp: StpPortConfig::default(),
            lacp_mode: LacpMode::Off,
            lacp_rate: LacpRate::Slow,
            hash: HashMode::L3,
            fallback: false,
        }
    }
}

pub const DEFAULT_BRIDGE_PRIORITY: u16 = 32768;
pub const DEFAULT_HELLO_SECS: u32 = 2;
pub const DEFAULT_FORWARD_DELAY_SECS: u32 = 15;
pub const DEFAULT_MAX_AGE_SECS: u32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StpConfig {
    pub enabled: bool,
    pub priority: u16,
    pub hello_secs: u32,
    pub forward_delay_secs: u32,
    pub max_age_secs: u32,
    pub config_name: String,
    pub config_revision: u16,
}

impl Default for StpConfig {
    fn default() -> Self {
        StpConfig {
            enabled: false,
            priority: DEFAULT_BRIDGE_PRIORITY,
            hello_secs: DEFAULT_HELLO_SECS,
            forward_delay_secs: DEFAULT_FORWARD_DELAY_SECS,
            max_age_secs: DEFAULT_MAX_AGE_SECS,
            config_name: String::new(),
            config_revision: 0,
        }
    }
}

/// Optional LLDP TLVs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LldpTlv {
    PortDesc,
    SysName,
    SysDesc,
    SysCaps,
    MgmtAddr,
}

impl LldpTlv {
    pub const ALL: [LldpTlv; 5] = [LldpTlv::PortDesc, LldpTlv::SysName, LldpTlv::SysDesc, LldpTlv::SysCaps, LldpTlv::MgmtAddr];
    pub fn name(&self) -> &'static str {
        match self {
            LldpTlv::PortDesc => "port-desc",
            LldpTlv::SysName => "sys-name",
            LldpTlv::SysDesc => "sys-desc",
            LldpTlv::SysCaps => "sys-caps",
            LldpTlv::MgmtAddr => "mgmt-addr",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "port-desc" | "port-description" => LldpTlv::PortDesc,
            "sys-name" | "system-name" => LldpTlv::SysName,
            "sys-desc" | "system-description" => LldpTlv::SysDesc,
            "sys-caps" | "system-capabilities" => LldpTlv::SysCaps,
            "mgmt-addr" | "management-address" => LldpTlv::MgmtAddr,
            _ => return None,
        })
    }
}

pub const DEFAULT_LLDP_TIMER: u32 = 30;
pub const DEFAULT_LLDP_HOLDTIME: u32 = 4;
pub const DEFAULT_LLDP_REINIT: u32 = 2;
pub const DEFAULT_LLDP_TXDELAY: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LldpConfig {
    pub enabled: bool,
    pub timer_secs: u32,
    pub holdtime: u32,
    pub reinit_secs: u32,
    pub txdelay_secs: u32,
    pub mgmt_ipv4: Option<[u8; 4]>,
    /// Enabled optional TLVs, indexed like [`LldpTlv::ALL`].
    pub tlvs: [bool; 5],
    pub sys_name: String,
    pub sys_desc: String,
}

impl Default for LldpConfig {
    fn default() -> Self {
        LldpConfig {
            enabled: true,
            timer_secs: DEFAULT_LLDP_TIMER,
            holdtime: DEFAULT_LLDP_HOLDTIME,
            reinit_secs: DEFAULT_LLDP_REINIT,
            txdelay_secs: DEFAULT_LLDP_TXDELAY,
            mgmt_ipv4: None,
            tlvs: [true; 5],
            sys_name: "ecm-switch".to_string(),
            sys_desc: "Evan's Computer Mod virtual switch".to_string(),
        }
    }
}

impl LldpConfig {
    pub fn tlv(&self, t: LldpTlv) -> bool {
        self.tlvs.get(t as usize).copied().unwrap_or(false)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogConfig {
    pub severity: Severity,
    pub console: bool,
    pub remotes: Vec<[u8; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticMac {
    pub mac: [u8; 6],
    pub vlan: u16,
    pub port: PortRef,
}

pub const DEFAULT_AGE_TIME_SECS: u32 = 300;
pub const MIN_AGE_TIME_SECS: u32 = 15;
pub const MAX_AGE_TIME_SECS: u32 = 1_000_000;

/// A complete, comparable snapshot of the bridge's configuration (no
/// runtime state). Used to verify `running_config` round trips.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgeConfig {
    pub age_time_secs: u32,
    pub vlans: Vec<VlanConfig>,
    pub eth: Vec<EthConfig>,
    pub lags: Vec<LagConfig>,
    pub stp: StpConfig,
    pub lldp: LldpConfig,
    pub log: LogConfig,
    pub statics: Vec<StaticMac>,
    /// (vlan, ip, prefix)
    pub svis: Vec<(u16, [u8; 4], u8)>,
}

/// Errors from the typed configuration API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeError {
    BadPort,
    BadVlan,
    VlanNotFound(u16),
    VlanInUse { vid: u16, port: PortRef },
    VlanHasSvi(u16),
    DefaultVlan,
    NotL2,
    NotTrunk,
    BadLagId,
    LagNotFound(u16),
    LagFull,
    FdbFull,
    NoSuchEntry,
    OutOfRange { what: &'static str, min: u32, max: u32 },
    Invalid(&'static str),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::BadPort => write!(f, "Invalid port"),
            BridgeError::BadVlan => write!(f, "VLAN id out of range (1-4094)"),
            BridgeError::VlanNotFound(v) => write!(f, "VLAN {} does not exist", v),
            BridgeError::VlanInUse { vid, port } => write!(f, "VLAN {} is in use on port {}", vid, port),
            BridgeError::VlanHasSvi(v) => write!(f, "VLAN {} has an SVI (remove 'interface vlan {}' first)", v, v),
            BridgeError::DefaultVlan => write!(f, "Cannot modify VLAN 1 (default)"),
            BridgeError::NotL2 => write!(f, "Run 'no routing' first to convert this port to L2"),
            BridgeError::NotTrunk => write!(f, "Port is not in trunk mode (set 'vlan trunk native <id>' first)"),
            BridgeError::BadLagId => write!(f, "LAG id out of range (1-256)"),
            BridgeError::LagNotFound(id) => write!(f, "LAG {} does not exist", id),
            BridgeError::LagFull => write!(f, "LAG member list full (max {})", MAX_LAG_MEMBERS),
            BridgeError::FdbFull => write!(f, "MAC address table full (all entries static)"),
            BridgeError::NoSuchEntry => write!(f, "No matching entry"),
            BridgeError::OutOfRange { what, min, max } => write!(f, "{} out of range ({}-{})", what, min, max),
            BridgeError::Invalid(s) => write!(f, "{}", s),
        }
    }
}
