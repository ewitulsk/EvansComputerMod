//! Running the pager on the computer's terminal, for `man`, `help` and `less`.

use crate::{Action, KeyReader, Pager};
use std::io::{Read, Write};

/// The terminal's size from `COLUMNS` / `LINES` (80x25 if unset).
pub fn size() -> (usize, usize) {
    let get = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(d);
    (get("COLUMNS", 80), get("LINES", 25))
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn fd_fdstat_get(fd: i32, buf: *mut u8) -> u16;
}

/// Is `fd` the computer's terminal (not a redirect to a file or a pipe)?
pub fn is_terminal(fd: i32) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        let mut buf = [0u8; 24];
        // SAFETY: fd_fdstat_get writes 24 bytes into buf.
        let r = unsafe { fd_fdstat_get(fd, buf.as_mut_ptr()) };
        r == 0 && buf[0] == 2
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = fd;
        false
    }
}

/// Print `lines`, or page them full screen when they don't fit and both
/// ends are the terminal (`force_plain`: always print).
pub fn show(title: &str, lines: Vec<String>, force_plain: bool) {
    let (w, h) = size();
    let mut out = std::io::stdout().lock();
    if force_plain || lines.len() < h || !is_terminal(0) || !is_terminal(1) {
        for l in &lines {
            let _ = writeln!(out, "{l}");
        }
        let _ = out.flush();
        return;
    }
    let mut pager = Pager::new(title, lines);
    // Full screen, cursor hidden, Ctrl+C read here (the alternate screen gives us it).
    let _ = write!(out, "\x1b[?1049h\x1b[?25l{}", pager.draw(w, h));
    let _ = out.flush();
    let mut keys = KeyReader::default();
    let mut stdin = std::io::stdin().lock();
    let mut buf = [0u8; 64];
    'run: loop {
        let n = match stdin.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for k in keys.feed(&buf[..n]) {
            if pager.key(k, h) == Action::Quit {
                break 'run;
            }
        }
        let _ = write!(out, "{}", pager.draw(w, h));
        let _ = out.flush();
    }
    let _ = write!(out, "\x1b[?25h\x1b[?1049l");
    let _ = out.flush();
}
