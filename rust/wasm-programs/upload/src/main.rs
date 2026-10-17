//! `upload` — copy a file from your real computer into this one
//! (abi/child-abi.toml `[imports.upload]`).
//!
//!   upload [DIR]     pick a file on your computer; it lands in DIR (default /home)
//!   upload status    whether you can upload to this computer
//!
//! Your system's file picker opens in your game. The file keeps its name (a
//! taken name gets " (1)", " (2)", ...). Ctrl+T stops waiting. Needs the
//! evanscomputermod-upload add-on on the server and in your game; the server
//! may limit the file size and who may upload.

use ecm_host_abi::upload::{self, State, Support};
use std::io::Write;
use std::time::Duration;

fn usage() -> ! {
    eprintln!("usage: upload [DIR]     pick a file on your computer to copy into DIR (default /home)");
    eprintln!("       upload status    whether you can upload to this computer");
    std::process::exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(|s| s.as_str()) {
        Some("-h") | Some("--help") | Some("help") => usage(),
        Some("status") if args.len() == 1 => status(),
        _ if args.len() > 1 => usage(),
        dir => run(dir.unwrap_or("/home")),
    };
    std::process::exit(code);
}

fn status() -> i32 {
    match upload::supported() {
        Support::Ready => {
            println!("upload: ready: `upload [DIR]` opens a file picker in your game");
            0
        }
        s => {
            println!("upload: {}", s.message());
            1
        }
    }
}

fn human(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", n as f64 / (1024.0 * 1024.0))
    }
}

fn run(dir: &str) -> i32 {
    let support = upload::supported();
    if support != Support::Ready {
        eprintln!("upload: {}", support.message());
        return 1;
    }
    let ticket = match upload::begin(dir) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("upload: {dir}: {}", e.message());
            return 1;
        }
    };
    println!("upload: pick a file on your computer (the file picker is open in your game)...");
    let mut out = std::io::stdout();
    let mut shown = false;
    loop {
        match upload::poll(ticket) {
            State::Picking => {}
            State::Receiving { received, total } => {
                let pct = if total == 0 { 100 } else { received * 100 / total };
                print!("\rupload: {} of {} ({pct}%)   ", human(received), human(total));
                let _ = out.flush();
                shown = true;
            }
            State::Done(path) => {
                if shown {
                    println!();
                }
                println!("upload: saved {path}");
                return 0;
            }
            State::Failed(e) => {
                if shown {
                    println!();
                }
                upload::cancel(ticket);
                eprintln!("upload: {}", e.message());
                return 1;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
