//! The server's own link to the official package repository
//! (abi/child-abi.toml `[imports.repo]`, docs/REPO.md).
//!
//! When the server offers it to this computer, a program can talk to the
//! repository without any in-game network: the Minecraft server makes the
//! HTTPS request (only ever to the repository it is configured for) and hands
//! the answer back. `pkg` uses it for `host://repo/...` repositories.
//!
//! A request is polled like an upload: [`begin`], then [`poll`] and [`read`]
//! until it is done.

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn repo_supported() -> i32;
    fn repo_begin(req_ptr: i32, req_len: i32) -> i32;
    fn repo_poll(ticket: i32, out_ptr: i32, cap: i32) -> i32;
    fn repo_read(ticket: i32, buf_ptr: i32, cap: i32) -> i32;
    fn repo_error(ticket: i32, buf_ptr: i32, cap: i32) -> i32;
    fn repo_close(ticket: i32) -> i32;
}

/// Whether this computer may use the server's link.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Support {
    /// The server has no link (no internet add-on, or the repository is switched off).
    NotOffered,
    Ready,
    /// The server keeps its link for repository servers; this computer is not one.
    NotThisComputer,
}

impl Support {
    pub fn message(self) -> &'static str {
        match self {
            Support::NotOffered => "this server does not link to the package repository itself (use an https:// repository over the network)",
            Support::Ready => "ready",
            Support::NotThisComputer => "this server only lets its repository servers use that link; get packages from one of them over the network",
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn supported() -> Support {
    match unsafe { repo_supported() } {
        1 => Support::Ready,
        2 => Support::NotThisComputer,
        _ => Support::NotOffered,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn supported() -> Support {
    Support::NotOffered
}

/// Where a request is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// False while the server is still connecting; `status` and `total` are set once true.
    pub answered: bool,
    pub done: bool,
    pub status: u16,
    pub received: u64,
    /// The body's length when the repository said so.
    pub total: Option<u64>,
}

/// One request: the method, the path under the repository's origin, headers and a body.
pub fn encode_request(method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Vec<u8> {
    let mut out = alloc::format!("{method} {path}\n").into_bytes();
    for (k, v) in headers {
        out.extend_from_slice(alloc::format!("{k}: {v}\n").as_bytes());
    }
    out.push(b'\n');
    out.extend_from_slice(body);
    out
}

/// An open request.
pub struct Ticket(#[allow(dead_code)] i32);

fn code_message(code: i32) -> &'static str {
    match code {
        -1 => "bad request",
        -2 => "too many requests at once",
        -4 => Support::NotOffered.message(),
        -5 => "interrupted",
        -13 => Support::NotThisComputer.message(),
        _ => "the request failed",
    }
}

#[cfg(target_arch = "wasm32")]
impl Ticket {
    /// Start a request (see [`encode_request`]).
    pub fn begin(request: &[u8]) -> Result<Ticket, String> {
        let rc = unsafe { repo_begin(request.as_ptr() as i32, request.len() as i32) };
        if rc > 0 {
            Ok(Ticket(rc))
        } else {
            Err(String::from(code_message(rc)))
        }
    }

    /// Where it is now. `Err` carries the reason it failed.
    pub fn poll(&self) -> Result<Progress, String> {
        let mut out = [0u8; 24];
        let rc = unsafe { repo_poll(self.0, out.as_mut_ptr() as i32, out.len() as i32) };
        if rc < 0 {
            let mut buf = [0u8; 256];
            let n = unsafe { repo_error(self.0, buf.as_mut_ptr() as i32, buf.len() as i32) };
            let why = if n > 0 { String::from_utf8_lossy(&buf[..(n as usize).min(buf.len())]).into_owned() } else { String::from(code_message(rc)) };
            return Err(why);
        }
        let status = i32::from_le_bytes([out[0], out[1], out[2], out[3]]);
        let received = u64::from_le_bytes(out[4..12].try_into().unwrap_or([0; 8]));
        let total = u64::from_le_bytes(out[12..20].try_into().unwrap_or([0; 8]));
        Ok(Progress { answered: rc >= 1, done: rc == 2, status: status.max(0) as u16, received, total: (total != u64::MAX).then_some(total) })
    }

    /// Body bytes that have arrived (0: none right now, or the end once `done`).
    pub fn read(&self, buf: &mut [u8]) -> usize {
        let n = unsafe { repo_read(self.0, buf.as_mut_ptr() as i32, buf.len() as i32) };
        n.max(0) as usize
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Ticket {
    pub fn begin(_request: &[u8]) -> Result<Ticket, String> {
        Err(String::from(code_message(-4)))
    }

    pub fn poll(&self) -> Result<Progress, String> {
        Err(String::from(code_message(-4)))
    }

    pub fn read(&self, _buf: &mut [u8]) -> usize {
        0
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            repo_close(self.0);
        }
    }
}
