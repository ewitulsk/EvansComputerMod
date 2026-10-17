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
    /// Paused with Ctrl+Z (waits for `fg` or `bg`).
    pub stopped: bool,
    /// The stage that gets the keyboard: the first, or a pager at the end
    /// (`ls | less`, which reads its keys from /dev/tty).
    input: usize,
}

/// Programs that read the keyboard (from /dev/tty) at the end of a pipeline.
const PAGERS: &[&str] = &["less", "more"];

fn input_stage(pids: usize, command: &str) -> usize {
    let last = command.rsplit('|').next().unwrap_or("").split_whitespace().next().unwrap_or("");
    if pids > 1 && PAGERS.contains(&last) {
        pids - 1
    } else {
        0
    }
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
        let input = input_stage(pids.len(), &command);
        Job { id, pids, exit_codes, command, stopped: false, input }
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
            if let Some(&pid) = job.pids.get(job.input) {
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

    /// Ctrl+Z: pause the foreground job and put it with the background
    /// jobs, stopped. Returns its id and command.
    pub fn suspend_foreground(&mut self) -> Option<(usize, String)> {
        let mut job = self.fg.take()?;
        for &pid in &job.pids {
            hal::proc::stop(pid);
        }
        job.stopped = true;
        let r = (job.id, job.command.clone());
        self.bg.push(job);
        Some(r)
    }

    /// The background job `spec` names (`%N` or a pid; empty: the newest).
    fn find(&self, spec: &str) -> Option<usize> {
        if spec.is_empty() {
            return self.bg.len().checked_sub(1);
        }
        if let Some(n) = spec.strip_prefix('%') {
            n.parse::<usize>().ok().and_then(|id| self.bg.iter().position(|j| j.id == id))
        } else {
            spec.parse::<i32>().ok().and_then(|pid| self.bg.iter().position(|j| j.pids.contains(&pid)))
        }
    }

    /// `fg [%N|PID]`: bring a background (or stopped) job to the foreground.
    /// Returns its command.
    pub fn foreground(&mut self, spec: &str) -> Option<String> {
        match self.find(spec) {
            Some(i) if self.fg.is_none() => {
                let mut job = self.bg.remove(i);
                if job.stopped {
                    for &pid in &job.pids {
                        hal::proc::cont(pid);
                    }
                    job.stopped = false;
                }
                let cmd = job.command.clone();
                self.fg = Some(job);
                Some(cmd)
            }
            _ => None,
        }
    }

    /// `bg [%N|PID]`: let a stopped job go on running in the background.
    /// Returns `[id] command`.
    pub fn resume_background(&mut self, spec: &str) -> Result<String, &'static str> {
        let i = self.find(spec).ok_or("bg: no such job")?;
        let job = &mut self.bg[i];
        if !job.stopped {
            return Err("bg: job already in background");
        }
        for &pid in &job.pids {
            hal::proc::cont(pid);
        }
        job.stopped = false;
        Ok(format!("[{}]+ {} &", job.id, job.command))
    }

    pub fn list(&self, con: &mut dyn Term) {
        if self.bg.is_empty() {
            con.println("No background jobs.");
            return;
        }
        for j in &self.bg {
            con.println(&format!("[{}]  {:<10} {}", j.id, if j.stopped { "Stopped" } else { "Running" }, j.command));
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
