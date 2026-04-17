//! Mouse capture host-function wrappers for WASI child programs.
//!
//! The host captures mouse events on the client (Minecraft `Screen`)
//! and funnels them into a ring buffer on the server. WASI children
//! enable capture with [`enable`], then drain events one at a time
//! from the ring by calling [`poll`] in their frame loop.
//!
//! Capture is only authorized while the terminal's on-screen UI is
//! open AND the display is in a graphics mode — see
//! `ComputerInstance.bridgeMouseCaptureStart`. [`enable`] returns
//! `false` if either condition is not met.

use core::mem::MaybeUninit;

/// Kind of a mouse event. Values match the `KIND_*` constants on
/// `MouseInputPacket` and must stay in sync.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseKind {
    Move = 1,
    Down = 2,
    Up = 3,
    Scroll = 4,
    /// Unknown / reserved; returned when the host sends a value the
    /// ABI doesn't recognize.
    Other = 0,
}

impl MouseKind {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => MouseKind::Move,
            2 => MouseKind::Down,
            3 => MouseKind::Up,
            4 => MouseKind::Scroll,
            _ => MouseKind::Other,
        }
    }
}

/// Button bitmask bits for [`MouseEvent::buttons`]. Bits are OR-ed for
/// chorded presses.
pub mod button {
    pub const LEFT: u8 = 0x01;
    pub const RIGHT: u8 = 0x02;
    pub const MIDDLE: u8 = 0x04;
}

/// Decoded mouse event. Byte layout matches the 10-byte payload read
/// directly out of child memory by [`poll`].
#[derive(Clone, Copy, Debug)]
pub struct MouseEvent {
    pub kind: MouseKind,
    /// Framebuffer pixel X, already clamped to `[0, gfx_width)` by the host.
    pub x: i16,
    /// Framebuffer pixel Y, already clamped to `[0, gfx_height)` by the host.
    pub y: i16,
    /// Bitmask of currently-held buttons (see [`button`]).
    pub buttons: u8,
    /// For `Down`/`Up`: 0=L, 1=R, 2=M. Unspecified for other kinds.
    pub button_code: u8,
    /// For `Scroll`: -1 (down) or +1 (up). 0 for other kinds.
    pub scroll_dir: i8,
}

extern "C" {
    fn mouse_capture_start() -> i32;
    fn mouse_capture_stop();
    fn mouse_capture_is_active() -> i32;
    fn mouse_poll(buf_ptr: i32) -> i32;
}

/// Ask the host to start feeding mouse events into the ring buffer.
/// Returns `true` on success. A `false` return means the terminal is
/// not currently in a graphics mode or the host cannot serve mouse
/// capture for some other reason; callers should fall back to a
/// polite error message rather than spin-polling.
pub fn enable() -> bool {
    unsafe { mouse_capture_start() == 1 }
}

/// Disable capture and drop any events still in the ring. Safe to
/// call even if capture was never enabled.
pub fn disable() {
    unsafe { mouse_capture_stop() }
}

/// Returns whether capture is currently enabled on the host side.
/// Programs that need to notice a host-initiated disable (e.g. mode
/// change) can poll this.
pub fn is_active() -> bool {
    unsafe { mouse_capture_is_active() == 1 }
}

/// Pop one event off the ring, or `None` if the ring is empty. The
/// 10-byte payload is read into a stack buffer via the host.
pub fn poll() -> Option<MouseEvent> {
    let mut buf = MaybeUninit::<[u8; 10]>::uninit();
    let rc = unsafe { mouse_poll(buf.as_mut_ptr() as i32) };
    if rc != 1 {
        return None;
    }
    let bytes = unsafe { buf.assume_init() };
    Some(MouseEvent {
        kind: MouseKind::from_u8(bytes[0]),
        x: i16::from_le_bytes([bytes[1], bytes[2]]),
        y: i16::from_le_bytes([bytes[3], bytes[4]]),
        buttons: bytes[5],
        button_code: bytes[6],
        scroll_dir: bytes[7] as i8,
    })
}
