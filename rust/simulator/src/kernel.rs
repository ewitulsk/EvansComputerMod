//! One computer: the kernel instance (`terminal_os.wasm`) and its host.
//!
//! Implements every import in `abi/host-abi.toml` with the Java host's
//! semantics and links strictly (a missing or mismatched import is a load
//! error listing all problems). Buffer addresses come from the kernel's
//! `abi_scratch` + `abi_layout` exports — there are no fixed addresses.
//!
//! [`Kernel::step`] is one iteration of the Java `ComputerInstance`
//! worker loop: deliver interrupts (one coalesced IRQ_NETWORK), input,
//! tick when the deadline passed or anything happened, and service child
//! socket requests with `IPC_PENDING` retry. It never waits; the
//! simulation loop decides when to call it.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use wasmtime::{Caller, Linker, Memory, Module, Store, TypedFunc, UpdateDeadline};

use crate::child::{self, provided_sigs, NodeEnv};
use crate::fs::Storage;
use crate::ipc::{IpcReq, IpcResult, IPC_PENDING};
use crate::proc::{FileHandle, Handle, Pipe, TryWait};
use crate::runtime::{check_imports, interrupt_err, HostInterrupt};
use crate::screen::Screen;
use crate::util::Rng;

pub const IRQ_NETWORK: i32 = 3;
pub const IRQ_TERMINATE: i32 = 15;
/// Kernel calls running longer than this can be trapped by Ctrl+T.
pub const STUCK_KERNEL_CALL_MS: u64 = 250;
/// Java `kernelTick`: a failed tick retries 100 ms later, and the worker
/// loop wakes at least every 100 ms, so an on_tick of -1 means "tick again
/// in ≤100 ms".
const IDLE_TICK_MS: i64 = 100;
/// Kernel-side pipe capacity for `pipe_create` (matches WasiPipe default).
const KPIPE_CAP: usize = 16 * 1024;

// Kernel fd_open flags (shell.rs).
const O_WRONLY: i32 = 1;
const O_CREAT: i32 = 4;
const O_TRUNC: i32 = 8;
const O_APPEND: i32 = 16;

#[derive(Clone, Copy, Debug, Default)]
pub struct Layout {
    pub input: u32,
    pub input_cap: u32,
    pub irq: u32,
    pub irq_cap: u32,
    pub ipc_args: u32,
    pub ipc_args_cap: u32,
    pub ipc_result: u32,
    pub ipc_result_cap: u32,
    pub fb: u32,
    pub fb_cap: u32,
    pub gfx: u32,
    pub gfx_cap: u32,
    pub screen: u32,
    pub screen_cap: u32,
}

/// State shared with other threads (UI input thread, epoch callback).
pub struct Shared {
    pub input: Mutex<VecDeque<Vec<u8>>>,
    pub irqs: Mutex<VecDeque<(i32, Vec<u8>)>>,
    /// Ctrl+T on a stuck kernel call: trap it at the next epoch tick.
    pub interrupt: AtomicBool,
    /// Wall-clock ms (since `t0`) the current kernel call started, 0 = idle.
    pub call_started: AtomicU64,
    pub t0: Instant,
}

impl Shared {
    fn call_elapsed(&self) -> Option<Duration> {
        let s = self.call_started.load(Ordering::SeqCst);
        if s == 0 {
            return None;
        }
        let now = self.t0.elapsed().as_millis() as u64 + 1;
        Some(Duration::from_millis(now.saturating_sub(s)))
    }

    /// Keyboard input as `TerminalBlockEntity.onStringInput` delivers it:
    /// input containing Ctrl+T is not passed on; instead the host
    /// interrupts a stuck call and queues IRQ_TERMINATE.
    pub fn send_input(&self, bytes: &[u8], sched: &crate::sched::Sched) {
        if bytes.contains(&0x14) {
            self.ctrl_t();
        } else {
            self.input.lock().unwrap_or_else(|e| e.into_inner()).push_back(bytes.to_vec());
        }
        sched.notify_event();
    }

    pub fn ctrl_t(&self) {
        if self.call_elapsed().is_some_and(|d| d > Duration::from_millis(STUCK_KERNEL_CALL_MS)) {
            self.interrupt.store(true, Ordering::SeqCst);
        }
        self.irqs.lock().unwrap_or_else(|e| e.into_inner()).push_back((IRQ_TERMINATE, b"{}".to_vec()));
    }
}

pub struct KHost {
    env: Arc<NodeEnv>,
    storage: Arc<Storage>,
    kfds: BTreeMap<i32, Handle>,
    next_kfd: i32,
    rng: Rng,
    rx_rr: usize,
    memory: Option<Memory>,
    fb_base: u32,
    out_lines_left: usize,
    shared: Arc<Shared>,
    watchdog: Duration,
    trap_reason: Option<&'static str>,
}

pub struct Kernel {
    pub env: Arc<NodeEnv>,
    pub shared: Arc<Shared>,
    store: Store<KHost>,
    mem: Memory,
    f_input: TypedFunc<(i32, i32), ()>,
    f_irq: TypedFunc<(i32, i32, i32), ()>,
    f_tick: TypedFunc<i64, i64>,
    f_ipc: TypedFunc<(i32, i32, i32, i32, i32, i32), i32>,
    f_recover: TypedFunc<(), ()>,
    pub layout: Layout,
    pub faulted: Option<String>,
    /// Effective next tick time (sim ms).
    pub next_due: i64,
    waiting: Vec<IpcReq>,
    pub screen: Screen,
    pub calls: u64,
    pub recoveries: u64,
    last_fb_header: Option<[u8; 16]>,
}

// ------------------------------------------------------------ memory helpers

type KC<'a> = Caller<'a, KHost>;

fn krd(c: &KC<'_>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    let m = c.data().memory?;
    let d = m.data(c);
    let p = ptr as u32 as usize;
    let l = len as u32 as usize;
    d.get(p..p.checked_add(l)?).map(|s| s.to_vec())
}

fn krd_str(c: &KC<'_>, ptr: i32, len: i32) -> Option<String> {
    krd(c, ptr, len).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn kwr(c: &mut KC<'_>, ptr: i32, data: &[u8]) -> bool {
    let Some(m) = c.data().memory else { return false };
    let d = m.data_mut(c);
    let p = ptr as u32 as usize;
    match p.checked_add(data.len()).and_then(|e| d.get_mut(p..e)) {
        Some(s) => {
            s.copy_from_slice(data);
            true
        }
        None => false,
    }
}

fn fb_dims(c: &KC<'_>) -> (u16, u16) {
    let base = c.data().fb_base as i32;
    match krd(c, base, 6) {
        Some(b) => (u16::from_le_bytes([b[2], b[3]]), u16::from_le_bytes([b[4], b[5]])),
        None => (0, 0),
    }
}

fn nic_of(c: &KC<'_>, index: i32) -> Option<usize> {
    if index < 0 {
        return None;
    }
    let node = c.data().env.node;
    c.data().env.net.lock().unwrap_or_else(|e| e.into_inner()).nic(node, index as usize)
}

// ------------------------------------------------------------ linker

fn build_linker(engine: &wasmtime::Engine) -> Result<Linker<KHost>> {
    let mut l: Linker<KHost> = Linker::new(engine);
    const E: &str = "env";

    // --- time / entropy ---
    l.func_wrap(E, "get_time_ms", |c: KC<'_>| -> i64 { c.data().env.sched.now() })?;
    l.func_wrap(E, "__getrandom_v03_custom", |mut c: KC<'_>, ptr: i32, len: i32| -> i32 {
        if len <= 0 || len > 4096 {
            return -1;
        }
        let mut b = vec![0u8; len as usize];
        c.data_mut().rng.fill(&mut b);
        if kwr(&mut c, ptr, &b) {
            0
        } else {
            -1
        }
    })?;
    l.func_wrap(E, "fb_sync", |_c: KC<'_>| {})?;

    // --- network ---
    l.func_wrap(E, "net_get_interface_count", |c: KC<'_>| -> i32 {
        let node = c.data().env.node;
        c.data().env.net.lock().unwrap_or_else(|e| e.into_inner()).iface_count(node) as i32
    })?;
    l.func_wrap(E, "net_get_interface_mac", |mut c: KC<'_>, index: i32, buf: i32| -> i32 {
        let Some(nic) = nic_of(&c, index) else { return -1 };
        let mac = c.data().env.net.lock().unwrap_or_else(|e| e.into_inner()).nics[nic].mac;
        kwr(&mut c, buf, &mac);
        6
    })?;
    l.func_wrap(E, "net_tx_frame_on", |c: KC<'_>, index: i32, buf: i32, len: i32| -> i32 {
        let Some(nic) = nic_of(&c, index) else { return -1 };
        if !(14..=1518).contains(&len) {
            return -1;
        }
        let Some(frame) = krd(&c, buf, len) else { return -1 };
        let now = c.data().env.sched.now();
        let ok = c.data().env.net.lock().unwrap_or_else(|e| e.into_inner()).transmit(nic, &frame, now);
        if ok {
            0
        } else {
            -1
        }
    })?;
    l.func_wrap(E, "net_rx_frame_any", |mut c: KC<'_>, buf: i32, cap: i32, idx_ptr: i32| -> i32 {
        let node = c.data().env.node;
        let mut rr = c.data().rx_rr;
        let got = c.data().env.net.lock().unwrap_or_else(|e| e.into_inner()).rx_any(node, &mut rr);
        c.data_mut().rx_rr = rr;
        match got {
            Some((iface, frame)) => {
                let n = frame.len().min(cap.max(0) as usize);
                kwr(&mut c, buf, &frame[..n]);
                kwr(&mut c, idx_ptr, &(iface as i32).to_le_bytes());
                n as i32
            }
            None => -1,
        }
    })?;
    l.func_wrap(E, "net_set_promiscuous_on", |c: KC<'_>, index: i32, on: i32| -> i32 {
        let Some(nic) = nic_of(&c, index) else { return -1 };
        c.data().env.net.lock().unwrap_or_else(|e| e.into_inner()).set_promisc(nic, on != 0);
        0
    })?;
    l.func_wrap(E, "net_set_link_state", |c: KC<'_>, index: i32, up: i32| -> i32 {
        let Some(nic) = nic_of(&c, index) else { return -1 };
        c.data().env.net.lock().unwrap_or_else(|e| e.into_inner()).set_admin(nic, up != 0);
        0
    })?;
    l.func_wrap(E, "net_get_link_state", |c: KC<'_>, index: i32| -> i32 {
        let Some(nic) = nic_of(&c, index) else { return 0 };
        c.data().env.net.lock().unwrap_or_else(|e| e.into_inner()).carrier(nic) as i32
    })?;

    // --- processes ---
    l.func_wrap(
        E,
        "process_spawn",
        |c: KC<'_>, pp: i32, pl: i32, ap: i32, al: i32, sin: i32, sout: i32, serr: i32| -> i32 {
            let Some(path) = krd_str(&c, pp, pl) else { return -1 };
            let argv_s = krd_str(&c, ap, al).unwrap_or_default();
            let Some((real, _)) = c.data().storage.resolve_read(&path) else { return -1 };
            if !real.is_file() {
                c.data().env.log(format!("process_spawn: not found: {}", path));
                return -1;
            }
            let argv: Vec<String> = if argv_s.is_empty() { vec![path.clone()] } else { argv_s.split('\n').map(String::from).collect() };
            let (w, h) = fb_dims(&c);
            let envv = vec![("COLUMNS".to_string(), w.to_string()), ("LINES".to_string(), h.to_string())];
            let stdio = [sin, sout, serr].map(|fd| if fd >= 0 { c.data().kfds.get(&fd).cloned() } else { None });
            let env = c.data().env.clone();
            let pid = child::spawn(&env, real, argv, envv, stdio);
            env.log(format!("spawn pid {} {}", pid, argv_s.replace('\n', " ")));
            pid
        },
    )?;
    l.func_wrap(E, "process_try_wait", |mut c: KC<'_>, pid: i32, code_ptr: i32| -> i32 {
        match c.data().env.procs.try_wait(pid) {
            TryWait::Running => 0,
            TryWait::Unknown => -1,
            TryWait::Exited(code) => {
                kwr(&mut c, code_ptr, &code.to_le_bytes());
                1
            }
        }
    })?;
    l.func_wrap(E, "process_read_output", |mut c: KC<'_>, pid: i32, buf: i32, len: i32| -> i32 {
        let len = len.clamp(0, 65536) as usize;
        let budget = c.data().out_lines_left;
        let mut tmp = vec![0u8; len];
        let (n, nl) = c.data().env.procs.read_output(pid, &mut tmp, budget);
        c.data_mut().out_lines_left = budget.saturating_sub(nl);
        if n > 0 {
            kwr(&mut c, buf, &tmp[..n as usize]);
        }
        if c.data().out_lines_left == 0 && c.data().env.procs.output_pending(pid) {
            // Throttled: come back next round so every line gets its own
            // framebuffer snapshot (and lands in the scrollback).
            c.data().env.procs.mark_activity();
        }
        n
    })?;
    l.func_wrap(E, "process_write_input", |c: KC<'_>, pid: i32, buf: i32, len: i32| -> i32 {
        let len = len.clamp(0, 65536);
        let Some(data) = krd(&c, buf, len) else { return -1 };
        c.data().env.procs.write_input(pid, &data)
    })?;
    l.func_wrap(E, "process_kill", |c: KC<'_>, pid: i32, _sig: i32| -> i32 {
        let r = c.data().env.procs.kill(pid);
        c.data().env.log(format!("kill pid {} -> {}", pid, r));
        r
    })?;
    l.func_wrap(E, "process_list", |mut c: KC<'_>, buf: i32, cap: i32| -> i32 {
        let json = c.data().env.procs.list_json();
        let n = json.len().min(cap.max(0) as usize);
        kwr(&mut c, buf, &json.as_bytes()[..n]);
        n as i32
    })?;

    // --- files ---
    l.func_wrap(E, "file_write", |c: KC<'_>, pp: i32, pl: i32, dp: i32, dl: i32| -> i32 {
        let Some(path) = krd_str(&c, pp, pl) else { return -1 };
        if dl < 0 {
            return -1;
        }
        let Some(data) = krd(&c, dp, dl) else { return -1 };
        c.data().storage.write(&path, &data)
    })?;
    l.func_wrap(E, "file_read", |mut c: KC<'_>, pp: i32, pl: i32, bp: i32, bl: i32| -> i32 {
        let Some(path) = krd_str(&c, pp, pl) else { return -1 };
        let Some(data) = c.data().storage.read(&path) else { return -1 };
        let n = data.len().min(bl.max(0) as usize);
        kwr(&mut c, bp, &data[..n]);
        n as i32
    })?;
    l.func_wrap(E, "file_size", |c: KC<'_>, pp: i32, pl: i32| -> i32 {
        krd_str(&c, pp, pl).map(|p| c.data().storage.size(&p)).unwrap_or(-1)
    })?;
    l.func_wrap(E, "file_exists", |c: KC<'_>, pp: i32, pl: i32| -> i32 {
        krd_str(&c, pp, pl).is_some_and(|p| c.data().storage.exists(&p)) as i32
    })?;
    l.func_wrap(E, "file_is_dir", |c: KC<'_>, pp: i32, pl: i32| -> i32 {
        krd_str(&c, pp, pl).is_some_and(|p| c.data().storage.is_dir(&p)) as i32
    })?;
    l.func_wrap(E, "file_list_dir", |mut c: KC<'_>, pp: i32, pl: i32, bp: i32, bl: i32| -> i32 {
        let path = krd_str(&c, pp, pl).unwrap_or_default();
        let Some(s) = c.data().storage.list_dir(&path) else { return -1 };
        let n = s.len().min(bl.max(0) as usize);
        kwr(&mut c, bp, &s.as_bytes()[..n]);
        n as i32
    })?;
    // Java stubs fd_open/pipe_create (-1: the shell then rejects redirects
    // and pipelines). The simulator implements them.
    l.func_wrap(E, "fd_open", |mut c: KC<'_>, pp: i32, pl: i32, flags: i32| -> i32 {
        let Some(path) = krd_str(&c, pp, pl) else { return -1 };
        let write = flags & O_WRONLY != 0;
        let file = if write {
            let Some(p) = c.data().storage.resolve_write(&path) else { return -1 };
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::OpenOptions::new()
                .write(true)
                .create(flags & O_CREAT != 0)
                .truncate(flags & O_TRUNC != 0)
                .append(flags & O_APPEND != 0)
                .open(&p)
                .map(|f| (f, p))
        } else {
            let Some((p, _)) = c.data().storage.resolve_read(&path) else { return -1 };
            std::fs::File::open(&p).map(|f| (f, p))
        };
        let Ok((file, p)) = file else { return -1 };
        let h = Handle::File(Arc::new(Mutex::new(FileHandle { file, path: p, append: flags & O_APPEND != 0 })));
        let fd = c.data().next_kfd;
        c.data_mut().next_kfd += 1;
        c.data_mut().kfds.insert(fd, h);
        fd
    })?;
    l.func_wrap(E, "fd_close", |mut c: KC<'_>, fd: i32| -> i32 {
        if c.data_mut().kfds.remove(&fd).is_some() {
            0
        } else {
            -1
        }
    })?;
    l.func_wrap(E, "pipe_create", |mut c: KC<'_>, rp: i32, wp: i32| -> i32 {
        let sched = c.data().env.sched.clone();
        let p = Pipe::new(KPIPE_CAP, sched, None);
        let (r, w) = (Handle::pipe_r(&p), Handle::pipe_w(&p));
        let rfd = c.data().next_kfd;
        let wfd = rfd + 1;
        c.data_mut().next_kfd += 2;
        c.data_mut().kfds.insert(rfd, r);
        c.data_mut().kfds.insert(wfd, w);
        kwr(&mut c, rp, &rfd.to_le_bytes());
        kwr(&mut c, wp, &wfd.to_le_bytes());
        0
    })?;

    // --- screen cluster: no in-world screen attached ---
    l.func_wrap(E, "screen_is_attached", |_c: KC<'_>| -> i32 { 0 })?;
    l.func_wrap(E, "screen_get_gfx_width", |_c: KC<'_>| -> i32 { 0 })?;
    l.func_wrap(E, "screen_get_gfx_height", |_c: KC<'_>| -> i32 { 0 })?;
    l.func_wrap(E, "screen_fb_sync", |_c: KC<'_>| {})?;
    l.func_wrap(E, "screen_set_power", |_c: KC<'_>, _on: i32| {})?;
    l.func_wrap(E, "screen_set_pixel_format", |_c: KC<'_>, _f: i32| {})?;
    l.func_wrap(E, "open_visual_editor", |c: KC<'_>| {
        c.data().env.log("open_visual_editor (no-op in the simulator)".into());
    })?;
    Ok(l)
}

// ------------------------------------------------------------ kernel

impl Kernel {
    /// Instantiate, link strictly, read the layout, run `main`.
    pub fn boot(env: Arc<NodeEnv>, storage: Arc<Storage>, module: &Module, watchdog: Duration) -> Result<Kernel> {
        let engine = env.runtime.engine.clone();
        let shared = Arc::new(Shared {
            input: Mutex::new(VecDeque::new()),
            irqs: Mutex::new(VecDeque::new()),
            interrupt: AtomicBool::new(false),
            call_started: AtomicU64::new(0),
            t0: Instant::now(),
        });
        let host = KHost {
            rng: Rng::derive(env.seed, &format!("kernel:{}", env.name)),
            env: env.clone(),
            storage,
            kfds: BTreeMap::new(),
            next_kfd: 3,
            rx_rr: 0,
            memory: None,
            fb_base: 0,
            out_lines_left: usize::MAX,
            shared: shared.clone(),
            watchdog,
            trap_reason: None,
        };
        let mut store = Store::new(&engine, host);
        store.set_epoch_deadline(1);
        store.epoch_deadline_callback(|mut c| {
            let h = c.data_mut();
            if let Some(el) = h.shared.call_elapsed() {
                if h.shared.interrupt.swap(false, Ordering::SeqCst) {
                    h.trap_reason = Some("Ctrl+T on a stuck kernel call");
                    return Err(interrupt_err("ctrl-t"));
                }
                if el > h.watchdog {
                    h.trap_reason = Some("watchdog: kernel call ran too long");
                    return Err(interrupt_err("watchdog"));
                }
            }
            Ok(UpdateDeadline::Continue(1))
        });
        let linker = build_linker(&engine)?;
        let provided = provided_sigs(&linker, &mut store);
        check_imports(module, &provided).map_err(|p| anyhow!("Kernel/host ABI mismatch: {}", p))?;
        let inst = linker.instantiate(&mut store, module).context("instantiating kernel")?;
        let mem = inst.get_memory(&mut store, "memory").ok_or_else(|| anyhow!("kernel does not export 'memory'"))?;
        store.data_mut().memory = Some(mem);

        let mut missing = Vec::new();
        macro_rules! export {
            ($name:literal, $p:ty, $r:ty) => {
                match inst.get_typed_func::<$p, $r>(&mut store, $name) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        missing.push(format!("{}: {}", $name, e));
                        None
                    }
                }
            };
        }
        let f_main = export!("main", (), ());
        let f_input = export!("on_input", (i32, i32), ());
        let f_irq = export!("on_interrupt", (i32, i32, i32), ());
        let f_tick = export!("on_tick", i64, i64);
        let f_ipc = export!("handle_sock_ipc", (i32, i32, i32, i32, i32, i32), i32);
        let f_scratch = export!("abi_scratch", (), i32);
        let f_layout = export!("abi_layout", (i32, i32), i32);
        let f_recover = export!("kernel_recover", (), ());
        if !missing.is_empty() {
            bail!("kernel exports do not match abi/host-abi.toml: {}", missing.join("; "));
        }

        // readKernelLayout (before main, like ComputerInstance.loadModule).
        let scratch = f_scratch.unwrap().call(&mut store, ())?;
        let n = f_layout.unwrap().call(&mut store, (scratch, 15))?;
        if n < 15 {
            bail!("abi_layout returned {} words (need 15)", n);
        }
        let raw = mem.data(&store);
        let base = scratch as u32 as usize;
        let mut w = [0u32; 15];
        for (i, v) in w.iter_mut().enumerate() {
            let o = base + i * 4;
            let b = raw.get(o..o + 4).ok_or_else(|| anyhow!("layout scratch out of bounds"))?;
            *v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        }
        if w[0] != 1 {
            bail!("unsupported kernel ABI layout version {}", w[0]);
        }
        let layout = Layout {
            input: w[1],
            input_cap: w[2],
            irq: w[3],
            irq_cap: w[4],
            ipc_args: w[5],
            ipc_args_cap: w[6],
            ipc_result: w[7],
            ipc_result_cap: w[8],
            fb: w[9],
            fb_cap: w[10],
            gfx: w[11],
            gfx_cap: w[12],
            screen: w[13],
            screen_cap: w[14],
        };
        let mem_len = mem.data_size(&store) as u64;
        for (name, a, cap) in [
            ("input", layout.input, layout.input_cap),
            ("irq", layout.irq, layout.irq_cap),
            ("ipc_args", layout.ipc_args, layout.ipc_args_cap),
            ("ipc_result", layout.ipc_result, layout.ipc_result_cap),
            ("fb", layout.fb, layout.fb_cap),
        ] {
            if a as u64 + cap as u64 > mem_len || cap == 0 {
                bail!("kernel layout region {} at {:#x}+{} lies outside memory ({} bytes)", name, a, cap, mem_len);
            }
        }
        store.data_mut().fb_base = layout.fb;

        let mut k = Kernel {
            env,
            shared,
            store,
            mem,
            f_input: f_input.unwrap(),
            f_irq: f_irq.unwrap(),
            f_tick: f_tick.unwrap(),
            f_ipc: f_ipc.unwrap(),
            f_recover: f_recover.unwrap(),
            layout,
            faulted: None,
            next_due: 0, // tick right after boot
            waiting: Vec::new(),
            screen: Screen::new(),
            calls: 0,
            recoveries: 0,
            last_fb_header: None,
        };
        let main = f_main.unwrap();
        k.call("main", move |s| main.call(s, ()));
        if let Some(f) = &k.faulted {
            bail!("kernel main failed: {}", f);
        }
        k.snapshot();
        Ok(k)
    }

    /// callKernel: run one export. A host-forced trap (Ctrl+T while stuck,
    /// watchdog) is recovered through `kernel_recover`; any other trap
    /// faults the computer.
    fn call<R>(&mut self, what: &str, f: impl FnOnce(&mut Store<KHost>) -> Result<R>) -> Option<R> {
        if self.faulted.is_some() {
            return None;
        }
        self.calls += 1;
        let t = self.shared.t0.elapsed().as_millis() as u64 + 1;
        self.shared.call_started.store(t, Ordering::SeqCst);
        self.store.data_mut().out_lines_left = self.screen.h.max(3).saturating_sub(2).max(1);
        let r = f(&mut self.store);
        self.shared.call_started.store(0, Ordering::SeqCst);
        // Snapshot after every call so no more than one throttled batch of
        // lines can scroll by unseen.
        self.snapshot();
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                let forced = e.chain().any(|c| c.downcast_ref::<HostInterrupt>().is_some());
                if forced {
                    let why = self.store.data_mut().trap_reason.take().unwrap_or("host interrupt");
                    self.env.log(format!("{} trapped in {}; calling kernel_recover", why, what));
                    self.recover();
                } else {
                    let msg = format!("kernel trap in {}: {:#}", what, e);
                    self.env.log(msg.clone());
                    self.faulted = Some(msg);
                }
                None
            }
        }
    }

    fn recover(&mut self) {
        self.recoveries += 1;
        self.shared.interrupt.store(false, Ordering::SeqCst);
        let f = self.f_recover.clone();
        let t = self.shared.t0.elapsed().as_millis() as u64 + 1;
        self.shared.call_started.store(t, Ordering::SeqCst);
        let r = f.call(&mut self.store, ());
        self.shared.call_started.store(0, Ordering::SeqCst);
        if let Err(e) = r {
            let msg = format!("kernel_recover failed: {:#}", e);
            self.env.log(msg.clone());
            self.faulted = Some(msg);
        }
    }

    fn write_region(&mut self, addr: u32, data: &[u8]) {
        let d = self.mem.data_mut(&mut self.store);
        let a = addr as usize;
        if let Some(s) = d.get_mut(a..a + data.len()) {
            s.copy_from_slice(data);
        }
    }

    fn tick(&mut self, now: i64) {
        let f = self.f_tick.clone();
        match self.call("on_tick", move |s| f.call(s, now)) {
            Some(d) if d >= 0 => self.next_due = d.max(now),
            _ => self.next_due = now + IDLE_TICK_MS,
        }
    }

    fn deliver_irq(&mut self, irq: i32, payload: &[u8]) {
        let n = payload.len().min(self.layout.irq_cap as usize);
        if n > 0 {
            let a = self.layout.irq;
            self.write_region(a, &payload[..n]);
        }
        let (f, a) = (self.f_irq.clone(), self.layout.irq as i32);
        self.call("on_interrupt", move |s| f.call(s, (irq, a, n as i32)));
    }

    fn deliver_input(&mut self, bytes: &[u8]) {
        let cap = self.layout.input_cap.max(1) as usize;
        for chunk in bytes.chunks(cap) {
            let a = self.layout.input;
            self.write_region(a, chunk);
            let f = self.f_input.clone();
            let n = chunk.len() as i32;
            self.call("on_input", move |s| f.call(s, (a as i32, n)));
            if self.faulted.is_some() {
                return;
            }
        }
    }

    /// One worker-loop iteration. Returns true if anything happened (so
    /// the simulation loop must not jump the clock yet).
    pub fn step(&mut self, now: i64) -> bool {
        if self.faulted.is_some() {
            // A dead kernel can never answer; fail its children's calls.
            for r in self.waiting.drain(..).chain(self.env.ipc.take_incoming()) {
                self.env.ipc.complete(r, IpcResult::error());
            }
            return false;
        }
        let mut events = false;
        loop {
            let next = self.shared.irqs.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
            let Some((irq, payload)) = next else { break };
            self.deliver_irq(irq, &payload);
            events = true;
        }
        // Coalesced IRQ_NETWORK: the flag is cleared before delivery so a
        // frame arriving during the drain queues the next one.
        let net_irq = self.env.net.lock().unwrap_or_else(|e| e.into_inner()).take_irq(self.env.node);
        if net_irq {
            self.deliver_irq(IRQ_NETWORK, b"{}");
            events = true;
        }
        loop {
            let next = self.shared.input.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
            let Some(bytes) = next else { break };
            self.deliver_input(&bytes);
            events = true;
        }
        let child_event = self.env.procs.take_activity();
        let new_reqs = self.env.ipc.take_incoming();
        let new_req = !new_reqs.is_empty();
        self.waiting.extend(new_reqs);
        let due = now >= self.next_due;
        let worked = events || child_event || due || new_req;
        if worked {
            self.tick(now);
            if !self.waiting.is_empty() && self.service_ipc() > 0 {
                // Completed requests may have queued frames or timers.
                self.tick(now);
            }
        }
        worked
    }

    /// Dispatch new requests and retry pending ones. Returns completions.
    fn service_ipc(&mut self) -> usize {
        let reqs = std::mem::take(&mut self.waiting);
        let mut completed = 0;
        for req in reqs {
            if req.is_abandoned() {
                continue;
            }
            if self.faulted.is_some() {
                self.env.ipc.complete(req, IpcResult::error());
                continue;
            }
            match self.dispatch(&req) {
                None => self.waiting.push(req),
                Some(r) => {
                    self.env.ipc.complete(req, r);
                    completed += 1;
                }
            }
        }
        completed
    }

    /// One `handle_sock_ipc` call. None = IPC_PENDING.
    fn dispatch(&mut self, req: &IpcReq) -> Option<IpcResult> {
        if req.args.len() > self.layout.ipc_args_cap as usize {
            return Some(IpcResult::error()); // never truncate args
        }
        let (aa, ra, rc) = (self.layout.ipc_args, self.layout.ipc_result, self.layout.ipc_result_cap);
        let args = req.args.clone();
        self.write_region(aa, &args);
        let f = self.f_ipc.clone();
        let (sess, sc, al) = (req.session, req.syscall, args.len() as i32);
        let rv = self.call("handle_sock_ipc", move |s| f.call(s, (sess, sc, aa as i32, al, ra as i32, rc as i32)))?;
        if rv == IPC_PENDING {
            return None;
        }
        if rv < 4 {
            return Some(IpcResult::error());
        }
        let len = (rv as u32).min(rc) as usize;
        let d = self.mem.data(&self.store);
        let raw = d.get(ra as usize..ra as usize + len)?;
        let status = i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        Some(IpcResult { status, payload: raw[4..].to_vec() })
    }

    /// Refresh the text capture from the framebuffer. Cheap when nothing
    /// changed: the VTE bumps the header's dirty counter on every flush.
    pub fn snapshot(&mut self) -> bool {
        let d = self.mem.data(&self.store);
        let a = self.layout.fb as usize;
        let cap = self.layout.fb_cap as usize;
        let Some(fb) = d.get(a..a + cap) else { return false };
        let hdr: [u8; 16] = match fb[..16].try_into() {
            Ok(h) => h,
            Err(_) => return false,
        };
        if self.last_fb_header == Some(hdr) {
            return false;
        }
        self.last_fb_header = Some(hdr);
        let fb = fb.to_vec();
        self.screen.update(&fb)
    }

    pub fn has_pending_ipc(&self) -> bool {
        !self.waiting.is_empty()
    }

    pub fn pending_input(&self) -> bool {
        !self.shared.input.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
            || !self.shared.irqs.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    pub fn storage_root(&self) -> PathBuf {
        self.store.data().storage.root.clone()
    }
}
