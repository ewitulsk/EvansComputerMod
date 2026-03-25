use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use wasmtime::Engine;

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
}
