//! Thin terminal I/O helpers for switch-os.
//!
//! All output routes through the physical VTE (which writes into the
//! memory-mapped framebuffer). Mirrors the `terminal` module in terminal-os,
//! but only the bits the minimal switch-os shell actually needs.

extern "C" {
    fn sleep_ms(milliseconds: i32);
    fn fb_sync();
}

/// Write a string into the physical VTE.
pub fn print(s: &str) {
    unsafe {
        if let Some(ref mut vte) = crate::PHYSICAL_VTE {
            vte.write_str(s);
        }
    }
}

/// Print with a trailing newline.
pub fn println(s: &str) {
    print(s);
    print("\n");
}

/// Clear the screen via ANSI escape (handled by VTE).
pub fn clear() {
    print("\x1b[2J\x1b[H");
}

/// Chunked sleep so interrupts can still be delivered cooperatively.
#[allow(dead_code)]
pub fn sleep(ms: u32) {
    let mut remaining = ms as i32;
    while remaining > 0 {
        let chunk = if remaining > 10 { 10 } else { remaining };
        unsafe { sleep_ms(chunk); }
        remaining -= chunk;
    }
}

#[allow(dead_code)]
pub fn raw_sleep_ms(ms: i32) {
    unsafe { sleep_ms(ms); }
}

#[allow(dead_code)]
pub fn sync() {
    unsafe { fb_sync(); }
}
