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
//!   half-second writes) for as long as it runs.
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
        _ => "Counter",
    };
    let win = core.create_window(title, Content::Static(label("")));
    if mode == "counter" {
        core.claim_hotkey(1, "Bump");
    }
    let mut audio = None;
    if mode == "tone" {
        if let Ok(mut ctl) = std::fs::OpenOptions::new().write(true).open("/dev/audioctl") {
            let _ = ctl.write_all(b"rate 8000\nbits 8\nchannels 1\nlatency 500\n");
        }
        audio = std::fs::OpenOptions::new().write(true).open("/dev/audio").ok();
        println!("phonetest: audio {}", if audio.is_some() { "open" } else { "unavailable" });
    }
    // Half a second of a 440 Hz square wave, 8-bit unsigned at 8 kHz.
    let tone: Vec<u8> = (0..4000).map(|i| if (i * 440 / 4000) % 2 == 0 { 0xa0 } else { 0x60 }).collect();
    let mut last = Instant::now();
    let mut last_wall = std::time::SystemTime::now();
    loop {
        core.set_root(win, column().padding(8).child(label(format!("{title} {count}"))));
        core.mark_all_dirty();
        if core.render().is_err() {
            break;
        }
        let timeout = if audio.is_some() { 0 } else { 100 };
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
            // Blocks until the speaker has room: paced by its clock.
            if a.write_all(&tone).is_err() {
                audio = None;
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
