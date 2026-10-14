//! UI protocol host calls (docs/gui/UI_PROTOCOL.md §3, `[imports.ui]` in
//! abi/child-abi.toml).
//!
//! A program talks to the compositor over one *UI file descriptor*: whole
//! protocol messages go out with [`send`] and come back with [`recv`]
//! (encode and decode them with the `ecm-ui-proto` crate). Bulk pixels skip
//! the message stream: [`put_pixels`] copies a pixel surface's or canvas's
//! rows straight from program memory, and [`upload`] sends an image
//! resource. The fd also works with `sock_poll` / `poll_oneoff`.
//!
//! In console mode (no compositor running) [`open`] fails with
//! [`UiError::NoDevice`]; most programs then fall back to text.

extern "C" {
    fn ui_open(role: i32, version: i32) -> i32;
    fn ui_send(fd: i32, buf: i32, len: i32) -> i32;
    fn ui_recv(fd: i32, buf: i32, cap: i32, timeout_ms: i32) -> i32;
    fn ui_put_pixels(fd: i32, target_id: i32, buf: i32, len: i32, damage_ptr: i32, n_rects: i32) -> i32;
    fn ui_upload(fd: i32, resource_id: i32, kind: i32, buf: i32, len: i32) -> i32;
}

/// [`open`] role: an ordinary app.
pub const ROLE_APP: i32 = 0;
/// [`open`] role: the shell (one per display).
pub const ROLE_SHELL: i32 = 1;
/// [`upload`] kind: a PNG file.
pub const UPLOAD_PNG: i32 = 0;
/// [`upload`] kind: raw RGBA8888, size from the resource's `ResourceCreate`.
pub const UPLOAD_RGBA: i32 = 1;

/// Errors from the UI calls.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UiError {
    /// Bad argument, or a message the compositor rejected (the connection is
    /// then closed with an `Error` message).
    Invalid,
    /// The shell role is already taken.
    Busy,
    /// Console mode: no compositor is running (or the simulator).
    NoDevice,
    /// Interrupted while waiting (the program is being killed), or the
    /// connection is closed.
    Interrupted,
    /// This app may not open a UI connection in that role.
    NotPermitted,
    /// Any other negative code from the host.
    Other(i32),
}

impl UiError {
    pub fn from_code(rc: i32) -> UiError {
        match rc {
            -1 => UiError::Invalid,
            -2 => UiError::Busy,
            -4 => UiError::NoDevice,
            -5 => UiError::Interrupted,
            -6 => UiError::NotPermitted,
            n => UiError::Other(n),
        }
    }
}

/// A damage rectangle for [`put_pixels`], in the target's pixels.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// Open a UI connection. `version` is the protocol version the program
/// speaks (`ecm_ui_proto::VERSION`). Returns the UI fd.
///
/// fds 0-2 are stdio, so a result below 3 means the host has no UI support
/// (hosts that auto-stub unknown imports return 0): that is console mode too.
pub fn open(role: i32, version: u16) -> Result<i32, UiError> {
    let rc = unsafe { ui_open(role, version as i32) };
    match rc {
        rc if rc < 0 => Err(UiError::from_code(rc)),
        0..=2 => Err(UiError::NoDevice),
        fd => Ok(fd),
    }
}

/// Send one or more whole messages.
pub fn send(fd: i32, bytes: &[u8]) -> Result<(), UiError> {
    let rc = unsafe { ui_send(fd, bytes.as_ptr() as i32, bytes.len() as i32) };
    if rc < 0 { Err(UiError::from_code(rc)) } else { Ok(()) }
}

/// Receive whole messages into `buf`, waiting up to `timeout_ms` (< 0 =
/// forever). Returns the number of bytes written; 0 means the timeout passed.
pub fn recv(fd: i32, buf: &mut [u8], timeout_ms: i32) -> Result<usize, UiError> {
    let rc = unsafe { ui_recv(fd, buf.as_mut_ptr() as i32, buf.len() as i32, timeout_ms) };
    if rc < 0 { Err(UiError::from_code(rc)) } else { Ok(rc as usize) }
}

/// Copy pixel rows into a pixel surface or canvas resource (`target_id`),
/// in the target's format. `damage` lists the changed rectangles (empty =
/// the whole target); they are shown at the surface's next `Commit`.
pub fn put_pixels(fd: i32, target_id: u32, pixels: &[u8], damage: &[Rect]) -> Result<(), UiError> {
    let rc = unsafe {
        ui_put_pixels(
            fd,
            target_id as i32,
            pixels.as_ptr() as i32,
            pixels.len() as i32,
            damage.as_ptr() as i32,
            damage.len() as i32,
        )
    };
    if rc < 0 { Err(UiError::from_code(rc)) } else { Ok(()) }
}

/// Send an image resource's data ([`UPLOAD_PNG`] or [`UPLOAD_RGBA`]).
pub fn upload(fd: i32, resource_id: u32, kind: i32, data: &[u8]) -> Result<(), UiError> {
    let rc = unsafe { ui_upload(fd, resource_id as i32, kind, data.as_ptr() as i32, data.len() as i32) };
    if rc < 0 { Err(UiError::from_code(rc)) } else { Ok(()) }
}
