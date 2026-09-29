//! The single owner of all kernel state.
//!
//! Every export enters the kernel through [`with`]. A borrow flag makes
//! re-entry (a host function calling back into the kernel while an export is
//! still on the stack) a loud trap instead of silent aliasing UB. The host
//! contract forbids re-entry, so tripping the flag means a host bug.

use core::cell::{Cell, UnsafeCell};

use crate::kernel::Kernel;

struct KernelCell {
    borrowed: Cell<bool>,
    kernel: UnsafeCell<Option<Kernel>>,
}

// The kernel is single-threaded (wasm32, no threads). Sync is required only
// to place the cell in a `static`.
unsafe impl Sync for KernelCell {}

static KERNEL: KernelCell = KernelCell {
    borrowed: Cell::new(false),
    kernel: UnsafeCell::new(None),
};

/// Install the kernel. Called once from `main` (wasm only).
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn install(k: Kernel) {
    assert!(!KERNEL.borrowed.get(), "kernel install while borrowed");
    unsafe { *KERNEL.kernel.get() = Some(k) };
}

/// Run `f` with exclusive access to the kernel. Returns `None` before boot.
pub fn with<R>(f: impl FnOnce(&mut Kernel) -> R) -> Option<R> {
    if KERNEL.borrowed.replace(true) {
        panic!("kernel re-entered: host called an export while another export was running");
    }
    let r = unsafe { (*KERNEL.kernel.get()).as_mut().map(f) };
    KERNEL.borrowed.set(false);
    r
}

/// Clear the borrow flag after a host-initiated trap unwound out of an
/// export (the flag was left set because the export never returned).
pub fn force_release() {
    KERNEL.borrowed.set(false);
}
