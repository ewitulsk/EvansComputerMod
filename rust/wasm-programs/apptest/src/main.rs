//! `apptest`: probes for the app platform tests (AppPlatformTest). Each command
//! tries one thing and prints one line saying what happened, so a test can tell
//! a confined app from an unconfined program.
//!
//!   apptest sock                 open a TCP socket            -> "sock ok" | "sock denied <rc>"
//!   apptest read <path>          read a file                  -> "read ok <len>" | "read denied <err>"
//!   apptest write <path> <text>  write a file                 -> "write ok" | "write denied <err>"
//!   apptest sh <until> <cmd...>  run <cmd> in a shell session, print its output until a line has <until>
//!   apptest whoami               the app id the app manager sees ("" = not an app)
//!   apptest wait <ms>            print "ready", then sleep (a process to suspend)
//!   apptest /<path>              what "open with" runs: read the file -> "opened <path> ok <len>"

use ecm_host_abi::{apps, ipc, socket};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    match a.as_slice() {
        ["sock"] => {
            let fd = socket::socket(socket::AF_INET, socket::SOCK_STREAM, 0);
            if fd >= 0 {
                println!("sock ok");
                socket::close(fd);
            } else {
                println!("sock denied {}", fd);
            }
        }
        ["read", path] => match std::fs::read(path) {
            Ok(b) => println!("read ok {}", b.len()),
            Err(e) => println!("read denied {}", e),
        },
        ["write", path, text] => match std::fs::write(path, text.as_bytes()) {
            Ok(()) => println!("write ok"),
            Err(e) => println!("write denied {}", e),
        },
        ["sh", until, cmd @ ..] => shell(until, &cmd.join(" ")),
        ["whoami"] => match apps::ctl(&["whoami"]) {
            Some((s, r)) => println!("whoami {} [{}]", s, String::from_utf8_lossy(&r)),
            None => println!("whoami none"),
        },
        ["wait", ms] => {
            println!("ready");
            let ms: u64 = ms.parse().unwrap_or(60_000);
            let mut left = ms;
            while left > 0 {
                let step = left.min(100);
                std::thread::sleep(std::time::Duration::from_millis(step));
                left -= step;
            }
            println!("done");
        }
        // Opened with a file ("open with" passes the path): read it, like an editor would.
        [path] if path.starts_with('/') => match std::fs::read(path) {
            Ok(b) => println!("opened {} ok {}", path, b.len()),
            Err(e) => println!("opened {} denied {}", path, e),
        },
        _ => println!("usage: apptest sock | read <path> | write <path> <text> | sh <until> <cmd> | whoami | wait <ms>"),
    }
}

/// A shell session (like sshd's): type `cmd`, relay output until a line contains `until`.
fn shell(until: &str, cmd: &str) {
    let id = ipc::spawn_shell("app");
    if id < 0 {
        println!("sh failed {}", id);
        return;
    }
    let mut line = cmd.to_string();
    line.push('\n');
    ipc::session_write(id, line.as_bytes());
    let mut seen = String::new();
    let mut buf = [0u8; 2048];
    for _ in 0..100 {
        let n = ipc::session_read_blocking(id, &mut buf, 100);
        if n < 0 {
            break;
        }
        if n > 0 {
            seen.push_str(&String::from_utf8_lossy(&buf[..n as usize]));
            if seen.lines().any(|l| l.contains(until) && !l.contains(cmd)) {
                break;
            }
        }
    }
    for l in seen.lines() {
        if l.contains(until) && !l.contains(cmd) {
            println!("sh| {}", l.trim());
        }
    }
    ipc::session_close(id);
}
