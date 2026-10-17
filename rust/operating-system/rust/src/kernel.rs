//! The kernel: all state, and the handlers behind each export.
//!
//! Everything here runs to completion quickly and returns. Waiting happens
//! in the host between calls: `on_tick` returns the next time the kernel
//! wants to run, and host events (input, frames, child output, socket
//! requests) call in earlier.

use crate::console::{Console, Term};
use crate::gfx::Plane;
use crate::gfx_test::GfxTest;
use crate::jobs::{JobEvent, Jobs};
use crate::lineedit::{self, Editor, Event, InputRules, NoCompletion, Piece, ShellCompleter, TermOut};
use crate::net::ipc::{IpcEffects, SocketIpc, IPC_PENDING};
use crate::net::{self, min_opt, Net};
use crate::router_svc::RouterService;
use crate::sessions::{Services, Sessions};
use crate::shell::{Outcome, Shell, SwitchCmd};
use crate::switch_svc::{CliState, SwitchService};

// Remote shell session syscalls (sshd), carried over handle_sock_ipc.
// Must match WasiFunctions.java (SocketFd.SESSION_*).
const SESSION_SPAWN: i32 = 20;
const SESSION_WRITE: i32 = 21;
const SESSION_READ: i32 = 22;
const SESSION_READ_BLOCKING: i32 = 23;
const SESSION_STATUS: i32 = 24;
const SESSION_CLOSE: i32 = 25;
const SESSION_RESIZE: i32 = 26;
/// Wi-Fi control channel (`net/wifi.rs` WifiDev::ctl): one text request in,
/// `[status i32][reply]` out. Must match SocketFd.WIFI_CTL / simulator child.rs.
const WIFI_CTL: i32 = 48;

pub const IRQ_KEYBOARD: i32 = 1;
pub const IRQ_REDSTONE: i32 = 2;
pub const IRQ_NETWORK: i32 = 3;
pub const IRQ_MOUSE: i32 = 4;
/// Wi-Fi module: frames or transmit statuses ready (coalesced, like IRQ_NETWORK).
pub const IRQ_WIFI: i32 = 5;
pub const IRQ_TERMINATE: i32 = 15;

/// While a program runs, poll its output at least this often even if the
/// host misses a wake-up.
const JOB_POLL_MS: i64 = 50;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ui {
    /// Shell prompt is showing; input goes to the line editor.
    Prompt,
    /// A foreground program owns the keyboard.
    Program,
    /// The switch CLI is open.
    SwitchCli,
    RouterCli,
    /// `gfxtest` is running.
    Gfx,
}

pub struct Kernel {
    con: Console,
    editor: Editor,
    shell: Shell,
    jobs: Jobs,
    net: Net,
    ipc: SocketIpc,
    switch: SwitchService,
    router: RouterService,
    gfx: Option<(GfxTest, i64)>,
    ui: Ui,
    sessions: Sessions,
    /// pid -> (started_ms, timeout_ms) of a pending SESSION_READ_BLOCKING.
    session_waits: std::collections::BTreeMap<i32, (i64, i64)>,
}

impl Kernel {
    pub fn boot(now: i64) -> Kernel {
        let mut con = Console::new();
        con.clear();
        let mut net = Net::new(now);
        if net::config::load(&mut net.stack, now) {
            // Interfaces saved as `down` are physically disabled too.
            for i in 0..net.port_count() {
                if net.stack.iface(i).is_some_and(|f| !f.admin_up) {
                    crate::hal::net::set_admin_state(i, false);
                }
            }
            con.println("Network config restored.");
        }
        con.println(
            "================================================================================",
        );
        con.println(
            "                         TERMINAL OS v2.0                                       ",
        );
        con.println(
            "================================================================================",
        );
        con.println("");
        con.println("Welcome to Terminal OS!");
        con.println("Type 'help' for the commands, 'man intro' to get started.");
        con.println("");
        let mut k = Kernel {
            con,
            editor: Editor::new(),
            shell: Shell::new(),
            jobs: Jobs::new(),
            net,
            ipc: SocketIpc::new(),
            switch: SwitchService::new(),
            router: RouterService::new(),
            gfx: None,
            ui: Ui::Prompt,
            sessions: Sessions::new(),
            session_waits: std::collections::BTreeMap::new(),
        };
        for line in crate::services::commands() {
            // Kernel services cannot take foreground input during boot.
            if line == "switch on" {
                k.switch.start_detached(&mut k.net, &mut k.con, now);
            } else if line == "router on" {
                k.router.start_detached(&mut k.net, &mut k.con, now);
            } else if line.ends_with('&') {
                k.run_line(&line, now);
            }
        }
        lineedit::load_history(&mut k.editor);
        k.prompt();
        k
    }

    fn prompt(&mut self) {
        for n in self.jobs.take_notices() {
            self.con.println(&n);
        }
        let p = match self.ui {
            Ui::SwitchCli => self.switch.prompt(),
            Ui::RouterCli => self.router.prompt(),
            _ => self.shell.prompt(),
        };
        self.editor.start(&p, &mut TermOut(&mut self.con));
    }

    // ------------------------------------------------------------ input

    pub fn on_input(&mut self, bytes: &[u8], now: i64) {
        let rules = match self.ui {
            Ui::Program => InputRules::for_program(self.con.modes(), true),
            Ui::Gfx => InputRules::for_program(Default::default(), false),
            _ => InputRules::for_prompt(true),
        };
        for piece in lineedit::split_input(bytes, rules) {
            match piece {
                Piece::Scroll(by) => self.con.scroll(by),
                Piece::Terminate => {
                    self.con.live();
                    self.terminate(now);
                }
                Piece::Interrupt => {
                    self.con.live();
                    self.interrupt(now);
                }
                Piece::Suspend => {
                    self.con.live();
                    self.suspend();
                }
                Piece::Data(data) => {
                    self.con.live();
                    self.input_data(&data, now);
                }
            }
        }
    }

    /// Keys for whatever owns the keyboard: the line editor or the program.
    fn input_data(&mut self, data: &[u8], now: i64) {
        let mut i = 0;
        while i < data.len() {
            match self.ui {
                Ui::Program => {
                    // The rest of the chunk is the program's (it may have just started).
                    self.jobs.send_input(&data[i..]);
                    return;
                }
                Ui::Gfx => return,
                Ui::Prompt | Ui::SwitchCli | Ui::RouterCli => {
                    let b = data[i];
                    i += 1;
                    let ev = if self.ui == Ui::Prompt {
                        let cwd = self.shell.cwd.clone();
                        self.editor.feed_byte(b, &mut TermOut(&mut self.con), &mut ShellCompleter { cwd: &cwd })
                    } else {
                        self.editor.feed_byte(b, &mut TermOut(&mut self.con), &mut NoCompletion)
                    };
                    match ev {
                        Some(Event::Line(line)) => {
                            if self.ui == Ui::Prompt {
                                lineedit::save_history(&mut self.editor);
                            }
                            self.run_line(&line, now);
                        }
                        Some(Event::Terminate) => self.terminate(now),
                        Some(Event::Interrupt) => self.prompt(),
                        Some(Event::Eof) | None => {}
                    }
                }
            }
        }
    }

    /// Ctrl+Z while a program runs: pause it as a stopped job (`fg`/`bg`).
    fn suspend(&mut self) {
        if self.ui != Ui::Program {
            return;
        }
        if let Some((id, cmd)) = self.jobs.suspend_foreground() {
            self.con.println("^Z");
            self.con.println(&format!("[{id}]+  Stopped    {cmd}"));
            self.shell.status = 148;
            self.ui = Ui::Prompt;
            self.prompt();
        }
    }

    /// Ctrl+C while a program runs (and doesn't read Ctrl+C itself): stop it.
    fn interrupt(&mut self, now: i64) {
        if self.ui == Ui::Program {
            self.jobs.kill_foreground();
            self.con.println("^C");
            self.pump_jobs(now);
        }
    }

    fn run_line(&mut self, line: &str, now: i64) {
        if self.ui == Ui::RouterCli {
            if self.router.exec(line, &mut self.net, &mut self.con, now) {
                self.ui = Ui::Prompt;
            }
            self.prompt();
            return;
        }
        if self.ui == Ui::SwitchCli {
            match self.switch.exec(line, &mut self.net, &mut self.con, now) {
                CliState::Open => {}
                CliState::Closed => self.ui = Ui::Prompt,
            }
            self.prompt();
            return;
        }
        match self.shell.execute(line, &mut self.con) {
            Outcome::Done => {}
            Outcome::Foreground(pids, desc) => {
                self.jobs.start_foreground(pids, desc);
                self.ui = Ui::Program;
                return; // prompt when the job ends
            }
            Outcome::Background(pids, desc) => {
                let first = pids[0];
                let id = self.jobs.start_background(pids, desc);
                self.con.println(&format!("[{}] {}", id, first));
            }
            Outcome::Resume(spec) => {
                if let Some(cmd) = self.jobs.foreground(&spec) {
                    self.con.println(&cmd);
                    self.ui = Ui::Program;
                    return;
                }
                self.con.println("fg: no such job");
                self.shell.status = 1;
            }
            Outcome::ResumeBackground(spec) => match self.jobs.resume_background(&spec) {
                Ok(msg) => self.con.println(&msg),
                Err(e) => {
                    self.con.println(e);
                    self.shell.status = 1;
                }
            },
            Outcome::ListJobs => self.jobs.list(&mut self.con),
            Outcome::History(args) => lineedit::history_cmd(&mut self.editor, &args, &mut self.con),
            Outcome::Exit => self
                .con
                .println("Use Ctrl+T to stop a program; the shell always stays running."),
            Outcome::Switch(cmd) => match cmd {
                SwitchCmd::EnterCli => {
                    if self.switch.enter_cli(&mut self.net, &mut self.con, now) {
                        self.ui = Ui::SwitchCli;
                    }
                }
                SwitchCmd::StartDetached => {
                    self.switch
                        .start_detached(&mut self.net, &mut self.con, now)
                }
                SwitchCmd::Stop => self.switch.stop_cmd(&mut self.net, &mut self.con),
            },
            Outcome::Router(cmd) => match cmd {
                SwitchCmd::EnterCli => {
                    if self.router.enter_cli(&mut self.net, &mut self.con, now) {
                        self.ui = Ui::RouterCli;
                    }
                }
                SwitchCmd::StartDetached => {
                    self.router
                        .start_detached(&mut self.net, &mut self.con, now)
                }
                SwitchCmd::Stop => self.router.stop_cmd(&mut self.net, &mut self.con),
            },
            Outcome::GfxTest(args) => {
                if let Some((job, at)) = GfxTest::start(&args, &mut self.con, now) {
                    self.gfx = Some((job, at));
                    self.ui = Ui::Gfx;
                    return;
                }
            }
        }
        self.prompt();
    }

    /// Ctrl+T: stop whatever owns the terminal and return to the prompt.
    /// Background jobs and a background switch keep running.
    fn terminate(&mut self, now: i64) {
        match self.ui {
            Ui::Program => {
                self.jobs.kill_foreground();
                // Collect the exit so its IPC session is released promptly.
                self.pump_jobs(now);
            }
            Ui::Gfx => {
                if let Some((mut job, _)) = self.gfx.take() {
                    job.abort();
                }
            }
            Ui::SwitchCli => self.switch.leave_cli(&mut self.net, &mut self.con),
            Ui::RouterCli => self.router.leave_cli(),
            Ui::Prompt => {}
        }
        Plane::Terminal.reset();
        self.editor.clear();
        self.ui = Ui::Prompt;
        self.con.println("");
        self.con.println("^T - Program terminated");
        self.prompt();
    }

    // ------------------------------------------------------------ events

    pub fn on_interrupt(&mut self, irq: i32, _payload: &[u8], now: i64) {
        match irq {
            IRQ_NETWORK => {
                self.net.rx(now);
                self.show_console_log();
            }
            IRQ_WIFI => self.net.rx_wifi(now),
            IRQ_TERMINATE => self.terminate(now),
            // Keyboard/redstone/mouse IRQs are for WASI programs, which read
            // them through their own host functions.
            _ => {}
        }
        self.report_wakes();
    }

    /// Tell the host about wake sockets (SO_ECM_WAKE) that just became readable, so it can
    /// wake their suspended apps (docs/gui/APPS.md "Wake sockets").
    fn report_wakes(&mut self) {
        if !self.ipc.has_wake_sockets() {
            return;
        }
        for pid in self.ipc.wake_check(&self.net.stack, crate::hal::now_ms()) {
            crate::hal::socket_wake(pid);
        }
    }

    /// A child exited: release its sockets, and if it was an sshd, close
    /// the shell sessions it owned (and their programs).
    fn child_exited(&mut self, pid: i32) {
        self.ipc.destroy_session(&mut self.net.stack, pid);
        self.session_waits.remove(&pid);
        for killed in self.sessions.close_owned_by(pid) {
            self.ipc.destroy_session(&mut self.net.stack, killed);
        }
    }

    fn pump_jobs(&mut self, _now: i64) {
        for pid in self.sessions.pump() {
            self.child_exited(pid);
        }
        for ev in self.jobs.pump(&mut self.con) {
            match ev {
                JobEvent::Exited(pid) => self.child_exited(pid),
                JobEvent::ForegroundDone(status) => {
                    self.shell.status = status;
                    if self.ui == Ui::Program {
                        self.con.reset_modes();
                        if status != 0 {
                            self.con
                                .println(&format!("Process exited with code {}", status));
                        }
                        self.con.println("");
                        self.ui = Ui::Prompt;
                        self.prompt();
                    }
                }
            }
        }
    }

    /// The host installed a display panel with a different text grid: resize
    /// the console, keeping its text, and redraw the prompt line if the shell
    /// (or a CLI) is waiting for input. A running program keeps writing; it
    /// sees the new size through its terminal. Returns the size set.
    pub fn resize_console(&mut self, cols: u16, rows: u16) -> (u16, u16) {
        let (w, h) = (self.con.width(), self.con.height());
        if (w, h) == (cols, rows) {
            return (w, h);
        }
        let size = self.con.resize(cols, rows);
        if matches!(self.ui, Ui::Prompt | Ui::SwitchCli | Ui::RouterCli) {
            // The console re-wrapped the text; draw the line again in place.
            self.editor.refresh(&mut TermOut(&mut self.con));
        }
        size
    }

    /// Show switch log events (`logging console`) without losing what the
    /// user is typing: print them on their own lines, then redraw the prompt.
    fn show_console_log(&mut self) {
        let lines = self.net.take_console_log();
        if lines.is_empty() {
            return;
        }
        let at_prompt = matches!(self.ui, Ui::Prompt | Ui::SwitchCli | Ui::RouterCli);
        self.con.print("\r\x1b[K");
        for l in &lines {
            self.con.println(l);
        }
        if at_prompt {
            self.editor.reprint(&mut TermOut(&mut self.con));
        }
    }

    /// Timers, child output, gfx frames. Returns the next absolute deadline
    /// in ms, or -1 if the kernel has nothing scheduled.
    pub fn on_tick(&mut self, now: i64) -> i64 {
        self.pump_jobs(now);
        let mut deadline = self.net.poll(now);
        self.show_console_log();
        if let Some((job, at)) = self.gfx.as_mut() {
            if now >= *at {
                match job.step(&mut self.con, now) {
                    Some(next) => *at = next,
                    None => {
                        self.gfx = None;
                        if self.ui == Ui::Gfx {
                            self.ui = Ui::Prompt;
                            self.prompt();
                        }
                    }
                }
            }
            if let Some((_, at)) = self.gfx.as_ref() {
                deadline = min_opt(deadline, Some(*at));
            }
        }
        if self.jobs.any_running() || self.sessions.any_jobs() {
            deadline = min_opt(deadline, Some(now + JOB_POLL_MS));
        }
        deadline = min_opt(deadline, self.ipc.next_deadline());
        self.report_wakes();
        for &(start, timeout) in self.session_waits.values() {
            deadline = min_opt(deadline, Some(start + timeout));
        }
        deadline.map(|d| d.max(now)).unwrap_or(-1)
    }

    pub fn sock_ipc(
        &mut self,
        pid: i32,
        syscall: i32,
        args: &[u8],
        result: &mut [u8],
        now: i64,
    ) -> i32 {
        if syscall == WIFI_CTL {
            let (status, reply) = self.net.wifi_ctl(args, now);
            if result.len() < 4 {
                return -1;
            }
            result[..4].copy_from_slice(&status.to_le_bytes());
            let n = reply.len().min(result.len() - 4);
            result[4..4 + n].copy_from_slice(&reply[..n]);
            return (4 + n) as i32;
        }
        if (SESSION_SPAWN..=SESSION_RESIZE).contains(&syscall) {
            let r = self.session_ipc(pid, syscall, args, result, now);
            self.net.flush(now);
            return r;
        }
        // Tun sockets (radio0) are handled beside net::ipc, not inside it.
        if let Some(r) = self.net.tun_ipc(pid, syscall, args, result, now) {
            self.net.flush(now);
            self.report_wakes();
            return r;
        }
        let mut fx = IpcEffects::default();
        let r = self.ipc.dispatch(
            &mut self.net.stack,
            pid,
            syscall,
            args,
            result,
            now,
            &mut fx,
        );
        for (idx, up) in fx.admin {
            if idx < self.net.port_count() {
                crate::hal::net::set_admin_state(idx, up);
            }
        }
        if fx.config_changed {
            net::config::save(&self.net.stack);
        }
        // Socket calls queue frames; send them now rather than next tick.
        self.net.flush(now);
        self.report_wakes();
        if r == IPC_PENDING {
            IPC_PENDING
        } else {
            r
        }
    }

    /// Remote shell session syscalls from sshd. Result encoding matches the
    /// socket syscalls: `[status i32][payload]`, or IPC_PENDING.
    fn session_ipc(
        &mut self,
        pid: i32,
        syscall: i32,
        args: &[u8],
        result: &mut [u8],
        now: i64,
    ) -> i32 {
        fn i32_at(a: &[u8], off: usize) -> Option<i32> {
            a.get(off..off + 4)
                .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        }
        fn blob(a: &[u8], off: usize) -> Option<&[u8]> {
            let n = a
                .get(off..off + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)?;
            a.get(off + 2..off + 2 + n)
        }
        let out = |result: &mut [u8], status: i32, payload: &[u8]| -> i32 {
            if result.len() < 4 {
                return -1;
            }
            result[..4].copy_from_slice(&status.to_le_bytes());
            let n = payload.len().min(result.len() - 4);
            result[4..4 + n].copy_from_slice(&payload[..n]);
            (4 + n) as i32
        };
        if syscall == SESSION_SPAWN {
            let user = blob(args, 0)
                .and_then(|b| core::str::from_utf8(b).ok())
                .unwrap_or("");
            // [user][command]: a command session (ipc_spawn_command).
            let command = blob(args, 0)
                .map(|u| 2 + u.len())
                .and_then(|off| blob(args, off))
                .and_then(|b| core::str::from_utf8(b).ok())
                .filter(|c| !c.trim().is_empty());
            if let Some(line) = command {
                let line = line.to_string();
                let user = user.to_string();
                let mut svc = Services {
                    net: &mut self.net,
                    switch: &mut self.switch,
                    router: &mut self.router,
                };
                let id = self.sessions.spawn_command(pid, &user, &line, &mut svc, now);
                self.pump_jobs(now);
                return match id {
                    Some(id) => out(result, id, &[]),
                    None => out(result, -1, &[]),
                };
            }
            return match self.sessions.spawn(pid, user) {
                Some(id) => out(result, id, &[]),
                None => out(result, -1, &[]),
            };
        }
        let Some(id) = i32_at(args, 0) else {
            return out(result, -1, &[]);
        };
        // Only the sshd that created a session may drive it.
        let Some(owner) = self.sessions.get(id).map(|s| s.owner) else {
            return out(result, -1, &[]);
        };
        if owner != pid {
            return out(result, -1, &[]);
        }
        match syscall {
            SESSION_WRITE => {
                let Some(data) = blob(args, 4) else {
                    return out(result, -1, &[]);
                };
                let data = data.to_vec();
                let mut svc = Services {
                    net: &mut self.net,
                    switch: &mut self.switch,
                    router: &mut self.router,
                };
                self.sessions.input(id, &data, &mut svc, now);
                // The line may have started programs or produced output.
                self.pump_jobs(now);
                out(result, data.len() as i32, &[])
            }
            SESSION_READ | SESSION_READ_BLOCKING => {
                let max = i32_at(args, 4)
                    .unwrap_or(0)
                    .clamp(0, (result.len().saturating_sub(4)) as i32)
                    as usize;
                self.pump_jobs(now);
                if self.sessions.has_output(id) {
                    self.session_waits.remove(&pid);
                    let data = self.sessions.read(id, max).unwrap_or_default();
                    return out(result, data.len() as i32, &data);
                }
                if self.sessions.get(id).is_some_and(|s| s.exited) {
                    self.session_waits.remove(&pid);
                    return out(result, if syscall == SESSION_READ { 0 } else { -1 }, &[]);
                }
                if syscall == SESSION_READ {
                    return out(result, 0, &[]);
                }
                let timeout = i32_at(args, 8).unwrap_or(0).max(0) as i64;
                let (start, _) = *self.session_waits.entry(pid).or_insert((now, timeout));
                if now - start >= timeout {
                    self.session_waits.remove(&pid);
                    return out(result, 0, &[]);
                }
                IPC_PENDING
            }
            SESSION_STATUS => {
                let exited = self.sessions.get(id).is_some_and(|s| s.exited);
                out(result, exited as i32, &[])
            }
            SESSION_CLOSE => {
                for killed in self.sessions.close(id) {
                    self.ipc.destroy_session(&mut self.net.stack, killed);
                }
                out(result, 0, &[])
            }
            SESSION_RESIZE => {
                let w = i32_at(args, 4).unwrap_or(80).clamp(0, 1000) as u16;
                let h = i32_at(args, 8).unwrap_or(24).clamp(0, 1000) as u16;
                self.sessions.resize(id, w, h);
                out(result, 0, &[])
            }
            _ => out(result, -1, &[]),
        }
    }

    /// After a host-forced trap unwound an export mid-operation: return to a
    /// clean prompt. Jobs, sockets and the switch are left as they are.
    pub fn recover(&mut self, now: i64) {
        if let Some((mut job, _)) = self.gfx.take() {
            job.abort();
        }
        if self.ui == Ui::Program {
            self.jobs.kill_foreground();
            self.pump_jobs(now);
        }
        if self.ui == Ui::SwitchCli {
            self.switch.leave_cli(&mut self.net, &mut self.con);
        }
        if self.ui == Ui::RouterCli {
            self.router.leave_cli();
        }
        self.editor.clear();
        self.ui = Ui::Prompt;
        self.con.println("");
        self.con.println("^T - Program terminated");
        self.prompt();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framebuffer;

    fn row_text(y: u16) -> String {
        (0..framebuffer::width()).map(|x| framebuffer::read_cell(x, y).0 as char).collect::<String>()
    }

    /// A panel swap resizes the console without a reboot: the text and the
    /// line being typed survive, and the prompt line is redrawn.
    #[test]
    fn console_resizes_and_redraws_the_prompt() {
        let _g = crate::hal::test_lock();
        let mut k = Kernel::boot(0);
        assert_eq!((framebuffer::width(), framebuffer::height()), (160, 50));
        k.on_input(b"echo half-typed", 0);
        assert_eq!(k.resize_console(80, 25), (80, 25));
        assert_eq!((framebuffer::width(), framebuffer::height()), (80, 25));
        assert_eq!((k.con.width(), k.con.height()), (80, 25));
        let (cx, cy) = (k.con.cursor().0, k.con.cursor().1);
        assert!(cy < 25 && cx < 80);
        let line = row_text(cy);
        assert!(line.contains("echo half-typed"), "prompt line not redrawn: {line:?}");
        // Finishing the line still runs it at the new size.
        k.on_input(b"\n", 0);
        let screen: String = (0..25).map(row_text).collect::<Vec<_>>().join("\n");
        // (No echo program in the test host: the shell answers instead.)
        assert!(screen.contains("Unknown command: echo"), "{screen}");
        // Fill past the bottom: scrolling works in the smaller grid.
        for i in 0..40 {
            k.on_input(format!("echo line{i}\n").as_bytes(), 0);
        }
        assert!(k.con.cursor().1 < 25);
        // Back to 160x50 and on to the 160x50 of Tier 3: unchanged size is a no-op.
        assert_eq!(k.resize_console(160, 50), (160, 50));
        assert_eq!(k.resize_console(160, 50), (160, 50));
        assert_eq!(framebuffer::width(), 160);
        // Oversized requests are clamped to the framebuffer region.
        let (w, h) = k.resize_console(1000, 1000);
        assert!(w as usize * h as usize * framebuffer::CELL_SIZE + framebuffer::FB_HEADER_SIZE <= crate::hal::FB_CAP);
        framebuffer::init(framebuffer::DEFAULT_WIDTH, framebuffer::DEFAULT_HEIGHT);
    }

    fn screen() -> String {
        (0..framebuffer::height()).map(|y| row_text(y).trim_end().to_string()).collect::<Vec<_>>().join("\n")
    }

    /// The console's own keys: editing in the middle of a line, history saved
    /// to disk, Tab completion, and Shift+PgUp showing the scrollback.
    #[test]
    fn console_line_editing_history_and_scrollback() {
        let _g = crate::hal::test_lock();
        crate::hal::ffi::FILES.with(|f| f.borrow_mut().clear());
        crate::hal::ffi::FILES.with(|f| f.borrow_mut().insert("bin/hello.wasm".into(), vec![0]));
        framebuffer::init(framebuffer::DEFAULT_WIDTH, framebuffer::DEFAULT_HEIGHT);
        let mut k = Kernel::boot(0);
        k.resize_console(80, 25);
        // Type "nosuchcmd", go left 3 and insert: the line runs as edited.
        k.on_input(b"nosuchcmd\x1b[D\x1b[D\x1b[DXY\n", 0);
        assert!(screen().contains("Unknown command: nosuchXYcmd"), "{}", screen());
        // Tab completes a program name.
        k.on_input(b"hel\t", 0);
        let (_, cy) = k.con.cursor();
        assert!(row_text(cy).contains("/ > hello "), "{:?}", row_text(cy));
        // Ctrl+C abandons the line at the prompt.
        k.on_input(b"\x03", 0);
        assert!(screen().contains("hello ^C"));
        let saved = crate::fs::read_to_string(lineedit::HISTORY_FILE).unwrap_or_default();
        assert!(saved.contains("nosuchXYcmd"), "history saved: {saved:?}");
        // A new boot reads it back: Up recalls it.
        let mut k = Kernel::boot(0);
        k.resize_console(80, 25);
        k.on_input(b"\x1b[A", 0);
        assert_eq!(k.editor.pending(), "nosuchXYcmd");
        k.on_input(b"\x15", 0);
        for i in 0..40 {
            k.on_input(format!("line{i}\n").as_bytes(), 0);
        }
        assert!(!screen().contains("Unknown command: line5\n"));
        // Shift+PgUp: an older page with the "-N" tag; the cursor is hidden.
        k.on_input(b"\x1b[5;2~", 0);
        assert!(k.con.scrolled());
        let view = screen();
        assert!(view.contains("Unknown command: line25") && !view.contains("line39"), "{view}");
        assert!(row_text(0).trim_end().ends_with("-24"), "{:?}", row_text(0));
        // Shift+Up: one more line back.
        k.on_input(b"\x1b[1;2A", 0);
        assert!(row_text(0).trim_end().ends_with("-25"));
        // Any key goes back to the live screen and is typed.
        k.on_input(b"z", 0);
        assert!(!k.con.scrolled());
        let (_, cy) = k.con.cursor();
        assert!(row_text(cy).starts_with("/ > z"), "{:?}", row_text(cy));
        // Shift+Home/End: the very top, then live again.
        k.on_input(b"\x1b[1;2H", 0);
        assert!(screen().contains("TERMINAL OS"));
        k.on_input(b"\x1b[1;2F", 0);
        assert!(!k.con.scrolled());
        framebuffer::init(framebuffer::DEFAULT_WIDTH, framebuffer::DEFAULT_HEIGHT);
    }
}
