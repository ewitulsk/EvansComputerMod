//! `phone` — the ePhone shell (`dev.ecm.phone`, docs/gui/EPHONE.md). The
//! compositor of an ePhone starts it at boot (`--restart` after a crash: the
//! open apps are still there, so the last session isn't reopened again).
//!
//! The status comes from `/dev/uictl phone` (battery, signal, background
//! modes, MIC, media sessions, hotkey claims) once a second; the apps from
//! `app_ctl list` and their `icon.png` every few seconds; `/dev/mixer` twice
//! a second while the notification sheet is open (its volume strips).

use ecm_host_abi::apps;
use ecm_ui_proto as proto;
use phone_shell::{AppEntry, Env, PhoneShell, Status};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

struct Live;

impl Env for Live {
    fn clock(&mut self) -> String {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("{:02}:{:02}", (secs / 3600) % 24, (secs / 60) % 60)
    }

    fn status(&mut self) -> Status {
        Status::parse(&uictl("phone"))
    }

    fn apps(&mut self) -> Vec<AppEntry> {
        let Some((0, list)) = apps::ctl(&["list"]) else { return Vec::new() };
        let mut out = Vec::new();
        for line in String::from_utf8_lossy(&list).lines() {
            let p: Vec<&str> = line.split('\t').collect();
            if p.len() < 3 {
                continue;
            }
            let id = p[0].to_string();
            let icon = icon_of(&id).and_then(|f| std::fs::read(format!("/apps/{id}/{f}")).ok());
            out.push(AppEntry { id, name: p[2].to_string(), icon });
        }
        out
    }

    fn session(&mut self) -> Vec<String> {
        match apps::ctl(&["session"]) {
            Some((0, s)) => String::from_utf8_lossy(&s).lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect(),
            _ => Vec::new(),
        }
    }

    fn mixer(&mut self) -> String {
        std::fs::read_to_string(ecm_audio::mixer::DEVICE).unwrap_or_default()
    }

    fn mixer_command(&mut self, cmd: &str) {
        if let Err(e) = ecm_audio::mixer::command(cmd) {
            eprintln!("phone: mixer: {cmd}: {e}");
        }
    }
}

/// The `icon = "..."` of an installed app's manifest.
fn icon_of(id: &str) -> Option<String> {
    let toml = std::fs::read_to_string(format!("/apps/{id}/app.toml")).ok()?;
    for line in toml.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("icon") {
            let v = rest.trim_start().strip_prefix('=')?.trim();
            return Some(v.trim_matches('"').to_string());
        }
    }
    None
}

fn uictl(cmd: &str) -> String {
    let Ok(mut dev) = std::fs::OpenOptions::new().read(true).write(true).open("/dev/uictl") else { return String::new() };
    if dev.write_all(cmd.as_bytes()).is_err() {
        return String::new();
    }
    let mut answer = String::new();
    let _ = dev.read_to_string(&mut answer);
    answer
}

fn main() {
    let restarted = std::env::args().nth(1).as_deref() == Some("--restart");
    let conn = match ecm_ui::conn::default_connection(proto::host::ROLE_SHELL) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("phone: {e}");
            std::process::exit(1);
        }
    };
    let mut ps = match PhoneShell::new(conn, Live, restarted) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("phone: connect: {e}");
            std::process::exit(1);
        }
    };
    println!("phone: shell up, display {}x{}{}", ps.sh.display.w, ps.sh.display.h, if restarted { " (restarted)" } else { "" });
    let mut last_status = Instant::now();
    let mut last_apps = Instant::now();
    let mut last_mixer = Instant::now();
    loop {
        if let Err(e) = ps.render() {
            eprintln!("phone: {e}");
            std::process::exit(2);
        }
        // While the notification sheet is open its meters move: wake up for them.
        let timeout = if ps.sheet_open() { 250 } else { 1000 };
        let events = match ps.sh.step(timeout) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("phone: {e}");
                std::process::exit(2);
            }
        };
        ps.handle(events);
        if last_status.elapsed() >= Duration::from_millis(1000) {
            last_status = Instant::now();
            ps.refresh_status();
        }
        // The sheet's volume strips, about twice a second, only while it is open.
        if ps.sheet_open() && last_mixer.elapsed() >= Duration::from_millis(500) {
            last_mixer = Instant::now();
            ps.refresh_mixer();
        }
        if last_apps.elapsed() >= Duration::from_secs(4) {
            last_apps = Instant::now();
            ps.refresh_apps();
        }
    }
}
