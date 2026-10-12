//! Cellular v2 in the simulator: a simulated framed-PHY radio medium
//! (`ecm_cell::medium`) and native cell agents (phone, cell site, carrier
//! core) driving the `ecm-cell` state machines.
//!
//! An agent runs on a node's computer the way the phase-3 programs will:
//! it is a process of that computer's kernel (its own pid / IPC session) and
//! uses the kernel's real sockets: TCP + UDP to reach the core, a `tun`
//! interface for the phone's `wwan0` (plus a default route by netlink) and
//! for the core's interface towards the phones. Only the radio is simulated
//! natively (as the SDR's framed mode is Java hardware in game).
//!
//! ```toml
//! [radio]                       # the medium (optional)
//! exponent = 3.5                # path-loss exponent
//! [[radio.wall]]
//! min = [300, -50, 0]
//! max = [310, 50, 40]
//! loss_db = 25
//!
//! [node.cell]                   # makes the node run an agent
//! role = "site"                 # site | core | ue
//! pos = [0, 0, 20]              # metres (= blocks)
//! ```
//! See `rust/simulator/README.md` "Cellular" for every key.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use ecm_cell::air::Neighbour;
use ecm_cell::backhaul::{CONTROL_PORT, TUNNEL_PORT};
use ecm_cell::codec::{fmt_ip, parse_ip, unhex};
use ecm_cell::core::{Core, CoreConfig, CoreOut, Entropy, SockAddr};
use ecm_cell::db::Db;
use ecm_cell::ip::{from_kernel, to_kernel, TunFrame};
use ecm_cell::medium::{Medium, MediumConfig, RadioId, Wall};
use ecm_cell::phy::{PhyCmd, PhyEvent, Profile};
use ecm_cell::simauth::SoftSim;
use ecm_cell::site::{Site, SiteConfig, SiteOut};
use ecm_cell::ue::UeConfig;
use ecm_cell::Now;
use serde::Deserialize;

use crate::child::NodeEnv;
use crate::ipc::IpcResult;
use crate::sched::ActorId;
use crate::topology::Dur;
use crate::util::Rng;

// ------------------------------------------------------------ configuration

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct RadioSpec {
    pub exponent: Option<f64>,
    #[serde(default)]
    pub wall: Vec<WallSpec>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct WallSpec {
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub loss_db: f64,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct NeighbourSpec {
    pub cell: u32,
    pub channel: u8,
    pub profile: Option<String>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct CellSpec {
    pub role: String,
    #[serde(default)]
    pub pos: [f64; 3],
    pub autostart: Option<bool>,
    pub carrier: Option<String>,
    pub profile: Option<String>,
    /// Radio frame address, hex.
    pub address: Option<String>,
    pub power: Option<f64>,
    // site
    pub site_id: Option<String>,
    pub secret: Option<String>,
    pub cell_id: Option<u32>,
    pub channel: Option<u8>,
    pub area: Option<u16>,
    pub core: Option<String>,
    pub core_port: Option<u16>,
    pub core_tunnel_port: Option<u16>,
    pub tunnel_port: Option<u16>,
    #[serde(default)]
    pub neighbours: Vec<NeighbourSpec>,
    pub idle_after: Option<Dur>,
    // core
    pub db: Option<String>,
    pub listen: Option<u16>,
    pub tun: Option<String>,
    // ue
    pub imsi: Option<String>,
    pub key: Option<String>,
    #[serde(default)]
    pub channels: Vec<u8>,
}

#[derive(Clone)]
enum Kind {
    Site { cfg: SiteConfig, core: [u8; 4], core_port: u16, core_tunnel_port: u16 },
    Core { db: String, listen: u16, tunnel_port: u16, tun: String },
    Ue { cfg: UeConfig, sim: SoftSim, tun: String },
}

fn need<T: Clone>(v: &Option<T>, node: &str, key: &str) -> Result<T, String> {
    v.clone().ok_or_else(|| format!("node {}: [node.cell] needs '{}'", node, key))
}

fn hex_u32(s: &str, node: &str) -> Result<u32, String> {
    u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| format!("node {}: bad address '{}'", node, s))
}

fn profile(s: &Option<String>, node: &str) -> Result<Profile, String> {
    match s {
        None => Ok(Profile::Lte),
        Some(p) => Profile::parse(p).ok_or_else(|| format!("node {}: unknown profile '{}'", node, p)),
    }
}

impl CellSpec {
    fn kind(&self, node: &str, index: usize) -> Result<Kind, String> {
        let carrier = || need(&self.carrier, node, "carrier");
        match self.role.as_str() {
            "site" => {
                let secret = unhex::<16>(&need(&self.secret, node, "secret")?).ok_or_else(|| format!("node {}: secret must be 32 hex digits", node))?;
                let cell_id = need(&self.cell_id, node, "cell_id")?;
                let address = match &self.address {
                    Some(a) => hex_u32(a, node)?,
                    None => 0xA000_0000 | cell_id,
                };
                let mut cfg = SiteConfig::new(&need(&self.site_id, node, "site_id")?, secret, &carrier()?, cell_id, address);
                cfg.profile = profile(&self.profile, node)?;
                cfg.channel = self.channel.unwrap_or(1);
                cfg.area = self.area.unwrap_or(1);
                cfg.power_dbm = self.power;
                cfg.tunnel_port = self.tunnel_port.unwrap_or(TUNNEL_PORT);
                cfg.idle_after_ms = self.idle_after.as_ref().map(|d| d.ms()).transpose()?.unwrap_or(0) as u64;
                for n in &self.neighbours {
                    cfg.neighbours.push(Neighbour { cell_id: n.cell, profile: profile(&n.profile, node)?.code(), channel: n.channel, ul_mask: 0 });
                }
                let core = parse_ip(&need(&self.core, node, "core")?).ok_or_else(|| format!("node {}: bad core address", node))?;
                Ok(Kind::Site { cfg, core, core_port: self.core_port.unwrap_or(CONTROL_PORT), core_tunnel_port: self.core_tunnel_port.unwrap_or(TUNNEL_PORT) })
            }
            "core" => Ok(Kind::Core {
                db: self.db.clone().unwrap_or_else(|| "cored.db".into()),
                listen: self.listen.unwrap_or(CONTROL_PORT),
                tunnel_port: self.tunnel_port.unwrap_or(TUNNEL_PORT),
                tun: self.tun.clone().unwrap_or_else(|| "cell0".into()),
            }),
            "ue" => {
                let key = unhex::<16>(&need(&self.key, node, "key")?).ok_or_else(|| format!("node {}: key must be 32 hex digits", node))?;
                let address = match &self.address {
                    Some(a) => hex_u32(a, node)?,
                    None => 0xB000_0000 | index as u32,
                };
                let mut cfg = UeConfig::new(profile(&self.profile, node)?, address);
                cfg.channels = self.channels.clone();
                cfg.power_dbm = self.power;
                let sim = SoftSim::new(&carrier()?, &need(&self.imsi, node, "imsi")?, key);
                Ok(Kind::Ue { cfg, sim, tun: self.tun.clone().unwrap_or_else(|| "wwan0".into()) })
            }
            o => Err(format!("node {}: unknown cell role '{}' (site, core, ue)", node, o)),
        }
    }
}

// ------------------------------------------------------------ the world

struct Running {
    stop: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
    actor: ActorId,
    join: JoinHandle<()>,
}

pub struct Agent {
    pub node: usize,
    pub name: String,
    pub role: String,
    kind: Kind,
    autostart: bool,
    radio: Option<RadioId>,
    pub log: Arc<Mutex<Vec<String>>>,
    pub status: Arc<Mutex<String>>,
    admin: Arc<Mutex<VecDeque<String>>>,
    /// `expect_cell` matches log lines from here on.
    pub mark: usize,
    starts: u32,
    running: Option<Running>,
}

pub struct CellSim {
    pub medium: Arc<Mutex<Medium>>,
    pub agents: Vec<Agent>,
}

impl CellSim {
    pub fn new(radio: Option<&RadioSpec>, nodes: &[(String, Option<CellSpec>)], seed: u64) -> Result<CellSim, String> {
        let mut mc = MediumConfig { seed, ..Default::default() };
        if let Some(r) = radio {
            if let Some(e) = r.exponent {
                mc.exponent = e;
            }
            mc.walls = r.wall.iter().map(|w| Wall { min: w.min, max: w.max, loss_db: w.loss_db }).collect();
        }
        let mut medium = Medium::new(mc);
        let mut agents = Vec::new();
        for (i, (name, spec)) in nodes.iter().enumerate() {
            let Some(spec) = spec else { continue };
            let kind = spec.kind(name, i + 1)?;
            let radio = match kind {
                Kind::Core { .. } => None,
                _ => Some(medium.add_radio(name, spec.pos)),
            };
            agents.push(Agent {
                node: i,
                name: name.clone(),
                role: spec.role.clone(),
                kind,
                autostart: spec.autostart.unwrap_or(true),
                radio,
                log: Arc::new(Mutex::new(Vec::new())),
                status: Arc::new(Mutex::new("not started".into())),
                admin: Arc::new(Mutex::new(VecDeque::new())),
                mark: 0,
                starts: 0,
                running: None,
            });
        }
        Ok(CellSim { medium: Arc::new(Mutex::new(medium)), agents })
    }

    pub fn agent(&self, node: &str) -> Result<usize, String> {
        self.agents.iter().position(|a| a.name == node).ok_or_else(|| format!("node {} runs no cell agent", node))
    }

    pub fn radio(&self, node: &str) -> Result<RadioId, String> {
        let a = &self.agents[self.agent(node)?];
        a.radio.ok_or_else(|| format!("node {} has no radio", node))
    }

    pub fn is_running(&self, a: usize) -> bool {
        self.agents[a].running.as_ref().is_some_and(|r| !r.join.is_finished())
    }

    /// Start agent `a` on its node (no-op if running).
    pub fn start(&mut self, a: usize, env: &Arc<NodeEnv>) {
        if self.is_running(a) {
            return;
        }
        if let Some(r) = self.agents[a].running.take() {
            let _ = r.join.join();
        }
        let ag = &mut self.agents[a];
        ag.starts += 1;
        if let Some(r) = ag.radio {
            self.medium.lock().unwrap_or_else(|e| e.into_inner()).set_on(r, true);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let killed = Arc::new(AtomicBool::new(false));
        let pid = env.procs.alloc_pid();
        let label = format!("{}:cell-{}({})", ag.name, ag.role, pid);
        let actor = env.sched.register(&label);
        let cx = Cx {
            env: env.clone(),
            pid,
            actor,
            stop: stop.clone(),
            killed: killed.clone(),
            medium: self.medium.clone(),
            radio: ag.radio,
            log: ag.log.clone(),
            status: ag.status.clone(),
            admin: ag.admin.clone(),
            socks: Mutex::new(Vec::new()),
            seed: env.seed ^ (ag.starts as u64) << 40,
        };
        let kind = ag.kind.clone();
        let name = ag.name.clone();
        let join = std::thread::Builder::new()
            .name(format!("{}-cell", name))
            .spawn(move || {
                cx.log(format!("{} agent started (pid {})", role_name(&kind), cx.pid));
                match kind {
                    Kind::Site { cfg, core, core_port, core_tunnel_port } => run_site(&cx, cfg, core, core_port, core_tunnel_port),
                    Kind::Core { db, listen, tunnel_port, tun } => run_core(&cx, &db, listen, tunnel_port, &tun),
                    Kind::Ue { cfg, sim, tun } => run_ue(&cx, cfg, sim, &tun),
                }
                cx.cleanup();
                cx.log("agent stopped".into());
                *cx.status.lock().unwrap_or_else(|e| e.into_inner()) = "stopped".into();
                cx.env.sched.exit(cx.actor);
            })
            .expect("spawn agent thread");
        self.agents[a].running = Some(Running { stop, killed, actor, join });
    }

    /// Ask agent `a` to stop (power off): its radio goes silent at once,
    /// the program closes its sockets and exits on its own.
    pub fn stop(&mut self, a: usize, env: &Arc<NodeEnv>) {
        let ag = &mut self.agents[a];
        if let Some(r) = ag.radio {
            self.medium.lock().unwrap_or_else(|e| e.into_inner()).set_on(r, false);
        }
        if let Some(r) = &ag.running {
            r.stop.store(true, Ordering::SeqCst);
            env.sched.wake(r.actor);
        }
    }

    pub fn autostart_all(&mut self, envs: &[Arc<NodeEnv>]) {
        for a in 0..self.agents.len() {
            if self.agents[a].autostart {
                let env = envs[self.agents[a].node].clone();
                self.start(a, &env);
            }
        }
    }

    pub fn admin(&self, a: usize, cmd: &str, env: &Arc<NodeEnv>) {
        self.agents[a].admin.lock().unwrap_or_else(|e| e.into_inner()).push_back(cmd.into());
        if let Some(r) = &self.agents[a].running {
            env.sched.wake(r.actor);
        }
    }

    pub fn shutdown(&mut self, envs: &[Arc<NodeEnv>]) {
        for ag in &mut self.agents {
            if let Some(r) = &ag.running {
                r.stop.store(true, Ordering::SeqCst);
                r.killed.store(true, Ordering::SeqCst);
                envs[ag.node].sched.wake(r.actor);
            }
        }
        for ag in &mut self.agents {
            if let Some(r) = ag.running.take() {
                let _ = r.join.join();
            }
        }
    }

    /// Log lines since the mark.
    pub fn log_since_mark(&self, a: usize) -> Vec<String> {
        let ag = &self.agents[a];
        let l = ag.log.lock().unwrap_or_else(|e| e.into_inner());
        l[ag.mark.min(l.len())..].to_vec()
    }

    pub fn log_len(&self, a: usize) -> usize {
        self.agents[a].log.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn report(&self) -> String {
        let mut s = String::new();
        for ag in &self.agents {
            let l = ag.log.lock().unwrap_or_else(|e| e.into_inner());
            s.push_str(&format!("  --- {} ({}) {} ---\n", ag.name, ag.role, ag.status.lock().unwrap_or_else(|e| e.into_inner())));
            for line in l.iter().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
                s.push_str(&format!("    {}\n", line));
            }
        }
        s
    }
}

fn role_name(k: &Kind) -> &'static str {
    match k {
        Kind::Site { .. } => "celld-site",
        Kind::Core { .. } => "cored",
        Kind::Ue { .. } => "celld",
    }
}

// ------------------------------------------------------------ an agent's process

const SOCK_SOCKET: i32 = 0;
const SOCK_BIND: i32 = 1;
const SOCK_CONNECT: i32 = 2;
const SOCK_LISTEN: i32 = 3;
const SOCK_ACCEPT: i32 = 4;
const SOCK_SEND: i32 = 5;
const SOCK_RECV: i32 = 6;
const SOCK_CLOSE: i32 = 7;
const SOCK_SETSOCKOPT: i32 = 8;
const SOCK_SENDTO: i32 = 9;
const SOCK_RECVFROM: i32 = 10;
const SOCK_POLL: i32 = 15;
const AF_INET: i32 = 2;
const AF_NETLINK: i32 = 16;
const AF_ECM_TUN: i32 = 1024;
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const SOCK_RAW: i32 = 3;
const SOL_TUN: i32 = 1024;
const TUN_SETIFF: i32 = 1;
const MSG_DONTWAIT: i32 = 0x40;
const POLLIN: i16 = 0x1;
const POLLHUP: i16 = 0x10;
const POLLERR: i16 = 0x8;
const TICK_MS: i64 = 5;

struct Stopped;

/// The agent's process: its kernel session, clock and radio.
struct Cx {
    env: Arc<NodeEnv>,
    pid: i32,
    actor: ActorId,
    stop: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
    medium: Arc<Mutex<Medium>>,
    radio: Option<RadioId>,
    log: Arc<Mutex<Vec<String>>>,
    status: Arc<Mutex<String>>,
    admin: Arc<Mutex<VecDeque<String>>>,
    socks: Mutex<Vec<i32>>,
    seed: u64,
}

fn sockaddr(ip: [u8; 4], port: u16) -> Vec<u8> {
    let mut a = vec![0u8; 16];
    a[0..2].copy_from_slice(&(AF_INET as u16).to_le_bytes());
    a[2..4].copy_from_slice(&port.to_be_bytes());
    a[4..8].copy_from_slice(&ip);
    a
}

fn i32le(v: i32) -> [u8; 4] {
    v.to_le_bytes()
}

impl Cx {
    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn ms(&self) -> u64 {
        self.env.sched.elapsed().max(0) as u64
    }

    fn now(&self) -> Now {
        let ms = self.ms();
        let slot = match self.radio {
            Some(r) => self.medium.lock().unwrap_or_else(|e| e.into_inner()).slot(r, ms * 1000),
            None => ms,
        };
        Now { ms, slot }
    }

    fn log(&self, line: String) {
        let t = self.env.sched.elapsed();
        self.env.log(format!("cell: {}", line));
        self.log.lock().unwrap_or_else(|e| e.into_inner()).push(format!("[{:>9.3}s] {}", t as f64 / 1000.0, line));
    }

    fn set_status(&self, s: String) {
        *self.status.lock().unwrap_or_else(|e| e.into_inner()) = s;
    }

    fn sleep_until(&self, ms: u64) {
        let deadline = self.env.sched.now() + (ms as i64 - self.ms() as i64).max(1);
        let stop = self.stop.clone();
        let admin = self.admin.clone();
        self.env.sched.wait(self.actor, Some(deadline), || stop.load(Ordering::SeqCst) || !admin.lock().unwrap_or_else(|e| e.into_inner()).is_empty());
    }

    fn call(&self, sc: i32, args: Vec<u8>) -> Result<IpcResult, Stopped> {
        self.env.ipc.call(self.pid, sc, args, self.actor, &self.killed).map_err(|_| Stopped)
    }

    fn socket(&self, d: i32, t: i32, p: i32) -> Result<i32, Stopped> {
        let mut a = i32le(d).to_vec();
        a.extend_from_slice(&i32le(t));
        a.extend_from_slice(&i32le(p));
        let r = self.call(SOCK_SOCKET, a)?.status;
        if r >= 0 {
            self.socks.lock().unwrap_or_else(|e| e.into_inner()).push(r);
        }
        Ok(r)
    }

    fn addr_call(&self, sc: i32, id: i32, ip: [u8; 4], port: u16) -> Result<i32, Stopped> {
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&sockaddr(ip, port));
        Ok(self.call(sc, a)?.status)
    }

    fn close(&self, id: i32) {
        self.socks.lock().unwrap_or_else(|e| e.into_inner()).retain(|s| *s != id);
        let _ = self.call(SOCK_CLOSE, i32le(id).to_vec());
    }

    fn send_all(&self, id: i32, data: &[u8]) -> Result<bool, Stopped> {
        for chunk in data.chunks(4096) {
            let mut off = 0;
            while off < chunk.len() {
                let mut a = i32le(id).to_vec();
                a.extend_from_slice(&((chunk.len() - off) as u16).to_le_bytes());
                a.extend_from_slice(&chunk[off..]);
                let n = self.call(SOCK_SEND, a)?.status;
                if n <= 0 {
                    return Ok(false);
                }
                off += n as usize;
            }
        }
        Ok(true)
    }

    /// Non-blocking TCP/tun receive: Ok(Some(bytes)) (empty = EOF for TCP), Ok(None) = nothing waiting.
    fn recv(&self, id: i32, max: i32) -> Result<Option<Vec<u8>>, Stopped> {
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&i32le(max));
        a.extend_from_slice(&i32le(MSG_DONTWAIT));
        let r = self.call(SOCK_RECV, a)?;
        match r.status {
            -2 => Ok(None),
            n if n < 0 => Ok(Some(Vec::new())),
            n => Ok(Some(r.payload[..(n as usize).min(r.payload.len())].to_vec())),
        }
    }

    fn sendto(&self, id: i32, ip: [u8; 4], port: u16, data: &[u8]) -> Result<i32, Stopped> {
        let addr = sockaddr(ip, port);
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&(addr.len() as u16).to_le_bytes());
        a.extend_from_slice(&addr);
        a.extend_from_slice(&(data.len() as u16).to_le_bytes());
        a.extend_from_slice(data);
        Ok(self.call(SOCK_SENDTO, a)?.status)
    }

    fn recvfrom(&self, id: i32) -> Result<Option<(SockAddr, Vec<u8>)>, Stopped> {
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&i32le(2048));
        a.extend_from_slice(&i32le(MSG_DONTWAIT));
        let r = self.call(SOCK_RECVFROM, a)?;
        if r.status <= 0 || r.payload.len() < 16 {
            return Ok(None);
        }
        let p = &r.payload;
        let from = SockAddr { ip: [p[4], p[5], p[6], p[7]], port: u16::from_be_bytes([p[2], p[3]]) };
        let n = (r.status as usize).min(p.len() - 16);
        Ok(Some((from, p[16..16 + n].to_vec())))
    }

    fn accept(&self, id: i32) -> Result<Option<(i32, [u8; 4])>, Stopped> {
        let r = self.call(SOCK_ACCEPT, i32le(id).to_vec())?;
        if r.status < 0 {
            return Ok(None);
        }
        self.socks.lock().unwrap_or_else(|e| e.into_inner()).push(r.status);
        let p = &r.payload;
        let peer = if p.len() >= 8 { [p[4], p[5], p[6], p[7]] } else { [0; 4] };
        Ok(Some((r.status, peer)))
    }

    /// revents per id (timeout 0 = just look).
    fn poll(&self, ids: &[i32], timeout_ms: i32) -> Result<Vec<i16>, Stopped> {
        let mut a = i32le(timeout_ms).to_vec();
        a.extend_from_slice(&i32le(ids.len() as i32));
        for id in ids {
            a.extend_from_slice(&i32le(*id));
            a.extend_from_slice(&POLLIN.to_le_bytes());
            a.extend_from_slice(&[0, 0]);
        }
        let r = self.call(SOCK_POLL, a)?;
        Ok((0..ids.len()).map(|i| r.payload.get(2 * i..2 * i + 2).map(|b| i16::from_le_bytes([b[0], b[1]])).unwrap_or(0)).collect())
    }

    fn tcp_connect(&self, ip: [u8; 4], port: u16) -> Result<Option<i32>, Stopped> {
        let s = self.socket(AF_INET, SOCK_STREAM, 6)?;
        if s < 0 {
            return Ok(None);
        }
        if self.addr_call(SOCK_CONNECT, s, ip, port)? < 0 {
            self.close(s);
            return Ok(None);
        }
        Ok(Some(s))
    }

    fn udp_bound(&self, port: u16) -> Result<i32, Stopped> {
        let s = self.socket(AF_INET, SOCK_DGRAM, 17)?;
        if s >= 0 {
            self.addr_call(SOCK_BIND, s, [0; 4], port)?;
        }
        Ok(s)
    }

    fn tun_open(&self, name: &str, mac: [u8; 6], ip: [u8; 4], prefix: u8) -> Result<i32, Stopped> {
        let s = self.socket(AF_ECM_TUN, SOCK_RAW, 0)?;
        if s < 0 {
            return Ok(s);
        }
        let mut v = mac.to_vec();
        v.extend_from_slice(&ip);
        v.push(prefix);
        v.extend_from_slice(name.as_bytes());
        let mut a = i32le(s).to_vec();
        a.extend_from_slice(&i32le(SOL_TUN));
        a.extend_from_slice(&i32le(TUN_SETIFF));
        a.extend_from_slice(&v);
        let rc = self.call(SOCK_SETSOCKOPT, a)?.status;
        if rc < 0 {
            self.close(s);
            return Ok(rc);
        }
        Ok(s)
    }

    fn tun_write(&self, fd: i32, frame: &[u8]) -> Result<(), Stopped> {
        let mut a = i32le(fd).to_vec();
        a.extend_from_slice(&(frame.len() as u16).to_le_bytes());
        a.extend_from_slice(frame);
        self.call(SOCK_SEND, a)?;
        Ok(())
    }

    /// `ip route add default via <gw>` through netlink (the kernel picks the
    /// interface whose subnet holds the gateway: the tun).
    fn default_route(&self, gw: [u8; 4]) -> Result<i32, Stopped> {
        let s = self.socket(AF_NETLINK, SOCK_DGRAM, 0)?;
        if s < 0 {
            return Ok(s);
        }
        let mut m = vec![0u8; 16];
        // rtmsg: family, dst_len, src_len, tos, table, protocol, scope, type, flags
        m.extend_from_slice(&[2, 0, 0, 0, 254, 4, 0, 1, 0, 0, 0, 0]);
        // RTA_GATEWAY
        m.extend_from_slice(&8u16.to_le_bytes());
        m.extend_from_slice(&5u16.to_le_bytes());
        m.extend_from_slice(&gw);
        let len = m.len() as u32;
        m[0..4].copy_from_slice(&len.to_le_bytes());
        m[4..6].copy_from_slice(&24u16.to_le_bytes()); // RTM_NEWROUTE
        m[6..8].copy_from_slice(&(0x0001u16 | 0x0004 | 0x0400).to_le_bytes());
        m[8..12].copy_from_slice(&7u32.to_le_bytes());
        self.sendto(s, [0; 4], 0, &m)?;
        let mut a = i32le(s).to_vec();
        a.extend_from_slice(&i32le(256));
        a.extend_from_slice(&i32le(0));
        let r = self.call(SOCK_RECVFROM, a)?;
        self.close(s);
        // NLMSG_ERROR with error 0 = done
        let err = r.payload.get(16 + 16..16 + 20).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(-1);
        Ok(err)
    }

    /// The resolver (`RTM_ECM_DNS`, the kernel's private netlink message).
    fn set_dns(&self, dns: [u8; 4]) -> Result<i32, Stopped> {
        let s = self.socket(AF_NETLINK, SOCK_DGRAM, 0)?;
        if s < 0 {
            return Ok(s);
        }
        let mut m = vec![0u8; 16];
        m.extend_from_slice(&dns);
        let len = m.len() as u32;
        m[0..4].copy_from_slice(&len.to_le_bytes());
        m[4..6].copy_from_slice(&0x7E11u16.to_le_bytes());
        m[6..8].copy_from_slice(&(0x0001u16 | 0x0004).to_le_bytes());
        m[8..12].copy_from_slice(&8u32.to_le_bytes());
        self.sendto(s, [0; 4], 0, &m)?;
        let mut a = i32le(s).to_vec();
        a.extend_from_slice(&i32le(256));
        a.extend_from_slice(&i32le(0));
        let r = self.call(SOCK_RECVFROM, a)?;
        self.close(s);
        Ok(r.payload.get(16 + 16..16 + 20).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(-1))
    }

    fn radio_events(&self) -> Vec<PhyEvent> {
        let Some(r) = self.radio else { return Vec::new() };
        let us = self.ms() * 1000;
        let mut m = self.medium.lock().unwrap_or_else(|e| e.into_inner());
        let mut v = Vec::new();
        while let Some(e) = m.poll(r, us) {
            v.push(e);
        }
        v
    }

    fn phy(&self, c: PhyCmd) {
        let Some(r) = self.radio else { return };
        let us = self.ms() * 1000;
        let mut m = self.medium.lock().unwrap_or_else(|e| e.into_inner());
        let res = match &c {
            PhyCmd::Configure(cfg) => m.configure(r, cfg),
            PhyCmd::Tx(t) => m.send(r, t, us),
            PhyCmd::Beacon { index, period_ms, payload } => m.beacon(r, *index, *period_ms, payload, us),
        };
        if let Err(e) = res {
            drop(m);
            self.log(format!("radio: {}", e.0));
        }
    }

    fn take_admin(&self) -> Vec<String> {
        self.admin.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
    }

    fn cleanup(&self) {
        if let Some(r) = self.radio {
            self.medium.lock().unwrap_or_else(|e| e.into_inner()).set_on(r, false);
        }
        if self.killed.load(Ordering::SeqCst) {
            return;
        }
        let socks: Vec<i32> = self.socks.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for s in socks {
            self.close(s);
        }
    }
}

struct SeededEntropy(Rng);
impl Entropy for SeededEntropy {
    fn fill(&mut self, b: &mut [u8]) {
        self.0.fill(b);
    }
}

// ------------------------------------------------------------ the phone (celld)

/// The phone agent is the real `celld` daemon (`wasm-programs/celld`, the same
/// `Daemon` the WASI program runs) over this node's kernel and the simulated
/// radio: the computer's side is [`SimIo`], the SIM a `SoftSim`. Its control
/// socket listens on 127.0.0.1:7710 of the node, so the `cell` program typed at
/// the node's prompt talks to it as on a phone.
struct SimIo<'a> {
    cx: &'a Cx,
    tun: Option<i32>,
    pending: VecDeque<PhyEvent>,
    stopped: bool,
}

impl SimIo<'_> {
    fn ok<T>(&mut self, r: Result<T, Stopped>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(Stopped) => {
                self.stopped = true;
                None
            }
        }
    }
}

impl celld::Io for SimIo<'_> {
    fn now(&mut self) -> Now {
        self.cx.now()
    }
    fn phy(&mut self, cmd: PhyCmd) -> Result<(), String> {
        self.cx.phy(cmd);
        Ok(())
    }
    fn phy_poll(&mut self) -> Option<PhyEvent> {
        if self.pending.is_empty() {
            self.pending.extend(self.cx.radio_events());
        }
        self.pending.pop_front()
    }
    fn tun_open(&mut self, name: &str, mac: [u8; 6], ip: [u8; 4], prefix: u8) -> Result<(), String> {
        celld::Io::tun_close(self);
        let r = self.cx.tun_open(name, mac, ip, prefix);
        match self.ok(r) {
            Some(fd) if fd >= 0 => {
                self.tun = Some(fd);
                Ok(())
            }
            Some(fd) => Err(format!("tun failed ({fd})")),
            None => Err("stopped".into()),
        }
    }
    fn tun_close(&mut self) {
        if let Some(fd) = self.tun.take() {
            self.cx.close(fd);
        }
    }
    fn tun_read(&mut self) -> Option<Vec<u8>> {
        let fd = self.tun?;
        let r = self.cx.recv(fd, 2048);
        self.ok(r).flatten().filter(|f| !f.is_empty())
    }
    fn tun_write(&mut self, frame: &[u8]) {
        if let Some(fd) = self.tun {
            let r = self.cx.tun_write(fd, frame);
            self.ok(r);
        }
    }
    fn default_route(&mut self, gw: [u8; 4]) -> Result<(), String> {
        let r = self.cx.default_route(gw);
        match self.ok(r) {
            Some(0) => Ok(()),
            Some(e) => Err(e.to_string()),
            None => Err("stopped".into()),
        }
    }
    fn set_dns(&mut self, dns: [u8; 4]) -> Result<(), String> {
        let r = self.cx.set_dns(dns);
        match self.ok(r) {
            Some(0) => Ok(()),
            Some(e) => Err(e.to_string()),
            None => Err("stopped".into()),
        }
    }
    fn log(&mut self, line: &str) {
        self.cx.log(line.to_string());
    }
}

fn run_ue(cx: &Cx, cfg: UeConfig, mut sim: SoftSim, tun_name: &str) {
    let _ = run_ue_inner(cx, cfg, &mut sim, tun_name);
}

fn run_ue_inner(cx: &Cx, cfg: UeConfig, sim: &mut SoftSim, tun_name: &str) -> Result<(), Stopped> {
    let opts = celld::Options {
        profile: cfg.profile,
        address: cfg.address,
        modem: "modem".into(),
        tun: tun_name.into(),
        power_dbm: cfg.power_dbm,
        config: celld::config::Config { channels: cfg.channels.clone(), ..Default::default() },
    };
    let ctl = cx.socket(AF_INET, SOCK_DGRAM, 17)?;
    if ctl >= 0 && cx.addr_call(SOCK_BIND, ctl, celld::ctl::LOOPBACK, celld::ctl::PORT)? < 0 {
        cx.log(format!("control socket 127.0.0.1:{} is taken", celld::ctl::PORT));
    }
    let mut io = SimIo { cx, tun: None, pending: VecDeque::new(), stopped: false };
    let mut d = celld::Daemon::new(opts, cx.ms());
    d.start(&mut io, sim);
    while !cx.stopping() && !io.stopped {
        let now = cx.now();
        d.step(&mut io, sim);
        for _ in 0..8 {
            let Some((from, data)) = cx.recvfrom(ctl)? else { break };
            if from.ip[0] != 127 {
                continue;
            }
            let answer = d.control(&String::from_utf8_lossy(&data), &mut io, sim);
            cx.sendto(ctl, from.ip, from.port, answer.as_bytes())?;
        }
        for c in cx.take_admin() {
            // the old agent's words for `cell down` / `cell up`
            let line = match c.as_str() {
                "detach" => "down",
                "attach" => "up",
                o => o,
            };
            let answer = d.control(line, &mut io, sim);
            cx.log(format!("admin {}: {}", c, answer.trim_end()));
        }
        let st = d.status(now.ms);
        cx.set_status(format!(
            "{} cell {} ch {} rssi {} ip {}",
            st.state,
            st.cell.map(|c| c.to_string()).unwrap_or("-".into()),
            st.channel,
            st.rssi.map(|r| format!("{:.1}", r)).unwrap_or("-".into()),
            if st.ip.is_empty() { "-".to_string() } else { st.ip.split('/').next().unwrap_or("-").to_string() }
        ));
        if io.stopped {
            break;
        }
        cx.sleep_until(now.ms + TICK_MS as u64);
    }
    Ok(())
}

// ------------------------------------------------------------ the cell site (celld-site)

fn run_site(cx: &Cx, cfg: SiteConfig, core: [u8; 4], core_port: u16, core_tunnel_port: u16) {
    let _ = run_site_inner(cx, cfg, core, core_port, core_tunnel_port);
}

fn run_site_inner(cx: &Cx, cfg: SiteConfig, core: [u8; 4], core_port: u16, core_tunnel_port: u16) -> Result<(), Stopped> {
    let tunnel_port = cfg.tunnel_port;
    let mut site = Site::with_entropy(cfg, Box::new(SeededEntropy(Rng::derive(cx.seed, "celld-site"))));
    site.start(cx.now());
    let udp = cx.udp_bound(tunnel_port)?;
    let mut tcp: Option<i32> = None;
    let mut next_connect = 0u64;
    while !cx.stopping() {
        let now = cx.now();
        if tcp.is_none() && site.wants_core(now) && now.ms >= next_connect {
            match cx.tcp_connect(core, core_port)? {
                Some(fd) => {
                    tcp = Some(fd);
                    cx.log(format!("connected to core {}:{}", fmt_ip(core), core_port));
                    site.core_connected(cx.now());
                }
                None => {
                    next_connect = cx.ms() + 2000;
                }
            }
        }
        let now = cx.now();
        let mut ids = vec![udp];
        ids.extend(tcp);
        let rev = cx.poll(&ids, 0)?;
        if let Some(fd) = tcp {
            if rev.get(1).is_some_and(|r| r & (POLLIN | POLLHUP | POLLERR) != 0) {
                let mut closed = false;
                for _ in 0..16 {
                    match cx.recv(fd, 4096)? {
                        Some(b) if b.is_empty() => {
                            closed = true;
                            break;
                        }
                        Some(b) => {
                            site.on_bytes(&b, now);
                            if !site.core_up() && site.status().core == "down" {
                                // the site gave up on this connection (refused, failed authentication)
                                break;
                            }
                        }
                        None => break,
                    }
                }
                if closed {
                    cx.close(fd);
                    tcp = None;
                    next_connect = now.ms + 1000;
                    site.core_disconnected(now);
                }
            }
        }
        if rev.first().is_some_and(|r| r & POLLIN != 0) {
            for _ in 0..64 {
                let Some((_, d)) = cx.recvfrom(udp)? else { break };
                site.on_tunnel(&d, now);
            }
        }
        for e in cx.radio_events() {
            site.on_phy(e, now);
        }
        site.tick(now);
        for o in site.take_out() {
            match o {
                SiteOut::Phy(c) => cx.phy(c),
                SiteOut::Send(b) => {
                    if let Some(fd) = tcp {
                        if !cx.send_all(fd, &b)? {
                            cx.close(fd);
                            tcp = None;
                            site.core_disconnected(now);
                        }
                    }
                }
                SiteOut::CloseCore => {
                    if let Some(fd) = tcp.take() {
                        cx.close(fd);
                        next_connect = now.ms + 1000;
                    }
                }
                SiteOut::Tunnel(d) => {
                    cx.sendto(udp, core, core_tunnel_port, &d)?;
                }
                SiteOut::Log(l) => cx.log(l),
            }
        }
        let st = site.status();
        let ues: Vec<String> = st.ues.iter().map(|u| format!("{}:{}", if u.imsi.is_empty() { format!("{:08x}", u.addr) } else { u.imsi.clone() }, u.state)).collect();
        cx.set_status(format!("core {} phones [{}] ul_mask {:04b}", st.core, ues.join(" "), st.ul_mask));
        cx.sleep_until(now.ms + TICK_MS as u64);
    }
    Ok(())
}

// ------------------------------------------------------------ the core (cored)

fn run_core(cx: &Cx, db_file: &str, listen: u16, tunnel_port: u16, tun_name: &str) {
    let _ = run_core_inner(cx, db_file, listen, tunnel_port, tun_name);
}

fn run_core_inner(cx: &Cx, db_file: &str, listen: u16, tunnel_port: u16, tun_name: &str) -> Result<(), Stopped> {
    let path = cx.env.root.join(db_file);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            cx.log(format!("cannot read {}: {}", db_file, e));
            return Ok(());
        }
    };
    let db = match Db::parse(&text) {
        Ok(d) => d,
        Err(e) => {
            cx.log(format!("{}: {}", db_file, e));
            return Ok(());
        }
    };
    let (gw, prefix) = (db.gateway, db.prefix);
    cx.log(format!("loaded {} ({} subscribers, {} sites, carrier {})", db_file, db.subs.len(), db.sites.len(), db.carrier));
    let mut core = Core::new(db, CoreConfig::default(), Box::new(SeededEntropy(Rng::derive(cx.seed, "cored"))));
    let lis = cx.socket(AF_INET, SOCK_STREAM, 6)?;
    cx.addr_call(SOCK_BIND, lis, [0; 4], listen)?;
    let mut a = i32le(lis).to_vec();
    a.extend_from_slice(&i32le(16));
    cx.call(SOCK_LISTEN, a)?;
    let udp = cx.udp_bound(tunnel_port)?;
    let mac = [0x02, 0xce, 0xc0, 0x4e, gw[2], gw[3]];
    let tun = cx.tun_open(tun_name, mac, gw, prefix)?;
    if tun < 0 {
        cx.log(format!("{}: tun failed ({})", tun_name, tun));
        return Ok(());
    }
    cx.log(format!("listening on {} (control) and {} (tunnels); {} {}/{}", listen, tunnel_port, tun_name, fmt_ip(gw), prefix));
    let mut conns: BTreeMap<i32, ()> = BTreeMap::new();
    while !cx.stopping() {
        let now = cx.ms();
        let mut ids = vec![lis, udp];
        ids.extend(conns.keys().copied().take(30));
        let rev = cx.poll(&ids, 0)?;
        if rev[0] & POLLIN != 0 {
            while let Some((fd, peer)) = cx.accept(lis)? {
                conns.insert(fd, ());
                core.on_connect(fd as u32, peer, now);
                if cx.poll(&[lis], 0)?[0] & POLLIN == 0 {
                    break;
                }
            }
        }
        if rev[1] & POLLIN != 0 {
            for _ in 0..64 {
                let Some((from, d)) = cx.recvfrom(udp)? else { break };
                core.on_tunnel(from, &d, now);
            }
        }
        for (i, fd) in ids.iter().enumerate().skip(2) {
            if rev[i] & (POLLIN | POLLHUP | POLLERR) == 0 {
                continue;
            }
            let mut closed = false;
            for _ in 0..16 {
                match cx.recv(*fd, 4096)? {
                    Some(b) if b.is_empty() => {
                        closed = true;
                        break;
                    }
                    Some(b) => core.on_bytes(*fd as u32, &b, now),
                    None => break,
                }
            }
            if closed {
                conns.remove(fd);
                cx.close(*fd);
                core.on_disconnect(*fd as u32, now);
            }
        }
        for _ in 0..64 {
            let Some(frame) = cx.recv(tun, 2048)? else { break };
            if frame.is_empty() {
                break;
            }
            match from_kernel(&frame, gw) {
                TunFrame::ArpReply(r) => cx.tun_write(tun, &r)?,
                TunFrame::Ipv4(p) => core.on_lan_packet(p, now),
                TunFrame::Other => {}
            }
        }
        for c in cx.take_admin() {
            match core.admin(&c, now) {
                Ok(r) => cx.log(format!("admin {}: {}", c, r.trim_end().replace('\n', "; "))),
                Err(e) => cx.log(format!("admin {}: error: {}", c, e)),
            }
        }
        core.tick(now);
        for o in core.take_out() {
            match o {
                CoreOut::Send(c, b) => {
                    if conns.contains_key(&(c as i32)) && !cx.send_all(c as i32, &b)? {
                        conns.remove(&(c as i32));
                        cx.close(c as i32);
                        core.on_disconnect(c, now);
                    }
                }
                CoreOut::Close(c) => {
                    if conns.remove(&(c as i32)).is_some() {
                        cx.close(c as i32);
                    }
                }
                CoreOut::Tunnel(addr, d) => {
                    cx.sendto(udp, addr.ip, addr.port, &d)?;
                }
                CoreOut::ToLan(p) => cx.tun_write(tun, &to_kernel(&p, mac))?,
                CoreOut::SaveDb(t) => {
                    if let Err(e) = std::fs::write(&path, t) {
                        cx.log(format!("cannot write {}: {}", db_file, e));
                    }
                }
                CoreOut::Log(l) => cx.log(l),
            }
        }
        let sessions: Vec<String> = core.sessions().iter().map(|s| format!("{}={}:{}@{}", s.imsi, fmt_ip(s.ip), s.state, s.cell.map(|c| c.to_string()).unwrap_or("-".into()))).collect();
        cx.set_status(format!("sites [{}] sessions [{}]", core.site_ids().into_iter().collect::<Vec<_>>().join(" "), sessions.join(" ")));
        cx.sleep_until(now + TICK_MS as u64);
    }
    Ok(())
}
