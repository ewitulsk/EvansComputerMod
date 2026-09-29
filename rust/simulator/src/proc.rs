//! Pipes, file handles and the per-computer process table.
//!
//! Semantics follow `ProcessManager`/`WasiPipe` in the Java host: every
//! child gets a 16 KiB terminal-output pipe (stdout+stderr) and a 4 KiB
//! stdin pipe; the kernel reads/writes them without blocking
//! (`process_read_output` / `process_write_input`), the child blocks.
//! Blocking waits go through the scheduler so virtual time knows the child
//! is idle.

use std::collections::{BTreeMap, VecDeque};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::sched::{ActorId, Sched, Wake};

pub const STDOUT_PIPE_CAP: usize = 16 * 1024;
pub const STDIN_PIPE_CAP: usize = 4 * 1024;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A blocking wait was abandoned because the process was killed.
#[derive(Debug)]
pub struct Killed;

impl std::fmt::Display for Killed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "process killed")
    }
}
impl std::error::Error for Killed {}

// ============================================================ pipes

struct PipeInner {
    buf: VecDeque<u8>,
    writers: usize,
    readers: usize,
    waiters: Vec<ActorId>,
}

pub struct Pipe {
    inner: Mutex<PipeInner>,
    cap: usize,
    sched: Arc<Sched>,
    /// Fired after data is written or the write side closes (wakes the
    /// kernel loop for terminal-output pipes).
    on_activity: Option<Box<dyn Fn() + Send + Sync>>,
}

impl Pipe {
    pub fn new(cap: usize, sched: Arc<Sched>, on_activity: Option<Box<dyn Fn() + Send + Sync>>) -> Arc<Pipe> {
        Arc::new(Pipe {
            inner: Mutex::new(PipeInner { buf: VecDeque::new(), writers: 0, readers: 0, waiters: Vec::new() }),
            cap,
            sched,
            on_activity,
        })
    }

    fn wake(&self, g: &mut PipeInner) -> Vec<ActorId> {
        std::mem::take(&mut g.waiters)
    }

    fn finish(&self, woken: Vec<ActorId>, activity: bool) {
        self.sched.wake_all(&woken);
        if activity {
            if let Some(f) = &self.on_activity {
                f();
            }
        }
    }

    pub fn has_data(&self) -> bool {
        !lock(&self.inner).buf.is_empty()
    }

    /// Non-blocking read of at most `buf.len()` bytes, stopping after
    /// `max_newlines` newlines. Returns (bytes, newlines).
    pub fn try_read_lines(&self, buf: &mut [u8], max_newlines: usize) -> (usize, usize) {
        let (n, nl, woken) = {
            let mut g = lock(&self.inner);
            let mut n = 0;
            let mut nl = 0;
            while n < buf.len() && nl < max_newlines {
                let Some(b) = g.buf.pop_front() else { break };
                buf[n] = b;
                n += 1;
                if b == b'\n' {
                    nl += 1;
                }
            }
            let w = if n > 0 { self.wake(&mut g) } else { Vec::new() };
            (n, nl, w)
        };
        self.finish(woken, false);
        (n, nl)
    }

    pub fn try_read(&self, buf: &mut [u8]) -> usize {
        self.try_read_lines(buf, usize::MAX).0
    }

    /// Non-blocking write: as much as fits. -1 if no reader is left.
    pub fn try_write(&self, data: &[u8]) -> i32 {
        let (n, woken) = {
            let mut g = lock(&self.inner);
            if g.readers == 0 {
                return -1;
            }
            let n = data.len().min(self.cap.saturating_sub(g.buf.len()));
            g.buf.extend(&data[..n]);
            let w = if n > 0 { self.wake(&mut g) } else { Vec::new() };
            (n, w)
        };
        self.finish(woken, n > 0);
        n as i32
    }

    /// Blocking read (child side). Ok(0) = EOF.
    pub fn read(&self, actor: ActorId, killed: &AtomicBool, buf: &mut [u8]) -> Result<usize, Killed> {
        loop {
            let n = self.try_read(buf);
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
            let r = self.sched.wait(actor, None, || {
                if killed.load(Ordering::SeqCst) {
                    return true;
                }
                let mut g = lock(&self.inner);
                if !g.buf.is_empty() || g.writers == 0 {
                    return true;
                }
                if !g.waiters.contains(&actor) {
                    g.waiters.push(actor);
                }
                false
            });
            let _ = r;
            if killed.load(Ordering::SeqCst) {
                return Err(Killed);
            }
            let g = lock(&self.inner);
            if g.buf.is_empty() && g.writers == 0 {
                return Ok(0);
            }
        }
    }

    /// Blocking write of all of `data` (child side). -1 = broken pipe.
    pub fn write(&self, actor: ActorId, killed: &AtomicBool, data: &[u8]) -> Result<i32, Killed> {
        let mut off = 0;
        while off < data.len() {
            let n = self.try_write(&data[off..]);
            if n < 0 {
                return Ok(if off > 0 { off as i32 } else { -1 });
            }
            off += n as usize;
            if off >= data.len() {
                break;
            }
            self.sched.wait(actor, None, || {
                if killed.load(Ordering::SeqCst) {
                    return true;
                }
                let mut g = lock(&self.inner);
                if g.buf.len() < self.cap || g.readers == 0 {
                    return true;
                }
                if !g.waiters.contains(&actor) {
                    g.waiters.push(actor);
                }
                false
            });
            if killed.load(Ordering::SeqCst) {
                return Err(Killed);
            }
        }
        Ok(data.len() as i32)
    }

    /// Is a blocked reader going to make progress (data or EOF)?
    pub fn readable(&self) -> bool {
        let g = lock(&self.inner);
        !g.buf.is_empty() || g.writers == 0
    }

    pub fn register_waiter(&self, actor: ActorId) {
        let mut g = lock(&self.inner);
        if !g.waiters.contains(&actor) {
            g.waiters.push(actor);
        }
    }
}

/// Read end handle (counted: EOF for writers when the last reader closes).
pub struct PipeReader(pub Arc<Pipe>);
/// Write end handle (counted: EOF for readers when the last writer closes).
pub struct PipeWriter(pub Arc<Pipe>);

impl PipeReader {
    pub fn new(p: &Arc<Pipe>) -> Self {
        lock(&p.inner).readers += 1;
        PipeReader(p.clone())
    }
}
impl PipeWriter {
    pub fn new(p: &Arc<Pipe>) -> Self {
        lock(&p.inner).writers += 1;
        PipeWriter(p.clone())
    }
}
impl Clone for PipeReader {
    fn clone(&self) -> Self {
        PipeReader::new(&self.0)
    }
}
impl Clone for PipeWriter {
    fn clone(&self) -> Self {
        PipeWriter::new(&self.0)
    }
}
impl Drop for PipeReader {
    fn drop(&mut self) {
        let woken = {
            let mut g = lock(&self.0.inner);
            g.readers = g.readers.saturating_sub(1);
            if g.readers == 0 {
                self.0.wake(&mut g)
            } else {
                Vec::new()
            }
        };
        self.0.finish(woken, false);
    }
}
impl Drop for PipeWriter {
    fn drop(&mut self) {
        let (woken, eof) = {
            let mut g = lock(&self.0.inner);
            g.writers = g.writers.saturating_sub(1);
            if g.writers == 0 {
                (self.0.wake(&mut g), true)
            } else {
                (Vec::new(), false)
            }
        };
        self.0.finish(woken, eof);
    }
}

// ============================================================ files

pub struct FileHandle {
    pub file: File,
    #[allow(dead_code)]
    pub path: PathBuf,
    pub append: bool,
}

impl FileHandle {
    pub fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
    pub fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.append {
            self.file.seek(SeekFrom::End(0))?;
        }
        self.file.write_all(data)?;
        Ok(data.len())
    }
    pub fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(pos)
    }
    pub fn size(&self) -> u64 {
        self.file.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

/// Anything a descriptor can refer to, shared by the kernel's `fd_*`
/// table and the children's WASI tables.
#[derive(Clone)]
pub enum Handle {
    PipeR(Arc<PipeReader>),
    PipeW(Arc<PipeWriter>),
    File(Arc<Mutex<FileHandle>>),
}

impl Handle {
    pub fn pipe_r(p: &Arc<Pipe>) -> Handle {
        Handle::PipeR(Arc::new(PipeReader::new(p)))
    }
    pub fn pipe_w(p: &Arc<Pipe>) -> Handle {
        Handle::PipeW(Arc::new(PipeWriter::new(p)))
    }
    /// A fresh counted reference (child's own copy of an inherited fd).
    pub fn dup(&self) -> Handle {
        match self {
            Handle::PipeR(r) => Handle::PipeR(Arc::new((**r).clone())),
            Handle::PipeW(w) => Handle::PipeW(Arc::new((**w).clone())),
            Handle::File(f) => Handle::File(f.clone()),
        }
    }
}

// ============================================================ processes

pub struct ProcEntry {
    pub name: String,
    pub zombie: bool,
    pub exit_code: Option<i32>,
    pub exit_reported: bool,
    /// Terminal output. `output_reader` is the host's counted read end:
    /// holding it keeps the child's writes from failing as a broken pipe.
    pub output: Arc<Pipe>,
    #[allow(dead_code)]
    pub output_reader: Option<PipeReader>,
    /// Keyboard input (the host's write end); None once the child exited.
    pub input: Option<PipeWriter>,
    pub input_pipe: Arc<Pipe>,
    pub killed: Arc<AtomicBool>,
    pub actor: ActorId,
}

pub struct ProcTable {
    procs: Mutex<BTreeMap<i32, ProcEntry>>,
    next_pid: AtomicI32,
    activity: Arc<AtomicBool>,
    sched: Arc<Sched>,
}

pub enum TryWait {
    Running,
    Exited(i32),
    Unknown,
}

impl ProcTable {
    pub fn new(sched: Arc<Sched>) -> Arc<Self> {
        Arc::new(ProcTable {
            procs: Mutex::new(BTreeMap::new()),
            next_pid: AtomicI32::new(1),
            activity: Arc::new(AtomicBool::new(false)),
            sched,
        })
    }

    pub fn alloc_pid(&self) -> i32 {
        self.next_pid.fetch_add(1, Ordering::SeqCst)
    }

    pub fn mark_activity(&self) {
        self.activity.store(true, Ordering::SeqCst);
        self.sched.notify_event();
    }

    pub fn take_activity(&self) -> bool {
        self.activity.swap(false, Ordering::SeqCst)
    }

    pub fn insert(&self, pid: i32, e: ProcEntry) {
        lock(&self.procs).insert(pid, e);
    }

    /// Child thread finished.
    pub fn set_exited(&self, pid: i32, code: i32) {
        let mut g = lock(&self.procs);
        if let Some(e) = g.get_mut(&pid) {
            e.zombie = true;
            e.exit_code = Some(code);
            // Kernel can't feed a dead process; the child's stdin closes.
            e.input = None;
        }
    }

    pub fn try_wait(&self, pid: i32) -> TryWait {
        let mut g = lock(&self.procs);
        let Some(e) = g.get_mut(&pid) else { return TryWait::Unknown };
        if !e.zombie {
            return TryWait::Running;
        }
        // Report the exit only once the terminal output is drained, so an
        // output throttle can never cut off the tail of a program's output.
        if e.output.has_data() {
            return TryWait::Running;
        }
        e.exit_reported = true;
        TryWait::Exited(e.exit_code.unwrap_or(-1))
    }

    /// Returns bytes read, 0 if none, -1 if the pid is unknown.
    pub fn read_output(&self, pid: i32, buf: &mut [u8], max_newlines: usize) -> (i32, usize) {
        let mut g = lock(&self.procs);
        let Some(e) = g.get(&pid) else { return (-1, 0) };
        let (n, nl) = e.output.try_read_lines(buf, max_newlines);
        if n == 0 && e.exit_reported && !e.output.has_data() {
            g.remove(&pid);
        }
        (n as i32, nl)
    }

    pub fn output_pending(&self, pid: i32) -> bool {
        lock(&self.procs).get(&pid).is_some_and(|e| e.output.has_data())
    }

    pub fn write_input(&self, pid: i32, data: &[u8]) -> i32 {
        let g = lock(&self.procs);
        let Some(e) = g.get(&pid) else { return -1 };
        if e.input.is_none() {
            return -1;
        }
        e.input_pipe.try_write(data)
    }

    pub fn kill(&self, pid: i32) -> i32 {
        let (actor, zombie) = {
            let g = lock(&self.procs);
            let Some(e) = g.get(&pid) else { return -1 };
            e.killed.store(true, Ordering::SeqCst);
            (e.actor, e.zombie)
        };
        if !zombie {
            self.sched.wake(actor);
        }
        0
    }

    pub fn kill_all(&self) {
        let pids: Vec<i32> = lock(&self.procs).keys().copied().collect();
        for p in pids {
            self.kill(p);
        }
    }

    pub fn list_json(&self) -> String {
        let g = lock(&self.procs);
        let items: Vec<String> = g
            .iter()
            .map(|(pid, e)| {
                format!(
                    "{{\"pid\":{},\"name\":\"{}\",\"state\":\"{}\"}}",
                    pid,
                    e.name.replace('"', "'"),
                    if e.zombie { "ZOMBIE" } else { "RUNNING" }
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    }


}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sched::ClockMode;

    #[test]
    fn pipe_eof_and_broken_pipe() {
        let s = Arc::new(Sched::new(ClockMode::Virtual));
        let p = Pipe::new(8, s.clone(), None);
        let r = PipeReader::new(&p);
        let w = PipeWriter::new(&p);
        assert_eq!(p.try_write(b"hello world"), 8, "partial write up to capacity");
        let mut b = [0u8; 16];
        assert_eq!(p.try_read(&mut b), 8);
        drop(w);
        let a = s.register("t");
        let k = AtomicBool::new(false);
        assert_eq!(p.read(a, &k, &mut b).unwrap(), 0, "EOF after last writer");
        drop(r);
        assert_eq!(p.try_write(b"x"), -1, "broken pipe after last reader");
        s.exit(a);
    }

    #[test]
    fn line_throttle() {
        let s = Arc::new(Sched::new(ClockMode::Virtual));
        let p = Pipe::new(1024, s, None);
        let _r = PipeReader::new(&p);
        let _w = PipeWriter::new(&p);
        p.try_write(b"a\nb\nc\n");
        let mut b = [0u8; 64];
        assert_eq!(p.try_read_lines(&mut b, 2), (4, 2));
        assert_eq!(p.try_read_lines(&mut b, 2), (2, 1));
    }

    #[test]
    fn blocked_reader_wakes_on_write() {
        let s = Arc::new(Sched::new(ClockMode::Virtual));
        let p = Pipe::new(64, s.clone(), None);
        let r = PipeReader::new(&p);
        let w = PipeWriter::new(&p);
        let a = s.register("reader");
        let (p2, s2) = (p.clone(), s.clone());
        let h = std::thread::spawn(move || {
            let _r = r;
            let k = AtomicBool::new(false);
            let mut b = [0u8; 8];
            let n = p2.read(a, &k, &mut b).unwrap();
            s2.exit(a);
            n
        });
        assert!(s.quiesce(std::time::Duration::from_secs(5)));
        assert_eq!(p.try_write(b"hi"), 2);
        assert_eq!(h.join().unwrap(), 2);
        drop(w);
    }
}

#[allow(unused)]
fn _assert_send() {
    fn is_send<T: Send + Sync>() {}
    is_send::<Pipe>();
    is_send::<ProcTable>();
    let _ = Wake::Ready;
}
