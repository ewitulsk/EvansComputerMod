//! IPC host function wrappers for intra-computer communication.

extern "C" {
    fn ipc_spawn_shell(user_ptr: i32, user_len: i32) -> i32;
    fn ipc_session_write(session_id: i32, buf_ptr: i32, buf_len: i32) -> i32;
    fn ipc_session_read(session_id: i32, buf_ptr: i32, buf_len: i32) -> i32;
    fn ipc_session_read_blocking(
        session_id: i32,
        buf_ptr: i32,
        buf_len: i32,
        timeout_ms: i32,
    ) -> i32;
    fn ipc_session_status(session_id: i32) -> i32;
    fn ipc_session_close(session_id: i32) -> i32;
    fn ipc_session_resize(session_id: i32, width: i32, height: i32) -> i32;
}

/// Spawn a new shell session for the given user. Returns session_id or -1.
pub fn spawn_shell(username: &str) -> i32 {
    unsafe { ipc_spawn_shell(username.as_ptr() as i32, username.len() as i32) }
}

/// Write input bytes to a shell session's stdin. Returns bytes written or -1.
pub fn session_write(session_id: i32, data: &[u8]) -> i32 {
    unsafe { ipc_session_write(session_id, data.as_ptr() as i32, data.len() as i32) }
}

/// Non-blocking read from a shell session's output. Returns bytes read, 0 if none, -1 on error.
pub fn session_read(session_id: i32, buf: &mut [u8]) -> i32 {
    unsafe { ipc_session_read(session_id, buf.as_mut_ptr() as i32, buf.len() as i32) }
}

/// Blocking read with timeout. Returns bytes read, 0 on timeout, -1 on error/closed.
pub fn session_read_blocking(session_id: i32, buf: &mut [u8], timeout_ms: i32) -> i32 {
    unsafe {
        ipc_session_read_blocking(
            session_id,
            buf.as_mut_ptr() as i32,
            buf.len() as i32,
            timeout_ms,
        )
    }
}

/// Query session status: 0=running, 1=exited, -1=invalid.
pub fn session_status(session_id: i32) -> i32 {
    unsafe { ipc_session_status(session_id) }
}

/// Close and clean up a shell session.
pub fn session_close(session_id: i32) -> i32 {
    unsafe { ipc_session_close(session_id) }
}

/// Resize the virtual TTY for an IPC session (e.g. on SSH window-change).
pub fn session_resize(session_id: i32, width: u16, height: u16) -> i32 {
    unsafe { ipc_session_resize(session_id, width as i32, height as i32) }
}
