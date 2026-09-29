//! Job control without blocking.
//!
//! Running a program spawns it and records a job; `pump` (called on every
//! tick) moves child output into the console and notices exits. Nothing here
//! waits: the old blocking `process_wait` import is gone.

use crate::console::Term;
use crate::hal::{self, proc::Wait};

/// Upper bound on child output copied per tick, so one chatty program can't
/// starve the rest of the kernel.
const OUTPUT_BUDGET: usize = 64 * 1024;

pub struct Job {
    pub id: usize,
    pub pids: Vec<i32>,
    exit_codes: Vec<Option<i32>>,
    pub command: String,
}

impl Job {
    fn done(&self) -> bool {
        self.exit_codes.iter().all(|c| c.is_some())
    }
    /// Exit status of the last stage (the one whose status a shell reports).
    fn status(&self) -> i32 {
        self.exit_codes.last().copied().flatten().unwrap_or(0)
    }
}

/// Something `pump` observed that the kernel must act on.
pub enum JobEvent {
    /// The foreground job finished with the given status.
    ForegroundDone(i32),
    /// A process exited; its socket IPC session must be destroyed.
    Exited(i32),
}

pub struct Jobs {
    fg: Option<Job>,
    bg: Vec<Job>,
    next_id: usize,
    notices: Vec<String>,
}

impl Jobs {
    pub fn new() -> Self {
        Self { fg: None, bg: Vec::new(), next_id: 1, notices: Vec::new() }
    }

    pub fn has_foreground(&self) -> bool {
        self.fg.is_some()
    }

    pub fn any_running(&self) -> bool {
        self.fg.is_some() || !self.bg.is_empty()
    }

    fn make(&mut self, pids: Vec<i32>, command: String) -> Job {
        let id = self.next_id;
        self.next_id += 1;
        let exit_codes = vec![None; pids.len()];
        Job { id, pids, exit_codes, command }
    }

    pub fn start_foreground(&mut self, pids: Vec<i32>, command: String) {
        let job = self.make(pids, command);
        self.fg = Some(job);
    }

    /// Returns the job id.
    pub fn start_background(&mut self, pids: Vec<i32>, command: String) -> usize {
        let job = self.make(pids, command);
        let id = job.id;
        self.bg.push(job);
        id
    }

    /// Keyboard input for the foreground job goes to its first stage.
    pub fn send_input(&mut self, data: &[u8]) {
        if let Some(job) = &self.fg {
            if let Some(&pid) = job.pids.first() {
                hal::proc::write_input(pid, data);
            }
        }
    }

    /// Kill the foreground job (Ctrl+T). Returns true if there was one.
    pub fn kill_foreground(&mut self) -> bool {
        match &self.fg {
            Some(job) => {
                for &pid in &job.pids {
                    hal::proc::kill(pid);
                }
                true
            }
            None => false,
        }
    }

    /// Kill every job (session closed). Returns the pids that were killed.
    pub fn kill_all(&mut self) -> Vec<i32> {
        let mut pids = Vec::new();
        for job in self.fg.take().into_iter().chain(self.bg.drain(..)) {
            for &pid in &job.pids {
                hal::proc::kill(pid);
                pids.push(pid);
            }
        }
        pids
    }

    /// `fg %N` or `fg PID`: bring a background job to the foreground.
    pub fn foreground(&mut self, spec: &str) -> bool {
        let pos = if let Some(n) = spec.strip_prefix('%') {
            n.parse::<usize>().ok().and_then(|id| self.bg.iter().position(|j| j.id == id))
        } else {
            spec.parse::<i32>().ok().and_then(|pid| self.bg.iter().position(|j| j.pids.contains(&pid)))
        };
        match pos {
            Some(i) if self.fg.is_none() => {
                self.fg = Some(self.bg.remove(i));
                true
            }
            _ => false,
        }
    }

    pub fn list(&self, con: &mut dyn Term) {
        if self.bg.is_empty() {
            con.println("No background jobs.");
            return;
        }
        for j in &self.bg {
            con.println(&format!("[{}]  Running    {}", j.id, j.command));
        }
    }

    /// Messages to show before the next prompt ("[1]+ Done ...").
    pub fn take_notices(&mut self) -> Vec<String> {
        core::mem::take(&mut self.notices)
    }

    /// Copy child output to the console and detect exits.
    pub fn pump(&mut self, con: &mut dyn Term) -> Vec<JobEvent> {
        let mut events = Vec::new();
        let mut budget = OUTPUT_BUDGET;
        let mut buf = [0u8; 4096];

        let mut pump_job = |job: &mut Job, con: &mut dyn Term, budget: &mut usize, events: &mut Vec<JobEvent>| {
            for (i, &pid) in job.pids.iter().enumerate() {
                while *budget > 0 {
                    let n = hal::proc::read_output(pid, &mut buf[..(*budget).min(4096)]);
                    if n == 0 {
                        break;
                    }
                    con.write(&buf[..n]);
                    *budget -= n;
                }
                if job.exit_codes[i].is_none() {
                    match hal::proc::try_wait(pid) {
                        Wait::Running => {}
                        Wait::Exited(code) => {
                            // Drain whatever the child wrote right before exiting.
                            loop {
                                let n = hal::proc::read_output(pid, &mut buf);
                                if n == 0 {
                                    break;
                                }
                                con.write(&buf[..n]);
                            }
                            job.exit_codes[i] = Some(code);
                            events.push(JobEvent::Exited(pid));
                        }
                        Wait::Unknown => {
                            job.exit_codes[i] = Some(-1);
                            events.push(JobEvent::Exited(pid));
                        }
                    }
                }
            }
        };

        if let Some(job) = self.fg.as_mut() {
            pump_job(job, con, &mut budget, &mut events);
            if job.done() {
                let status = job.status();
                self.fg = None;
                events.push(JobEvent::ForegroundDone(status));
            }
        }
        for job in self.bg.iter_mut() {
            pump_job(job, con, &mut budget, &mut events);
        }
        let mut i = 0;
        while i < self.bg.len() {
            if self.bg[i].done() {
                let j = self.bg.remove(i);
                self.notices.push(format!("[{}]+ Done    {}", j.id, j.command));
            } else {
                i += 1;
            }
        }
        events
    }
}
