//! The simulation: N computers on a cable topology, one scheduler.
//!
//! All kernels run on the calling (main) thread, one worker-loop iteration
//! each per round, in node order — kernel execution is therefore
//! deterministic. Children run on their own threads. In virtual-clock mode
//! every round starts by waiting for quiescence (all children blocked in a
//! host call or exited); when a round finds nothing to do, the clock jumps
//! to the earliest pending deadline (kernel `on_tick` deadlines, child
//! sleeps/timeouts, delayed frames). A round cap per virtual millisecond
//! stops zero-latency livelocks (e.g. a broadcast storm on a loop) from
//! freezing the clock: after `max_rounds_per_ms` busy rounds at the same
//! instant, time moves on by 1 ms — so a storm shows up as a huge but
//! finite frame rate, like on real hardware.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use wasmtime::Linker;

use crate::child::{build_linker, ChildCtx, NodeEnv};
use crate::fs::Storage;
use crate::ipc::IpcBridge;
use crate::kernel::Kernel;
use crate::net::Network;
use crate::proc::ProcTable;
use crate::runtime::Runtime;
use crate::sched::{ClockMode, Sched};
use crate::topology::{TopoFile, DEFAULT_IFACES};

#[derive(Clone, Debug)]
pub struct SimConfig {
    pub clock: ClockMode,
    pub seed: u64,
    pub kernel: PathBuf,
    pub programs: Option<PathBuf>,
    /// Per-node storage lives in `<storage>/<node name>/`.
    pub storage: PathBuf,
    pub out_dir: PathBuf,
    pub watchdog: Duration,
    /// Virtual mode: how long to wait for a compute-bound child before
    /// processing events anyway.
    pub stall: Duration,
    pub max_rounds_per_ms: u32,
}

#[derive(Clone, Debug)]
pub struct Mark {
    pub line: u64,
    pub col: usize,
    pub typed: String,
}

pub struct Node {
    pub name: String,
    pub kernel: Kernel,
    pub env: Arc<NodeEnv>,
    pub boot: Vec<String>,
    /// Where the last `send` was typed: absolute screen line, cursor
    /// column, and the text typed (its echo, if any, is excluded from
    /// `expect`).
    pub mark: Option<Mark>,
}

pub struct Sim {
    pub cfg: SimConfig,
    pub sched: Arc<Sched>,
    /// Keeps the engine and its epoch ticker alive.
    #[allow(dead_code)]
    pub rt: Arc<Runtime>,
    pub net: Arc<Mutex<Network>>,
    pub nodes: Vec<Node>,
    #[allow(dead_code)]
    pub linker: Arc<Linker<ChildCtx>>,
    rounds_at: i64,
    rounds: u32,
    stall_warned: bool,
    pub warnings: Vec<String>,
    pub wall_deadline: Option<Instant>,
}

impl Sim {
    pub fn new(cfg: SimConfig, topo: &TopoFile) -> Result<Sim> {
        let sched = Arc::new(Sched::new(cfg.clock));
        let rt = Runtime::new()?;
        let mut net = Network::new(cfg.seed);
        for n in &topo.node {
            net.add_node(&n.name, n.ifaces.unwrap_or(DEFAULT_IFACES));
        }
        let mut pcaps = Vec::new();
        for l in &topo.link {
            let a = net.endpoint(&l.a).map_err(|e| anyhow!("link: {}", e))?;
            let b = net.endpoint(&l.b).map_err(|e| anyhow!("link: {}", e))?;
            let f = l.faults().to_faults().map_err(|e| anyhow!("link {}-{}: {}", l.a, l.b, e))?;
            let s = net.add_segment(l.name.clone(), &[a, b], f).map_err(|e| anyhow!("link: {}", e))?;
            if let Some(p) = &l.pcap {
                pcaps.push((s, p.clone()));
            }
        }
        for sg in &topo.segment {
            let mut m = Vec::new();
            for e in &sg.members {
                m.push(net.endpoint(e).map_err(|e| anyhow!("segment: {}", e))?);
            }
            let f = sg.faults().to_faults().map_err(|e| anyhow!("segment: {}", e))?;
            let s = net.add_segment(sg.name.clone(), &m, f).map_err(|e| anyhow!("segment: {}", e))?;
            if let Some(p) = &sg.pcap {
                pcaps.push((s, p.clone()));
            }
        }
        for (s, p) in pcaps {
            let path = cfg.out_dir.join(p);
            net.start_pcap(s, &path).map_err(|e| anyhow!(e))?;
        }
        let net = Arc::new(Mutex::new(net));
        let linker = Arc::new(build_linker(&rt)?);
        let module = rt
            .module(&cfg.kernel)
            .with_context(|| format!("loading kernel {}", cfg.kernel.display()))?;

        let mut nodes = Vec::new();
        for (i, n) in topo.node.iter().enumerate() {
            let root = cfg.storage.join(&n.name);
            let storage = Arc::new(Storage::new(root.clone(), cfg.programs.clone()));
            let env = Arc::new(NodeEnv {
                node: i,
                name: n.name.clone(),
                runtime: rt.clone(),
                linker: linker.clone(),
                sched: sched.clone(),
                procs: ProcTable::new(sched.clone()),
                ipc: IpcBridge::new(sched.clone()),
                net: net.clone(),
                root,
                seed: cfg.seed,
                log: Arc::new(Mutex::new(Vec::new())),
            });
            let kernel = Kernel::boot(env.clone(), storage, &module, cfg.watchdog)
                .with_context(|| format!("booting {}", n.name))?;
            nodes.push(Node { name: n.name.clone(), kernel, env, boot: n.boot.clone(), mark: None });
        }
        Ok(Sim {
            cfg,
            sched,
            rt,
            net,
            nodes,
            linker,
            rounds_at: i64::MIN,
            rounds: 0,
            stall_warned: false,
            warnings: Vec::new(),
            wall_deadline: None,
        })
    }

    pub fn node(&self, name: &str) -> Result<usize, String> {
        self.nodes
            .iter()
            .position(|n| n.name == name)
            .ok_or_else(|| format!("unknown node '{}' (have: {})", name, self.node_names().join(", ")))
    }

    pub fn node_names(&self) -> Vec<String> {
        self.nodes.iter().map(|n| n.name.clone()).collect()
    }

    pub fn now(&self) -> i64 {
        self.sched.now()
    }

    fn warn(&mut self, msg: String) {
        if std::env::var_os("ECM_SIM_VERBOSE").is_some() {
            eprintln!("[sim] {}", msg);
        }
        self.warnings.push(msg);
    }

    /// One round.
    pub fn pump(&mut self, limit: Option<i64>) {
        match self.cfg.clock {
            ClockMode::Virtual => self.pump_virtual(limit),
            ClockMode::Real => self.pump_real(limit),
        }
    }

    fn step_all(&mut self, now: i64) -> bool {
        self.net.lock().unwrap_or_else(|e| e.into_inner()).deliver_due(now);
        let mut did = false;
        for n in self.nodes.iter_mut() {
            did |= n.kernel.step(now);
        }
        did
    }

    fn next_event(&self, limit: Option<i64>) -> Option<i64> {
        let mut next = limit;
        let mut min = |t: Option<i64>| {
            if let Some(t) = t {
                next = Some(next.map_or(t, |n: i64| n.min(t)));
            }
        };
        for n in &self.nodes {
            if n.kernel.faulted.is_none() {
                min(Some(n.kernel.next_due));
            }
        }
        min(self.net.lock().unwrap_or_else(|e| e.into_inner()).next_delivery());
        min(self.sched.next_actor_deadline());
        next
    }

    fn pump_virtual(&mut self, limit: Option<i64>) {
        let quiet = self.sched.quiesce(self.cfg.stall);
        if !quiet && !self.stall_warned {
            self.stall_warned = true;
            let who = self.sched.running_labels().join(", ");
            self.warn(format!(
                "child process(es) still running after {:?} of wall time without blocking ({}); \
                 processing events anyway (virtual time is no longer strictly deterministic)",
                self.cfg.stall, who
            ));
        }
        let now = self.sched.now();
        let did = self.step_all(now);
        if did || !quiet {
            if self.rounds_at == now {
                self.rounds += 1;
            } else {
                self.rounds_at = now;
                self.rounds = 1;
            }
            if self.rounds >= self.cfg.max_rounds_per_ms {
                self.sched.advance_to(now + 1);
            }
            return;
        }
        if self.sched.busy() > 0 {
            return;
        }
        match self.next_event(limit) {
            Some(t) if t > now => self.sched.advance_to(t),
            _ => self.sched.advance_to(now + 1),
        }
    }

    fn pump_real(&mut self, limit: Option<i64>) {
        let seen = self.sched.events();
        let now = self.sched.now();
        if self.step_all(now) {
            return;
        }
        let next = self.next_event(limit).unwrap_or(now + 50);
        let wait = (next - now).clamp(1, 50) as u64;
        self.sched.wait_event(seen, Duration::from_millis(wait));
    }

    /// Run until `pred` holds or the clock reaches `deadline`.
    pub fn run_until(&mut self, deadline: i64, mut pred: impl FnMut(&Sim) -> bool) -> Result<bool, String> {
        loop {
            if pred(self) {
                return Ok(true);
            }
            if self.now() >= deadline {
                return Ok(false);
            }
            if let Some(w) = self.wall_deadline {
                if Instant::now() > w {
                    return Err("wall-clock budget for this run exceeded".into());
                }
            }
            self.pump(Some(deadline));
        }
    }

    pub fn run_for(&mut self, ms: i64) -> Result<(), String> {
        let d = self.now() + ms;
        self.run_until(d, |_| false).map(|_| ())
    }

    /// Type text on a node's keyboard (with Enter if `enter`).
    pub fn send(&mut self, node: usize, text: &str, enter: bool) {
        let n = &mut self.nodes[node];
        let s = &n.kernel.screen;
        n.mark = Some(Mark { line: s.cursor_abs(), col: s.cursor.0, typed: text.to_string() });
        let mut b = text.as_bytes().to_vec();
        if enter {
            b.push(b'\n');
        }
        n.kernel.shared.send_input(&b, &self.sched);
    }

    pub fn type_raw(&mut self, node: usize, bytes: &[u8]) {
        let n = &self.nodes[node];
        n.kernel.shared.send_input(bytes, &self.sched);
    }

    pub fn ctrl_t(&mut self, node: usize) {
        self.nodes[node].kernel.shared.ctrl_t();
        self.sched.notify_event();
    }

    /// Is `node` showing a prompt on a line after its last command?
    pub fn at_prompt(&self, node: usize) -> bool {
        let n = &self.nodes[node];
        let s = &n.kernel.screen;
        if let Some(m) = &n.mark {
            if s.cursor_abs() <= m.line {
                return false;
            }
        }
        let p = s.cursor_line_prefix();
        let t = p.trim_end();
        t.ends_with('>') || t.ends_with('#')
    }

    /// Type each node's boot commands, one per prompt.
    pub fn run_boot_commands(&mut self, timeout_ms: i64) -> Result<(), String> {
        for i in 0..self.nodes.len() {
            let cmds = self.nodes[i].boot.clone();
            for c in cmds {
                let d = self.now() + timeout_ms;
                if !self.run_until(d, |s| s.at_prompt(i))? {
                    return Err(format!("{}: no prompt before boot command '{}'", self.nodes[i].name, c));
                }
                self.send(i, &c, true);
            }
            if !self.nodes[i].boot.is_empty() {
                let d = self.now() + timeout_ms;
                if !self.run_until(d, |s| s.at_prompt(i))? {
                    return Err(format!("{}: boot commands did not return to the prompt", self.nodes[i].name));
                }
            }
        }
        Ok(())
    }

    pub fn host_log(&self, node: usize) -> Vec<String> {
        self.nodes[node].env.log.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Kill every child and give them a moment to unwind.
    pub fn shutdown(&mut self) {
        for n in &self.nodes {
            n.env.procs.kill_all();
        }
        let _ = self.sched.quiesce(Duration::from_secs(2));
    }

    pub fn out_path(&self, p: &str) -> PathBuf {
        let pp = Path::new(p);
        if pp.is_absolute() {
            pp.to_path_buf()
        } else {
            self.cfg.out_dir.join(pp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake kernel whose on_input spins forever: exercises epoch
    /// interruption, Ctrl+T-on-a-stuck-call and the watchdog, followed by
    /// kernel_recover.
    const SPIN_KERNEL: &str = r#"
    (module
      (import "env" "get_time_ms" (func $t (result i64)))
      (memory (export "memory") 2)
      (global $recovered (mut i32) (i32.const 0))
      (func (export "main"))
      (func (export "abi_scratch") (result i32) (i32.const 1024))
      (func (export "abi_layout") (param $p i32) (param $cap i32) (result i32)
        (i32.store offset=0  (local.get $p) (i32.const 1))
        (i32.store offset=4  (local.get $p) (i32.const 4096))
        (i32.store offset=8  (local.get $p) (i32.const 256))
        (i32.store offset=12 (local.get $p) (i32.const 8192))
        (i32.store offset=16 (local.get $p) (i32.const 256))
        (i32.store offset=20 (local.get $p) (i32.const 12288))
        (i32.store offset=24 (local.get $p) (i32.const 256))
        (i32.store offset=28 (local.get $p) (i32.const 16384))
        (i32.store offset=32 (local.get $p) (i32.const 256))
        (i32.store offset=36 (local.get $p) (i32.const 20480))
        (i32.store offset=40 (local.get $p) (i32.const 1024))
        (i32.const 15))
      (func (export "on_input") (param i32 i32) (loop $l (br $l)))
      (func (export "on_interrupt") (param i32 i32 i32))
      (func (export "on_tick") (param i64) (result i64) (i64.const -1))
      (func (export "handle_sock_ipc") (param i32 i32 i32 i32 i32 i32) (result i32) (i32.const -1))
      (func (export "kernel_recover") (global.set $recovered (i32.const 1))))
    "#;

    fn spin_sim(watchdog_ms: u64) -> Sim {
        let dir = std::env::temp_dir().join(format!("ecm-sim-spin-{}-{}", std::process::id(), watchdog_ms));
        std::fs::create_dir_all(&dir).unwrap();
        let k = dir.join("spin.wat");
        std::fs::write(&k, SPIN_KERNEL).unwrap();
        let cfg = SimConfig {
            clock: ClockMode::Virtual,
            seed: 1,
            kernel: k,
            programs: None,
            storage: dir.join("storage"),
            out_dir: dir.join("out"),
            watchdog: Duration::from_millis(watchdog_ms),
            stall: Duration::from_secs(5),
            max_rounds_per_ms: 16,
        };
        Sim::new(cfg, &TopoFile::unconnected(1)).unwrap()
    }

    #[test]
    fn ctrl_t_traps_a_stuck_kernel_call_and_recovers() {
        let mut sim = spin_sim(60_000);
        let shared = sim.nodes[0].kernel.shared.clone();
        let sched = sim.sched.clone();
        sim.type_raw(0, b"x");
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(600));
            // TerminalBlockEntity: Ctrl+T in the input string.
            shared.send_input(b"\x14", &sched);
        });
        let t0 = Instant::now();
        sim.pump(None);
        t.join().unwrap();
        assert!(t0.elapsed() < Duration::from_secs(10));
        let k = &sim.nodes[0].kernel;
        assert_eq!(k.recoveries, 1, "kernel_recover called once");
        assert!(k.faulted.is_none(), "a host-forced trap does not fault the computer");
    }

    #[test]
    fn watchdog_traps_runaway_kernel_call() {
        let mut sim = spin_sim(300);
        sim.type_raw(0, b"x");
        sim.pump(None);
        assert_eq!(sim.nodes[0].kernel.recoveries, 1);
        assert!(sim.host_log(0).iter().any(|l| l.contains("watchdog")));
    }
}
