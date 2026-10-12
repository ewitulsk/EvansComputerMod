//! `test-shell` — the minimal shell of ROADMAP P2: it tiles every window side
//! by side under a one-line status bar, focuses new windows and clicked ones,
//! and binds three chords. Enough to prove the shell protocol (§8); the real
//! desktop is P3.
//!
//!   test-shell [--launch PROGRAM[:ARG:ARG...]]...
//!
//! Chords: Alt+Tab focuses the next window, Alt+N launches `notes`, Alt+F4
//! asks the focused window to close.

use ecm_ui::shell::{keys, tiling, Bar, Shell, ShellEvent, Wallpaper};
use ecm_ui_proto as proto;

const CHORD_NEXT: u32 = 1;
const CHORD_NOTES: u32 = 2;
const CHORD_CLOSE: u32 = 3;

fn main() {
    let conn = match ecm_ui::conn::default_connection(proto::host::ROLE_SHELL) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("test-shell: {e}");
            std::process::exit(1);
        }
    };
    let mut sh = match Shell::connect(conn) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("test-shell: connect: {e}");
            std::process::exit(1);
        }
    };
    println!("test-shell: connected, display {}x{} scale {}", sh.display.w, sh.display.h, sh.display.scale);

    let mut bar = Bar::new(1, tiling::BAR_H);
    let mut wallpaper = Wallpaper::new(2, tiling::WALLPAPER);
    sh.send(bar.create(proto::Layer::Top));
    sh.send(wallpaper.create());
    sh.bind_chord(CHORD_NEXT, proto::mods::ALT, &[keys::TAB]);
    sh.bind_chord(CHORD_NOTES, proto::mods::ALT, &[keys::N]);
    sh.bind_chord(CHORD_CLOSE, proto::mods::ALT, &[keys::F4]);

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--launch" {
            if let Some(spec) = args.next() {
                let mut parts = spec.split(':').map(String::from);
                if let Some(prog) = parts.next() {
                    let rest: Vec<String> = parts.collect();
                    sh.launch(&prog, &rest);
                }
            }
        }
    }

    let mut placed: Vec<u32> = Vec::new();
    loop {
        bar.set_text(&tiling::bar_text(&sh));
        for m in bar.render().into_iter().chain(wallpaper.render()) {
            sh.send(m);
        }
        let events = match sh.step(-1) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("test-shell: {e}");
                std::process::exit(2);
            }
        };
        let mut relayout = false;
        for ev in events {
            match ev {
                ShellEvent::Opened(id) => {
                    println!("test-shell: window {id} opened");
                    sh.focus(id);
                    relayout = true;
                }
                ShellEvent::Closed(id) => {
                    println!("test-shell: window {id} closed");
                    if sh.focused.is_none() {
                        if let Some(last) = sh.windows.last().map(|w| w.id) {
                            sh.focus(last);
                        }
                    }
                    relayout = true;
                }
                ShellEvent::Changed(_) => {}
                ShellEvent::Request { window, action: proto::WindowAction::Activate, .. } => {
                    sh.focus(window);
                    relayout = true;
                }
                ShellEvent::Request { .. } => {}
                ShellEvent::Chord(CHORD_NEXT) => {
                    if let Some(next) = sh.next_window() {
                        sh.focus(next);
                        relayout = true;
                    }
                }
                ShellEvent::Chord(CHORD_NOTES) => sh.launch("notes", &[]),
                ShellEvent::Chord(CHORD_CLOSE) => {
                    if let Some(f) = sh.focused {
                        sh.close_window(f);
                    }
                }
                ShellEvent::Chord(_) => {}
                ShellEvent::Display(_) => relayout = true,
                ShellEvent::Configure(c) => {
                    if !bar.configure(&c) {
                        wallpaper.configure(&c);
                    }
                }
                ShellEvent::Message(_) => {}
            }
        }
        if relayout {
            tiling::retile(&mut sh, &mut placed);
        }
    }
}
