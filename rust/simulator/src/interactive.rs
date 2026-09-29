//! Interactive mode: one terminal view per node, switchable.
//!
//! Keys: everything goes to the selected computer, except
//! - F1..F12 or Ctrl+N / Ctrl+P: switch node
//! - Ctrl+]: simulator command line (any scenario step, e.g.
//!   `link down h1:eth0`, `pcap link=h1:eth0 file=x.pcap`, `frames link=..`,
//!   `link set h1:eth0 drop=20`), Enter runs it, Esc cancels
//! - Ctrl+Q: quit
//!
//! Ctrl+T is delivered like the Minecraft terminal does it (interrupt a
//! stuck kernel call + IRQ_TERMINATE).

use std::io::{stdout, Write};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::{cursor, execute, queue, terminal};

use crate::scenario;
use crate::sim::{Sim, SimConfig};
use crate::topology::TopoFile;

enum Ui {
    Key(KeyEvent),
    Resize,
}

fn key_bytes(k: &KeyEvent) -> Option<Vec<u8>> {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    Some(match k.code {
        KeyCode::Char(c) if ctrl && c.is_ascii_alphabetic() => vec![(c.to_ascii_lowercase() as u8) & 0x1f],
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => b"\n".to_vec(),
        KeyCode::Backspace => vec![0x08],
        KeyCode::Tab => b"\t".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    })
}

fn color(n: u8) -> Color {
    Color::AnsiValue(n & 0x0f)
}

struct View {
    selected: usize,
    cmdline: Option<String>,
    message: String,
    drawn_version: u64,
    force: bool,
}

fn render(sim: &Sim, v: &mut View) -> std::io::Result<()> {
    let node = &sim.nodes[v.selected];
    let s = &node.kernel.screen;
    if !v.force && s.version == v.drawn_version {
        return Ok(());
    }
    v.force = false;
    v.drawn_version = s.version;
    let (tw, th) = terminal::size().unwrap_or((80, 25));
    let (tw, th) = (tw as usize, th as usize);
    let rows = s.h.min(th.saturating_sub(1));
    let cols = s.w.min(tw);
    let mut out = stdout();
    queue!(out, cursor::Hide, cursor::MoveTo(0, 0))?;
    for y in 0..rows {
        queue!(out, cursor::MoveTo(0, y as u16))?;
        let mut last_attr: Option<u8> = None;
        let mut run = String::new();
        for x in 0..cols {
            let c = s.cells.get(y * s.w + x).copied().unwrap_or([b' ', 0x0a, 0]);
            let attr = if c[2] & 0x08 != 0 { c[1].rotate_left(4) } else { c[1] };
            if last_attr != Some(attr) {
                if !run.is_empty() {
                    queue!(out, Print(std::mem::take(&mut run)))?;
                }
                queue!(out, SetForegroundColor(color(attr)), SetBackgroundColor(color(attr >> 4)))?;
                last_attr = Some(attr);
            }
            run.push(if (0x20..0x7f).contains(&c[0]) { c[0] as char } else { ' ' });
        }
        queue!(out, Print(run), ResetColor)?;
    }
    // Status line.
    let names: Vec<String> = sim
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let f = if n.kernel.faulted.is_some() { "!" } else { "" };
            if i == v.selected {
                format!("[F{} {}{}]", i + 1, n.name, f)
            } else {
                format!(" F{} {}{} ", i + 1, n.name, f)
            }
        })
        .collect();
    let status = match &v.cmdline {
        Some(c) => format!("sim> {}", c),
        None => format!(
            "{}  t={:.1}s  {}  ^]=sim cmd ^N/^P=switch ^Q=quit",
            names.join(""),
            sim.sched.elapsed() as f64 / 1000.0,
            v.message
        ),
    };
    let mut status: String = status.chars().take(tw.saturating_sub(1)).collect();
    while status.len() < tw.saturating_sub(1) {
        status.push(' ');
    }
    queue!(
        out,
        cursor::MoveTo(0, th.saturating_sub(1) as u16),
        SetForegroundColor(Color::Black),
        SetBackgroundColor(Color::Grey),
        Print(status),
        ResetColor
    )?;
    if v.cmdline.is_none() && s.cursor_visible && s.cursor.1 < rows && s.cursor.0 < cols {
        queue!(out, cursor::MoveTo(s.cursor.0 as u16, s.cursor.1 as u16), cursor::Show)?;
    }
    out.flush()
}

pub fn run(cfg: SimConfig, topo: &TopoFile) -> Result<()> {
    let mut sim = Sim::new(cfg, topo)?;
    let (tx, rx) = mpsc::channel::<Ui>();
    let sched = sim.sched.clone();
    std::thread::spawn(move || loop {
        match event::read() {
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => {
                if tx.send(Ui::Key(k)).is_err() {
                    break;
                }
                sched.notify_event();
            }
            Ok(Event::Resize(_, _)) => {
                let _ = tx.send(Ui::Resize);
                sched.notify_event();
            }
            Ok(_) => {}
            Err(_) => break,
        }
    });

    terminal::enable_raw_mode()?;
    execute!(stdout(), terminal::EnterAlternateScreen, terminal::Clear(terminal::ClearType::All))?;
    let mut v = View { selected: 0, cmdline: None, message: String::new(), drawn_version: u64::MAX, force: true };
    let mut boot_sent = false;
    let result = (|| -> Result<()> {
        loop {
            sim.pump(None);
            if !boot_sent && (0..sim.nodes.len()).all(|i| sim.at_prompt(i)) {
                boot_sent = true;
                // Boot commands are typed without waiting, one prompt each.
                let _ = sim.run_boot_commands(30_000);
            }
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    Ui::Resize => {
                        execute!(stdout(), terminal::Clear(terminal::ClearType::All))?;
                        v.force = true;
                    }
                    Ui::Key(k) => {
                        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
                        if let Some(cmd) = v.cmdline.as_mut() {
                            match k.code {
                                KeyCode::Enter => {
                                    let c = v.cmdline.take().unwrap_or_default();
                                    v.message = run_command(&mut sim, &c);
                                }
                                KeyCode::Esc => v.cmdline = None,
                                KeyCode::Backspace => {
                                    cmd.pop();
                                }
                                KeyCode::Char(ch) => cmd.push(ch),
                                _ => {}
                            }
                            v.force = true;
                            continue;
                        }
                        match k.code {
                            KeyCode::Char('q') if ctrl => return Ok(()),
                            KeyCode::Char(']') if ctrl => {
                                v.cmdline = Some(String::new());
                                v.force = true;
                            }
                            KeyCode::Char('5') if ctrl => {
                                // Ctrl+] arrives as Ctrl+5 on some terminals.
                                v.cmdline = Some(String::new());
                                v.force = true;
                            }
                            KeyCode::Char('n') if ctrl => {
                                v.selected = (v.selected + 1) % sim.nodes.len();
                                v.force = true;
                            }
                            KeyCode::Char('p') if ctrl => {
                                v.selected = (v.selected + sim.nodes.len() - 1) % sim.nodes.len();
                                v.force = true;
                            }
                            KeyCode::F(n) if (n as usize) <= sim.nodes.len() && n >= 1 => {
                                v.selected = n as usize - 1;
                                v.force = true;
                            }
                            _ => {
                                if let Some(b) = key_bytes(&k) {
                                    let sel = v.selected;
                                    sim.type_raw(sel, &b);
                                }
                            }
                        }
                    }
                }
            }
            render(&sim, &mut v)?;
            std::thread::sleep(Duration::from_millis(0));
        }
    })();
    let _ = execute!(stdout(), terminal::LeaveAlternateScreen, cursor::Show);
    let _ = terminal::disable_raw_mode();
    sim.shutdown();
    result
}

fn run_command(sim: &mut Sim, line: &str) -> String {
    match scenario::parse_steps(line) {
        Err(e) => e,
        Ok(steps) if steps.is_empty() => String::new(),
        Ok(steps) => {
            let mut msgs = Vec::new();
            let r = scenario::run_steps(sim, &steps, 10_000, &mut |s| msgs.push(s.trim().to_string()));
            match r {
                Ok(()) => msgs.last().cloned().unwrap_or_default(),
                Err(f) => f.reason,
            }
        }
    }
}
