use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use wasmtime::*;

use crate::fd::{FdTable, FileDescriptor};
use crate::filesystem::FileSystem;
use crate::host::wasi_io::WasiState;
use crate::interrupts::InterruptQueue;
use crate::wasm_host::{ChannelReceiver, HostState};

/// State of a managed process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Stopped,
    Zombie,
}

/// Entry in the process table.
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    pub join_handle: Option<thread::JoinHandle<()>>,
    pub exit_notify: Arc<(Mutex<bool>, Condvar)>,
}

/// Manages all processes for a single computer.
pub struct ProcessManager {
    engine: Engine,
    processes: HashMap<u32, ProcessEntry>,
    next_pid: AtomicU32,
}

impl ProcessManager {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            processes: HashMap::new(),
            next_pid: AtomicU32::new(2), // PID 1 reserved for kernel
        }
    }

    /// Get a reference to the shared engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Register the kernel as PID 1.
    pub fn register_kernel(&mut self) {
        self.processes.insert(1, ProcessEntry {
            pid: 1,
            parent_pid: 0,
            name: "kernel".to_string(),
            state: ProcessState::Running,
            exit_code: None,
            join_handle: None,
            exit_notify: Arc::new((Mutex::new(false), Condvar::new())),
        });
    }

    /// Allocate the next PID.
    pub fn alloc_pid(&self) -> u32 {
        self.next_pid.fetch_add(1, Ordering::SeqCst)
    }

    /// Get a process entry by PID.
    pub fn get(&self, pid: u32) -> Option<&ProcessEntry> {
        self.processes.get(&pid)
    }

    /// Get a mutable process entry by PID.
    pub fn get_mut(&mut self, pid: u32) -> Option<&mut ProcessEntry> {
        self.processes.get_mut(&pid)
    }

    /// Insert a process entry.
    pub fn insert(&mut self, entry: ProcessEntry) {
        self.processes.insert(entry.pid, entry);
    }

    /// Remove a process entry (reap zombie).
    pub fn remove(&mut self, pid: u32) -> Option<ProcessEntry> {
        self.processes.remove(&pid)
    }

    /// List all processes.
    pub fn list(&self) -> Vec<&ProcessEntry> {
        let mut entries: Vec<_> = self.processes.values().collect();
        entries.sort_by_key(|e| e.pid);
        entries
    }

    /// Spawn a new WASI process from wasm bytes.
    ///
    /// Creates a new wasmtime Store+Instance with WASI host functions,
    /// wires up the FD table with the provided stdio descriptors, and
    /// runs the process on a new thread.
    ///
    /// Returns the PID of the spawned process.
    pub fn spawn(
        &mut self,
        wasm_bytes: &[u8],
        name: String,
        argv: Vec<String>,
        env: Vec<(String, String)>,
        stdin: Box<dyn FileDescriptor>,
        stdout: Box<dyn FileDescriptor>,
        stderr: Box<dyn FileDescriptor>,
        filesystem: FileSystem,
        shutdown: Arc<AtomicBool>,
    ) -> Result<u32, String> {
        let pid = self.alloc_pid();
        let exit_notify = Arc::new((Mutex::new(false), Condvar::new()));
        let exit_notify_thread = exit_notify.clone();

        // Set up FdTable with stdio
        let mut fd_table = FdTable::new();
        fd_table.insert_at(0, stdin);
        fd_table.insert_at(1, stdout);
        fd_table.insert_at(2, stderr);

        // Create a dummy input channel (WASI processes use FD-based I/O, not the channel)
        let (_dummy_tx, dummy_rx) = std::sync::mpsc::channel::<String>();

        // Create HostState for this process
        let mut host_state = HostState {
            renderer: crate::terminal_io::FramebufferRenderer::new(80, 24),
            filesystem,
            redstone: crate::redstone::RedstoneState::new(),
            interrupt_queue: InterruptQueue::new(),
            input_rx: Arc::new(ChannelReceiver(std::sync::Mutex::new(dummy_rx))),
            shutdown: shutdown.clone(),
            last_interrupt_payload_len: 0,
            next_object_handle: 1,
            force_render: false,
            custom: HashMap::new(),
        };

        host_state.insert_custom(fd_table);
        host_state.insert_custom(WasiState {
            argv,
            env_vars: env,
        });

        let engine = self.engine.clone();
        let wasm_bytes = wasm_bytes.to_vec();

        let join_handle = thread::Builder::new()
            .name(format!("wasm-pid-{}", pid))
            .spawn(move || {
                let exit_code = match Self::run_wasi_process(&engine, host_state, &wasm_bytes, pid)
                {
                    Ok(code) => code,
                    Err(e) => {
                        eprintln!("[PID {}] Failed to start: {}", pid, e);
                        1
                    }
                };

                let _ = exit_code; // TODO: store exit code in shared state

                // Notify waiters
                let (lock, cvar) = &*exit_notify_thread;
                let mut done = lock.lock().unwrap();
                *done = true;
                cvar.notify_all();
            })
            .map_err(|e| format!("Failed to spawn thread: {}", e))?;

        self.insert(ProcessEntry {
            pid,
            parent_pid: 1,
            name,
            state: ProcessState::Running,
            exit_code: None,
            join_handle: Some(join_handle),
            exit_notify,
        });

        Ok(pid)
    }

    /// Run a WASI process to completion. Returns exit code.
    fn run_wasi_process(
        engine: &Engine,
        host_state: HostState,
        wasm_bytes: &[u8],
        pid: u32,
    ) -> Result<i32, String> {
        let module =
            Module::new(engine, wasm_bytes).map_err(|e| format!("Module load error: {}", e))?;
        let mut store = Store::new(engine, host_state);
        let mut linker = Linker::new(engine);

        // Register ONLY WASI functions (not kernel "env" functions)
        crate::host::wasi_io::register(&mut linker)
            .map_err(|e| format!("WASI IO registration error: {}", e))?;

        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| format!("Instantiation error: {}", e))?;

        // Try _start first, then main as fallback
        match instance.get_typed_func::<(), ()>(&mut store, "_start") {
            Ok(start_fn) => match start_fn.call(&mut store, ()) {
                Ok(()) => Ok(0),
                Err(e) => Self::extract_exit_code(&e, pid),
            },
            Err(_) => match instance.get_typed_func::<(), ()>(&mut store, "main") {
                Ok(main_fn) => match main_fn.call(&mut store, ()) {
                    Ok(()) => Ok(0),
                    Err(e) => Self::extract_exit_code(&e, pid),
                },
                Err(_) => {
                    eprintln!("[PID {}] No _start or main export found", pid);
                    Ok(127)
                }
            },
        }
    }

    /// Extract exit code from a proc_exit trap or report an error.
    fn extract_exit_code(err: &anyhow::Error, pid: u32) -> Result<i32, String> {
        let msg = format!("{}", err);
        // proc_exit traps look like "proc_exit(N)"
        if let Some(rest) = msg.find("proc_exit(") {
            let after = &msg[rest + "proc_exit(".len()..];
            if let Some(end) = after.find(')') {
                if let Ok(code) = after[..end].parse::<i32>() {
                    return Ok(code);
                }
            }
        }
        eprintln!("[PID {}] Runtime error: {}", pid, msg);
        Ok(1)
    }

    /// Wait for a process to exit. Blocks until the process finishes.
    /// Returns the exit code.
    pub fn wait(&mut self, pid: u32) -> Option<i32> {
        let notify = {
            let entry = self.get(pid)?;
            entry.exit_notify.clone()
        };

        // Block until the process signals completion
        let (lock, cvar) = &*notify;
        let mut done = lock.lock().unwrap();
        while !*done {
            done = cvar.wait(done).unwrap();
        }

        // Mark as zombie
        if let Some(entry) = self.get_mut(pid) {
            entry.state = ProcessState::Zombie;
        }

        // Reap: remove from table and join the thread
        if let Some(mut entry) = self.remove(pid) {
            if let Some(handle) = entry.join_handle.take() {
                let _ = handle.join();
            }
            Some(entry.exit_code.unwrap_or(0))
        } else {
            Some(0)
        }
    }
}
