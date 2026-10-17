//! `phonetest` — probe apps for the ePhone lifecycle tests (PhoneKernelTest,
//! the `phone_gui` scenario and the `ecm_phone` GameTests). Installed as an
//! app; the mode comes from its app id (`ECM_APP_ID`) or the first argument:
//!
//! * `...counter`: a window "Counter N" counting every 250 ms of its own run
//!   time; hotkey slot 1 ("Bump") adds 100. Saves `{"count": N}` on
//!   WillSuspend and starts from it after Restore.
//! * `...refresh`: counts like the counter and posts a notification
//!   "Refresh N" every fourth tick and on the first tick of a refresh wake
//!   (a gap in wall time its own clock didn't see).
//! * `...tone`: plays a 440 Hz square wave on `/dev/audio` (8 kHz, 8-bit,
//!   half-second writes) for as long as it runs (the `audio` mode starts by itself).
//! * `...voice`: a call: says `BackgroundMode{call, on}`, then plays a louder 660 Hz tone as a
//!   `voice` stream (ducking media on the same speaker) in bursts: 1 s on, 1 s off.
//! * `...wake`: listens on TCP port 7999 with wake sockets (SO_ECM_WAKE: the listener and
//!   every connection it accepts) and posts a notification "Wake: <line>" for every line.
//!
//! Every mode writes its count to `$ECM_APP_DATA/count` on each tick, so a
//! test can watch it without the GUI.

use ecm_ui::element::{column, label};
use ecm_ui::window::Content;
use ecm_ui::{AppEvent, Core, Json};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::io::Write;
use std::time::{Duration, Instant};

fn main() {
    let id = std::env::var("ECM_APP_ID").unwrap_or_else(|_| std::env::args().nth(1).unwrap_or_else(|| "dev.ecm.test.counter".into()));
    let mode = id.rsplit('.').next().unwrap_or("counter").to_string();
    let data = std::env::var("ECM_APP_DATA").unwrap_or_else(|_| ".".into());
    let conn = match ecm_ui::conn::default_connection(proto::host::ROLE_APP) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("phonetest: {e}");
            std::process::exit(1);
        }
    };
    let mut core = match Core::connect(&id, conn, proto::Role::App) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("phonetest: connect: {e}");
            std::process::exit(1);
        }
    };
    let mut count: i64 = core
        .restore()
        .and_then(|b| std::str::from_utf8(b).ok())
        .and_then(|s| Json::parse(s).ok())
        .and_then(|j| j.get("count").and_then(|c| c.as_i64()))
        .unwrap_or(0);
    println!("phonetest: {mode} starts at {count}");
    let title = match mode.as_str() {
        "refresh" => "Refresh",
        "tone" => "Tone",
        "voice" => "Voice",
        "wake" => "Wake",
        _ => "Counter",
    };
    let win = core.create_window(title, Content::Static(label("")));
    if mode == "counter" {
        core.claim_hotkey(1, "Bump");
    }
    let mut audio = None;
    if mode == "tone" || mode == "voice" {
        let role = if mode == "voice" { "role voice\n" } else { "" };
        if let Ok(mut ctl) = std::fs::OpenOptions::new().write(true).open("/dev/audioctl") {
            let _ = ctl.write_all(format!("rate 8000\nbits 8\nchannels 1\nlatency 500\n{role}").as_bytes());
        }
        audio = std::fs::OpenOptions::new().write(true).open("/dev/audio").ok();
        println!("phonetest: audio {}", if audio.is_some() { "open" } else { "unavailable" });
        if mode == "voice" {
            core.background_mode("call", true);
        }
    }
    let mut wake_sock = -1;
    let mut conns: Vec<(i32, Vec<u8>)> = Vec::new();
    if mode == "wake" {
        use ecm_host_abi::socket;
        let fd = socket::socket(socket::AF_INET, socket::SOCK_STREAM, 0);
        socket::setsockopt(fd, socket::SOL_SOCKET, socket::SO_REUSEADDR, &1i32.to_le_bytes());
        if fd >= 0 && socket::bind(fd, &socket::SockAddrIn::any(7999)) == 0 && socket::listen(fd, 4) == 0 {
            println!("phonetest: wake socket on TCP 7999 ({})", if socket::set_wake(fd, true) == 0 { "wake on" } else { "no wake" });
            wake_sock = fd;
        } else {
            println!("phonetest: no TCP listener");
        }
    }
    // Half a second of a square wave, 8-bit unsigned at 8 kHz: 440 Hz for tone, 660 Hz and louder for voice.
    let (hz, hi, lo) = if mode == "voice" { (660, 0xe0u8, 0x20u8) } else { (440, 0xa0, 0x60) };
    let tone: Vec<u8> = (0..4000).map(|i| if (i * hz / 4000) % 2 == 0 { hi } else { lo }).collect();
    let quiet = vec![0x80u8; 4000];
    let mut burst = 0u64;
    let mut last = Instant::now();
    let mut last_wall = std::time::SystemTime::now();
    loop {
        core.set_root(win, column().padding(8).child(label(format!("{title} {count}"))));
        core.mark_all_dirty();
        if core.render().is_err() {
            break;
        }
        let timeout = if audio.is_some() { 0 } else if wake_sock >= 0 { 50 } else { 100 };
        let events = match core.step(timeout) {
            Ok(e) => e,
            Err(_) => break,
        };
        for ev in events {
            match ev {
                AppEvent::Suspend { .. } => {
                    core.save_state(format!("{{\"count\": {count}}}").into_bytes());
                    let _ = core.flush();
                }
                AppEvent::Hotkey { slot: 1, down: true } => {
                    count += 100;
                    write_count(&data, count);
                }
                AppEvent::CloseRequested { .. } => return,
                _ => {}
            }
        }
        if let Some(a) = audio.as_mut() {
            // Blocks until the speaker has room: paced by its clock. A voice talks in 1 s bursts.
            burst += 1;
            let talk = mode != "voice" || (burst / 2) % 2 == 0;
            if a.write_all(if talk { &tone } else { &quiet }).is_err() {
                audio = None;
            }
        }
        if wake_sock >= 0 {
            use ecm_host_abi::socket::{self, PollFd, POLLIN};
            let mut fds = [PollFd { fd: wake_sock, events: POLLIN, revents: 0 }];
            socket::poll(&mut fds, 0);
            if fds[0].revents & POLLIN != 0 {
                let mut peer = socket::SockAddrIn::default();
                let c = socket::accept(wake_sock, &mut peer);
                if c >= 0 {
                    socket::set_wake(c, true);
                    conns.push((c, Vec::new()));
                }
            }
            let mut buf = [0u8; 512];
            let mut lines = Vec::new();
            conns.retain_mut(|(c, pending)| {
                let mut f = [PollFd { fd: *c, events: POLLIN, revents: 0 }];
                socket::poll(&mut f, 0);
                if f[0].revents == 0 {
                    return true;
                }
                let n = socket::recv(*c, &mut buf, socket::MSG_DONTWAIT);
                if n == -2 {
                    return true;
                }
                if n <= 0 {
                    socket::close(*c);
                    return false;
                }
                pending.extend_from_slice(&buf[..n as usize]);
                while let Some(i) = pending.iter().position(|&b| b == b'\n') {
                    let l: Vec<u8> = pending.drain(..=i).collect();
                    lines.push(String::from_utf8_lossy(&l).trim().to_string());
                }
                true
            });
            for text in lines {
                println!("phonetest: wake line '{text}'");
                core.send(Message::Notify(proto::Notify {
                    tag: "wake".into(),
                    title: format!("Wake: {text}"),
                    body: "arrived on a wake socket".into(),
                    icon: String::new(),
                    actions: Vec::new(),
                }));
                let _ = core.flush();
                // Done with what woke us: the refresh window may end now.
                core.send(Message::BackgroundDone(proto::BackgroundDone));
            }
        }
        if last.elapsed() >= Duration::from_millis(250) {
            last = Instant::now();
            count += 1;
            write_count(&data, count);
            // A gap in wall time the app's own clock didn't see: it was suspended and got a refresh wake.
            let now_wall = std::time::SystemTime::now();
            let woke = now_wall.duration_since(last_wall).map(|d| d > Duration::from_millis(1000)).unwrap_or(false);
            last_wall = now_wall;
            if mode == "refresh" && (woke || count % 4 == 0) {
                core.send(Message::Notify(proto::Notify {
                    tag: "refresh".into(),
                    title: format!("Refresh {count}"),
                    body: "posted from a background refresh".into(),
                    icon: String::new(),
                    actions: Vec::new(),
                }));
            }
        }
    }
}

fn write_count(dir: &str, count: i64) {
    let _ = std::fs::write(format!("{dir}/count"), count.to_string());
}
