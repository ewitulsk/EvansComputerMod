//! The kernel: all state, and the handlers behind each export.
//!
//! Everything here runs to completion quickly and returns. Waiting happens
//! in the host between calls: `on_tick` returns the next time the kernel
//! wants to run, and host events (input, frames, child output, socket
//! requests) call in earlier.

use crate::console::Console;
use crate::gfx::Plane;
use crate::gfx_test::GfxTest;
use crate::jobs::{JobEvent, Jobs};
use crate::lineedit::{Key, LineEditor};
use crate::net::ipc::{IpcEffects, SocketIpc, IPC_PENDING};
use crate::net::{self, min_opt, Net};
use crate::shell::{Outcome, Shell, SwitchCmd};
use crate::switch_svc::{CliState, SwitchService};

pub const IRQ_KEYBOARD: i32 = 1;
pub const IRQ_REDSTONE: i32 = 2;
pub const IRQ_NETWORK: i32 = 3;
pub const IRQ_MOUSE: i32 = 4;
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
    /// `gfxtest` is running.
    Gfx,
}

pub struct Kernel {
    con: Console,
    editor: LineEditor,
    shell: Shell,
    jobs: Jobs,
    net: Net,
    ipc: SocketIpc,
    switch: SwitchService,
    gfx: Option<(GfxTest, i64)>,
    ui: Ui,
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
        con.println("================================================================================");
        con.println("                         TERMINAL OS v2.0                                       ");
        con.println("================================================================================");
        con.println("");
        con.println("Welcome to Terminal OS!");
        con.println("Type 'help' for a list of available commands.");
        con.println("");
        let mut k = Kernel {
            con,
            editor: LineEditor::new(),
            shell: Shell::new(),
            jobs: Jobs::new(),
            net,
            ipc: SocketIpc::new(),
            switch: SwitchService::new(),
            gfx: None,
            ui: Ui::Prompt,
        };
        k.prompt();
        k
    }

    fn prompt(&mut self) {
        for n in self.jobs.take_notices() {
            self.con.println(&n);
        }
        let p = match self.ui {
            Ui::SwitchCli => self.switch.prompt(),
            _ => self.shell.prompt(),
        };
        self.con.print(&p);
    }

    // ------------------------------------------------------------ input

    pub fn on_input(&mut self, bytes: &[u8], now: i64) {
        let mut i = 0;
        while i < bytes.len() {
            match self.ui {
                Ui::Program => {
                    // Everything up to a Ctrl+T goes to the program's stdin.
                    let end = bytes[i..].iter().position(|&b| b == 0x14).map(|p| i + p).unwrap_or(bytes.len());
                    if end > i {
                        self.jobs.send_input(&bytes[i..end]);
                    }
                    if end < bytes.len() {
                        self.terminate(now);
                        i = end + 1;
                    } else {
                        i = end;
                    }
                }
                Ui::Gfx => {
                    if bytes[i] == 0x14 {
                        self.terminate(now);
                    }
                    i += 1;
                }
                Ui::Prompt | Ui::SwitchCli => {
                    let b = bytes[i];
                    i += 1;
                    match self.editor.feed(b, &mut self.con) {
                        Some(Key::Line(line)) => self.run_line(&line, now),
                        Some(Key::Terminate) => self.terminate(now),
                        None => {}
                    }
                }
            }
        }
    }

    fn run_line(&mut self, line: &str, now: i64) {
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
                if self.jobs.foreground(&spec) {
                    self.ui = Ui::Program;
                    return;
                }
                self.con.println("fg: no such job");
            }
            Outcome::ListJobs => self.jobs.list(&mut self.con),
            Outcome::Switch(cmd) => match cmd {
                SwitchCmd::EnterCli => {
                    if self.switch.enter_cli(&mut self.net, &mut self.con, now) {
                        self.ui = Ui::SwitchCli;
                    }
                }
                SwitchCmd::StartDetached => self.switch.start_detached(&mut self.net, &mut self.con, now),
                SwitchCmd::Stop => self.switch.stop_cmd(&mut self.net, &mut self.con),
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
            IRQ_TERMINATE => self.terminate(now),
            // Keyboard/redstone/mouse IRQs are for WASI programs, which read
            // them through their own host functions.
            _ => {}
        }
    }

    fn pump_jobs(&mut self, _now: i64) {
        for ev in self.jobs.pump(&mut self.con) {
            match ev {
                JobEvent::Exited(pid) => self.ipc.destroy_session(&mut self.net.stack, pid),
                JobEvent::ForegroundDone(status) => {
                    if self.ui == Ui::Program {
                        if status != 0 {
                            self.con.println(&format!("Process exited with code {}", status));
                        }
                        self.con.println("");
                        self.ui = Ui::Prompt;
                        self.prompt();
                    }
                }
            }
        }
    }

    /// Show switch log events (`logging console`) without losing what the
    /// user is typing: print them on their own lines, then redraw the prompt.
    fn show_console_log(&mut self) {
        let lines = self.net.take_console_log();
        if lines.is_empty() {
            return;
        }
        let at_prompt = matches!(self.ui, Ui::Prompt | Ui::SwitchCli);
        self.con.print("\r\x1b[K");
        for l in &lines {
            self.con.println(l);
        }
        if at_prompt {
            self.prompt();
            let pending = self.editor.pending().to_string();
            self.con.print(&pending);
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
        if self.jobs.any_running() {
            deadline = min_opt(deadline, Some(now + JOB_POLL_MS));
        }
        deadline = min_opt(deadline, self.ipc.next_deadline());
        deadline.map(|d| d.max(now)).unwrap_or(-1)
    }

    pub fn sock_ipc(&mut self, pid: i32, syscall: i32, args: &[u8], result: &mut [u8], now: i64) -> i32 {
        let mut fx = IpcEffects::default();
        let r = self.ipc.dispatch(&mut self.net.stack, pid, syscall, args, result, now, &mut fx);
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
        if r == IPC_PENDING {
            IPC_PENDING
        } else {
            r
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
        self.editor.clear();
        self.ui = Ui::Prompt;
        self.con.println("");
        self.con.println("^T - Program terminated");
        self.prompt();
    }
}
