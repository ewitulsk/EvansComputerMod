//! Shared wasmtime engine, module cache and the epoch ticker.
//!
//! Epoch interruption is on for every store. A background thread bumps the
//! engine epoch every [`EPOCH_TICK_MS`]; each store's deadline callback then
//! decides: kernels trap on Ctrl+T-while-stuck or the watchdog, children
//! trap when killed (so even a compute-bound child dies on Ctrl+T), and
//! everything else continues.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context, Result};
use wasmtime::{Config, Engine, ExternType, Module, ValType};

pub const EPOCH_TICK_MS: u64 = 10;

pub struct Runtime {
    pub engine: Engine,
    cache: Mutex<HashMap<PathBuf, (SystemTime, Module)>>,
    stop: Arc<AtomicBool>,
}

impl Runtime {
    pub fn new() -> Result<Arc<Self>> {
        let mut cfg = Config::new();
        cfg.epoch_interruption(true);
        // Compiled-module cache on disk makes repeated scenario runs (and the
        // large python/ssh binaries) start fast. Optional.
        let _ = cfg.cache_config_load_default();
        let engine = Engine::new(&cfg)?;
        let stop = Arc::new(AtomicBool::new(false));
        let (e2, s2) = (engine.clone(), stop.clone());
        std::thread::Builder::new()
            .name("epoch-ticker".into())
            .spawn(move || {
                while !s2.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(EPOCH_TICK_MS));
                    e2.increment_epoch();
                }
            })?;
        Ok(Arc::new(Runtime { engine, cache: Mutex::new(HashMap::new()), stop }))
    }

    /// Compile (or fetch from cache) a module.
    pub fn module(&self, path: &Path) -> Result<Module> {
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .with_context(|| format!("{}", path.display()))?;
        let key = path.to_path_buf();
        if let Some((t, m)) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            if *t == mtime {
                return Ok(m.clone());
            }
        }
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let m = Module::new(&self.engine, &bytes).with_context(|| format!("compiling {}", path.display()))?;
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (mtime, m.clone()));
        Ok(m)
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn sig_string(params: impl Iterator<Item = ValType>, results: impl Iterator<Item = ValType>) -> String {
    let p: Vec<String> = params.map(|v| v.to_string()).collect();
    let r: Vec<String> = results.map(|v| v.to_string()).collect();
    format!("({}) -> ({})", p.join(", "), r.join(", "))
}

/// Strict linking: every function import of `module` must be provided with
/// the exact signature. `provided` maps "module::name" to a signature string
/// built with [`sig_string`]. Returns every problem at once.
pub fn check_imports(module: &Module, provided: &HashMap<String, String>) -> Result<(), String> {
    let mut problems = Vec::new();
    for imp in module.imports() {
        let key = format!("{}::{}", imp.module(), imp.name());
        match imp.ty() {
            ExternType::Func(ft) => {
                let want = sig_string(ft.params(), ft.results());
                match provided.get(&key) {
                    None => problems.push(format!("missing {} {}", key, want)),
                    Some(have) if *have != want => {
                        problems.push(format!("signature mismatch {}: module wants {}, host has {}", key, want, have))
                    }
                    Some(_) => {}
                }
            }
            ExternType::Memory(_) => problems.push(format!("unsupported memory import {}", key)),
            ExternType::Table(_) => problems.push(format!("unsupported table import {}", key)),
            _ => problems.push(format!("unsupported non-function import {}", key)),
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The error a guest's `proc_exit` unwinds with.
#[derive(Debug)]
pub struct ProcExit(pub i32);
impl std::fmt::Display for ProcExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "proc_exit({})", self.0)
    }
}
impl std::error::Error for ProcExit {}

/// Why the host forced a trap.
#[derive(Debug)]
pub struct HostInterrupt(pub &'static str);
impl std::fmt::Display for HostInterrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "host interrupt: {}", self.0)
    }
}
impl std::error::Error for HostInterrupt {}

pub fn interrupt_err(why: &'static str) -> anyhow::Error {
    anyhow!(HostInterrupt(why))
}
