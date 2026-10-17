//! `gui` — start and control the GUI (docs/gui/UI_PROTOCOL.md §2).
//!
//!   gui                    start the compositor and its shell, and show it
//!   gui console | show     switch to the console / back to the GUI (both keep running;
//!                          Ctrl+Alt+Backspace does the same)
//!   gui stop               close the GUI
//!   gui status             what runs, and its windows
//!   gui launch PROG ARGS   start a program as a GUI app
//!
//! The shell and theme come from `/etc/ui.toml` (the desktop, `dev.ecm.desktop`, by
//! default; `shell = "test-shell"` for the P2 tiling shell).

use std::io::{Read, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("help") || args.first().map(String::as_str) == Some("--help") {
        println!("usage: gui [start|console|show|toggle|stop|status|launch PROG ARGS...|wakelock [MS]]");
        return;
    }
    let cmd = if args.is_empty() { "start".to_string() } else { args.join(" ") };
    let mut dev = match std::fs::OpenOptions::new().read(true).write(true).open("/dev/uictl") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gui: no GUI device on this computer ({e})");
            std::process::exit(1);
        }
    };
    if let Err(e) = dev.write_all(cmd.as_bytes()) {
        eprintln!("gui: {e}");
        std::process::exit(1);
    }
    let mut answer = String::new();
    let _ = dev.read_to_string(&mut answer);
    print!("{answer}");
    if answer.contains("can't") || answer.contains("unknown") {
        std::process::exit(1);
    }
}
