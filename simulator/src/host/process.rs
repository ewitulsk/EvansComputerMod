//! Process management host functions for the kernel.
//!
//! Provides host functions in the "env" namespace that let the kernel
//! spawn, wait for, and manage child processes.

use std::sync::{Arc, Mutex};

use wasmtime::*;

use crate::fd::NullFd;
use crate::filesystem::FileSystem;
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

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // process_spawn(path_ptr, path_len, argv_ptr, argv_len, stdin_fd, stdout_fd, stderr_fd) -> pid or -1
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

            let argv_str = if argv_len > 0 {
                match memory::read_string(&mut caller, argv_ptr, argv_len) {
                    Some(s) => s,
                    None => return -1,
                }
            } else {
                String::new()
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

            // 5. Create stdio FDs - use NullFd for simplicity (terminal FDs will be refined later)
            let stdin: Box<dyn crate::fd::FileDescriptor> = Box::new(NullFd);
            let stdout: Box<dyn crate::fd::FileDescriptor> = Box::new(NullFd);
            let stderr: Box<dyn crate::fd::FileDescriptor> = Box::new(NullFd);

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

            let result = proc_mgr.lock().unwrap().spawn(
                &wasm_bytes,
                name,
                argv,
                vec![],
                stdin,
                stdout,
                stderr,
                filesystem,
                shutdown,
            );

            match result {
                Ok(pid) => pid as i32,
                Err(e) => {
                    eprintln!("[process_spawn] Failed: {}", e);
                    -1
                }
            }
        },
    )?;

    // process_wait(pid) -> exit_code or -1
    linker.func_wrap(
        "env",
        "process_wait",
        |caller: Caller<'_, HostState>, pid: i32| -> i32 {
            let proc_mgr = match caller
                .data()
                .get_custom::<Arc<Mutex<ProcessManager>>>()
            {
                Some(pm) => pm.clone(),
                None => return -1,
            };

            // Drop the caller borrow before blocking on wait
            drop(caller);

            let result = proc_mgr.lock().unwrap().wait(pid as u32);
            match result {
                Some(code) => code,
                None => -1,
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

    // process_list(buf_ptr, buf_len) -> bytes_written or -1
    linker.func_wrap(
        "env",
        "process_list",
        |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
            let json = {
                let proc_mgr = match caller
                    .data()
                    .get_custom::<Arc<Mutex<ProcessManager>>>()
                {
                    Some(pm) => pm.clone(),
                    None => return -1,
                };

                let mgr = proc_mgr.lock().unwrap();
                let entries: Vec<String> = mgr
                    .list()
                    .iter()
                    .map(|e| {
                        let state_str = match e.state {
                            ProcessState::Running => "running",
                            ProcessState::Stopped => "stopped",
                            ProcessState::Zombie => "zombie",
                        };
                        format!(
                            "{{\"pid\":{},\"name\":\"{}\",\"state\":\"{}\"}}",
                            e.pid,
                            e.name.replace('\"', "\\\""),
                            state_str
                        )
                    })
                    .collect();
                format!("[{}]", entries.join(","))
            };

            let json_bytes = json.as_bytes();
            if json_bytes.len() > buf_len as usize {
                return -1;
            }

            memory::write_bytes(&mut caller, buf_ptr, json_bytes);
            json_bytes.len() as i32
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

            let mgr = proc_mgr.lock().unwrap();
            match mgr.get(pid as u32) {
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
