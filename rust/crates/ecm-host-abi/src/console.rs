//! The console a program runs on (abi/child-abi.toml `[imports.console]`):
//! raw keys from the Terminal, the world's clock, the computer's number, and
//! reading stdin without waiting. ecm-console's event queue is built on these.

extern "C" {
    fn key_capture_start() -> i32;
    fn key_poll(out: i32) -> i32;
    fn world_time(out: i32) -> i32;
    fn computer_number() -> i32;
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn fd_fdstat_set_flags(fd: u32, flags: u32) -> u16;
}

/// A raw key: USB HID usage, modifier bits (shift 1, ctrl 2, alt 4, super 8)
/// and kind (1 pressed, 2 auto-repeat, 3 released, 4 a typed character:
/// `hid` holds its byte). Keys and characters come in the order they were typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub hid: u16,
    pub mods: u8,
    pub kind: u8,
}

/// Ask for the Terminal's raw keys while this program runs on its console.
/// False if this console has none (an SSH session, the GUI's Terminal app).
pub fn capture_keys() -> bool {
    unsafe { key_capture_start() == 1 }
}

/// The next raw key, if one is waiting.
pub fn poll_key() -> Option<Key> {
    let mut buf = [0u8; 4];
    if unsafe { key_poll(buf.as_mut_ptr() as i32) } != 1 {
        return None;
    }
    Some(Key { hid: u16::from_le_bytes([buf[0], buf[1]]), mods: buf[2], kind: buf[3] })
}

/// The world's day time in ticks (as `/time query daytime` counts it, not
/// wrapped at a day), or None when the computer isn't in a world.
pub fn day_ticks() -> Option<i64> {
    let mut buf = [0u8; 8];
    if unsafe { world_time(buf.as_mut_ptr() as i32) } != 0 {
        return None;
    }
    Some(i64::from_le_bytes(buf))
}

/// This computer's number: small, unique in the world and kept for the
/// computer's life. None if the host gives computers none.
pub fn computer_id() -> Option<i32> {
    let id = unsafe { computer_number() };
    (id >= 0).then_some(id)
}

/// Make reads of stdin return at once when nothing was typed (or wait again).
pub fn stdin_nonblocking(on: bool) {
    const NONBLOCK: u32 = 4;
    unsafe {
        fd_fdstat_set_flags(0, if on { NONBLOCK } else { 0 });
    }
}
