//! The connection to the compositor, behind a trait so the toolkit runs
//! against the real UI fd ([`HostConnection`], wasm32 only) or a scripted
//! mock in tests ([`MockConnection`]).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Why a connection call failed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ConnError {
    /// Console mode: no compositor (host code -4).
    Console,
    /// The connection is closed (or the program is being killed).
    Closed,
    /// The shell role is taken (-2) / the app may not open a connection (-6).
    Refused,
    /// The compositor sent bytes that don't decode, or answered wrongly.
    Protocol,
    /// Any other host error code.
    Host(i32),
}

impl std::fmt::Display for ConnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnError::Console => write!(f, "no GUI is running (console mode)"),
            ConnError::Closed => write!(f, "the UI connection is closed"),
            ConnError::Refused => write!(f, "the compositor refused the connection"),
            ConnError::Protocol => write!(f, "UI protocol error"),
            ConnError::Host(c) => write!(f, "UI host error {c}"),
        }
    }
}

/// A UI connection: whole protocol messages in and out, plus the bulk paths.
pub trait Connection {
    /// Send one or more whole messages.
    fn send(&mut self, bytes: &[u8]) -> Result<(), ConnError>;
    /// Whole messages, waiting up to `timeout_ms` (< 0 forever). Empty = timeout.
    fn recv(&mut self, timeout_ms: i32) -> Result<Vec<u8>, ConnError>;
    /// Pixel rows for a pixel surface or canvas resource; damage = (x, y, w, h).
    fn put_pixels(&mut self, target: u32, pixels: &[u8], damage: &[[i32; 4]]) -> Result<(), ConnError>;
    /// An image resource's data (kind 0 = PNG, 1 = raw RGBA).
    fn upload(&mut self, resource: u32, kind: i32, data: &[u8]) -> Result<(), ConnError>;
    /// A monotonic clock in ms for caret blinking, double clicks and timers;
    /// `None` (scripted connections) turns those off, so tests stay deterministic.
    fn now_ms(&mut self) -> Option<u64> {
        None
    }
}

/// What a [`MockConnection`] recorded and still has to deliver.
#[derive(Default, Debug)]
pub struct MockState {
    /// Chunks `recv` returns, in order. When it runs dry `recv` reports the
    /// connection closed (or a timeout, with `idle_timeouts`).
    pub incoming: VecDeque<Vec<u8>>,
    /// Every byte sent, concatenated.
    pub sent: Vec<u8>,
    pub pixels: Vec<(u32, Vec<u8>, Vec<[i32; 4]>)>,
    pub uploads: Vec<(u32, i32, Vec<u8>)>,
    /// Return timeouts instead of `Closed` when `incoming` is empty.
    pub idle_timeouts: bool,
    /// The clock `now_ms` reports (None = no clock).
    pub now_ms: Option<u64>,
}

/// A scripted connection for tests. Clones share the same state, so a test
/// keeps one handle while the app owns another.
#[derive(Clone, Default)]
pub struct MockConnection {
    pub state: Rc<RefCell<MockState>>,
}

impl MockConnection {
    pub fn new() -> MockConnection {
        MockConnection::default()
    }

    /// Queue messages to be returned by one `recv`.
    pub fn push(&self, msgs: &[ecm_ui_proto::Message]) {
        self.state.borrow_mut().incoming.push_back(ecm_ui_proto::encode_all(msgs));
    }

    /// Queue raw bytes for one `recv`.
    pub fn push_bytes(&self, bytes: Vec<u8>) {
        self.state.borrow_mut().incoming.push_back(bytes);
    }

    /// Everything sent so far.
    pub fn sent(&self) -> Vec<u8> {
        self.state.borrow().sent.clone()
    }

    /// Everything sent so far, decoded (panics on bytes that don't decode:
    /// the toolkit must only send valid messages).
    pub fn sent_messages(&self) -> Vec<ecm_ui_proto::Message> {
        ecm_ui_proto::Message::decode_all(&self.state.borrow().sent).expect("toolkit sent an invalid message")
    }

    /// Forget what was sent so far.
    pub fn clear_sent(&self) {
        self.state.borrow_mut().sent.clear();
    }
}

impl Connection for MockConnection {
    fn send(&mut self, bytes: &[u8]) -> Result<(), ConnError> {
        // Every send must be whole, valid messages.
        ecm_ui_proto::Message::decode_all(bytes).map_err(|_| ConnError::Protocol)?;
        self.state.borrow_mut().sent.extend_from_slice(bytes);
        Ok(())
    }

    fn recv(&mut self, _timeout_ms: i32) -> Result<Vec<u8>, ConnError> {
        let mut s = self.state.borrow_mut();
        match s.incoming.pop_front() {
            Some(b) => Ok(b),
            None if s.idle_timeouts => Ok(Vec::new()),
            None => Err(ConnError::Closed),
        }
    }

    fn put_pixels(&mut self, target: u32, pixels: &[u8], damage: &[[i32; 4]]) -> Result<(), ConnError> {
        self.state.borrow_mut().pixels.push((target, pixels.to_vec(), damage.to_vec()));
        Ok(())
    }

    fn upload(&mut self, resource: u32, kind: i32, data: &[u8]) -> Result<(), ConnError> {
        self.state.borrow_mut().uploads.push((resource, kind, data.to_vec()));
        Ok(())
    }

    fn now_ms(&mut self) -> Option<u64> {
        self.state.borrow().now_ms
    }
}

/// The real UI fd (`ecm_host_abi::ui`).
#[cfg(target_arch = "wasm32")]
pub struct HostConnection {
    fd: i32,
    buf: Vec<u8>,
}

#[cfg(target_arch = "wasm32")]
fn host_err(e: ecm_host_abi::ui::UiError) -> ConnError {
    use ecm_host_abi::ui::UiError;
    match e {
        UiError::NoDevice => ConnError::Console,
        UiError::Interrupted => ConnError::Closed,
        UiError::Busy | UiError::NotPermitted => ConnError::Refused,
        UiError::Invalid => ConnError::Protocol,
        UiError::Other(c) => ConnError::Host(c),
    }
}

#[cfg(target_arch = "wasm32")]
impl HostConnection {
    /// Open the UI fd in `role` (0 app, 1 shell).
    pub fn open(role: i32) -> Result<HostConnection, ConnError> {
        let fd = ecm_host_abi::ui::open(role, ecm_ui_proto::VERSION).map_err(host_err)?;
        Ok(HostConnection { fd, buf: vec![0; ecm_ui_proto::MAX_MESSAGE as usize + ecm_ui_proto::HEADER_LEN] })
    }
}

#[cfg(target_arch = "wasm32")]
impl Connection for HostConnection {
    fn send(&mut self, bytes: &[u8]) -> Result<(), ConnError> {
        ecm_host_abi::ui::send(self.fd, bytes).map_err(host_err)
    }
    fn recv(&mut self, timeout_ms: i32) -> Result<Vec<u8>, ConnError> {
        let n = ecm_host_abi::ui::recv(self.fd, &mut self.buf, timeout_ms).map_err(host_err)?;
        Ok(self.buf[..n].to_vec())
    }
    fn put_pixels(&mut self, target: u32, pixels: &[u8], damage: &[[i32; 4]]) -> Result<(), ConnError> {
        let rects: Vec<ecm_host_abi::ui::Rect> =
            damage.iter().map(|d| ecm_host_abi::ui::Rect { x: d[0], y: d[1], w: d[2], h: d[3] }).collect();
        ecm_host_abi::ui::put_pixels(self.fd, target, pixels, &rects).map_err(host_err)
    }
    fn upload(&mut self, resource: u32, kind: i32, data: &[u8]) -> Result<(), ConnError> {
        ecm_host_abi::ui::upload(self.fd, resource, kind, data).map_err(host_err)
    }
    fn now_ms(&mut self) -> Option<u64> {
        Some(monotonic_ms())
    }
}

/// Milliseconds since the program's first call (WASI monotonic clock).
#[cfg(target_arch = "wasm32")]
pub fn monotonic_ms() -> u64 {
    use std::sync::OnceLock;
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// The default connection for `App::new`: the UI fd on wasm32; elsewhere
/// there is no compositor.
pub fn default_connection(role: i32) -> Result<Box<dyn Connection>, ConnError> {
    #[cfg(target_arch = "wasm32")]
    {
        Ok(Box::new(HostConnection::open(role)?))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = role;
        Err(ConnError::Console)
    }
}
