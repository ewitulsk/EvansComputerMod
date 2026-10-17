//! Remote shell sessions (SSH).
//!
//! `sshd` is an ordinary WASI program. When a client logs in it asks the
//! kernel for a shell session (`ipc_spawn_shell`), then relays bytes both
//! ways (`session_write` / `session_read`). Each session is a complete shell
//! of its own — line editor, working directory, job control, optionally the
//! switch CLI — whose terminal is a byte buffer instead of the screen.
//! Programs started from it run like any other job, with their output going
//! to that session. A session dies with the `sshd` process that owns it.
//!
//! A *command session* (`ipc_spawn_command`) runs one command line instead:
//! no banner and no prompt, and it is marked exited when the command ends.
//! Any program can use one to run another as a normal process and read its
//! output (the Visual Programmer's Run).

use std::collections::BTreeMap;

use ecm_bridge::cli::{self, CliSession};

use crate::console::Term;
use crate::jobs::{JobEvent, Jobs};
use crate::lineedit::{self, Editor, Event, InputRules, NoCompletion, Piece, ShellCompleter, TermOut};
use ecm_vte::ModeScanner;
use crate::net::Net;
use crate::shell::{Outcome, Shell, SwitchCmd};
use crate::switch_svc::SwitchService;

/// Output buffered for the client before we stop accepting more (the
/// session's programs are throttled by the job output budget meanwhile).
const OUT_CAP: usize = 256 * 1024;
const MAX_SESSIONS: usize = 8;

/// A terminal that is a byte stream to a remote client. Newlines become
/// CRLF, as a remote terminal expects.
pub struct RemoteTerm {
    out: Vec<u8>,
    width: u16,
    height: u16,
    last: u8,
    /// The modes the session's programs switched (alternate screen, Ctrl+C, paste).
    modes: ModeScanner,
}

impl RemoteTerm {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            width: 80,
            height: 24,
            last: 0,
            modes: ModeScanner::default(),
        }
    }
    fn full(&self) -> bool {
        self.out.len() >= OUT_CAP
    }
}

impl Term for RemoteTerm {
    fn write(&mut self, data: &[u8]) {
        self.modes.feed(data);
        for &b in data {
            if self.out.len() >= OUT_CAP {
                return;
            }
            if b == b'\n' && self.last != b'\r' {
                self.out.push(b'\r');
            }
            self.out.push(b);
            self.last = b;
        }
    }
    fn width(&self) -> u16 {
        self.width
    }
    fn height(&self) -> u16 {
        self.height
    }
}

pub struct Session {
    /// The sshd process that owns this session.
    pub owner: i32,
    pub user: String,
    term: RemoteTerm,
    editor: Editor,
    shell: Shell,
    jobs: Jobs,
    cli: Option<CliSession>,
    router_cli: Option<ecm_router::cli::Session>,
    pub exited: bool,
    /// Runs one command line, then exits (no banner, no prompt).
    one_shot: bool,
}

pub struct Sessions {
    map: BTreeMap<i32, Session>,
    next_id: i32,
}

/// Kernel services a session needs to run commands.
pub struct Services<'a> {
    pub net: &'a mut Net,
    pub switch: &'a mut SwitchService,
    pub router: &'a mut crate::router_svc::RouterService,
}

impl Sessions {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
            next_id: 1,
        }
    }

    pub fn any_jobs(&self) -> bool {
        self.map.values().any(|s| s.jobs.any_running())
    }

    /// Create a session for `owner`. Returns its id.
    pub fn spawn(&mut self, owner: i32, user: &str) -> Option<i32> {
        if self.map.len() >= MAX_SESSIONS {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let mut s = Session {
            owner,
            user: user
                .chars()
                .filter(|c| c.is_ascii_graphic())
                .take(32)
                .collect(),
            term: RemoteTerm::new(),
            editor: Editor::new(),
            shell: Shell::new_remote(),
            jobs: Jobs::new(),
            cli: None,
            router_cli: None,
            exited: false,
            one_shot: false,
        };
        s.term.println(&format!(
            "Terminal OS - logged in as {}.",
            if s.user.is_empty() { "?" } else { &s.user }
        ));
        s.term
            .println("Type 'help' for commands, 'exit' to log out.");
        s.term.println("");
        lineedit::load_history(&mut s.editor);
        prompt(&mut s);
        self.map.insert(id, s);
        Some(id)
    }

    /// Create a command session for `owner` running `line`. Returns its id.
    pub fn spawn_command(&mut self, owner: i32, user: &str, line: &str, svc: &mut Services, now: i64) -> Option<i32> {
        if self.map.len() >= MAX_SESSIONS {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        let mut s = Session {
            owner,
            user: user.chars().filter(|c| c.is_ascii_graphic()).take(32).collect(),
            term: RemoteTerm::new(),
            editor: Editor::new(),
            shell: Shell::new_remote(),
            jobs: Jobs::new(),
            cli: None,
            router_cli: None,
            exited: false,
            one_shot: true,
        };
        run_line(&mut s, line, svc, now);
        self.map.insert(id, s);
        Some(id)
    }

    pub fn get(&self, id: i32) -> Option<&Session> {
        self.map.get(&id)
    }

    pub fn resize(&mut self, id: i32, w: u16, h: u16) -> bool {
        match self.map.get_mut(&id) {
            Some(s) => {
                s.term.width = w.clamp(10, 1000);
                s.term.height = h.clamp(4, 1000);
                true
            }
            None => false,
        }
    }

    /// Drain up to `max` bytes of output.
    pub fn read(&mut self, id: i32, max: usize) -> Option<Vec<u8>> {
        let s = self.map.get_mut(&id)?;
        let n = max.min(s.term.out.len());
        Some(s.term.out.drain(..n).collect())
    }

    pub fn has_output(&self, id: i32) -> bool {
        self.map.get(&id).is_some_and(|s| !s.term.out.is_empty())
    }

    /// Close a session: kill its programs. Returns the pids that were running.
    pub fn close(&mut self, id: i32) -> Vec<i32> {
        match self.map.remove(&id) {
            Some(mut s) => s.jobs.kill_all(),
            None => Vec::new(),
        }
    }

    /// Close every session owned by `pid` (sshd exited).
    pub fn close_owned_by(&mut self, pid: i32) -> Vec<i32> {
        let ids: Vec<i32> = self
            .map
            .iter()
            .filter(|(_, s)| s.owner == pid)
            .map(|(&id, _)| id)
            .collect();
        ids.into_iter().flat_map(|id| self.close(id)).collect()
    }

    /// Keystrokes from the client.
    pub fn input(&mut self, id: i32, data: &[u8], svc: &mut Services, now: i64) -> bool {
        let Some(s) = self.map.get_mut(&id) else {
            return false;
        };
        let rules = if s.jobs.has_foreground() {
            InputRules::for_program(s.term.modes.modes, false)
        } else {
            InputRules::for_prompt(false)
        };
        for piece in lineedit::split_input(data, rules) {
            if s.exited {
                break;
            }
            match piece {
                Piece::Interrupt | Piece::Terminate if s.jobs.has_foreground() => {
                    s.jobs.kill_foreground();
                    s.term.println("^C");
                }
                Piece::Terminate | Piece::Interrupt => {
                    s.editor.clear();
                    s.term.println("^C");
                    prompt(s);
                }
                Piece::Scroll(_) => {}
                Piece::Data(d) => input_data(s, &d, svc, now),
            }
        }
        true
    }

    /// Pump every session's programs. Returns pids that exited (their socket
    /// sessions must be released).
    pub fn pump(&mut self) -> Vec<i32> {
        let mut exited = Vec::new();
        for s in self.map.values_mut() {
            if s.term.full() {
                continue; // client isn't reading; let programs block on their pipe
            }
            for ev in s.jobs.pump(&mut s.term) {
                match ev {
                    JobEvent::Exited(pid) => exited.push(pid),
                    JobEvent::ForegroundDone(status) => {
                        s.term.modes = ModeScanner::default();
                        if status != 0 {
                            s.term
                                .println(&format!("Process exited with code {}", status));
                        }
                        if !s.one_shot {
                            s.term.println("");
                        }
                        prompt(s);
                    }
                }
            }
        }
        exited
    }
}

/// Keys for the line editor, or the rest of them for a program that started.
fn input_data(s: &mut Session, data: &[u8], svc: &mut Services, now: i64) {
    let mut i = 0;
    while i < data.len() && !s.exited {
        if s.jobs.has_foreground() {
            s.jobs.send_input(&data[i..]);
            return;
        }
        let b = data[i];
        i += 1;
        let ev = if s.cli.is_none() && s.router_cli.is_none() {
            let cwd = s.shell.cwd.clone();
            s.editor.feed_byte(b, &mut TermOut(&mut s.term), &mut ShellCompleter { cwd: &cwd })
        } else {
            s.editor.feed_byte(b, &mut TermOut(&mut s.term), &mut NoCompletion)
        };
        match ev {
            Some(Event::Line(line)) => {
                lineedit::save_history(&mut s.editor);
                run_line(s, &line, svc, now);
            }
            Some(Event::Interrupt) | Some(Event::Terminate) => {
                s.editor.clear();
                prompt(s);
            }
            Some(Event::Eof) | None => {}
        }
    }
}

fn prompt(s: &mut Session) {
    for n in s.jobs.take_notices() {
        s.term.println(&n);
    }
    if s.one_shot {
        // The command is done (or never started a foreground job).
        if !s.jobs.has_foreground() {
            s.exited = true;
        }
        return;
    }
    let p = if let Some(r) = &s.router_cli {
        r.prompt()
    } else {
        match &s.cli {
            Some(c) => cli::prompt(c),
            None => s.shell.prompt(),
        }
    };
    s.editor.start(&p, &mut TermOut(&mut s.term));
}

fn run_line(s: &mut Session, line: &str, svc: &mut Services, now: i64) {
    if let Some(mut c) = s.router_cli.take() {
        if !svc.router.exec_in(&mut c, line, svc.net, &mut s.term, now) {
            s.router_cli = Some(c);
        }
        prompt(s);
        return;
    }
    if let Some(mut c) = s.cli.take() {
        let leave = svc.switch.exec_in(&mut c, line, svc.net, &mut s.term, now);
        if !leave && SwitchService::running(svc.net) {
            s.cli = Some(c);
        } else {
            s.term.println("Exited switch mode.");
        }
        prompt(s);
        return;
    }
    match s.shell.execute(line, &mut s.term) {
        Outcome::Done => {}
        Outcome::Exit => {
            s.term.println("Connection closed.");
            s.exited = true;
            return;
        }
        Outcome::Foreground(pids, desc) => {
            s.jobs.start_foreground(pids, desc);
            return;
        }
        Outcome::Background(pids, desc) => {
            let first = pids[0];
            let id = s.jobs.start_background(pids, desc);
            s.term.println(&format!("[{}] {}", id, first));
        }
        Outcome::Resume(spec) => {
            if s.jobs.foreground(&spec) {
                return;
            }
            s.term.println("fg: no such job");
        }
        Outcome::ListJobs => s.jobs.list(&mut s.term),
        Outcome::History(args) => lineedit::history_cmd(&mut s.editor, &args, &mut s.term),
        Outcome::Switch(cmd) => match cmd {
            SwitchCmd::EnterCli => s.cli = svc.switch.enter_remote(svc.net, &mut s.term, now),
            SwitchCmd::StartDetached => svc.switch.start_detached(svc.net, &mut s.term, now),
            SwitchCmd::Stop => svc.switch.stop_cmd(svc.net, &mut s.term),
        },
        Outcome::Router(cmd) => match cmd {
            SwitchCmd::EnterCli => {
                if svc.router.start(svc.net, &mut s.term, now) {
                    s.router_cli = Some(ecm_router::cli::Session::new());
                }
            }
            SwitchCmd::StartDetached => svc.router.start_detached(svc.net, &mut s.term, now),
            SwitchCmd::Stop => svc.router.stop_cmd(svc.net, &mut s.term),
        },
        Outcome::GfxTest(_) => s.term.println("gfxtest: not available over SSH"),
    }
    prompt(s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_session_has_no_prompt_and_ends_with_its_command() {
        let mut s = Session {
            owner: 1,
            user: "visual".into(),
            term: RemoteTerm::new(),
            editor: Editor::new(),
            shell: Shell::new_remote(),
            jobs: Jobs::new(),
            cli: None,
            router_cli: None,
            exited: false,
            one_shot: true,
        };
        // A builtin finishes at once: the session ends without printing a prompt.
        s.shell.cwd.clear();
        prompt(&mut s);
        assert!(s.exited);
        assert!(!String::from_utf8_lossy(&s.term.out).contains('>'));
    }

    #[test]
    fn remote_term_converts_newlines_and_bounds_output() {
        let mut t = RemoteTerm::new();
        t.write(b"a\nb\r\nc");
        assert_eq!(t.out, b"a\r\nb\r\nc");
        let big = vec![b'x'; OUT_CAP * 2];
        t.write(&big);
        assert!(t.out.len() <= OUT_CAP);
    }
}
