//! The app manager's control channel (docs/gui/APPS.md, "app_ctl").
//!
//! One request per call: newline-separated words, the command first, e.g.
//! `reindex`, `list`, `launch\n<id>\n<arg>...`, `status\n<pid>`, `output\n<pid>`,
//! `kill\n<pid>`, `perm\nlist`, `perm\ngrant\n<id>\n<permission>`, `whoami`.
//! The Java app manager (computer/apps/AppManager.java) answers with a status and
//! a reply. Apps (confined processes) may only use `whoami`, unless they hold the
//! `packages` permission. Used by `pkg`.

use alloc::vec::Vec;

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn app_ctl(req_ptr: i32, req_len: i32, reply_ptr: i32, reply_cap: i32, status_ptr: i32) -> i32;
}

/// Status codes.
pub const OK: i32 = 0;
pub const ENOENT: i32 = -2;
pub const EACCES: i32 = -13;
pub const EINVAL: i32 = -22;

/// Send one request (words joined with '\n'). Returns `(status, reply)`; `None` if
/// there is no app manager (simulator, off-target builds).
pub fn ctl(words: &[&str]) -> Option<(i32, Vec<u8>)> {
    let mut req = Vec::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            req.push(b'\n');
        }
        req.extend_from_slice(w.as_bytes());
    }
    ctl_bytes(&req)
}

/// Largest reply read (the host never hands over more than `cap` bytes of `output`).
pub const REPLY_CAP: usize = 65536;

#[cfg(target_arch = "wasm32")]
pub fn ctl_bytes(req: &[u8]) -> Option<(i32, Vec<u8>)> {
    let mut reply = alloc::vec![0u8; REPLY_CAP];
    let mut status: i32 = 0;
    let n = unsafe {
        app_ctl(
            req.as_ptr() as i32,
            req.len() as i32,
            reply.as_mut_ptr() as i32,
            reply.len() as i32,
            &mut status as *mut i32 as i32,
        )
    };
    if n < 0 {
        return None;
    }
    reply.truncate((n as usize).min(REPLY_CAP));
    Some((status, reply))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn ctl_bytes(_req: &[u8]) -> Option<(i32, Vec<u8>)> {
    None
}
