//! `mixer` — the Volume Mixer from the console (see the library for the commands).

use mixer::{confirm, explain, parse, table, Cmd, USAGE};
use std::io::{Read, Write};

/// The terminal's width from `COLUMNS` (80 if unset).
fn width() -> usize {
    std::env::var("COLUMNS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(80)
}

fn height() -> usize {
    std::env::var("LINES").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or(25)
}

fn read() -> ecm_audio::mixer::Strips {
    match ecm_audio::mixer::read() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mixer: {}", explain("", &e, None));
            std::process::exit(1);
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    fn fd_fdstat_set_flags(fd: u32, flags: u32) -> u32;
}

/// Stdin in O_NONBLOCK mode (WASI FDFLAGS_NONBLOCK = 4) or back.
fn stdin_nonblocking(on: bool) {
    #[cfg(target_arch = "wasm32")]
    // SAFETY: a plain WASI call on fd 0.
    unsafe {
        fd_fdstat_set_flags(0, if on { 4 } else { 0 });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = on;
}

/// Live meters: redraw twice a second until q, Ctrl+C or Esc (Shift+Esc on a Terminal: plain Esc closes its screen).
fn watch() {
    let mut out = std::io::stdout();
    // The alternate screen hands us the keys (Ctrl+C included) one by one.
    let _ = write!(out, "\x1b[?1049h\x1b[?25l");
    stdin_nonblocking(true);
    let mut stdin = std::io::stdin();
    let mut buf = [0u8; 32];
    'run: loop {
        let mut frame = String::from("\x1b[H\x1b[2J");
        match ecm_audio::mixer::read() {
            Ok(s) => {
                let mut lines = table(&s, width());
                lines.truncate(height().saturating_sub(2));
                frame.push_str("Volume Mixer (q to quit)\r\n");
                for l in lines {
                    frame.push_str(&l);
                    frame.push_str("\r\n");
                }
            }
            Err(e) => frame.push_str(&format!("mixer: {}\r\n", explain("", &e, None))),
        }
        let _ = out.write_all(frame.as_bytes());
        let _ = out.flush();
        // Keys come in while we wait: look every 50 ms for half a second.
        for _ in 0..10 {
            if let Ok(n) = stdin.read(&mut buf) {
                if buf[..n].iter().any(|b| matches!(b, b'q' | b'Q' | 0x03 | 0x1b)) {
                    break 'run;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    stdin_nonblocking(false);
    let _ = write!(out, "\x1b[?25h\x1b[?1049l");
    let _ = out.flush();
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args) {
        Err(e) => {
            eprintln!("mixer: {e}");
            eprintln!("(mixer -h lists the commands)");
            std::process::exit(2);
        }
        Ok(Cmd::Help) => println!("{USAGE}"),
        Ok(Cmd::Show) => {
            for l in table(&read(), width()) {
                println!("{l}");
            }
        }
        Ok(Cmd::Watch) => watch(),
        Ok(Cmd::Send(cmd)) => match ecm_audio::mixer::command(&cmd) {
            Ok(()) => println!("{}", confirm(&cmd, &read(), width())),
            Err(e) => {
                let s = ecm_audio::mixer::read().ok();
                eprintln!("mixer: {}", explain(&cmd, &e, s.as_ref()));
                std::process::exit(1);
            }
        },
    }
}
