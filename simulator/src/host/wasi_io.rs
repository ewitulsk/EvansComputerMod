//! WASI snapshot_preview1 host functions.
//!
//! Registers the Tier 1 WASI functions in the `"wasi_snapshot_preview1"` namespace
//! so that WASI-compiled binaries can run on the simulator.

use wasmtime::*;
use crate::fd::FdTable;
use crate::host::memory;
use crate::host::wasi_stubs;
use crate::wasi::*;
use crate::wasm_host::HostState;

/// Per-instance WASI state (args, environment variables).
pub struct WasiState {
    pub argv: Vec<String>,
    pub env_vars: Vec<(String, String)>,
}

const WASI_NS: &str = "wasi_snapshot_preview1";

/// Register all WASI snapshot_preview1 host functions on the linker.
pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    register_fd_write(linker)?;
    register_fd_read(linker)?;
    register_fd_close(linker)?;
    register_fd_seek(linker)?;
    register_fd_fdstat_get(linker)?;
    register_fd_prestat_get(linker)?;
    register_fd_prestat_dir_name(linker)?;
    register_fd_fdstat_set_flags(linker)?;
    register_proc_exit(linker)?;
    register_args_sizes_get(linker)?;
    register_args_get(linker)?;
    register_environ_sizes_get(linker)?;
    register_environ_get(linker)?;
    register_clock_time_get(linker)?;
    register_random_get(linker)?;
    register_sched_yield(linker)?;
    register_path_open(linker)?;
    register_path_create_directory(linker)?;
    register_path_remove_directory(linker)?;
    register_path_unlink_file(linker)?;
    register_path_filestat_get(linker)?;
    register_fd_readdir(linker)?;
    wasi_stubs::register(linker)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_write — scatter-gather write
// ---------------------------------------------------------------------------
fn register_fd_write(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "fd_write",
        |mut caller: Caller<'_, HostState>,
         fd: i32,
         iovs_ptr: i32,
         iovs_len: i32,
         nwritten_ptr: i32|
         -> i32 {
            let mut total: u32 = 0;

            for i in 0..iovs_len {
                // Each iovec is 8 bytes: (buf_ptr: u32, buf_len: u32)
                let iov_addr = iovs_ptr + i * 8;
                let iov_bytes = match memory::read_bytes(&mut caller, iov_addr, 8) {
                    Some(b) => b,
                    None => return ERRNO_INVAL,
                };
                let buf_ptr =
                    u32::from_le_bytes([iov_bytes[0], iov_bytes[1], iov_bytes[2], iov_bytes[3]])
                        as i32;
                let buf_len =
                    u32::from_le_bytes([iov_bytes[4], iov_bytes[5], iov_bytes[6], iov_bytes[7]])
                        as i32;

                if buf_len == 0 {
                    continue;
                }

                let data = match memory::read_bytes(&mut caller, buf_ptr, buf_len) {
                    Some(d) => d,
                    None => return ERRNO_INVAL,
                };

                // Write through FdTable
                let written = {
                    let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                        Some(t) => t,
                        None => return ERRNO_BADF,
                    };
                    let descriptor = match fd_table.get_mut(fd) {
                        Some(d) => d,
                        None => return ERRNO_BADF,
                    };
                    match descriptor.write(&data) {
                        Ok(n) => n as u32,
                        Err(_) => return ERRNO_BADF,
                    }
                };
                total += written;
            }

            // Write total bytes written to nwritten_ptr
            memory::write_bytes(&mut caller, nwritten_ptr, &total.to_le_bytes());
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_read — scatter-gather read
// ---------------------------------------------------------------------------
fn register_fd_read(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "fd_read",
        |mut caller: Caller<'_, HostState>,
         fd: i32,
         iovs_ptr: i32,
         iovs_len: i32,
         nread_ptr: i32|
         -> i32 {
            let mut total: u32 = 0;

            for i in 0..iovs_len {
                let iov_addr = iovs_ptr + i * 8;
                let iov_bytes = match memory::read_bytes(&mut caller, iov_addr, 8) {
                    Some(b) => b,
                    None => return ERRNO_INVAL,
                };
                let buf_ptr =
                    u32::from_le_bytes([iov_bytes[0], iov_bytes[1], iov_bytes[2], iov_bytes[3]])
                        as i32;
                let buf_len =
                    u32::from_le_bytes([iov_bytes[4], iov_bytes[5], iov_bytes[6], iov_bytes[7]])
                        as i32;

                if buf_len == 0 {
                    continue;
                }

                // Read from FD into temp buffer
                let read_result = {
                    let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                        Some(t) => t,
                        None => return ERRNO_BADF,
                    };
                    let descriptor = match fd_table.get_mut(fd) {
                        Some(d) => d,
                        None => return ERRNO_BADF,
                    };
                    let mut temp = vec![0u8; buf_len as usize];
                    match descriptor.read(&mut temp) {
                        Ok(n) => Ok((temp, n)),
                        Err(_) => Err(()),
                    }
                };

                match read_result {
                    Ok((buf, n)) => {
                        if n > 0 {
                            memory::write_bytes(&mut caller, buf_ptr, &buf[..n]);
                        }
                        total += n as u32;
                        // Short read means no more data available right now
                        if (n as i32) < buf_len {
                            break;
                        }
                    }
                    Err(_) => return ERRNO_BADF,
                }
            }

            memory::write_bytes(&mut caller, nread_ptr, &total.to_le_bytes());
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_close
// ---------------------------------------------------------------------------
fn register_fd_close(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(WASI_NS, "fd_close", |mut caller: Caller<'_, HostState>, fd: i32| -> i32 {
        let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
            Some(t) => t,
            None => return ERRNO_BADF,
        };
        if fd_table.close(fd) {
            ERRNO_SUCCESS
        } else {
            ERRNO_BADF
        }
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_seek — stub returning ENOSYS
// ---------------------------------------------------------------------------
fn register_fd_seek(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "fd_seek",
        |_caller: Caller<'_, HostState>,
         _fd: i32,
         _offset: i64,
         _whence: i32,
         _newoffset_ptr: i32|
         -> i32 { ERRNO_NOSYS },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_fdstat_get — returns a 24-byte fdstat struct
// ---------------------------------------------------------------------------
fn register_fd_fdstat_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "fd_fdstat_get",
        |mut caller: Caller<'_, HostState>, fd: i32, buf_ptr: i32| -> i32 {
            // Check if fd exists
            let exists = {
                let fd_table = match caller.data().get_custom::<FdTable>() {
                    Some(t) => t,
                    None => return ERRNO_BADF,
                };
                fd_table.contains(fd)
            };
            if !exists {
                return ERRNO_BADF;
            }

            // Build the 24-byte __wasi_fdstat_t:
            //   u8  fs_filetype         (offset 0)
            //   u16 fs_flags            (offset 2, after 1 byte padding)
            //   u64 fs_rights_base      (offset 8)
            //   u64 fs_rights_inheriting (offset 16)
            let mut stat = [0u8; 24];
            // filetype: character device for stdio (0-2), regular file otherwise
            stat[0] = if fd <= 2 {
                FILETYPE_CHARACTER_DEVICE
            } else {
                FILETYPE_REGULAR_FILE
            };
            // Grant all rights (simplified)
            let all_rights: u64 = u64::MAX;
            stat[8..16].copy_from_slice(&all_rights.to_le_bytes());
            stat[16..24].copy_from_slice(&all_rights.to_le_bytes());

            memory::write_bytes(&mut caller, buf_ptr, &stat);
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_prestat_get — fd=3 returns preopened dir "/"
// ---------------------------------------------------------------------------
fn register_fd_prestat_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "fd_prestat_get",
        |mut caller: Caller<'_, HostState>, fd: i32, buf_ptr: i32| -> i32 {
            if fd != 3 {
                return ERRNO_BADF;
            }
            // __wasi_prestat_t: u8 pr_type (0 = dir), then padding, then u32 pr_name_len
            // Total 8 bytes: type(1) + pad(3) + name_len(4)
            let mut prestat = [0u8; 8];
            prestat[0] = PREOPENTYPE_DIR;
            let name = b"/";
            prestat[4..8].copy_from_slice(&(name.len() as u32).to_le_bytes());
            memory::write_bytes(&mut caller, buf_ptr, &prestat);
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_prestat_dir_name
// ---------------------------------------------------------------------------
fn register_fd_prestat_dir_name(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "fd_prestat_dir_name",
        |mut caller: Caller<'_, HostState>, fd: i32, path_ptr: i32, path_len: i32| -> i32 {
            if fd != 3 {
                return ERRNO_BADF;
            }
            let name = b"/";
            if (path_len as usize) < name.len() {
                return ERRNO_INVAL;
            }
            memory::write_bytes(&mut caller, path_ptr, name);
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_fdstat_set_flags — no-op, return SUCCESS
// ---------------------------------------------------------------------------
fn register_fd_fdstat_set_flags(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "fd_fdstat_set_flags",
        |_caller: Caller<'_, HostState>, _fd: i32, _flags: i32| -> i32 { ERRNO_SUCCESS },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// proc_exit — trap the wasm instance
// ---------------------------------------------------------------------------
fn register_proc_exit(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "proc_exit",
        |_caller: Caller<'_, HostState>, code: i32| -> Result<()> {
            Err(anyhow::anyhow!("proc_exit({})", code))
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// args_sizes_get
// ---------------------------------------------------------------------------
fn register_args_sizes_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "args_sizes_get",
        |mut caller: Caller<'_, HostState>, argc_ptr: i32, argv_buf_size_ptr: i32| -> i32 {
            let (argc, buf_size) = match caller.data().get_custom::<WasiState>() {
                Some(state) => {
                    let argc = state.argv.len() as u32;
                    let buf_size: u32 = state
                        .argv
                        .iter()
                        .map(|a| a.len() as u32 + 1) // +1 for NUL
                        .sum();
                    (argc, buf_size)
                }
                None => (0u32, 0u32),
            };
            memory::write_bytes(&mut caller, argc_ptr, &argc.to_le_bytes());
            memory::write_bytes(&mut caller, argv_buf_size_ptr, &buf_size.to_le_bytes());
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// args_get
// ---------------------------------------------------------------------------
fn register_args_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "args_get",
        |mut caller: Caller<'_, HostState>, argv_ptr: i32, argv_buf_ptr: i32| -> i32 {
            let args: Vec<String> = match caller.data().get_custom::<WasiState>() {
                Some(state) => state.argv.clone(),
                None => vec![],
            };

            let mut buf_offset: u32 = 0;
            for (i, arg) in args.iter().enumerate() {
                // Write pointer to current arg in the argv array
                let ptr_val = (argv_buf_ptr as u32) + buf_offset;
                memory::write_bytes(
                    &mut caller,
                    argv_ptr + (i as i32) * 4,
                    &ptr_val.to_le_bytes(),
                );
                // Write the arg string + NUL terminator into the buffer
                let mut arg_bytes = arg.as_bytes().to_vec();
                arg_bytes.push(0); // NUL
                memory::write_bytes(
                    &mut caller,
                    (argv_buf_ptr as u32 + buf_offset) as i32,
                    &arg_bytes,
                );
                buf_offset += arg_bytes.len() as u32;
            }
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// environ_sizes_get
// ---------------------------------------------------------------------------
fn register_environ_sizes_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "environ_sizes_get",
        |mut caller: Caller<'_, HostState>, count_ptr: i32, buf_size_ptr: i32| -> i32 {
            let (count, buf_size) = match caller.data().get_custom::<WasiState>() {
                Some(state) => {
                    let count = state.env_vars.len() as u32;
                    let buf_size: u32 = state
                        .env_vars
                        .iter()
                        .map(|(k, v)| (k.len() + 1 + v.len() + 1) as u32) // KEY=VALUE\0
                        .sum();
                    (count, buf_size)
                }
                None => (0u32, 0u32),
            };
            memory::write_bytes(&mut caller, count_ptr, &count.to_le_bytes());
            memory::write_bytes(&mut caller, buf_size_ptr, &buf_size.to_le_bytes());
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// environ_get
// ---------------------------------------------------------------------------
fn register_environ_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "environ_get",
        |mut caller: Caller<'_, HostState>, environ_ptr: i32, environ_buf_ptr: i32| -> i32 {
            let env_vars: Vec<(String, String)> = match caller.data().get_custom::<WasiState>() {
                Some(state) => state.env_vars.clone(),
                None => vec![],
            };

            let mut buf_offset: u32 = 0;
            for (i, (key, val)) in env_vars.iter().enumerate() {
                let ptr_val = (environ_buf_ptr as u32) + buf_offset;
                memory::write_bytes(
                    &mut caller,
                    environ_ptr + (i as i32) * 4,
                    &ptr_val.to_le_bytes(),
                );
                let entry = format!("{}={}\0", key, val);
                memory::write_bytes(
                    &mut caller,
                    (environ_buf_ptr as u32 + buf_offset) as i32,
                    entry.as_bytes(),
                );
                buf_offset += entry.len() as u32;
            }
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// clock_time_get
// ---------------------------------------------------------------------------
fn register_clock_time_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "clock_time_get",
        |mut caller: Caller<'_, HostState>,
         _clock_id: i32,
         _precision: i64,
         time_ptr: i32|
         -> i32 {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            let nanos = now.as_nanos() as u64;
            memory::write_bytes(&mut caller, time_ptr, &nanos.to_le_bytes());
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// random_get
// ---------------------------------------------------------------------------
fn register_random_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "random_get",
        |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let mut buf = vec![0u8; buf_len as usize];
            rng.fill(&mut buf[..]);
            memory::write_bytes(&mut caller, buf_ptr, &buf);
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// sched_yield
// ---------------------------------------------------------------------------
fn register_sched_yield(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(WASI_NS, "sched_yield", |_caller: Caller<'_, HostState>| -> i32 {
        std::thread::yield_now();
        ERRNO_SUCCESS
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// path_open — open a file or directory relative to a preopened directory
// ---------------------------------------------------------------------------
fn register_path_open(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "path_open",
        |mut caller: Caller<'_, HostState>,
         _dirfd: i32,
         _dirflags: i32,
         path_ptr: i32,
         path_len: i32,
         oflags: i32,
         fs_rights_base: i64,
         _fs_rights_inheriting: i64,
         fdflags: i32,
         fd_ptr: i32|
         -> i32 {
            // Read the path string from WASM memory
            let path = match memory::read_string(&mut caller, path_ptr, path_len) {
                Some(s) => s,
                None => return ERRNO_INVAL,
            };

            // Sanitize: strip leading slashes and "./" prefixes
            let clean = path.trim_start_matches('/').trim_start_matches("./");
            if clean.contains("..") {
                return ERRNO_ACCES;
            }

            let storage_dir = caller.data().filesystem.storage_path().to_path_buf();
            let full_path = storage_dir.join(clean);

            let want_dir = oflags & OFLAG_DIRECTORY != 0;
            let want_creat = oflags & OFLAG_CREAT != 0;
            let want_excl = oflags & OFLAG_EXCL != 0;
            let want_trunc = oflags & OFLAG_TRUNC != 0;
            let want_append = fdflags & FDFLAG_APPEND != 0;

            let rights = fs_rights_base as u64;
            let readable = rights & RIGHT_FD_READ != 0 || rights == 0;
            let writable = rights & RIGHT_FD_WRITE != 0;

            // If O_DIRECTORY, check it exists and is a dir
            if want_dir {
                if want_creat {
                    // Create directory if needed
                    if let Err(_) = std::fs::create_dir_all(&full_path) {
                        return ERRNO_ACCES;
                    }
                }
                if !full_path.is_dir() {
                    return ERRNO_NOTDIR;
                }
                // Allocate an FD for the directory (we use NullFd since
                // directory FDs are only used as anchors for further path ops)
                let fd_num = {
                    let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                        Some(t) => t,
                        None => return ERRNO_BADF,
                    };
                    fd_table.allocate(Box::new(crate::fd::NullFd))
                };
                memory::write_bytes(&mut caller, fd_ptr, &(fd_num as u32).to_le_bytes());
                return ERRNO_SUCCESS;
            }

            // Regular file open
            if want_excl && full_path.exists() {
                return ERRNO_EXIST;
            }

            // Build VfsFileFd flags
            let mut flags = if readable && writable {
                crate::fd::O_RDWR
            } else if writable {
                crate::fd::O_WRONLY
            } else {
                crate::fd::O_RDONLY
            };
            if want_creat || writable {
                flags |= crate::fd::O_CREAT;
            }
            if want_trunc {
                flags |= crate::fd::O_TRUNC;
            }
            if want_append {
                flags |= crate::fd::O_APPEND;
            }

            match crate::fd::VfsFileFd::open(&storage_dir, clean, flags) {
                Ok(vfs_fd) => {
                    let fd_num = {
                        let fd_table = match caller.data_mut().get_custom_mut::<FdTable>() {
                            Some(t) => t,
                            None => return ERRNO_BADF,
                        };
                        fd_table.allocate(Box::new(vfs_fd))
                    };
                    memory::write_bytes(&mut caller, fd_ptr, &(fd_num as u32).to_le_bytes());
                    ERRNO_SUCCESS
                }
                Err(e) => {
                    match e.kind() {
                        std::io::ErrorKind::NotFound => ERRNO_NOENT,
                        std::io::ErrorKind::PermissionDenied => ERRNO_ACCES,
                        std::io::ErrorKind::AlreadyExists => ERRNO_EXIST,
                        _ => ERRNO_INVAL,
                    }
                }
            }
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// path_create_directory
// ---------------------------------------------------------------------------
fn register_path_create_directory(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "path_create_directory",
        |mut caller: Caller<'_, HostState>,
         _dirfd: i32,
         path_ptr: i32,
         path_len: i32|
         -> i32 {
            let path = match memory::read_string(&mut caller, path_ptr, path_len) {
                Some(s) => s,
                None => return ERRNO_INVAL,
            };
            let clean = path.trim_start_matches('/').trim_start_matches("./");
            if clean.contains("..") {
                return ERRNO_ACCES;
            }
            let storage_dir = caller.data().filesystem.storage_path().to_path_buf();
            let full_path = storage_dir.join(clean);
            match std::fs::create_dir_all(&full_path) {
                Ok(_) => ERRNO_SUCCESS,
                Err(_) => ERRNO_ACCES,
            }
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// path_remove_directory
// ---------------------------------------------------------------------------
fn register_path_remove_directory(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "path_remove_directory",
        |mut caller: Caller<'_, HostState>,
         _dirfd: i32,
         path_ptr: i32,
         path_len: i32|
         -> i32 {
            let path = match memory::read_string(&mut caller, path_ptr, path_len) {
                Some(s) => s,
                None => return ERRNO_INVAL,
            };
            let clean = path.trim_start_matches('/').trim_start_matches("./");
            if clean.contains("..") {
                return ERRNO_ACCES;
            }
            let storage_dir = caller.data().filesystem.storage_path().to_path_buf();
            let full_path = storage_dir.join(clean);
            if !full_path.is_dir() {
                return ERRNO_NOTDIR;
            }
            match std::fs::remove_dir(&full_path) {
                Ok(_) => ERRNO_SUCCESS,
                Err(e) => {
                    match e.kind() {
                        std::io::ErrorKind::NotFound => ERRNO_NOENT,
                        _ => ERRNO_NOTEMPTY,
                    }
                }
            }
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// path_unlink_file
// ---------------------------------------------------------------------------
fn register_path_unlink_file(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "path_unlink_file",
        |mut caller: Caller<'_, HostState>,
         _dirfd: i32,
         path_ptr: i32,
         path_len: i32|
         -> i32 {
            let path = match memory::read_string(&mut caller, path_ptr, path_len) {
                Some(s) => s,
                None => return ERRNO_INVAL,
            };
            let clean = path.trim_start_matches('/').trim_start_matches("./");
            if clean.contains("..") {
                return ERRNO_ACCES;
            }
            let storage_dir = caller.data().filesystem.storage_path().to_path_buf();
            let full_path = storage_dir.join(clean);
            if full_path.is_dir() {
                return ERRNO_ISDIR;
            }
            match std::fs::remove_file(&full_path) {
                Ok(_) => ERRNO_SUCCESS,
                Err(e) => {
                    match e.kind() {
                        std::io::ErrorKind::NotFound => ERRNO_NOENT,
                        _ => ERRNO_ACCES,
                    }
                }
            }
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// path_filestat_get — return a 64-byte filestat struct
// ---------------------------------------------------------------------------
fn register_path_filestat_get(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "path_filestat_get",
        |mut caller: Caller<'_, HostState>,
         _dirfd: i32,
         _flags: i32,
         path_ptr: i32,
         path_len: i32,
         buf_ptr: i32|
         -> i32 {
            let path = match memory::read_string(&mut caller, path_ptr, path_len) {
                Some(s) => s,
                None => return ERRNO_INVAL,
            };
            let clean = path.trim_start_matches('/').trim_start_matches("./");
            if clean.contains("..") {
                return ERRNO_ACCES;
            }
            let storage_dir = caller.data().filesystem.storage_path().to_path_buf();
            let full_path = if clean.is_empty() {
                storage_dir.clone()
            } else {
                storage_dir.join(clean)
            };
            let meta = match std::fs::metadata(&full_path) {
                Ok(m) => m,
                Err(_) => return ERRNO_NOENT,
            };

            // __wasi_filestat_t: 64 bytes
            //   u64 dev        (offset 0)
            //   u64 ino        (offset 8)
            //   u8  filetype   (offset 16)
            //   u64 nlink      (offset 24)
            //   u64 size       (offset 32)
            //   u64 atim       (offset 40)
            //   u64 mtim       (offset 48)
            //   u64 ctim       (offset 56)
            let mut stat = [0u8; 64];
            // filetype at offset 16
            stat[16] = if meta.is_dir() {
                FILETYPE_DIRECTORY
            } else if meta.is_symlink() {
                FILETYPE_SYMBOLIC_LINK
            } else {
                FILETYPE_REGULAR_FILE
            };
            // nlink at offset 24
            stat[24..32].copy_from_slice(&1u64.to_le_bytes());
            // size at offset 32
            stat[32..40].copy_from_slice(&meta.len().to_le_bytes());
            // mtim at offset 48 (nanoseconds since epoch)
            if let Ok(mtime) = meta.modified() {
                let nanos = mtime
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64;
                stat[48..56].copy_from_slice(&nanos.to_le_bytes());
                // Use same for atim and ctim
                stat[40..48].copy_from_slice(&nanos.to_le_bytes());
                stat[56..64].copy_from_slice(&nanos.to_le_bytes());
            }

            memory::write_bytes(&mut caller, buf_ptr, &stat);
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// fd_readdir — read directory entries
// ---------------------------------------------------------------------------
fn register_fd_readdir(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(
        WASI_NS,
        "fd_readdir",
        |mut caller: Caller<'_, HostState>,
         _fd: i32,
         buf_ptr: i32,
         buf_len: i32,
         _cookie: i64,
         bufused_ptr: i32|
         -> i32 {
            // For now fd_readdir just returns 0 entries (empty buffer used).
            // Full support would require tracking which directory an FD refers to.
            // This is enough for WASI binaries that fall back gracefully.
            memory::write_bytes(&mut caller, bufused_ptr, &0u32.to_le_bytes());
            let _ = buf_ptr;
            let _ = buf_len;
            ERRNO_SUCCESS
        },
    )?;
    Ok(())
}

