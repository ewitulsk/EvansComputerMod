//! Host functions for process management (spawn, wait, etc.)

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wasmtime::*;

use crate::fd::{self, FileDescriptor, NullFd, PipeFd};
use crate::filesystem::FileSystem;
use crate::network::NetworkState;
use crate::process::{ProcessManager, ProcessState};
use crate::wasm_host::HostState;

use super::memory;

pub const FUNCTIONS: &[&str] = &[
    "process_spawn",
    "process_wait",
    "process_wait_any",
    "process_kill",
    "process_list",
    "process_state",
];

/// Tracks the read ends of pipes connected to child process stdout.
/// When process_wait drains these, the data is written to the kernel's VTE.
pub struct ChildOutputPipes {
    pub pipes: HashMap<u32, PipeFd>,
}

impl ChildOutputPipes {
    pub fn new() -> Self {
        Self {
            pipes: HashMap::new(),
        }
    }
}

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // process_spawn(path_ptr, path_len, argv_ptr, argv_len, stdin_fd, stdout_fd, stderr_fd) -> pid
    linker.func_wrap(
        "env",
        "process_spawn",
        |mut caller: Caller<'_, HostState>,
         path_ptr: i32,
         path_len: i32,
         argv_ptr: i32,
         argv_len: i32,
         _stdin_fd: i32,
         _stdout_fd: i32,
         _stderr_fd: i32|
         -> i32 {
            // 1. Read path and argv from WASM memory
            let path = match memory::read_string(&mut caller, path_ptr, path_len) {
                Some(s) => s,
                None => return -1,
            };
            let argv_str = match memory::read_string(&mut caller, argv_ptr, argv_len) {
                Some(s) => s,
                None => return -1,
            };

            let argv: Vec<String> = if argv_str.is_empty() {
                vec![path.clone()]
            } else {
                argv_str.split('\n').map(|s| s.to_string()).collect()
            };

            // 2. Read the .wasm file from the filesystem
            let storage_path = caller.data().filesystem.storage_path().to_path_buf();
            let wasm_bytes = {
                let full_path = storage_path.join(&path);
                match std::fs::read(&full_path) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        eprintln!("[process_spawn] Failed to read {}: {}", full_path.display(), e);
                        return -1;
                    }
                }
            };

            // 3. Extract shutdown flag
            let shutdown = caller.data().shutdown.clone();

            // 4. Create a new filesystem rooted at the same storage dir
            let filesystem = FileSystem::new(storage_path);

            // 5. Create stdio FDs via pipes so process_wait can drain output
            //    to the kernel's VTE framebuffer.
            let stdin: Box<dyn crate::fd::FileDescriptor> = Box::new(NullFd);

            // Create pipe: child writes to write_end, parent reads from read_end.
            // Both stdout and stderr share the same pipe so all output goes to VTE.
            let (stdout_read, stdout_write, stderr_write) =
                fd::create_pipe_with_two_writers(16384);
            let stdout: Box<dyn crate::fd::FileDescriptor> = Box::new(stdout_write);
            let stderr: Box<dyn crate::fd::FileDescriptor> = Box::new(stderr_write);

            // 5b. Clone network state so WASI programs can use the networking stack
            let network = caller.data().get_custom::<NetworkState>().map(|net| {
                NetworkState::new(net.macs.clone(), net.hub.clone())
            });

            // 6. Get ProcessManager from custom storage and spawn
            let proc_mgr = match caller
                .data()
                .get_custom::<Arc<Mutex<ProcessManager>>>()
            {
                Some(pm) => pm.clone(),
                None => {
                    eprintln!("[process_spawn] No ProcessManager in HostState");
                    return -1;
                }
            };

            let name = path
                .rsplit('/')
                .next()
                .unwrap_or(&path)
                .trim_end_matches(".wasm")
                .to_string();

            let result = proc_mgr.lock().unwrap().spawn_with_network(
                &wasm_bytes,
                name,
                argv,
                vec![],
                stdin,
                stdout,
                stderr,
                filesystem,
                shutdown,
                network,
            );

            match result {
                Ok(pid) => {
                    // Store the read end of the stdout pipe so process_wait can drain it
                    if caller.data().get_custom::<ChildOutputPipes>().is_none() {
                        caller.data_mut().insert_custom(ChildOutputPipes::new());
                    }
                    let pipes = caller.data_mut().get_custom_mut::<ChildOutputPipes>().unwrap();
                    pipes.pipes.insert(pid, stdout_read);

                    pid as i32
                }
                Err(e) => {
                    eprintln!("[process_spawn] Failed: {}", e);
                    -1
                }
            }
        },
    )?;

    // process_wait(pid) -> exit_code or -1
    //
    // While waiting for the child to exit, we drain its stdout pipe and
    // write the data to the kernel's VTE framebuffer memory. This way
    // child output appears on screen automatically.
    linker.func_wrap(
        "env",
        "process_wait",
        |mut caller: Caller<'_, HostState>, pid: i32| -> i32 {
            let proc_mgr = match caller
                .data()
                .get_custom::<Arc<Mutex<ProcessManager>>>()
            {
                Some(pm) => pm.clone(),
                None => return -1,
            };

            // Take the stdout pipe read-end for this PID
            let mut pipe = caller
                .data_mut()
                .get_custom_mut::<ChildOutputPipes>()
                .and_then(|p| p.pipes.remove(&(pid as u32)));

            // We need to poll the pipe while waiting for the process to exit.
            // We can't hold a borrow on caller while blocking, so we use a
            // polling loop with short timeouts.
            loop {
                // Check if process has exited
                {
                    let pm = proc_mgr.lock().unwrap();
                    if let Some(entry) = pm.get(pid as u32) {
                        if entry.exit_code.is_some() {
                            // Process exited — drain remaining output and return
                            if let Some(ref mut pipe) = pipe {
                                drain_pipe_to_framebuffer(&mut caller, pipe);
                            }
                            let code = entry.exit_code.unwrap_or(-1);
                            drop(pm);
                            // Reap the process
                            proc_mgr.lock().unwrap().wait(pid as u32);
                            return code;
                        }
                    } else {
                        return -1;
                    }
                }

                // Drain any pending output from the child's stdout pipe
                if let Some(ref mut pipe) = pipe {
                    drain_pipe_to_framebuffer(&mut caller, pipe);
                }

                // Render the framebuffer while we wait. Since on_input is
                // blocked (we're inside process_wait), the worker_loop can't
                // render. We do it here so the display stays updated.
                render_from_caller(&mut caller);

                // Brief sleep to avoid busy-spinning
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        },
    )?;

    // process_wait_any(status_ptr) -> pid or -1  (stub for now)
    linker.func_wrap(
        "env",
        "process_wait_any",
        |_caller: Caller<'_, HostState>, _status_ptr: i32| -> i32 { -1 },
    )?;

    // process_kill(pid, signal) -> 0 ok, -1 error  (stub for now)
    linker.func_wrap(
        "env",
        "process_kill",
        |_caller: Caller<'_, HostState>, _pid: i32, _signal: i32| -> i32 { -1 },
    )?;

    // process_list(buf_ptr, buf_len) -> bytes written or -1
    linker.func_wrap(
        "env",
        "process_list",
        |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
            let proc_mgr = match caller
                .data()
                .get_custom::<Arc<Mutex<ProcessManager>>>()
            {
                Some(pm) => pm.clone(),
                None => return -1,
            };

            let pm = proc_mgr.lock().unwrap();
            let entries = pm.list();

            let mut result = String::from("[");
            for (i, entry) in entries.iter().enumerate() {
                if i > 0 {
                    result.push(',');
                }
                result.push_str(&format!(
                    "{{\"pid\":{},\"name\":\"{}\",\"state\":\"{}\"}}",
                    entry.pid,
                    entry.name,
                    match entry.state {
                        ProcessState::Running => "running",
                        ProcessState::Stopped => "stopped",
                        ProcessState::Zombie => "zombie",
                    }
                ));
            }
            result.push(']');
            drop(pm);

            let bytes = result.as_bytes();
            if bytes.len() > buf_len as usize {
                return -1;
            }
            memory::write_bytes(&mut caller, buf_ptr, bytes);
            bytes.len() as i32
        },
    )?;

    // process_state(pid) -> 0=running, 1=stopped, 2=zombie, -1=not found
    linker.func_wrap(
        "env",
        "process_state",
        |caller: Caller<'_, HostState>, pid: i32| -> i32 {
            let proc_mgr = match caller
                .data()
                .get_custom::<Arc<Mutex<ProcessManager>>>()
            {
                Some(pm) => pm.clone(),
                None => return -1,
            };

            let pm = proc_mgr.lock().unwrap();
            match pm.get(pid as u32) {
                Some(entry) => match entry.state {
                    ProcessState::Running => 0,
                    ProcessState::Stopped => 1,
                    ProcessState::Zombie => 2,
                },
                None => -1,
            }
        },
    )?;

    Ok(())
}

/// Drain available data from a pipe and write it to the kernel's VTE
/// by calling the kernel's `on_input`-style mechanism.
///
/// We write directly to the kernel WASM memory at the framebuffer region
/// by calling the kernel's terminal print function. Since we have access
/// to the WASM instance via the Caller, we write the data through the
/// kernel's VTE by invoking its `terminal_print` export (if available),
/// or by directly feeding bytes to the framebuffer.
/// Render the kernel's framebuffer from inside a host function call.
/// This is used during process_wait to keep the display updated while
/// the kernel is blocked waiting for a child process.
fn render_from_caller(caller: &mut Caller<'_, HostState>) {
    use crate::terminal_io::FB_BASE;
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(m)) => m,
        _ => return,
    };

    // Read actual dimensions from framebuffer header to size the copy correctly
    let data = memory.data(&*caller);
    if data.len() < FB_BASE + 16 { return; }
    let width = u16::from_le_bytes([data[FB_BASE + 2], data[FB_BASE + 3]]) as usize;
    let height = u16::from_le_bytes([data[FB_BASE + 4], data[FB_BASE + 5]]) as usize;
    let fb_size = 64 + width * height * 4;
    let end = FB_BASE + fb_size;
    if data.len() < end { return; }
    let fb_copy = data[FB_BASE..end].to_vec();

    let renderer = &mut caller.data_mut().renderer;
    let _ = renderer.render_from_fb_slice(&fb_copy);
}

/// Drain child output from a pipe and write directly to the WASM framebuffer memory.
/// This avoids re-entrant WASM calls and is fast (pure memory writes).
fn drain_pipe_to_framebuffer(caller: &mut Caller<'_, HostState>, pipe: &mut PipeFd) {
    use crate::terminal_io::FB_BASE;

    let mut buf = [0u8; 4096];
    let n = match pipe.try_read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };
    let data = &buf[..n];

    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(m)) => m,
        _ => return,
    };
    let mem = memory.data_mut(&mut *caller);
    let fb = FB_BASE;

    // Bounds check
    if mem.len() < fb + 64 + 80 * 24 * 4 { return; }

    let width = u16::from_le_bytes([mem[fb + 2], mem[fb + 3]]) as usize;
    let height = u16::from_le_bytes([mem[fb + 4], mem[fb + 5]]) as usize;
    if width == 0 || height == 0 { return; }

    let mut cx = u16::from_le_bytes([mem[fb + 6], mem[fb + 7]]) as usize;
    let mut cy = u16::from_le_bytes([mem[fb + 8], mem[fb + 9]]) as usize;
    let cell_base = fb + 64;
    for &b in data {
        match b {
            b'\n' => { cx = 0; cy += 1; }
            b'\r' => { cx = 0; }
            0x20..=0x7E => {
                // Scroll if past bottom
                if cy >= height {
                    let row_bytes = width * 4;
                    let src = cell_base + row_bytes;
                    let dst = cell_base;
                    mem.copy_within(src..src + row_bytes * (height - 1), dst);
                    let last = cell_base + row_bytes * (height - 1);
                    for i in 0..width {
                        mem[last + i * 4] = b' ';
                        mem[last + i * 4 + 1] = 0x07;
                        mem[last + i * 4 + 2] = 0;
                        mem[last + i * 4 + 3] = 0;
                    }
                    cy = height - 1;
                }
                let off = cell_base + (cy * width + cx) * 4;
                if off + 3 < mem.len() {
                    mem[off] = b;
                    mem[off + 1] = 0x07; // white on black
                    mem[off + 2] = 0;
                    mem[off + 3] = 0;
                }
                cx += 1;
                if cx >= width { cx = 0; cy += 1; }
            }
            _ => {} // Ignore other control chars
        }
    }

    // Write back cursor and bump dirty counter
    mem[fb + 6..fb + 8].copy_from_slice(&(cx as u16).to_le_bytes());
    mem[fb + 8..fb + 10].copy_from_slice(&(cy as u16).to_le_bytes());
    let dc_off = fb + 0x0C;
    let dc = u32::from_le_bytes([mem[dc_off], mem[dc_off + 1], mem[dc_off + 2], mem[dc_off + 3]]);
    mem[dc_off..dc_off + 4].copy_from_slice(&dc.wrapping_add(1).to_le_bytes());
}
