//! Interrupt handler system for Terminal OS.
//!
//! Provides an interrupt dispatch table where handlers can be registered
//! for different IRQ numbers. Interrupts are delivered cooperatively by
//! the host (Java) calling `on_interrupt()` at yield points.

/// Keyboard input interrupt - fired when a key is pressed during execution
pub const IRQ_KEYBOARD: i32 = 1;
/// Redstone input change interrupt - fired when redstone signals change
pub const IRQ_REDSTONE: i32 = 2;
/// Network frame arrival interrupt
pub const IRQ_NETWORK: i32 = 3;
/// Mouse event interrupt. Payload is a 10-byte little-endian tuple; see
/// `ComputerInstance.queueInterrupt(int, byte[])` for the layout. The
/// kernel does not currently consume this directly — WASI children read
/// events via the `mouse_poll` host function backed by a ring buffer.
pub const IRQ_MOUSE: i32 = 4;
/// Terminate interrupt (non-maskable) - always resets to shell
pub const IRQ_TERMINATE: i32 = 15;

/// Maximum number of IRQ slots
const MAX_IRQ: usize = 16;

/// Rust-level interrupt handler function type
type HandlerFn = fn(irq: i32, data: &str);

/// Handler table for Rust-level interrupt handlers
static mut HANDLERS: [Option<HandlerFn>; MAX_IRQ] = [None; MAX_IRQ];

/// Flags indicating which IRQs have Python-level handlers registered.
/// When set, dispatch will call into the Python interpreter.
static mut PYTHON_HANDLERS: [bool; MAX_IRQ] = [false; MAX_IRQ];

/// Registers a Rust-level interrupt handler for the given IRQ.
pub fn register(irq: i32, handler: HandlerFn) {
    if irq >= 0 && (irq as usize) < MAX_IRQ {
        unsafe {
            HANDLERS[irq as usize] = Some(handler);
        }
    }
}

/// Unregisters any handler (Rust or Python) for the given IRQ.
pub fn unregister(irq: i32) {
    if irq >= 0 && (irq as usize) < MAX_IRQ {
        unsafe {
            HANDLERS[irq as usize] = None;
            PYTHON_HANDLERS[irq as usize] = false;
        }
    }
}

/// Marks an IRQ as having a Python-level handler.
/// The actual Python callable is stored in the python module.
pub fn register_python(irq: i32) {
    if irq >= 0 && (irq as usize) < MAX_IRQ {
        unsafe {
            PYTHON_HANDLERS[irq as usize] = true;
        }
    }
}

/// Clears all Python-level interrupt handlers.
/// Called when exiting Python mode to prevent stale handler references.
pub fn clear_all_python() {
    unsafe {
        for flag in PYTHON_HANDLERS.iter_mut() {
            *flag = false;
        }
    }
}

/// Returns whether a Python handler is registered for the given IRQ.
pub fn has_python_handler(irq: i32) -> bool {
    if irq >= 0 && (irq as usize) < MAX_IRQ {
        unsafe { PYTHON_HANDLERS[irq as usize] }
    } else {
        false
    }
}

/// Dispatches an interrupt to the registered handler.
/// If a Rust handler is registered, it is called directly.
/// If a Python handler is flagged, the caller (lib.rs) must route to the Python module.
/// Returns true if a handler was found (Rust-level), false otherwise.
pub fn dispatch_rust(irq: i32, data: &str) -> bool {
    if irq >= 0 && (irq as usize) < MAX_IRQ {
        unsafe {
            if let Some(handler) = HANDLERS[irq as usize] {
                handler(irq, data);
                return true;
            }
        }
    }
    false
}
