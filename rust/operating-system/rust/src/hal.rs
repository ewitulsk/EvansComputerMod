//! Host abstraction layer — the ONLY module that talks to the host.
//!
//! Every host import the kernel uses is declared here and wrapped in a safe
//! function. The rest of the kernel never sees a raw pointer into host-shared
//! memory: the regions the host reads or writes (input, interrupt payload,
//! socket IPC buffers, text framebuffer, gfx plane, screen plane) are kernel
//! statics whose addresses are published through the `abi_layout` export.
//!
//! Contract: `docs/refactor/ARCHITECTURE.md` §1 and §4. Every import listed
//! here is non-blocking and never calls back into the kernel.
//!
//! On non-wasm targets (host `cargo test`) the imports are replaced by small
//! in-process fakes so kernel logic can be unit-tested.

use core::cell::UnsafeCell;

// =====================================================================
// Shared memory regions
// =====================================================================

/// A fixed-size byte region shared with the host.
///
/// Soundness: the kernel is single-threaded and only touches a region from
/// inside a `KernelCell` borrow, and the host only touches it between export
/// calls (never concurrently with kernel execution), so handing out a
/// `&mut [u8]` for the duration of one kernel operation cannot alias.
pub struct Region<const N: usize>(UnsafeCell<[u8; N]>);

unsafe impl<const N: usize> Sync for Region<N> {}

impl<const N: usize> Region<N> {
    pub const fn new() -> Self {
        Self(UnsafeCell::new([0u8; N]))
    }
    pub const CAP: usize = N;
    pub fn addr(&self) -> u32 {
        self.0.get() as usize as u32
    }
    /// Borrow the whole region. Callers must not hold two borrows of the same
    /// region at once (enforced by convention: each region has one owner
    /// module).
    #[allow(clippy::mut_from_ref)]
    pub fn bytes(&self) -> &mut [u8] {
        unsafe { &mut *self.0.get() }
    }
}

pub const INPUT_CAP: usize = 4096;
pub const IRQ_CAP: usize = 4096;
pub const IPC_ARGS_CAP: usize = 8192;
pub const IPC_RESULT_CAP: usize = 8192;
/// 64-byte header + 160×50 cells × 4 bytes, rounded up.
pub const FB_CAP: usize = 64 + 256 * 64 * 4;
/// 1 KiB header+palette + 640×400 indexed pixels.
pub const GFX_CAP: usize = 0x400 + 640 * 400;
/// 1 KiB header+palette + 640×360 RGBA pixels, rounded to 1 MiB.
pub const SCREEN_CAP: usize = 0x10_0000;

pub static INPUT: Region<INPUT_CAP> = Region::new();
pub static IRQ: Region<IRQ_CAP> = Region::new();
pub static IPC_ARGS: Region<IPC_ARGS_CAP> = Region::new();
pub static IPC_RESULT: Region<IPC_RESULT_CAP> = Region::new();
pub static FB: Region<FB_CAP> = Region::new();
pub static GFX: Region<GFX_CAP> = Region::new();
pub static SCREEN: Region<SCREEN_CAP> = Region::new();

pub const ABI_LAYOUT_VERSION: u32 = 1;

/// Serialises host-side tests that touch the shared static regions.
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// The words written by the `abi_layout` export.
pub fn abi_layout_words() -> [u32; 15] {
    [
        ABI_LAYOUT_VERSION,
        INPUT.addr(), INPUT_CAP as u32,
        IRQ.addr(), IRQ_CAP as u32,
        IPC_ARGS.addr(), IPC_ARGS_CAP as u32,
        IPC_RESULT.addr(), IPC_RESULT_CAP as u32,
        FB.addr(), FB_CAP as u32,
        GFX.addr(), GFX_CAP as u32,
        SCREEN.addr(), SCREEN_CAP as u32,
    ]
}

// =====================================================================
// Raw imports
// =====================================================================

#[cfg(target_arch = "wasm32")]
mod ffi {
    extern "C" {
        // time / entropy
        pub fn get_time_ms() -> i64;
        pub fn __getrandom_v03_custom(buf_ptr: i32, buf_len: i32) -> i32;
        pub fn fb_sync();

        // network
        pub fn net_get_interface_count() -> i32;
        pub fn net_get_interface_mac(index: i32, buf_ptr: i32) -> i32;
        pub fn net_tx_frame_on(index: i32, buf_ptr: i32, len: i32) -> i32;
        pub fn net_rx_frame_any(buf_ptr: i32, buf_len: i32, iface_idx_ptr: i32) -> i32;
        pub fn net_set_promiscuous_on(index: i32, enabled: i32) -> i32;
        pub fn net_set_link_state(index: i32, up: i32) -> i32;
        pub fn net_get_link_state(index: i32) -> i32;

        // processes
        pub fn process_spawn(path_ptr: i32, path_len: i32, argv_ptr: i32, argv_len: i32,
                             stdin_fd: i32, stdout_fd: i32, stderr_fd: i32) -> i32;
        pub fn process_try_wait(pid: i32, code_ptr: i32) -> i32;
        pub fn process_read_output(pid: i32, buf_ptr: i32, buf_len: i32) -> i32;
        pub fn process_write_input(pid: i32, buf_ptr: i32, buf_len: i32) -> i32;
        pub fn process_kill(pid: i32, signal: i32) -> i32;
        pub fn process_list(buf_ptr: i32, buf_len: i32) -> i32;

        // files
        pub fn file_write(path_ptr: i32, path_len: i32, data_ptr: i32, data_len: i32) -> i32;
        pub fn file_read(path_ptr: i32, path_len: i32, buf_ptr: i32, buf_len: i32) -> i32;
        pub fn file_size(path_ptr: i32, path_len: i32) -> i32;
        pub fn file_exists(path_ptr: i32, path_len: i32) -> i32;
        pub fn file_is_dir(path_ptr: i32, path_len: i32) -> i32;
        pub fn file_list_dir(path_ptr: i32, path_len: i32, buf_ptr: i32, buf_len: i32) -> i32;
        pub fn fd_open(path_ptr: i32, path_len: i32, flags: i32) -> i32;
        pub fn fd_close(fd: i32) -> i32;
        pub fn pipe_create(read_fd_ptr: i32, write_fd_ptr: i32) -> i32;

        // screen cluster
        pub fn screen_is_attached() -> i32;
        pub fn screen_get_gfx_width() -> i32;
        pub fn screen_get_gfx_height() -> i32;
        pub fn screen_fb_sync();
        pub fn screen_set_power(on: i32);
        pub fn screen_set_pixel_format(format: i32);

        // misc
        pub fn open_visual_editor();
    }
}

/// In-process fakes used by host-side unit tests.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::missing_safety_doc)]
pub mod ffi {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    thread_local! {
        pub static NOW: RefCell<i64> = const { RefCell::new(0) };
        pub static FILES: RefCell<BTreeMap<String, Vec<u8>>> = const { RefCell::new(BTreeMap::new()) };
        pub static SPAWNED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
        pub static TX: RefCell<Vec<(i32, Vec<u8>)>> = const { RefCell::new(Vec::new()) };
    }

    unsafe fn s<'a>(p: usize, l: i32) -> &'a [u8] { core::slice::from_raw_parts(p as *const u8, l as usize) }
    fn key(p: usize, l: i32) -> String { unsafe { String::from_utf8_lossy(s(p, l)).into_owned() } }

    pub unsafe fn get_time_ms() -> i64 { NOW.with(|n| *n.borrow()) }
    pub unsafe fn __getrandom_v03_custom(p: usize, l: i32) -> i32 {
        let b = core::slice::from_raw_parts_mut(p as *mut u8, l as usize);
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        for v in b.iter_mut() { x ^= x << 13; x ^= x >> 7; x ^= x << 17; *v = x as u8; }
        0
    }
    pub unsafe fn fb_sync() {}
    pub unsafe fn net_get_interface_count() -> i32 { 0 }
    pub unsafe fn net_get_interface_mac(_: i32, _: usize) -> i32 { -1 }
    pub unsafe fn net_tx_frame_on(i: i32, p: usize, l: i32) -> i32 { TX.with(|t| t.borrow_mut().push((i, s(p, l).to_vec()))); 0 }
    pub unsafe fn net_rx_frame_any(_: usize, _: i32, _: usize) -> i32 { -1 }
    pub unsafe fn net_set_promiscuous_on(_: i32, _: i32) -> i32 { 0 }
    pub unsafe fn net_set_link_state(_: i32, _: i32) -> i32 { 0 }
    pub unsafe fn net_get_link_state(_: i32) -> i32 { 1 }
    pub unsafe fn process_spawn(p: usize, l: i32, a: usize, al: i32, _: i32, _: i32, _: i32) -> i32 {
        SPAWNED.with(|v| { let mut v = v.borrow_mut(); v.push((key(p, l), key(a, al))); v.len() as i32 })
    }
    pub unsafe fn process_try_wait(_: i32, c: usize) -> i32 { *(c as *mut i32) = 0; 1 }
    pub unsafe fn process_read_output(_: i32, _: usize, _: i32) -> i32 { 0 }
    pub unsafe fn process_write_input(_: i32, _: usize, l: i32) -> i32 { l }
    pub unsafe fn process_kill(_: i32, _: i32) -> i32 { 0 }
    pub unsafe fn process_list(_: usize, _: i32) -> i32 { 0 }
    pub unsafe fn file_write(p: usize, l: i32, d: usize, dl: i32) -> i32 {
        FILES.with(|f| f.borrow_mut().insert(key(p, l), s(d, dl).to_vec())); dl
    }
    pub unsafe fn file_read(p: usize, l: i32, b: usize, bl: i32) -> i32 {
        FILES.with(|f| match f.borrow().get(&key(p, l)) {
            Some(v) => { let n = v.len().min(bl as usize); core::ptr::copy_nonoverlapping(v.as_ptr(), b as *mut u8, n); n as i32 }
            None => -1,
        })
    }
    pub unsafe fn file_size(p: usize, l: i32) -> i32 { FILES.with(|f| f.borrow().get(&key(p, l)).map(|v| v.len() as i32).unwrap_or(-1)) }
    pub unsafe fn file_exists(p: usize, l: i32) -> i32 { FILES.with(|f| f.borrow().contains_key(&key(p, l)) as i32) }
    pub unsafe fn file_is_dir(_: usize, _: i32) -> i32 { 0 }
    pub unsafe fn file_list_dir(_: usize, _: i32, _: usize, _: i32) -> i32 { 0 }
    pub unsafe fn fd_open(_: usize, _: i32, _: i32) -> i32 { -1 }
    pub unsafe fn fd_close(_: i32) -> i32 { 0 }
    pub unsafe fn pipe_create(_: usize, _: usize) -> i32 { -1 }
    pub unsafe fn screen_is_attached() -> i32 { 0 }
    pub unsafe fn screen_get_gfx_width() -> i32 { 0 }
    pub unsafe fn screen_get_gfx_height() -> i32 { 0 }
    pub unsafe fn screen_fb_sync() {}
    pub unsafe fn screen_set_power(_: i32) {}
    pub unsafe fn screen_set_pixel_format(_: i32) {}
    pub unsafe fn open_visual_editor() {}
}

/// Pointer type crossing the host boundary: wasm32 imports take i32
/// addresses; the native test fakes need full-width pointers.
#[cfg(target_arch = "wasm32")]
pub type Ptr = i32;
#[cfg(not(target_arch = "wasm32"))]
pub type Ptr = usize;

#[inline]
fn p<T>(ptr: *const T) -> Ptr {
    ptr as usize as Ptr
}

// =====================================================================
// Safe wrappers
// =====================================================================

pub fn now_ms() -> i64 {
    unsafe { ffi::get_time_ms() }
}

pub fn fill_random(buf: &mut [u8]) -> bool {
    if buf.is_empty() {
        return true;
    }
    unsafe { ffi::__getrandom_v03_custom(p(buf.as_ptr()), buf.len() as i32) == 0 }
}

pub fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    fill_random(&mut b);
    u64::from_le_bytes(b)
}

pub fn fb_sync() {
    unsafe { ffi::fb_sync() }
}

pub fn open_visual_editor() {
    unsafe { ffi::open_visual_editor() }
}

pub mod net {
    use super::{ffi, p};

    pub fn interface_count() -> usize {
        let n = unsafe { ffi::net_get_interface_count() };
        if n < 0 { 0 } else { n as usize }
    }

    pub fn interface_mac(index: usize) -> Option<[u8; 6]> {
        let mut mac = [0u8; 6];
        let r = unsafe { ffi::net_get_interface_mac(index as i32, p(mac.as_mut_ptr())) };
        if r == 6 { Some(mac) } else { None }
    }

    pub fn tx(index: usize, frame: &[u8]) -> bool {
        if frame.len() < 14 || frame.len() > 1518 {
            return false;
        }
        unsafe { ffi::net_tx_frame_on(index as i32, p(frame.as_ptr()), frame.len() as i32) == 0 }
    }

    /// Non-blocking receive from any interface. Returns (iface, len).
    pub fn rx_any(buf: &mut [u8]) -> Option<(usize, usize)> {
        let mut idx: i32 = -1;
        let n = unsafe {
            ffi::net_rx_frame_any(p(buf.as_mut_ptr()), buf.len() as i32, p(&mut idx as *mut i32))
        };
        if n <= 0 || idx < 0 {
            return None;
        }
        Some((idx as usize, (n as usize).min(buf.len())))
    }

    pub fn set_promiscuous(index: usize, on: bool) {
        unsafe { ffi::net_set_promiscuous_on(index as i32, on as i32) };
    }

    /// Administrative up/down of a face (physically disables the port).
    pub fn set_admin_state(index: usize, up: bool) {
        unsafe { ffi::net_set_link_state(index as i32, up as i32) };
    }

    /// Carrier: true if a cable connects this face to a segment.
    pub fn carrier(index: usize) -> bool {
        unsafe { ffi::net_get_link_state(index as i32) == 1 }
    }
}

pub mod proc {
    use super::{ffi, p};

    pub enum Wait {
        Running,
        Exited(i32),
        Unknown,
    }

    /// Spawn a program. `argv` is newline-separated. fds of -1 mean "terminal".
    pub fn spawn(path: &str, argv: &str, stdin: i32, stdout: i32, stderr: i32) -> Option<i32> {
        let pid = unsafe {
            ffi::process_spawn(p(path.as_ptr()), path.len() as i32, p(argv.as_ptr()), argv.len() as i32,
                               stdin, stdout, stderr)
        };
        if pid > 0 { Some(pid) } else { None }
    }

    pub fn try_wait(pid: i32) -> Wait {
        let mut code: i32 = 0;
        match unsafe { ffi::process_try_wait(pid, p(&mut code as *mut i32)) } {
            1 => Wait::Exited(code),
            0 => Wait::Running,
            _ => Wait::Unknown,
        }
    }

    /// Non-blocking read of the child's terminal output. Returns bytes read.
    pub fn read_output(pid: i32, buf: &mut [u8]) -> usize {
        let n = unsafe { ffi::process_read_output(pid, p(buf.as_mut_ptr()), buf.len() as i32) };
        if n <= 0 { 0 } else { (n as usize).min(buf.len()) }
    }

    pub fn write_input(pid: i32, data: &[u8]) -> bool {
        unsafe { ffi::process_write_input(pid, p(data.as_ptr()), data.len() as i32) >= 0 }
    }

    pub fn kill(pid: i32) -> bool {
        unsafe { ffi::process_kill(pid, 15) == 0 }
    }

    /// JSON process list from the host.
    pub fn list() -> String {
        let mut buf = vec![0u8; 4096];
        let n = unsafe { ffi::process_list(p(buf.as_mut_ptr()), buf.len() as i32) };
        if n <= 0 {
            return String::new();
        }
        buf.truncate((n as usize).min(4096));
        String::from_utf8_lossy(&buf).into_owned()
    }
}

pub mod file {
    use super::{ffi, p};

    /// Largest file the kernel reads into memory.
    pub const MAX_READ: usize = 256 * 1024;

    pub fn write(path: &str, data: &[u8]) -> bool {
        unsafe { ffi::file_write(p(path.as_ptr()), path.len() as i32, p(data.as_ptr()), data.len() as i32) >= 0 }
    }

    pub fn read(path: &str) -> Option<Vec<u8>> {
        let size = unsafe { ffi::file_size(p(path.as_ptr()), path.len() as i32) };
        if size < 0 || size as usize > MAX_READ {
            return None;
        }
        let mut buf = vec![0u8; size as usize];
        let n = unsafe { ffi::file_read(p(path.as_ptr()), path.len() as i32, p(buf.as_mut_ptr()), buf.len() as i32) };
        if n < 0 {
            return None;
        }
        buf.truncate((n as usize).min(buf.len()));
        Some(buf)
    }

    pub fn exists(path: &str) -> bool {
        unsafe { ffi::file_exists(p(path.as_ptr()), path.len() as i32) == 1 }
    }

    pub fn is_dir(path: &str) -> bool {
        path.is_empty() || unsafe { ffi::file_is_dir(p(path.as_ptr()), path.len() as i32) == 1 }
    }

    /// Raw "d:name\nf:name\n" listing.
    pub fn list_dir(path: &str) -> String {
        let mut buf = vec![0u8; 16 * 1024];
        let n = unsafe { ffi::file_list_dir(p(path.as_ptr()), path.len() as i32, p(buf.as_mut_ptr()), buf.len() as i32) };
        if n <= 0 {
            return String::new();
        }
        buf.truncate((n as usize).min(buf.len()));
        String::from_utf8_lossy(&buf).into_owned()
    }

    pub fn fd_open(path: &str, flags: i32) -> i32 {
        unsafe { ffi::fd_open(p(path.as_ptr()), path.len() as i32, flags) }
    }

    pub fn fd_close(fd: i32) {
        unsafe { ffi::fd_close(fd) };
    }

    pub fn pipe() -> Option<(i32, i32)> {
        let mut r: i32 = -1;
        let mut w: i32 = -1;
        let rc = unsafe { ffi::pipe_create(p(&mut r as *mut i32), p(&mut w as *mut i32)) };
        if rc == 0 { Some((r, w)) } else { None }
    }
}

pub mod screen {
    use super::ffi;

    pub fn attached() -> bool {
        unsafe { ffi::screen_is_attached() != 0 }
    }
    pub fn host_dims() -> (u16, u16) {
        let w = unsafe { ffi::screen_get_gfx_width() };
        let h = unsafe { ffi::screen_get_gfx_height() };
        (w.clamp(0, u16::MAX as i32) as u16, h.clamp(0, u16::MAX as i32) as u16)
    }
    pub fn sync() {
        unsafe { ffi::screen_fb_sync() }
    }
    pub fn set_power(on: bool) {
        unsafe { ffi::screen_set_power(on as i32) }
    }
    pub fn set_pixel_format(format: u8) {
        unsafe { ffi::screen_set_pixel_format(format as i32) }
    }
}

// =====================================================================
// getrandom 0.2 backend → host entropy
// =====================================================================

fn host_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    if fill_random(buf) {
        Ok(())
    } else {
        Err(getrandom::Error::UNSUPPORTED)
    }
}

getrandom::register_custom_getrandom!(host_getrandom);
