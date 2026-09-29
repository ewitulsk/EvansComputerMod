//! WASI child processes: `wasi_snapshot_preview1` plus the `env` host
//! functions the programs in `rust/wasm-programs` import, with the same
//! semantics as the Java host (`WasiFunctions`, `ProcessManager`,
//! `SocketFd`, `NetIpcBridge`).
//!
//! Each child runs on its own OS thread with its own store. Every blocking
//! host call (stdin/pipe read, full-pipe write, socket IPC, sleep,
//! `poll_oneoff`) waits through the scheduler so virtual time only moves
//! when the child is idle, and so Ctrl+T (`process_kill`) can interrupt it.

use std::collections::{BTreeMap, HashMap};
use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use wasmtime::{Caller, Extern, Linker, Memory, Store, UpdateDeadline};

use crate::fs::{safe_join, MAX_WRITE};
use crate::ipc::{IpcBridge, IpcResult};
use crate::net::Network;
use crate::proc::{FileHandle, Handle, Killed, Pipe, PipeReader, PipeWriter, ProcEntry, ProcTable, STDIN_PIPE_CAP, STDOUT_PIPE_CAP};
use crate::runtime::{check_imports, sig_string, ProcExit, Runtime};
use crate::sched::{ActorId, Sched, Wake};
use crate::util::Rng;

// WASI errno values.
const ESUCCESS: i32 = 0;
const EAGAIN: i32 = 6;
const FDFLAGS_NONBLOCK: i32 = 4;
const EBADF: i32 = 8;
const EEXIST: i32 = 20;
const EINVAL: i32 = 28;
const EIO: i32 = 29;
const EISDIR: i32 = 31;
const ENOENT: i32 = 44;
const ENOSYS: i32 = 52;
const ENOTDIR: i32 = 54;
const ENOTEMPTY: i32 = 55;

const FT_CHAR: u8 = 2;
const FT_DIR: u8 = 3;
const FT_FILE: u8 = 4;

// Socket syscall ids (SocketFd.java).
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
const SOCK_GETADDRINFO: i32 = 11;
const SOCK_GETSOCKNAME: i32 = 12;
const SOCK_GETPEERNAME: i32 = 13;
const SOCK_SHUTDOWN: i32 = 14;
// Kernel-hosted shell sessions for sshd (SocketFd.SESSION_*).
const SESSION_SPAWN: i32 = 20;
const SESSION_WRITE: i32 = 21;
const SESSION_READ: i32 = 22;
const SESSION_READ_BLOCKING: i32 = 23;
const SESSION_STATUS: i32 = 24;
const SESSION_CLOSE: i32 = 25;
const SESSION_RESIZE: i32 = 26;
/// Max payload per send/sendto request (fits the kernel's arg region).
const MAX_SEND: usize = 4096;

/// After this many clock reads without blocking, a child in virtual mode
/// is assumed to be spin-waiting for time and is put to sleep for 1 ms.
const SPIN_CLOCK_READS: u32 = 2000;

pub enum Fd {
    H(Handle),
    Preopen,
    Dir(PathBuf),
    Socket(i32),
}

pub struct ChildCtx {
    pub pid: i32,
    pub node: usize,
    pub actor: ActorId,
    pub killed: Arc<AtomicBool>,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub fds: BTreeMap<i32, Fd>,
    /// Pipe ends with FDFLAGS_NONBLOCK set (fd_fdstat_set_flags).
    nonblock: std::collections::HashSet<i32>,
    next_fd: i32,
    pub root: PathBuf,
    pub sched: Arc<Sched>,
    pub ipc: Arc<IpcBridge>,
    pub net: Arc<Mutex<Network>>,
    pub rng: Rng,
    clock_reads: u32,
    memory: Option<Memory>,
}

impl ChildCtx {
    fn alloc(&mut self, fd: Fd) -> i32 {
        let n = self.next_fd;
        self.next_fd += 1;
        self.fds.insert(n, fd);
        n
    }

    fn sleep_until(&mut self, deadline: i64) -> Result<()> {
        self.clock_reads = 0;
        let killed = self.killed.clone();
        self.sched.wait(self.actor, Some(deadline), || killed.load(Ordering::SeqCst));
        if self.killed.load(Ordering::SeqCst) {
            return Err(anyhow!(Killed));
        }
        Ok(())
    }

    fn now(&mut self) -> Result<i64> {
        self.clock_reads += 1;
        if self.sched.is_virtual() && self.clock_reads > SPIN_CLOCK_READS {
            let t = self.sched.now() + 1;
            self.sleep_until(t)?;
        }
        Ok(self.sched.now())
    }

    fn call(&mut self, syscall: i32, args: Vec<u8>) -> Result<IpcResult> {
        self.clock_reads = 0;
        self.ipc.call(self.pid, syscall, args, self.actor, &self.killed).map_err(|k| anyhow!(k))
    }
}

/// Everything a node provides to spawn children.
pub struct NodeEnv {
    pub node: usize,
    pub name: String,
    pub runtime: Arc<Runtime>,
    pub linker: Arc<Linker<ChildCtx>>,
    pub sched: Arc<Sched>,
    pub procs: Arc<ProcTable>,
    pub ipc: Arc<IpcBridge>,
    pub net: Arc<Mutex<Network>>,
    pub root: PathBuf,
    pub seed: u64,
    pub log: Arc<Mutex<Vec<String>>>,
}

impl NodeEnv {
    pub fn log(&self, msg: String) {
        let t = self.sched.elapsed();
        let line = format!("[{:>9.3}s {}] {}", t as f64 / 1000.0, self.name, msg);
        if std::env::var_os("ECM_SIM_VERBOSE").is_some() {
            eprintln!("{}", line);
        }
        let mut l = self.log.lock().unwrap_or_else(|e| e.into_inner());
        l.push(line);
        if l.len() > 500 {
            l.remove(0);
        }
    }
}

// ============================================================ memory helpers

fn mem_of(c: &Caller<'_, ChildCtx>) -> Option<Memory> {
    c.data().memory
}

fn rd(c: &Caller<'_, ChildCtx>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    let m = mem_of(c)?;
    let d = m.data(c);
    let p = ptr as u32 as usize;
    let l = len as u32 as usize;
    d.get(p..p.checked_add(l)?).map(|s| s.to_vec())
}

fn rd_str(c: &Caller<'_, ChildCtx>, ptr: i32, len: i32) -> Option<String> {
    rd(c, ptr, len).map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn wr(c: &mut Caller<'_, ChildCtx>, ptr: i32, data: &[u8]) -> bool {
    let Some(m) = mem_of(c) else { return false };
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

fn wr_u32(c: &mut Caller<'_, ChildCtx>, ptr: i32, v: u32) -> bool {
    wr(c, ptr, &v.to_le_bytes())
}
fn wr_u64(c: &mut Caller<'_, ChildCtx>, ptr: i32, v: u64) -> bool {
    wr(c, ptr, &v.to_le_bytes())
}
fn rd_u32(c: &Caller<'_, ChildCtx>, ptr: i32) -> Option<u32> {
    rd(c, ptr, 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}
fn rd_u64(c: &Caller<'_, ChildCtx>, ptr: i32) -> Option<u64> {
    rd(c, ptr, 8).map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
}

fn i32le(v: i32) -> [u8; 4] {
    v.to_le_bytes()
}

/// Child paths: relative to storage, no traversal, no leading '/'
/// (`WasiFunctions.resolveChildPath`).
fn child_path(root: &std::path::Path, p: &str) -> Option<PathBuf> {
    if p.contains("..") || p.starts_with('/') {
        return None;
    }
    safe_join(root, p)
}

// ============================================================ spawn / run

/// Spawn a child. `stdio`: kernel-provided handles for fds 0/1/2 (None =
/// the terminal). Returns the pid, or -1.
pub fn spawn(env: &Arc<NodeEnv>, path: PathBuf, argv: Vec<String>, envv: Vec<(String, String)>, stdio: [Option<Handle>; 3]) -> i32 {
    let pid = env.procs.alloc_pid();
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "?".into());
    let procs = env.procs.clone();
    let out = Pipe::new(STDOUT_PIPE_CAP, env.sched.clone(), Some(Box::new(move || procs.mark_activity())));
    let inp = Pipe::new(STDIN_PIPE_CAP, env.sched.clone(), None);
    let actor = env.sched.register(&format!("{}:{}({})", env.name, name, pid));
    let killed = Arc::new(AtomicBool::new(false));

    let [sin, sout, serr] = stdio;
    let mut fds = BTreeMap::new();
    fds.insert(0, Fd::H(sin.map(|h| h.dup()).unwrap_or_else(|| Handle::pipe_r(&inp))));
    fds.insert(1, Fd::H(sout.map(|h| h.dup()).unwrap_or_else(|| Handle::pipe_w(&out))));
    fds.insert(2, Fd::H(serr.map(|h| h.dup()).unwrap_or_else(|| Handle::pipe_w(&out))));
    fds.insert(3, Fd::Preopen);

    env.procs.insert(
        pid,
        ProcEntry {
            name: name.clone(),
            zombie: false,
            exit_code: None,
            exit_reported: false,
            output: out.clone(),
            output_reader: Some(PipeReader::new(&out)),
            input: Some(PipeWriter::new(&inp)),
            input_pipe: inp.clone(),
            killed: killed.clone(),
            actor,
        },
    );

    let ctx = ChildCtx {
        pid,
        node: env.node,
        actor,
        killed: killed.clone(),
        argv,
        env: envv,
        fds,
        nonblock: Default::default(),
        next_fd: 4,
        root: env.root.clone(),
        sched: env.sched.clone(),
        ipc: env.ipc.clone(),
        net: env.net.clone(),
        rng: Rng::derive(env.seed, &format!("{}:pid{}", env.name, pid)),
        clock_reads: 0,
        memory: None,
    };
    let env2 = env.clone();
    let res = std::thread::Builder::new()
        .name(format!("{}-pid{}", env.name, pid))
        .stack_size(8 << 20)
        .spawn(move || {
            let code = run(&env2, ctx, &path, &name, &out);
            env2.procs.set_exited(pid, code);
            env2.procs.mark_activity();
            env2.sched.exit(actor);
        });
    if res.is_err() {
        env.procs.set_exited(pid, 1);
        env.sched.exit(actor);
    }
    pid
}

fn say(out: &Arc<Pipe>, msg: &str) {
    let w = PipeWriter::new(out);
    let _ = w.0.try_write(msg.as_bytes());
}

fn run(env: &Arc<NodeEnv>, ctx: ChildCtx, path: &std::path::Path, name: &str, out: &Arc<Pipe>) -> i32 {
    let pid = ctx.pid;
    let module = match env.runtime.module(path) {
        Ok(m) => m,
        Err(e) => {
            env.log(format!("pid {} ({}): cannot load: {:#}", pid, name, e));
            say(out, &format!("[sim] cannot load {}: {:#}\n", name, e));
            return 126;
        }
    };
    let mut store = Store::new(&env.runtime.engine, ctx);
    store.set_epoch_deadline(1);
    store.epoch_deadline_callback(|c| {
        if c.data().killed.load(Ordering::SeqCst) {
            Err(anyhow!(Killed))
        } else {
            Ok(UpdateDeadline::Continue(1))
        }
    });
    let provided = provided_sigs(&env.linker, &mut store);
    if let Err(problems) = check_imports(&module, &provided) {
        env.log(format!("pid {} ({}): host ABI mismatch: {}", pid, name, problems));
        say(out, &format!("[sim] {}: unsupported host imports: {}\n", name, problems));
        return 127;
    }
    let code = match env.linker.instantiate(&mut store, &module) {
        Err(e) => {
            env.log(format!("pid {} ({}): instantiate failed: {:#}", pid, name, e));
            say(out, &format!("[sim] {}: instantiate failed: {:#}\n", name, e));
            127
        }
        Ok(inst) => {
            store.data_mut().memory = inst.get_memory(&mut store, "memory");
            let entry = inst
                .get_typed_func::<(), ()>(&mut store, "_start")
                .or_else(|_| inst.get_typed_func::<(), ()>(&mut store, "main"));
            match entry {
                Err(_) => 127,
                Ok(f) => match f.call(&mut store, ()) {
                    Ok(()) => 0,
                    Err(e) => exit_code_of(env, pid, name, &e, store.data().killed.load(Ordering::SeqCst)),
                },
            }
        }
    };
    // FdTable.closeAll(): sockets get a graceful SOCK_CLOSE unless killed.
    let ctx = store.data_mut();
    let fds = std::mem::take(&mut ctx.fds);
    if !ctx.killed.load(Ordering::SeqCst) {
        for fd in fds.values() {
            if let Fd::Socket(id) = fd {
                let _ = ctx.call(SOCK_CLOSE, i32le(*id).to_vec());
            }
        }
    }
    drop(fds);
    code
}

fn exit_code_of(env: &NodeEnv, pid: i32, name: &str, e: &anyhow::Error, killed: bool) -> i32 {
    for cause in e.chain() {
        if let Some(ProcExit(c)) = cause.downcast_ref::<ProcExit>() {
            return *c;
        }
        if cause.downcast_ref::<Killed>().is_some() {
            return 130;
        }
    }
    if killed {
        return 130;
    }
    env.log(format!("pid {} ({}) crashed: {:#}", pid, name, e));
    1
}

pub fn provided_sigs<T>(linker: &Linker<T>, store: &mut Store<T>) -> HashMap<String, String> {
    let mut m = HashMap::new();
    let items: Vec<(String, String, Extern)> =
        linker.iter(&mut *store).map(|(a, b, e)| (a.to_string(), b.to_string(), e)).collect();
    for (module, name, ext) in items {
        if let Some(f) = ext.into_func() {
            let ty = f.ty(&*store);
            m.insert(format!("{}::{}", module, name), sig_string(ty.params(), ty.results()));
        }
    }
    m
}

// ============================================================ linker

type C<'a> = Caller<'a, ChildCtx>;

pub fn build_linker(rt: &Runtime) -> Result<Linker<ChildCtx>> {
    let mut l: Linker<ChildCtx> = Linker::new(&rt.engine);
    register_wasi(&mut l)?;
    register_env(&mut l)?;
    register_sockets(&mut l)?;
    register_sessions(&mut l)?;
    Ok(l)
}

const W: &str = "wasi_snapshot_preview1";
const E: &str = "env";

fn fd_write_impl(c: &mut C<'_>, fd: i32, data: &[u8]) -> Result<Result<usize, i32>> {
    let (actor, killed) = (c.data().actor, c.data().killed.clone());
    let h = match c.data().fds.get(&fd) {
        Some(Fd::H(h)) => h.clone(),
        Some(Fd::Socket(id)) => {
            let id = *id;
            let n = data.len().min(MAX_SEND);
            let mut a = Vec::with_capacity(6 + n);
            a.extend_from_slice(&i32le(id));
            a.extend_from_slice(&(n as u16).to_le_bytes());
            a.extend_from_slice(&data[..n]);
            let r = c.data_mut().call(SOCK_SEND, a)?;
            return Ok(if r.status < 0 { Err(EBADF) } else { Ok(r.status as usize) });
        }
        _ => return Ok(Err(EBADF)),
    };
    c.data_mut().clock_reads = 0;
    match h {
        Handle::PipeW(w) => {
            let n = w.0.write(actor, &killed, data).map_err(|k| anyhow!(k))?;
            Ok(if n < 0 { Err(EBADF) } else { Ok(n as usize) })
        }
        Handle::File(f) => {
            let r = f.lock().unwrap_or_else(|e| e.into_inner()).write(data);
            Ok(r.map_err(|_| EIO))
        }
        Handle::PipeR(_) => Ok(Err(EBADF)),
    }
}

fn fd_read_impl(c: &mut C<'_>, fd: i32, max: usize) -> Result<Result<Vec<u8>, i32>> {
    let (actor, killed) = (c.data().actor, c.data().killed.clone());
    let h = match c.data().fds.get(&fd) {
        Some(Fd::H(h)) => h.clone(),
        Some(Fd::Socket(id)) => {
            let mut a = Vec::with_capacity(12);
            a.extend_from_slice(&i32le(*id));
            a.extend_from_slice(&i32le(max as i32));
            a.extend_from_slice(&i32le(0));
            let r = c.data_mut().call(SOCK_RECV, a)?;
            if r.status < 0 {
                return Ok(Ok(Vec::new()));
            }
            let n = (r.status as usize).min(r.payload.len()).min(max);
            return Ok(Ok(r.payload[..n].to_vec()));
        }
        _ => return Ok(Err(EBADF)),
    };
    let mut buf = vec![0u8; max];
    if let Handle::PipeR(r) = &h {
        if c.data().nonblock.contains(&fd) {
            // PipeFd.read with O_NONBLOCK: what is there, EAGAIN if empty
            // and still open, 0 at EOF.
            let n = r.0.try_read(&mut buf);
            if n == 0 && !r.0.readable() {
                // Treat an EAGAIN poll loop like a clock spin so virtual
                // time can still advance.
                c.data_mut().now()?;
                return Ok(Err(EAGAIN));
            }
            buf.truncate(n);
            return Ok(Ok(buf));
        }
    }
    c.data_mut().clock_reads = 0;
    match h {
        Handle::PipeR(r) => {
            let n = r.0.read(actor, &killed, &mut buf).map_err(|k| anyhow!(k))?;
            buf.truncate(n);
            Ok(Ok(buf))
        }
        Handle::File(f) => match f.lock().unwrap_or_else(|e| e.into_inner()).read(&mut buf) {
            Ok(n) => {
                buf.truncate(n);
                Ok(Ok(buf))
            }
            Err(_) => Ok(Err(EIO)),
        },
        Handle::PipeW(_) => Ok(Err(EBADF)),
    }
}

fn iovecs(c: &C<'_>, iovs: i32, n: i32) -> Option<Vec<(i32, i32)>> {
    let mut v = Vec::new();
    for i in 0..n.max(0) {
        let a = iovs.checked_add(i.checked_mul(8)?)?;
        let p = rd_u32(c, a)? as i32;
        let l = rd_u32(c, a + 4)? as i32;
        v.push((p, l));
    }
    Some(v)
}

fn filestat(c: &mut C<'_>, buf: i32, ft: u8, size: u64) -> i32 {
    let mut b = [0u8; 64];
    b[16] = ft;
    b[24..32].copy_from_slice(&1u64.to_le_bytes());
    b[32..40].copy_from_slice(&size.to_le_bytes());
    if wr(c, buf, &b) {
        ESUCCESS
    } else {
        EINVAL
    }
}

fn base_dir(c: &C<'_>, dirfd: i32) -> PathBuf {
    match c.data().fds.get(&dirfd) {
        Some(Fd::Dir(p)) => p.clone(),
        _ => c.data().root.clone(),
    }
}

fn wasi_path(c: &C<'_>, dirfd: i32, ptr: i32, len: i32) -> Option<PathBuf> {
    let s = rd_str(c, ptr, len)?;
    let base = base_dir(c, dirfd);
    let root = c.data().root.clone();
    let p = child_path(&base, &s)?;
    if !p.starts_with(&root) {
        return None;
    }
    Some(p)
}

fn register_wasi(l: &mut Linker<ChildCtx>) -> Result<()> {
    l.func_wrap(W, "fd_write", |mut c: C<'_>, fd: i32, iovs: i32, n: i32, nw: i32| -> Result<i32> {
        let Some(v) = iovecs(&c, iovs, n) else { return Ok(EINVAL) };
        let mut total = 0usize;
        for (p, len) in v {
            if len <= 0 {
                continue;
            }
            let Some(data) = rd(&c, p, len) else { return Ok(EINVAL) };
            match fd_write_impl(&mut c, fd, &data)? {
                Ok(k) => {
                    total += k;
                    if k < data.len() {
                        break;
                    }
                }
                Err(e) => return Ok(e),
            }
        }
        wr_u32(&mut c, nw, total as u32);
        Ok(ESUCCESS)
    })?;
    l.func_wrap(W, "fd_read", |mut c: C<'_>, fd: i32, iovs: i32, n: i32, nr: i32| -> Result<i32> {
        let Some(v) = iovecs(&c, iovs, n) else { return Ok(EINVAL) };
        let mut total = 0usize;
        for (p, len) in v {
            if len <= 0 {
                continue;
            }
            match fd_read_impl(&mut c, fd, len as usize)? {
                Ok(data) => {
                    if data.is_empty() {
                        break;
                    }
                    if !wr(&mut c, p, &data) {
                        return Ok(EINVAL);
                    }
                    total += data.len();
                    if data.len() < len as usize {
                        break;
                    }
                }
                Err(EAGAIN) if total > 0 => break,
                Err(e) => return Ok(e),
            }
        }
        wr_u32(&mut c, nr, total as u32);
        Ok(ESUCCESS)
    })?;
    l.func_wrap(W, "fd_pread", |_c: C<'_>, _fd: i32, _iovs: i32, _n: i32, _off: i64, _nr: i32| -> i32 { ENOSYS })?;
    l.func_wrap(W, "fd_pwrite", |_c: C<'_>, _fd: i32, _iovs: i32, _n: i32, _off: i64, _nw: i32| -> i32 { ENOSYS })?;
    l.func_wrap(W, "fd_close", |mut c: C<'_>, fd: i32| -> Result<i32> {
        match c.data_mut().fds.remove(&fd) {
            Some(Fd::Socket(id)) => {
                c.data_mut().call(SOCK_CLOSE, i32le(id).to_vec())?;
                Ok(ESUCCESS)
            }
            Some(_) => Ok(ESUCCESS),
            // Java closes unknown fds silently.
            None => Ok(ESUCCESS),
        }
    })?;
    l.func_wrap(W, "fd_seek", |mut c: C<'_>, fd: i32, off: i64, whence: i32, out: i32| -> i32 {
        let Some(Fd::H(Handle::File(f))) = c.data().fds.get(&fd) else { return ENOSYS };
        let f = f.clone();
        let pos = match whence {
            0 => SeekFrom::Start(off.max(0) as u64),
            1 => SeekFrom::Current(off),
            2 => SeekFrom::End(off),
            _ => return EINVAL,
        };
        let r = f.lock().unwrap_or_else(|e| e.into_inner()).seek(pos);
        match r {
            Ok(p) => {
                wr_u64(&mut c, out, p);
                ESUCCESS
            }
            Err(_) => EINVAL,
        }
    })?;
    l.func_wrap(W, "fd_tell", |mut c: C<'_>, fd: i32, out: i32| -> i32 {
        let Some(Fd::H(Handle::File(f))) = c.data().fds.get(&fd) else { return ENOSYS };
        let f = f.clone();
        let r = f.lock().unwrap_or_else(|e| e.into_inner()).seek(SeekFrom::Current(0));
        match r {
            Ok(p) => {
                wr_u64(&mut c, out, p);
                ESUCCESS
            }
            Err(_) => EINVAL,
        }
    })?;
    l.func_wrap(W, "fd_fdstat_get", |mut c: C<'_>, fd: i32, buf: i32| -> i32 {
        let ft = match c.data().fds.get(&fd) {
            None => return EBADF,
            Some(_) if fd <= 2 => FT_CHAR,
            Some(Fd::Preopen) | Some(Fd::Dir(_)) => FT_DIR,
            Some(Fd::Socket(_)) => 6, // SOCKET_STREAM
            Some(Fd::H(Handle::File(_))) => FT_FILE,
            Some(Fd::H(_)) => 0, // pipe: unknown
        };
        let mut b = [0u8; 24];
        b[0] = ft;
        if c.data().nonblock.contains(&fd) {
            b[2..4].copy_from_slice(&(FDFLAGS_NONBLOCK as u16).to_le_bytes());
        }
        b[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        b[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        if wr(&mut c, buf, &b) {
            ESUCCESS
        } else {
            EINVAL
        }
    })?;
    // Only O_NONBLOCK on pipe ends has an effect (PipeFd.setNonBlocking).
    l.func_wrap(W, "fd_fdstat_set_flags", |mut c: C<'_>, fd: i32, fl: i32| -> i32 {
        let is_pipe = matches!(c.data().fds.get(&fd), Some(Fd::H(Handle::PipeR(_))) | Some(Fd::H(Handle::PipeW(_))));
        if is_pipe {
            if fl & FDFLAGS_NONBLOCK != 0 {
                c.data_mut().nonblock.insert(fd);
            } else {
                c.data_mut().nonblock.remove(&fd);
            }
        }
        ESUCCESS
    })?;
    l.func_wrap(W, "fd_fdstat_set_rights", |_c: C<'_>, _fd: i32, _a: i64, _b: i64| -> i32 { ESUCCESS })?;
    l.func_wrap(W, "fd_sync", |_c: C<'_>, _fd: i32| -> i32 { ESUCCESS })?;
    l.func_wrap(W, "fd_datasync", |_c: C<'_>, _fd: i32| -> i32 { ESUCCESS })?;
    l.func_wrap(W, "fd_advise", |_c: C<'_>, _fd: i32, _o: i64, _l: i64, _a: i32| -> i32 { ESUCCESS })?;
    l.func_wrap(W, "fd_allocate", |_c: C<'_>, _fd: i32, _o: i64, _l: i64| -> i32 { ENOSYS })?;
    l.func_wrap(W, "fd_filestat_get", |mut c: C<'_>, fd: i32, buf: i32| -> i32 {
        let (ft, size) = match c.data().fds.get(&fd) {
            None => return EBADF,
            Some(Fd::H(Handle::File(f))) => (FT_FILE, f.lock().unwrap_or_else(|e| e.into_inner()).size()),
            Some(Fd::Preopen) | Some(Fd::Dir(_)) => (FT_DIR, 0),
            Some(_) => (FT_CHAR, 0),
        };
        filestat(&mut c, buf, ft, size)
    })?;
    l.func_wrap(W, "fd_filestat_set_size", |c: C<'_>, fd: i32, size: i64| -> i32 {
        let Some(Fd::H(Handle::File(f))) = c.data().fds.get(&fd) else { return EBADF };
        match f.lock().unwrap_or_else(|e| e.into_inner()).file.set_len(size.max(0) as u64) {
            Ok(()) => ESUCCESS,
            Err(_) => EIO,
        }
    })?;
    l.func_wrap(W, "fd_filestat_set_times", |_c: C<'_>, _fd: i32, _a: i64, _m: i64, _f: i32| -> i32 { ESUCCESS })?;
    l.func_wrap(W, "fd_prestat_get", |mut c: C<'_>, fd: i32, buf: i32| -> i32 {
        if fd == 3 {
            let mut b = [0u8; 8];
            b[4..8].copy_from_slice(&1u32.to_le_bytes());
            wr(&mut c, buf, &b);
            ESUCCESS
        } else {
            EBADF
        }
    })?;
    l.func_wrap(W, "fd_prestat_dir_name", |mut c: C<'_>, fd: i32, buf: i32, _len: i32| -> i32 {
        if fd == 3 {
            wr(&mut c, buf, b"/");
            ESUCCESS
        } else {
            EBADF
        }
    })?;
    l.func_wrap(W, "fd_renumber", |mut c: C<'_>, from: i32, to: i32| -> i32 {
        match c.data_mut().fds.remove(&from) {
            Some(f) => {
                c.data_mut().fds.insert(to, f);
                ESUCCESS
            }
            None => EBADF,
        }
    })?;
    l.func_wrap(W, "fd_readdir", |mut c: C<'_>, fd: i32, buf: i32, len: i32, cookie: i64, used: i32| -> i32 {
        let dir = match c.data().fds.get(&fd) {
            Some(Fd::Dir(p)) => p.clone(),
            Some(Fd::Preopen) => c.data().root.clone(),
            _ => return EBADF,
        };
        let mut names: Vec<(String, u8)> = match std::fs::read_dir(&dir) {
            Ok(rd) => rd
                .flatten()
                .map(|e| {
                    let ft = if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { FT_DIR } else { FT_FILE };
                    (e.file_name().to_string_lossy().into_owned(), ft)
                })
                .collect(),
            Err(_) => return EIO,
        };
        names.sort();
        let mut out = Vec::new();
        for (i, (name, ft)) in names.iter().enumerate().skip(cookie.max(0) as usize) {
            let mut e = Vec::with_capacity(24 + name.len());
            e.extend_from_slice(&((i + 1) as u64).to_le_bytes());
            e.extend_from_slice(&((i + 1) as u64).to_le_bytes());
            e.extend_from_slice(&(name.len() as u32).to_le_bytes());
            e.push(*ft);
            e.extend_from_slice(&[0, 0, 0]);
            e.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&e);
            if out.len() >= len.max(0) as usize {
                break;
            }
        }
        out.truncate(len.max(0) as usize);
        wr(&mut c, buf, &out);
        wr_u32(&mut c, used, out.len() as u32);
        ESUCCESS
    })?;
    l.func_wrap(W, "proc_exit", |_c: C<'_>, code: i32| -> Result<()> { Err(anyhow!(ProcExit(code))) })?;
    l.func_wrap(W, "proc_raise", |_c: C<'_>, _sig: i32| -> i32 { ENOSYS })?;
    l.func_wrap(W, "args_sizes_get", |mut c: C<'_>, argc: i32, size: i32| -> i32 {
        let n = c.data().argv.len() as u32;
        let s: usize = c.data().argv.iter().map(|a| a.len() + 1).sum();
        wr_u32(&mut c, argc, n);
        wr_u32(&mut c, size, s as u32);
        ESUCCESS
    })?;
    l.func_wrap(W, "args_get", |mut c: C<'_>, argv: i32, buf: i32| -> i32 {
        let args = c.data().argv.clone();
        let mut p = buf;
        for (i, a) in args.iter().enumerate() {
            wr_u32(&mut c, argv + i as i32 * 4, p as u32);
            let mut b = a.as_bytes().to_vec();
            b.push(0);
            wr(&mut c, p, &b);
            p += b.len() as i32;
        }
        ESUCCESS
    })?;
    l.func_wrap(W, "environ_sizes_get", |mut c: C<'_>, cnt: i32, size: i32| -> i32 {
        let n = c.data().env.len() as u32;
        let s: usize = c.data().env.iter().map(|(k, v)| k.len() + v.len() + 2).sum();
        wr_u32(&mut c, cnt, n);
        wr_u32(&mut c, size, s as u32);
        ESUCCESS
    })?;
    l.func_wrap(W, "environ_get", |mut c: C<'_>, envp: i32, buf: i32| -> i32 {
        let env = c.data().env.clone();
        let mut p = buf;
        for (i, (k, v)) in env.iter().enumerate() {
            wr_u32(&mut c, envp + i as i32 * 4, p as u32);
            let b = format!("{}={}\0", k, v).into_bytes();
            wr(&mut c, p, &b);
            p += b.len() as i32;
        }
        ESUCCESS
    })?;
    l.func_wrap(W, "clock_res_get", |mut c: C<'_>, _id: i32, out: i32| -> i32 {
        wr_u64(&mut c, out, 1_000_000);
        ESUCCESS
    })?;
    l.func_wrap(W, "clock_time_get", |mut c: C<'_>, _id: i32, _prec: i64, out: i32| -> Result<i32> {
        let now = c.data_mut().now()?;
        wr_u64(&mut c, out, (now as u64).wrapping_mul(1_000_000));
        Ok(ESUCCESS)
    })?;
    l.func_wrap(W, "random_get", |mut c: C<'_>, buf: i32, len: i32| -> i32 {
        let mut b = vec![0u8; len.max(0) as usize];
        c.data_mut().rng.fill(&mut b);
        if wr(&mut c, buf, &b) {
            ESUCCESS
        } else {
            EINVAL
        }
    })?;
    l.func_wrap(W, "sched_yield", |mut c: C<'_>| -> Result<i32> {
        let _ = c.data_mut().now()?;
        std::thread::yield_now();
        Ok(ESUCCESS)
    })?;
    l.func_wrap(
        W,
        "path_open",
        |mut c: C<'_>, dirfd: i32, _lookup: i32, pp: i32, pl: i32, oflags: i32, _rb: i64, _ri: i64, fdflags: i32, out: i32| -> i32 {
            let Some(path) = wasi_path(&c, dirfd, pp, pl) else { return ENOENT };
            let create = oflags & 1 != 0;
            let directory = oflags & 2 != 0;
            let excl = oflags & 4 != 0;
            let trunc = oflags & 8 != 0;
            let append = fdflags & 1 != 0;
            if path.is_dir() {
                let fd = c.data_mut().alloc(Fd::Dir(path));
                wr_u32(&mut c, out, fd as u32);
                return ESUCCESS;
            }
            if directory {
                return if path.exists() { ENOTDIR } else { ENOENT };
            }
            if path.exists() && create && excl {
                return EEXIST;
            }
            if !path.exists() && !create {
                return ENOENT;
            }
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(create)
                .truncate(trunc)
                .open(&path)
                .or_else(|_| std::fs::OpenOptions::new().read(true).open(&path));
            match f {
                Ok(file) => {
                    let h = Handle::File(Arc::new(Mutex::new(FileHandle { file, path, append })));
                    let fd = c.data_mut().alloc(Fd::H(h));
                    wr_u32(&mut c, out, fd as u32);
                    ESUCCESS
                }
                Err(_) => ENOENT,
            }
        },
    )?;
    l.func_wrap(W, "path_create_directory", |c: C<'_>, dirfd: i32, pp: i32, pl: i32| -> i32 {
        let Some(p) = wasi_path(&c, dirfd, pp, pl) else { return ENOENT };
        if p.exists() {
            return EEXIST;
        }
        match std::fs::create_dir_all(p) {
            Ok(()) => ESUCCESS,
            Err(_) => ENOENT,
        }
    })?;
    l.func_wrap(W, "path_remove_directory", |c: C<'_>, dirfd: i32, pp: i32, pl: i32| -> i32 {
        let Some(p) = wasi_path(&c, dirfd, pp, pl) else { return ENOENT };
        if !p.is_dir() {
            return ENOTDIR;
        }
        match std::fs::remove_dir(p) {
            Ok(()) => ESUCCESS,
            Err(_) => ENOTEMPTY,
        }
    })?;
    l.func_wrap(W, "path_unlink_file", |c: C<'_>, dirfd: i32, pp: i32, pl: i32| -> i32 {
        let Some(p) = wasi_path(&c, dirfd, pp, pl) else { return ENOENT };
        if p.is_dir() {
            return EISDIR;
        }
        match std::fs::remove_file(p) {
            Ok(()) => ESUCCESS,
            Err(_) => ENOENT,
        }
    })?;
    l.func_wrap(W, "path_rename", |c: C<'_>, fd1: i32, p1: i32, l1: i32, fd2: i32, p2: i32, l2: i32| -> i32 {
        let (Some(a), Some(b)) = (wasi_path(&c, fd1, p1, l1), wasi_path(&c, fd2, p2, l2)) else { return ENOENT };
        match std::fs::rename(a, b) {
            Ok(()) => ESUCCESS,
            Err(_) => ENOENT,
        }
    })?;
    l.func_wrap(W, "path_filestat_get", |mut c: C<'_>, dirfd: i32, _fl: i32, pp: i32, pl: i32, buf: i32| -> i32 {
        let Some(p) = wasi_path(&c, dirfd, pp, pl) else { return ENOENT };
        match std::fs::metadata(&p) {
            Ok(m) => filestat(&mut c, buf, if m.is_dir() { FT_DIR } else { FT_FILE }, m.len()),
            Err(_) => ENOENT,
        }
    })?;
    l.func_wrap(W, "path_filestat_set_times", |_c: C<'_>, _fd: i32, _f: i32, _p: i32, _l: i32, _a: i64, _m: i64, _fl: i32| -> i32 {
        ESUCCESS
    })?;
    l.func_wrap(W, "path_readlink", |_c: C<'_>, _fd: i32, _p: i32, _l: i32, _b: i32, _bl: i32, _u: i32| -> i32 { EINVAL })?;
    l.func_wrap(W, "path_symlink", |_c: C<'_>, _p: i32, _l: i32, _fd: i32, _p2: i32, _l2: i32| -> i32 { ENOSYS })?;
    l.func_wrap(W, "path_link", |_c: C<'_>, _a: i32, _b: i32, _p: i32, _l: i32, _fd: i32, _p2: i32, _l2: i32| -> i32 { ENOSYS })?;
    l.func_wrap(W, "poll_oneoff", poll_oneoff)?;
    Ok(())
}

/// Clock subscriptions sleep on the simulation clock; fd_read
/// subscriptions on pipes complete when data (or EOF) is there; other fds
/// are always ready. Superset of the Java host (which only honours clocks).
fn poll_oneoff(mut c: C<'_>, inp: i32, outp: i32, nsubs: i32, nevents: i32) -> Result<i32> {
    if nsubs <= 0 {
        return Ok(EINVAL);
    }
    let now = c.data().sched.now();
    let mut clock: Option<(i64, u64)> = None; // (deadline, userdata)
    let mut fdsubs: Vec<(u64, u8, i32)> = Vec::new();
    for i in 0..nsubs {
        let s = inp + i * 48;
        let (Some(ud), Some(tagb)) = (rd_u64(&c, s), rd(&c, s + 8, 1)) else { return Ok(EINVAL) };
        match tagb[0] {
            0 => {
                let timeout = rd_u64(&c, s + 24).unwrap_or(0);
                let flags = rd(&c, s + 40, 2).map(|b| u16::from_le_bytes([b[0], b[1]])).unwrap_or(0);
                let dl = if flags & 1 != 0 {
                    ((timeout / 1_000_000) as i64).max(now)
                } else {
                    now + (timeout.div_ceil(1_000_000)) as i64
                };
                if clock.is_none_or(|(d, _)| dl < d) {
                    clock = Some((dl, ud));
                }
            }
            t @ (1 | 2) => {
                let fd = rd_u32(&c, s + 16).unwrap_or(u32::MAX) as i32;
                fdsubs.push((ud, t, fd));
            }
            _ => return Ok(EINVAL),
        }
    }
    let pipes: Vec<(usize, Arc<Pipe>)> = fdsubs
        .iter()
        .enumerate()
        .filter(|(_, (_, t, _))| *t == 1)
        .filter_map(|(k, (_, _, fd))| match c.data().fds.get(fd) {
            Some(Fd::H(Handle::PipeR(r))) => Some((k, r.0.clone())),
            _ => None,
        })
        .collect();
    let ready_now = |fdsubs: &Vec<(u64, u8, i32)>, pipes: &Vec<(usize, Arc<Pipe>)>| -> Vec<usize> {
        (0..fdsubs.len())
            .filter(|k| match pipes.iter().find(|(i, _)| i == k) {
                Some((_, p)) => p.readable(),
                None => true,
            })
            .collect()
    };
    let mut ready = ready_now(&fdsubs, &pipes);
    let mut clock_fired = false;
    if ready.is_empty() {
        let (sched, actor, killed) = (c.data().sched.clone(), c.data().actor, c.data().killed.clone());
        c.data_mut().clock_reads = 0;
        let r = sched.wait(actor, clock.map(|(d, _)| d), || {
            if killed.load(Ordering::SeqCst) {
                return true;
            }
            let mut any = false;
            for (_, p) in &pipes {
                if p.readable() {
                    any = true;
                } else {
                    p.register_waiter(actor);
                }
            }
            any
        });
        if killed.load(Ordering::SeqCst) {
            return Err(anyhow!(Killed));
        }
        ready = ready_now(&fdsubs, &pipes);
        clock_fired = matches!(r, Wake::Timeout) || (ready.is_empty() && clock.is_some());
    }
    let mut events: Vec<[u8; 32]> = Vec::new();
    if clock_fired {
        if let Some((_, ud)) = clock {
            let mut e = [0u8; 32];
            e[0..8].copy_from_slice(&ud.to_le_bytes());
            e[10] = 0;
            events.push(e);
        }
    }
    for k in ready {
        let (ud, t, _) = fdsubs[k];
        let mut e = [0u8; 32];
        e[0..8].copy_from_slice(&ud.to_le_bytes());
        e[10] = t;
        e[16..24].copy_from_slice(&1u64.to_le_bytes());
        events.push(e);
    }
    for (i, e) in events.iter().enumerate() {
        wr(&mut c, outp + i as i32 * 32, e);
    }
    wr_u32(&mut c, nevents, events.len() as u32);
    Ok(ESUCCESS)
}

fn register_env(l: &mut Linker<ChildCtx>) -> Result<()> {
    // --- files (ecm_host_abi::fs), resolved against the child's storage ---
    l.func_wrap(E, "file_read", |mut c: C<'_>, pp: i32, pl: i32, buf: i32, bl: i32| -> i32 {
        let Some(p) = rd_str(&c, pp, pl).and_then(|s| child_path(&c.data().root, &s)) else { return -1 };
        if !p.is_file() {
            return -1;
        }
        match std::fs::read(&p) {
            Ok(d) => {
                let n = d.len().min(bl.max(0) as usize);
                wr(&mut c, buf, &d[..n]);
                n as i32
            }
            Err(_) => -1,
        }
    })?;
    l.func_wrap(E, "file_write", |c: C<'_>, pp: i32, pl: i32, dp: i32, dl: i32| -> i32 {
        let Some(p) = rd_str(&c, pp, pl).and_then(|s| child_path(&c.data().root, &s)) else { return -1 };
        if dl < 0 || dl as usize > MAX_WRITE {
            return -1;
        }
        let Some(d) = rd(&c, dp, dl) else { return -1 };
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&p, d) {
            Ok(()) => dl,
            Err(_) => -1,
        }
    })?;
    l.func_wrap(E, "file_size", |c: C<'_>, pp: i32, pl: i32| -> i32 {
        let Some(p) = rd_str(&c, pp, pl).and_then(|s| child_path(&c.data().root, &s)) else { return -1 };
        std::fs::metadata(p).map(|m| m.len().min(i32::MAX as u64) as i32).unwrap_or(-1)
    })?;
    l.func_wrap(E, "file_exists", |c: C<'_>, pp: i32, pl: i32| -> i32 {
        rd_str(&c, pp, pl).and_then(|s| child_path(&c.data().root, &s)).is_some_and(|p| p.exists()) as i32
    })?;
    l.func_wrap(E, "file_delete", |c: C<'_>, pp: i32, pl: i32| -> i32 {
        let Some(p) = rd_str(&c, pp, pl).and_then(|s| child_path(&c.data().root, &s)) else { return 0 };
        let r = if p.is_dir() { std::fs::remove_dir(&p) } else { std::fs::remove_file(&p) };
        r.is_ok() as i32
    })?;
    l.func_wrap(E, "file_mkdir", |c: C<'_>, pp: i32, pl: i32| -> i32 {
        let Some(p) = rd_str(&c, pp, pl).and_then(|s| child_path(&c.data().root, &s)) else { return -1 };
        if std::fs::create_dir_all(p).is_ok() {
            0
        } else {
            -1
        }
    })?;
    l.func_wrap(E, "file_is_dir", |c: C<'_>, pp: i32, pl: i32| -> i32 {
        let Some(s) = rd_str(&c, pp, pl) else { return 0 };
        if s.is_empty() {
            return 1;
        }
        child_path(&c.data().root, &s).is_some_and(|p| p.is_dir()) as i32
    })?;
    l.func_wrap(E, "file_list", |mut c: C<'_>, buf: i32, bl: i32| -> i32 {
        let root = c.data().root.clone();
        let Ok(rd_) = std::fs::read_dir(&root) else { return 0 };
        let mut names: Vec<String> =
            rd_.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        let s = names.join("\n");
        let n = s.len().min(bl.max(0) as usize);
        wr(&mut c, buf, &s.as_bytes()[..n]);
        n as i32
    })?;
    l.func_wrap(E, "file_list_dir", |mut c: C<'_>, pp: i32, pl: i32, buf: i32, bl: i32| -> i32 {
        let Some(s) = rd_str(&c, pp, pl) else { return -1 };
        let dir = if s.is_empty() { Some(c.data().root.clone()) } else { child_path(&c.data().root, &s) };
        let Some(dir) = dir.filter(|d| d.is_dir()) else { return -1 };
        let Ok(rd_) = std::fs::read_dir(&dir) else { return -1 };
        let mut items: Vec<String> = rd_
            .flatten()
            .map(|e| format!("{}{}", if e.path().is_dir() { "d:" } else { "f:" }, e.file_name().to_string_lossy()))
            .collect();
        items.sort();
        let s = items.join("\n");
        let n = s.len().min(bl.max(0) as usize);
        wr(&mut c, buf, &s.as_bytes()[..n]);
        n as i32
    })?;

    // --- redstone: no world in the simulator ---
    l.func_wrap(E, "redstone_set_output", |_c: C<'_>, _s: i32, _p: i32| -> i32 { 0 })?;
    l.func_wrap(E, "redstone_get_input", |_c: C<'_>, _s: i32| -> i32 { 0 })?;
    l.func_wrap(E, "redstone_get_all_input", |mut c: C<'_>, buf: i32| -> i32 {
        wr(&mut c, buf, &[0u8; 24]);
        0
    })?;

    // --- time ---
    l.func_wrap(E, "sleep_ms", |mut c: C<'_>, ms: i32| -> Result<()> {
        let t = c.data().sched.now() + ms.max(0) as i64;
        c.data_mut().sleep_until(t)
    })?;
    l.func_wrap(E, "get_time_ms", |mut c: C<'_>| -> Result<i64> { c.data_mut().now() })?;
    l.func_wrap(E, "__getrandom_v03_custom", |mut c: C<'_>, buf: i32, len: i32| -> i32 {
        let mut b = vec![0u8; len.clamp(0, 1 << 20) as usize];
        c.data_mut().rng.fill(&mut b);
        if wr(&mut c, buf, &b) {
            0
        } else {
            -1
        }
    })?;

    // --- raw capture (tcpdump) on this computer's NICs ---
    l.func_wrap(E, "net_set_promiscuous_on", |c: C<'_>, idx: i32, on: i32| -> i32 {
        let node = c.data().node;
        let mut n = c.data().net.lock().unwrap_or_else(|e| e.into_inner());
        match n.nic(node, idx.max(0) as usize).filter(|_| idx >= 0) {
            Some(nic) => {
                n.set_promisc(nic, on != 0);
                0
            }
            None => -1,
        }
    })?;
    l.func_wrap(E, "net_pcap_enable", |c: C<'_>, idx: i32, on: i32| -> i32 {
        let node = c.data().node;
        let mut n = c.data().net.lock().unwrap_or_else(|e| e.into_inner());
        match n.nic(node, idx.max(0) as usize).filter(|_| idx >= 0) {
            Some(nic) => {
                n.set_pcap_mirror(nic, on != 0);
                0
            }
            None => -1,
        }
    })?;
    l.func_wrap(E, "net_pcap_rx", |mut c: C<'_>, idx: i32, buf: i32, bl: i32| -> i32 {
        if bl <= 0 || idx < 0 {
            return -1;
        }
        let node = c.data().node;
        let f = {
            let mut n = c.data().net.lock().unwrap_or_else(|e| e.into_inner());
            match n.nic(node, idx as usize) {
                Some(nic) => n.pcap_mirror_rx(nic),
                None => return -1,
            }
        };
        match f {
            Some(f) => {
                let k = f.len().min(bl as usize);
                wr(&mut c, buf, &f[..k]);
                k as i32
            }
            None => -1,
        }
    })?;

    // --- graphics / screen / mouse / video: no in-world screen ---
    l.func_wrap(E, "gfx_init", |_c: C<'_>, _t: i32, _w: i32, _h: i32| -> i32 { -1 })?;
    l.func_wrap(E, "gfx_set_mode", |_c: C<'_>, _t: i32, _m: i32| -> i32 { -1 })?;
    l.func_wrap(E, "gfx_blit_rect", |_c: C<'_>, _t: i32, _x: i32, _y: i32, _w: i32, _h: i32, _b: i32, _l: i32, _f: i32| -> i32 {
        -1
    })?;
    l.func_wrap(E, "screen_query_dims", |_c: C<'_>, _o: i32| -> i32 { -1 })?;
    l.func_wrap(E, "screen_set_power", |_c: C<'_>, _on: i32| {})?;
    l.func_wrap(E, "screen_set_pixel_format", |_c: C<'_>, _f: i32| {})?;
    l.func_wrap(E, "screen_put_frame_rgba", |_c: C<'_>, _p: i32, _w: i32, _h: i32| -> i32 { -1 })?;
    l.func_wrap(E, "mouse_capture_start", |_c: C<'_>| -> i32 { 0 })?;
    l.func_wrap(E, "mouse_capture_stop", |_c: C<'_>| {})?;
    l.func_wrap(E, "mouse_capture_is_active", |_c: C<'_>| -> i32 { 0 })?;
    l.func_wrap(E, "mouse_poll", |_c: C<'_>, _b: i32| -> i32 { 0 })?;
    l.func_wrap(E, "video_open", |_c: C<'_>, _p: i32, _l: i32, _w: i32, _h: i32, _f: i32| -> i32 { -1 })?;
    l.func_wrap(E, "video_get_info", |_c: C<'_>, _h: i32, _o: i32| -> i32 { -1 })?;
    l.func_wrap(E, "video_decode_to_gfx", |_c: C<'_>, _h: i32, _t: i32| -> i64 { -2 })?;
    l.func_wrap(E, "video_seek", |_c: C<'_>, _h: i32, _p: i64| -> i32 { -1 })?;
    l.func_wrap(E, "video_close", |_c: C<'_>, _h: i32| -> i32 { -1 })?;

    // --- auth: not provided by the Java host either ---
    l.func_wrap(E, "ipc_auth_set_password", |_c: C<'_>, _a: i32, _b: i32, _d: i32, _e: i32| -> i32 { -1 })?;
    Ok(())
}

fn sock_of(c: &C<'_>, fd: i32) -> Option<i32> {
    match c.data().fds.get(&fd) {
        Some(Fd::Socket(id)) => Some(*id),
        _ => None,
    }
}

fn write_name(c: &mut C<'_>, r: &IpcResult, addr: i32, alen: i32) -> i32 {
    if r.status < 0 || r.payload.len() < 16 {
        return -1;
    }
    wr(c, addr, &r.payload[..16]);
    if alen != 0 {
        wr_u32(c, alen, 16);
    }
    0
}

/// POSIX socket host functions (ecm_host_abi::socket), proxied to the
/// kernel exactly like `WasiFunctions.registerSocketFunctions`.
fn register_sockets(l: &mut Linker<ChildCtx>) -> Result<()> {
    l.func_wrap(E, "sock_socket", |mut c: C<'_>, d: i32, t: i32, p: i32| -> Result<i32> {
        let mut a = Vec::new();
        a.extend_from_slice(&i32le(d));
        a.extend_from_slice(&i32le(t));
        a.extend_from_slice(&i32le(p));
        let r = c.data_mut().call(SOCK_SOCKET, a)?;
        if r.status < 0 {
            return Ok(-1);
        }
        Ok(c.data_mut().alloc(Fd::Socket(r.status)))
    })?;
    for (name, sc) in [("sock_bind", SOCK_BIND), ("sock_connect", SOCK_CONNECT)] {
        l.func_wrap(E, name, move |mut c: C<'_>, fd: i32, ap: i32, al: i32| -> Result<i32> {
            let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
            let addr = rd(&c, ap, al.clamp(0, 16)).unwrap_or_default();
            let mut a = i32le(id).to_vec();
            a.extend_from_slice(&addr);
            Ok(c.data_mut().call(sc, a)?.status)
        })?;
    }
    l.func_wrap(E, "sock_listen", |mut c: C<'_>, fd: i32, backlog: i32| -> Result<i32> {
        let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&i32le(backlog));
        Ok(c.data_mut().call(SOCK_LISTEN, a)?.status)
    })?;
    l.func_wrap(E, "sock_accept", |mut c: C<'_>, fd: i32, ap: i32, alp: i32| -> Result<i32> {
        let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
        let r = c.data_mut().call(SOCK_ACCEPT, i32le(id).to_vec())?;
        if r.status < 0 {
            return Ok(r.status);
        }
        let nfd = c.data_mut().alloc(Fd::Socket(r.status));
        if ap != 0 && r.payload.len() >= 16 {
            wr(&mut c, ap, &r.payload[..16]);
            if alp != 0 {
                wr_u32(&mut c, alp, 16);
            }
        }
        Ok(nfd)
    })?;
    l.func_wrap(E, "sock_send", |mut c: C<'_>, fd: i32, bp: i32, bl: i32, _fl: i32| -> Result<i32> {
        let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
        let n = bl.clamp(0, MAX_SEND as i32);
        let data = rd(&c, bp, n).unwrap_or_default();
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&(data.len() as u16).to_le_bytes());
        a.extend_from_slice(&data);
        Ok(c.data_mut().call(SOCK_SEND, a)?.status)
    })?;
    l.func_wrap(E, "sock_recv", |mut c: C<'_>, fd: i32, bp: i32, bl: i32, fl: i32| -> Result<i32> {
        let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
        let bl = bl.max(0);
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&i32le(bl));
        a.extend_from_slice(&i32le(fl));
        let r = c.data_mut().call(SOCK_RECV, a)?;
        if r.status <= 0 {
            return Ok(r.status);
        }
        let n = (r.status as usize).min(r.payload.len()).min(bl as usize);
        wr(&mut c, bp, &r.payload[..n]);
        Ok(n as i32)
    })?;
    l.func_wrap(E, "sock_sendto", |mut c: C<'_>, fd: i32, bp: i32, bl: i32, _fl: i32, ap: i32, al: i32| -> Result<i32> {
        let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
        let data = rd(&c, bp, bl.clamp(0, MAX_SEND as i32)).unwrap_or_default();
        let addr = rd(&c, ap, al.clamp(0, 16)).unwrap_or_default();
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&(addr.len() as u16).to_le_bytes());
        a.extend_from_slice(&addr);
        a.extend_from_slice(&(data.len() as u16).to_le_bytes());
        a.extend_from_slice(&data);
        Ok(c.data_mut().call(SOCK_SENDTO, a)?.status)
    })?;
    l.func_wrap(E, "sock_recvfrom", |mut c: C<'_>, fd: i32, bp: i32, bl: i32, _fl: i32, ap: i32, alp: i32| -> Result<i32> {
        let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
        let bl = bl.max(0);
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&i32le(bl));
        a.extend_from_slice(&i32le(0));
        let r = c.data_mut().call(SOCK_RECVFROM, a)?;
        if r.status <= 0 {
            return Ok(r.status);
        }
        // payload = [sockaddr_in 16][data]
        if ap != 0 && r.payload.len() >= 16 {
            wr(&mut c, ap, &r.payload[..16]);
            if alp != 0 {
                wr_u32(&mut c, alp, 16);
            }
        }
        let n = (r.status as i64).min(r.payload.len() as i64 - 16).min(bl as i64);
        if n > 0 {
            let n = n as usize;
            wr(&mut c, bp, &r.payload[16..16 + n]);
            return Ok(n as i32);
        }
        Ok(0)
    })?;
    l.func_wrap(E, "sock_setsockopt", |mut c: C<'_>, fd: i32, lvl: i32, name: i32, vp: i32, vl: i32| -> Result<i32> {
        let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
        let v = rd(&c, vp, vl.clamp(0, 64)).unwrap_or_default();
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&i32le(lvl));
        a.extend_from_slice(&i32le(name));
        a.extend_from_slice(&v);
        Ok(c.data_mut().call(SOCK_SETSOCKOPT, a)?.status)
    })?;
    for (name, sc) in [("sock_getsockname", SOCK_GETSOCKNAME), ("sock_getpeername", SOCK_GETPEERNAME)] {
        l.func_wrap(E, name, move |mut c: C<'_>, fd: i32, ap: i32, alp: i32| -> Result<i32> {
            let id = sock_of(&c, fd).unwrap_or(-1);
            let r = c.data_mut().call(sc, i32le(id).to_vec())?;
            Ok(write_name(&mut c, &r, ap, alp))
        })?;
    }
    l.func_wrap(E, "sock_shutdown", |mut c: C<'_>, fd: i32, how: i32| -> Result<i32> {
        let Some(id) = sock_of(&c, fd) else { return Ok(-1) };
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&i32le(how));
        Ok(c.data_mut().call(SOCK_SHUTDOWN, a)?.status)
    })?;
    l.func_wrap(E, "sock_getaddrinfo", |mut c: C<'_>, hp: i32, hl: i32, rp: i32, rl: i32| -> Result<i32> {
        let host = rd(&c, hp, hl).unwrap_or_default();
        if host.len() > 255 {
            return Ok(-1);
        }
        let mut a = (host.len() as u16).to_le_bytes().to_vec();
        a.extend_from_slice(&host);
        let r = c.data_mut().call(SOCK_GETADDRINFO, a)?;
        if r.status < 0 || r.payload.len() < 16 {
            return Ok(-1);
        }
        let n = 16.min(rl.max(0) as usize);
        wr(&mut c, rp, &r.payload[..n]);
        Ok(n as i32)
    })?;
    Ok(())
}

fn session_read(c: &mut C<'_>, syscall: i32, id: i32, bp: i32, bl: i32, timeout: i32) -> Result<i32> {
    let mut a = i32le(id).to_vec();
    a.extend_from_slice(&i32le(bl.clamp(0, 4096)));
    a.extend_from_slice(&i32le(timeout.max(0)));
    let r = c.data_mut().call(syscall, a)?;
    if r.status <= 0 {
        return Ok(r.status);
    }
    let n = (r.status as usize).min(r.payload.len()).min(bl.max(0) as usize);
    wr(c, bp, &r.payload[..n]);
    Ok(n as i32)
}

/// Remote shell sessions for sshd, forwarded to the kernel exactly like
/// `WasiFunctions.registerShellSessionFunctions`.
fn register_sessions(l: &mut Linker<ChildCtx>) -> Result<()> {
    l.func_wrap(E, "ipc_spawn_shell", |mut c: C<'_>, up: i32, ul: i32| -> Result<i32> {
        let user = rd(&c, up, ul.clamp(0, 64)).unwrap_or_default();
        let mut a = (user.len() as u16).to_le_bytes().to_vec();
        a.extend_from_slice(&user);
        Ok(c.data_mut().call(SESSION_SPAWN, a)?.status)
    })?;
    l.func_wrap(E, "ipc_session_write", |mut c: C<'_>, id: i32, bp: i32, bl: i32| -> Result<i32> {
        let data = rd(&c, bp, bl.max(0)).unwrap_or_default();
        let mut written = 0usize;
        while written < data.len() {
            let n = (data.len() - written).min(4096);
            let mut a = i32le(id).to_vec();
            a.extend_from_slice(&(n as u16).to_le_bytes());
            a.extend_from_slice(&data[written..written + n]);
            let st = c.data_mut().call(SESSION_WRITE, a)?.status;
            if st < 0 {
                return Ok(if written > 0 { written as i32 } else { -1 });
            }
            written += n;
        }
        Ok(written as i32)
    })?;
    l.func_wrap(E, "ipc_session_read", |mut c: C<'_>, id: i32, bp: i32, bl: i32| -> Result<i32> {
        session_read(&mut c, SESSION_READ, id, bp, bl, 0)
    })?;
    l.func_wrap(E, "ipc_session_read_blocking", |mut c: C<'_>, id: i32, bp: i32, bl: i32, t: i32| -> Result<i32> {
        session_read(&mut c, SESSION_READ_BLOCKING, id, bp, bl, t)
    })?;
    l.func_wrap(E, "ipc_session_status", |mut c: C<'_>, id: i32| -> Result<i32> {
        Ok(c.data_mut().call(SESSION_STATUS, i32le(id).to_vec())?.status)
    })?;
    l.func_wrap(E, "ipc_session_close", |mut c: C<'_>, id: i32| -> Result<i32> {
        Ok(c.data_mut().call(SESSION_CLOSE, i32le(id).to_vec())?.status)
    })?;
    l.func_wrap(E, "ipc_session_resize", |mut c: C<'_>, id: i32, w: i32, h: i32| -> Result<i32> {
        let mut a = i32le(id).to_vec();
        a.extend_from_slice(&i32le(w));
        a.extend_from_slice(&i32le(h));
        Ok(c.data_mut().call(SESSION_RESIZE, a)?.status)
    })?;
    Ok(())
}

/// Signatures the child linker provides (for `--check-programs`).
pub fn provided_sigs_static(rt: &Runtime, linker: &Linker<ChildCtx>) -> HashMap<String, String> {
    let sched = Arc::new(Sched::new(crate::sched::ClockMode::Virtual));
    let ctx = ChildCtx {
        pid: 0,
        node: 0,
        actor: 0,
        killed: Arc::new(AtomicBool::new(false)),
        argv: Vec::new(),
        env: Vec::new(),
        fds: BTreeMap::new(),
        nonblock: Default::default(),
        next_fd: 4,
        root: PathBuf::new(),
        sched: sched.clone(),
        ipc: IpcBridge::new(sched),
        net: Arc::new(Mutex::new(Network::new(0))),
        rng: Rng::new(0),
        clock_reads: 0,
        memory: None,
    };
    let mut store = Store::new(&rt.engine, ctx);
    provided_sigs(linker, &mut store)
}
