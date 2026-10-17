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
//!   apptest ui-image <n> [late]  a window with an n x n raw RGBA image resource and a
//!                                "Clicks N" button that counts clicks (UiLargeResourceTest)

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
        ["ui-image", n] => ui_image(n.parse().unwrap_or(64), false),
        ["ui-image", n, "late"] => ui_image(n.parse().unwrap_or(64), true),
        ["ui-image", n, "page"] => ui_image_page(n.parse().unwrap_or(64)),
        _ => println!("usage: apptest sock | read <path> | write <path> <text> | sh <until> <cmd> | whoami | wait <ms> | ui-image <n> [late|page]"),
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

/// A window with an `n` x `n` image and a click counter: the app must keep
/// answering input after a large image upload. `late`: the image is uploaded
/// by the first click, while the window is up (as Music loads cover art).
fn ui_image(n: i32, late: bool) {
    use ecm_ui::element::{button, column, image, label};
    use std::cell::Cell;
    use std::rc::Rc;
    let mut app = ecm_ui::App::new("dev.test.image");
    let pixels = move || {
        let mut rgba = Vec::with_capacity((n * n * 4) as usize);
        for y in 0..n {
            for x in 0..n {
                rgba.extend_from_slice(&[(x * 255 / n.max(1)) as u8, (y * 255 / n.max(1)) as u8, ((x ^ y) & 0xFF) as u8, 255]);
            }
        }
        rgba
    };
    let res = Rc::new(Cell::new(0u32));
    if !late {
        match app.core.image_rgba(n, n, &pixels()) {
            Ok(r) => res.set(r),
            Err(e) => {
                println!("ui-image upload failed: {e}");
                return;
            }
        }
        println!("ui-image uploaded {n}");
    }
    let clicks = Rc::new(Cell::new(0u32));
    let (view_clicks, view_res) = (clicks.clone(), res.clone());
    app.window("Image Test", move |_| {
        let (c, r) = (view_clicks.clone(), view_res.clone());
        let mut col = column()
            .gap(8)
            .padding(8)
            .child(button(format!("Clicks {}", view_clicks.get())).on_click(move |cx, _| {
                if r.get() == 0 {
                    match cx.core().image_rgba(n, n, &pixels()) {
                        Ok(id) => r.set(id),
                        Err(e) => println!("ui-image upload failed: {e}"),
                    }
                }
                c.set(c.get() + 1)
            }))
            .child(label("big image below"));
        if view_res.get() != 0 {
            col = col.child(image(view_res.get(), n, n));
        }
        col
    });
    app.run();
}

/// Like Music's Now Playing: a scrolling page with a centred `n` x `n` image
/// between a ticking position label and the controls, redrawn every loop turn.
fn ui_image_page(n: i32) {
    use ecm_ui::element::{button, column, image, label, row, scroll_view, spacer};
    use std::cell::Cell;
    use std::rc::Rc;
    let mut app = ecm_ui::App::new("dev.test.image");
    let mut rgba = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            rgba.extend_from_slice(&[(x * 255 / n.max(1)) as u8, (y * 255 / n.max(1)) as u8, ((x ^ y) & 0xFF) as u8, 255]);
        }
    }
    let res = app.core.image_rgba(n, n, &rgba).unwrap_or(0);
    let clicks = Rc::new(Cell::new(0u32));
    let turns = Rc::new(Cell::new(0u32));
    let (view_clicks, view_turns) = (clicks.clone(), turns.clone());
    app.window("Image Test", move |_| {
        let c = view_clicks.clone();
        let page = column()
            .gap(8)
            .child(label(format!("turn {}", view_turns.get())))
            .child(row().child(spacer()).child(image(res, n, n).id("art")).child(spacer()))
            .child(button(format!("Clicks {}", view_clicks.get())).on_click(move |_, _| c.set(c.get() + 1)))
            .child(label("one"))
            .child(label("two"))
            .child(label("three"));
        column().gap(4).grow(1).child(scroll_view(page).id("page").grow(1))
    });
    // A Now Playing session about once a second, with 64x64 art (as Music sends).
    let art: Vec<u8> = (0..64 * 64).flat_map(|i| [(i % 64 * 4) as u8, (i / 64 * 4) as u8, 128, 255]).collect();
    while app.step(50) {
        turns.set(turns.get() + 1);
        if turns.get() % 20 == 1 {
            app.core.media_session(&ecm_ui::MediaSession {
                title: "Song".into(),
                artist: "Artist".into(),
                art: Some((64, 64, art.clone())),
                state: ecm_ui::MediaState::Playing,
                position_ms: turns.get() * 50,
                duration_ms: 600_000,
                actions: vec!["pause".into(), "next".into()],
                ..Default::default()
            });
        }
    }
}
