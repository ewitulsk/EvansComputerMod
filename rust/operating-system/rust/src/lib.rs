//! Terminal OS — the per-computer kernel.
//!
//! Event-driven and non-blocking: every export does a bounded amount of work
//! and returns. The host drives the kernel with input, interrupts, socket
//! requests and `on_tick` (whose return value is the next deadline). See
//! `docs/refactor/ARCHITECTURE.md` for the host contract.

pub mod bgp_svc;
pub mod console;
pub mod framebuffer;
pub mod fs;
pub mod gfx;
pub mod gfx_test;
pub mod hal;
pub mod jobs;
mod kcell;
pub mod kernel;
pub mod lineedit;
pub mod net;
pub mod parse;
pub mod router_svc;
pub mod services;
pub mod sessions;
pub mod shell;
pub mod switch_svc;
pub mod vte;

#[cfg(target_arch = "wasm32")]
use kernel::Kernel;

fn region_slice(ptr: usize, len: usize, region_addr: u32, cap: usize) -> Option<(usize, usize)> {
    // Host-supplied pointers must lie inside the region the host was told
    // to use; anything else is rejected instead of dereferenced.
    let base = region_addr as usize;
    let off = ptr.checked_sub(base)?;
    if off.checked_add(len)? <= cap {
        Some((off, len))
    } else {
        None
    }
}

/// Boot the kernel. (Not built for host tests, whose harness owns `main`.)
#[cfg(target_arch = "wasm32")]
#[unsafe(no_mangle)]
pub extern "C" fn main() {
    let now = hal::now_ms();
    kcell::install(Kernel::boot(now));
}

/// Keyboard bytes. `ptr` must point into the input region.
#[unsafe(no_mangle)]
pub extern "C" fn on_input(ptr: usize, len: usize) {
    let Some((off, len)) = region_slice(ptr, len, hal::INPUT.addr(), hal::INPUT_CAP) else {
        return;
    };
    let data = hal::INPUT.bytes()[off..off + len].to_vec();
    kcell::with(|k| k.on_input(&data, hal::now_ms()));
}

/// Interrupt with a binary payload in the IRQ region.
#[unsafe(no_mangle)]
pub extern "C" fn on_interrupt(irq: i32, ptr: usize, len: usize) {
    let payload = match region_slice(ptr, len, hal::IRQ.addr(), hal::IRQ_CAP) {
        Some((off, len)) => hal::IRQ.bytes()[off..off + len].to_vec(),
        None => Vec::new(),
    };
    kcell::with(|k| k.on_interrupt(irq, &payload, hal::now_ms()));
}

/// Run timers and jobs. Returns the next absolute deadline (ms) or -1.
#[unsafe(no_mangle)]
pub extern "C" fn on_tick(now_ms: i64) -> i64 {
    kcell::with(|k| k.on_tick(now_ms)).unwrap_or(-1)
}

/// Socket syscall for child `session`. Args/result must be in the IPC
/// regions. Returns the result length or `IPC_PENDING` (-11).
#[unsafe(no_mangle)]
pub extern "C" fn handle_sock_ipc(
    session: i32,
    syscall: i32,
    args_ptr: usize,
    args_len: usize,
    result_ptr: usize,
    result_len: usize,
) -> i32 {
    let Some((aoff, alen)) =
        region_slice(args_ptr, args_len, hal::IPC_ARGS.addr(), hal::IPC_ARGS_CAP)
    else {
        return -1;
    };
    let Some((roff, rlen)) = region_slice(
        result_ptr,
        result_len,
        hal::IPC_RESULT.addr(),
        hal::IPC_RESULT_CAP,
    ) else {
        return -1;
    };
    let args = hal::IPC_ARGS.bytes()[aoff..aoff + alen].to_vec();
    let result = &mut hal::IPC_RESULT.bytes()[roff..roff + rlen];
    kcell::with(|k| k.sock_ipc(session, syscall, &args, result, hal::now_ms())).unwrap_or(-1)
}

/// Write the shared-memory layout (u32 LE words) at `ptr`, up to `cap`
/// words. Returns the number of words written. `ptr` must be inside the
/// input region (the only region the host may use before it knows the
/// layout — its address is also returned as word 1).
#[unsafe(no_mangle)]
pub extern "C" fn abi_layout(ptr: usize, cap: usize) -> i32 {
    let words = hal::abi_layout_words();
    let n = words.len().min(cap);
    let Some((off, _)) = region_slice(ptr, n * 4, hal::INPUT.addr(), hal::INPUT_CAP) else {
        return -1;
    };
    let buf = hal::INPUT.bytes();
    for (i, w) in words.iter().take(n).enumerate() {
        buf[off + i * 4..off + i * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    n as i32
}

/// Address of the input region, so a host can find a scratch buffer for
/// `abi_layout` without any prior knowledge.
#[unsafe(no_mangle)]
pub extern "C" fn abi_scratch() -> usize {
    hal::INPUT.addr() as usize
}

/// The host forced a trap out of an export (Ctrl+T on a runaway loop).
#[unsafe(no_mangle)]
pub extern "C" fn kernel_recover() {
    kcell::force_release();
    kcell::with(|k| k.recover(hal::now_ms()));
}
