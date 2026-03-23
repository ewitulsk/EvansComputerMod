//! Sleep host function.

use wasmtime::*;
use crate::wasm_host::HostState;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &["sleep_ms"];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap("env", "sleep_ms", |caller: Caller<'_, HostState>, ms: i32| {
        let ms = ms.clamp(0, 60_000);
        let shutdown = caller.data().shutdown.clone();
        let mut remaining = ms;
        while remaining > 0 && !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            let chunk = remaining.min(50);
            std::thread::sleep(std::time::Duration::from_millis(chunk as u64));
            remaining -= chunk;
        }
    })?;

    Ok(())
}
