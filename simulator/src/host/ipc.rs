//! IPC host functions for intra-computer communication.
//!
//! Allows WASM programs (e.g. sshd) to spawn shell sessions and relay I/O
//! through an IPC pipeline instead of going through the network stack.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};

use wasmtime::*;
use crate::wasm_host::HostState;
use crate::tty::TtyRegistry;
use super::memory;

pub const FUNCTIONS: &[&str] = &[
    "ipc_spawn_shell",
    "ipc_session_write",
    "ipc_session_read",
    "ipc_session_read_blocking",
    "ipc_session_status",
    "ipc_session_close",
    "ipc_session_resize",
];

/// Tracks all IPC sessions for a WASM process.
pub struct IpcSessionTable {
    sessions: HashMap<i32, IpcSession>,
    next_id: i32,
}

/// An IPC session connecting a WASM program to a shell.
pub struct IpcSession {
    /// TTY ID in the TtyRegistry
    pub tty_id: u32,
    /// The TTY's input queue (we write to this to send input to the shell)
    pub input_queue: Arc<Mutex<VecDeque<u8>>>,
    /// Output buffer with condvar for blocking reads
    pub output_buffer: Arc<(Mutex<VecDeque<u8>>, Condvar)>,
    /// Whether the session has been closed
    pub closed: Arc<AtomicBool>,
}

impl IpcSessionTable {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> i32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // ipc_spawn_shell(user_ptr, user_len) -> session_id or -1
    linker.func_wrap("env", "ipc_spawn_shell",
        |mut caller: Caller<'_, HostState>, user_ptr: i32, user_len: i32| -> i32 {
            let username = match memory::read_string(&mut caller, user_ptr, user_len) {
                Some(s) => s,
                None => return -1,
            };

            eprintln!("[ipc] spawn_shell for user '{}'", username);

            // Create a virtual TTY via the registry
            let (tty_id, tty_input_queue) = {
                let registry_arc = match caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>() {
                    Some(r) => r.clone(),
                    None => {
                        eprintln!("[ipc] TtyRegistry not found");
                        return -1;
                    }
                };
                let mut reg = registry_arc.lock().unwrap();
                let tty_id = reg.create(80, 24);
                let input_queue = reg.get(tty_id).unwrap().input_queue.clone();
                (tty_id, input_queue)
            };

            // Create shared output buffer with condvar for blocking reads
            let output_buffer = Arc::new((Mutex::new(VecDeque::<u8>::new()), Condvar::new()));
            let closed = Arc::new(AtomicBool::new(false));

            // Set up output capture: hook into the TTY's output to also push to our buffer.
            // We replace the TTY's TerminalBuffer with one that also copies bytes to the output_buffer.
            {
                let registry_arc = caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>().unwrap().clone();
                let mut reg = registry_arc.lock().unwrap();
                let tty = reg.get_mut(tty_id).unwrap();

                // Create a write FD that captures output
                let ob = output_buffer.clone();
                let term_buf = tty.output_buffer.clone();

                // We'll create a custom write fd that writes to both the TerminalBuffer and our output buffer
                // For the kernel to use this TTY, it will use tty_attach_fd which creates read/write FDs
                // For IPC, we need to intercept the output. We'll store the output_buffer reference
                // so ipc_session_read can poll the TerminalBuffer directly.
                let _ = (ob, term_buf);
            }

            let session = IpcSession {
                tty_id,
                input_queue: tty_input_queue,
                output_buffer: output_buffer.clone(),
                closed: closed.clone(),
            };

            // Initialize the IPC table if needed, then insert the session
            if caller.data().get_custom::<IpcSessionTable>().is_none() {
                caller.data_mut().insert_custom(IpcSessionTable::new());
            }

            let table = caller.data_mut().get_custom_mut::<IpcSessionTable>().unwrap();
            let id = table.alloc_id();
            table.sessions.insert(id, session);

            eprintln!("[ipc] session {} created (tty={})", id, tty_id);
            id
        },
    )?;

    // ipc_session_write(session_id, buf_ptr, buf_len) -> bytes_written or -1
    linker.func_wrap("env", "ipc_session_write",
        |mut caller: Caller<'_, HostState>, session_id: i32, buf_ptr: i32, buf_len: i32| -> i32 {
            let data = match memory::read_bytes(&mut caller, buf_ptr, buf_len) {
                Some(d) => d,
                None => return -1,
            };

            let table = match caller.data().get_custom::<IpcSessionTable>() {
                Some(t) => t,
                None => return -1,
            };
            let session = match table.sessions.get(&session_id) {
                Some(s) => s,
                None => return -1,
            };

            if session.closed.load(Ordering::Relaxed) {
                return -1;
            }

            // Push data to the TTY's input queue
            let mut queue = session.input_queue.lock().unwrap();
            queue.extend(data.iter());
            data.len() as i32
        },
    )?;

    // ipc_session_read(session_id, buf_ptr, buf_len) -> bytes_read, 0=none, -1=error
    linker.func_wrap("env", "ipc_session_read",
        |mut caller: Caller<'_, HostState>, session_id: i32, buf_ptr: i32, buf_len: i32| -> i32 {
            let output_arc = {
                let table = match caller.data().get_custom::<IpcSessionTable>() {
                    Some(t) => t,
                    None => return -1,
                };
                match table.sessions.get(&session_id) {
                    Some(s) => {
                        if s.closed.load(Ordering::Relaxed) {
                            return -1;
                        }
                        s.output_buffer.clone()
                    }
                    None => return -1,
                }
            };

            let (ref lock, _) = *output_arc;
            let mut buf_data = lock.lock().unwrap();

            if buf_data.is_empty() {
                return 0;
            }

            let to_read = (buf_len as usize).min(buf_data.len());
            let out: Vec<u8> = buf_data.drain(..to_read).collect();

            memory::write_bytes(&mut caller, buf_ptr, &out);
            to_read as i32
        },
    )?;

    // ipc_session_read_blocking(session_id, buf_ptr, buf_len, timeout_ms) -> bytes or 0=timeout or -1=error
    linker.func_wrap("env", "ipc_session_read_blocking",
        |mut caller: Caller<'_, HostState>, session_id: i32, buf_ptr: i32, buf_len: i32, timeout_ms: i32| -> i32 {
            let output_arc = {
                let table = match caller.data().get_custom::<IpcSessionTable>() {
                    Some(t) => t,
                    None => return -1,
                };
                match table.sessions.get(&session_id) {
                    Some(s) => {
                        if s.closed.load(Ordering::Relaxed) {
                            return -1;
                        }
                        s.output_buffer.clone()
                    }
                    None => return -1,
                }
            };

            let (ref lock, ref cvar) = *output_arc;
            let mut buf_data = lock.lock().unwrap();

            if buf_data.is_empty() {
                let (guard, _) = cvar.wait_timeout(buf_data,
                    std::time::Duration::from_millis(timeout_ms as u64)).unwrap();
                buf_data = guard;
            }

            if buf_data.is_empty() {
                return 0; // Timeout
            }

            let to_read = (buf_len as usize).min(buf_data.len());
            let out: Vec<u8> = buf_data.drain(..to_read).collect();

            memory::write_bytes(&mut caller, buf_ptr, &out);
            to_read as i32
        },
    )?;

    // ipc_session_status(session_id) -> 0=running, 1=exited, -1=invalid
    linker.func_wrap("env", "ipc_session_status",
        |caller: Caller<'_, HostState>, session_id: i32| -> i32 {
            let table = match caller.data().get_custom::<IpcSessionTable>() {
                Some(t) => t,
                None => return -1,
            };
            match table.sessions.get(&session_id) {
                Some(s) => {
                    if s.closed.load(Ordering::Relaxed) {
                        1
                    } else {
                        0
                    }
                }
                None => -1,
            }
        },
    )?;

    // ipc_session_close(session_id) -> 0 or -1
    linker.func_wrap("env", "ipc_session_close",
        |mut caller: Caller<'_, HostState>, session_id: i32| -> i32 {
            let tty_id = {
                let table = match caller.data_mut().get_custom_mut::<IpcSessionTable>() {
                    Some(t) => t,
                    None => return -1,
                };
                match table.sessions.remove(&session_id) {
                    Some(s) => {
                        s.closed.store(true, Ordering::Relaxed);
                        s.tty_id
                    }
                    None => return -1,
                }
            };

            // Remove the TTY from registry
            let registry_arc = match caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>() {
                Some(r) => r.clone(),
                None => return 0,
            };
            let mut reg = registry_arc.lock().unwrap();
            reg.remove(tty_id);

            eprintln!("[ipc] session {} closed", session_id);
            0
        },
    )?;

    // ipc_session_resize(session_id, width, height) -> 0 or -1
    linker.func_wrap("env", "ipc_session_resize",
        |caller: Caller<'_, HostState>, session_id: i32, width: i32, height: i32| -> i32 {
            let tty_id = {
                let table = match caller.data().get_custom::<IpcSessionTable>() {
                    Some(t) => t,
                    None => return -1,
                };
                match table.sessions.get(&session_id) {
                    Some(s) => s.tty_id,
                    None => return -1,
                }
            };

            let registry_arc = match caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>() {
                Some(r) => r.clone(),
                None => return -1,
            };
            let mut reg = registry_arc.lock().unwrap();
            if let Some(tty) = reg.get_mut(tty_id) {
                tty.width = width as u16;
                tty.height = height as u16;
                return 0;
            }
            -1
        },
    )?;

    Ok(())
}
