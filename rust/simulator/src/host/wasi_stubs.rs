//! WASI stub functions returning ENOSYS.
//!
//! Registers all remaining `wasi_snapshot_preview1` functions that are not
//! implemented by `wasi_io.rs`. Each stub returns `ERRNO_NOSYS` (52) so that
//! WASI binaries get a clean "not supported" error instead of a link-time
//! failure.

use wasmtime::*;
use crate::wasi::ERRNO_NOSYS;
use crate::wasm_host::HostState;

const WASI_NS: &str = "wasi_snapshot_preview1";

/// Register all stub WASI functions.
pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // --- fd operations ---

    // fd_advise(fd, offset, len, advice) -> errno
    linker.func_wrap(WASI_NS, "fd_advise",
        |_: Caller<'_, HostState>, _: i32, _: i64, _: i64, _: i32| -> i32 { ERRNO_NOSYS })?;

    // fd_allocate(fd, offset, len) -> errno
    linker.func_wrap(WASI_NS, "fd_allocate",
        |_: Caller<'_, HostState>, _: i32, _: i64, _: i64| -> i32 { ERRNO_NOSYS })?;

    // fd_datasync(fd) -> errno
    linker.func_wrap(WASI_NS, "fd_datasync",
        |_: Caller<'_, HostState>, _: i32| -> i32 { ERRNO_NOSYS })?;

    // fd_sync(fd) -> errno
    linker.func_wrap(WASI_NS, "fd_sync",
        |_: Caller<'_, HostState>, _: i32| -> i32 { ERRNO_NOSYS })?;

    // fd_renumber(from_fd, to_fd) -> errno
    linker.func_wrap(WASI_NS, "fd_renumber",
        |_: Caller<'_, HostState>, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // fd_pread(fd, iovs, iovs_len, offset, nread) -> errno
    linker.func_wrap(WASI_NS, "fd_pread",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i64, _: i32| -> i32 { ERRNO_NOSYS })?;

    // fd_pwrite(fd, iovs, iovs_len, offset, nwritten) -> errno
    linker.func_wrap(WASI_NS, "fd_pwrite",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i64, _: i32| -> i32 { ERRNO_NOSYS })?;

    // fd_filestat_get(fd, buf) -> errno
    linker.func_wrap(WASI_NS, "fd_filestat_get",
        |_: Caller<'_, HostState>, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // fd_filestat_set_size(fd, size) -> errno
    linker.func_wrap(WASI_NS, "fd_filestat_set_size",
        |_: Caller<'_, HostState>, _: i32, _: i64| -> i32 { ERRNO_NOSYS })?;

    // fd_filestat_set_times(fd, atim, mtim, fst_flags) -> errno
    linker.func_wrap(WASI_NS, "fd_filestat_set_times",
        |_: Caller<'_, HostState>, _: i32, _: i64, _: i64, _: i32| -> i32 { ERRNO_NOSYS })?;

    // --- path operations ---

    // path_link(old_fd, old_flags, old_path, old_path_len, new_fd, new_path, new_path_len) -> errno
    linker.func_wrap(WASI_NS, "path_link",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // path_symlink(old_path, old_path_len, dirfd, new_path, new_path_len) -> errno
    linker.func_wrap(WASI_NS, "path_symlink",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // path_readlink(dirfd, path, path_len, buf, buf_len, bufused) -> errno
    linker.func_wrap(WASI_NS, "path_readlink",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // path_rename(old_fd, old_path, old_path_len, new_fd, new_path, new_path_len) -> errno
    linker.func_wrap(WASI_NS, "path_rename",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // path_filestat_set_times(dirfd, flags, path, path_len, atim, mtim, fst_flags) -> errno
    linker.func_wrap(WASI_NS, "path_filestat_set_times",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i64, _: i64, _: i32| -> i32 { ERRNO_NOSYS })?;

    // --- polling / sockets ---

    // poll_oneoff(in, out, nsubscriptions, nevents) -> errno
    linker.func_wrap(WASI_NS, "poll_oneoff",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // sock_accept(fd, flags, result_fd) -> errno
    linker.func_wrap(WASI_NS, "sock_accept",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // sock_recv(fd, ri_data, ri_data_len, ri_flags, ro_datalen, ro_flags) -> errno
    linker.func_wrap(WASI_NS, "sock_recv",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // sock_send(fd, si_data, si_data_len, si_flags, so_datalen) -> errno
    linker.func_wrap(WASI_NS, "sock_send",
        |_: Caller<'_, HostState>, _: i32, _: i32, _: i32, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // sock_shutdown(fd, how) -> errno
    linker.func_wrap(WASI_NS, "sock_shutdown",
        |_: Caller<'_, HostState>, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    // --- misc ---

    // proc_raise(sig) -> errno
    linker.func_wrap(WASI_NS, "proc_raise",
        |_: Caller<'_, HostState>, _: i32| -> i32 { ERRNO_NOSYS })?;

    // clock_res_get(clock_id, resolution_ptr) -> errno
    linker.func_wrap(WASI_NS, "clock_res_get",
        |_: Caller<'_, HostState>, _: i32, _: i32| -> i32 { ERRNO_NOSYS })?;

    Ok(())
}
