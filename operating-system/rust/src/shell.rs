//! Shell command parsing: pipes, redirects, backgrounding, and ShellInstance.

use crate::terminal;
use crate::ssh;
use crate::net;

/// A redirect specification.
#[derive(Debug, Clone)]
pub enum Redirect {
    /// Redirect to/from a file. `append` controls >> vs >.
    File { path: String, append: bool },
    /// Merge with another FD, e.g., 2>&1.
    MergeWith(i32),
}

/// One stage of a pipeline.
#[derive(Debug, Clone)]
pub struct PipelineStage {
    pub command: String,
    pub args: Vec<String>,
    pub stdin_redirect: Option<Redirect>,
    pub stdout_redirect: Option<Redirect>,
    pub stderr_redirect: Option<Redirect>,
}

/// A parsed command line.
#[derive(Debug)]
pub struct Pipeline {
    pub stages: Vec<PipelineStage>,
    pub background: bool,
}

/// Parse a command line into a Pipeline.
pub fn parse_pipeline(input: &str) -> Pipeline {
    let input = input.trim();

    // Check for trailing &
    let (input, background) = if input.ends_with('&') {
        (input[..input.len() - 1].trim(), true)
    } else {
        (input, false)
    };

    // Split on pipes (but not inside quotes)
    let pipe_segments = split_on_pipes(input);

    let mut stages = Vec::new();
    for segment in pipe_segments {
        stages.push(parse_stage(segment.trim()));
    }

    Pipeline { stages, background }
}

/// Split a command line on unquoted `|` characters.
fn split_on_pipes(input: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'|' if !in_single_quote && !in_double_quote => {
                segments.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    segments.push(&input[start..]);
    segments
}

/// Parse a single pipeline stage (one command with its redirects).
fn parse_stage(input: &str) -> PipelineStage {
    let tokens = tokenize(input);

    let mut command = String::new();
    let mut args = Vec::new();
    let mut stdin_redirect = None;
    let mut stdout_redirect = None;
    let mut stderr_redirect = None;

    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];

        if token == ">" || token == ">>" {
            // stdout redirect
            let append = token == ">>";
            if i + 1 < tokens.len() {
                stdout_redirect = Some(Redirect::File {
                    path: tokens[i + 1].clone(),
                    append,
                });
                i += 2;
                continue;
            }
        } else if token == "<" {
            // stdin redirect
            if i + 1 < tokens.len() {
                stdin_redirect = Some(Redirect::File {
                    path: tokens[i + 1].clone(),
                    append: false,
                });
                i += 2;
                continue;
            }
        } else if token == "2>" {
            // stderr redirect
            if i + 1 < tokens.len() {
                stderr_redirect = Some(Redirect::File {
                    path: tokens[i + 1].clone(),
                    append: false,
                });
                i += 2;
                continue;
            }
        } else if token == "2>&1" {
            stderr_redirect = Some(Redirect::MergeWith(1));
            i += 1;
            continue;
        } else if command.is_empty() {
            command = token.clone();
        } else {
            args.push(token.clone());
        }

        i += 1;
    }

    PipelineStage {
        command,
        args,
        stdin_redirect,
        stdout_redirect,
        stderr_redirect,
    }
}

/// Tokenize a command string, handling quoted strings.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
            }
            '>' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    // Check for "2>" pattern
                    if current == "2" {
                        current.push('>');
                        // Check for "2>&1"
                        if chars.peek() == Some(&'&') {
                            chars.next(); // consume '&'
                            if chars.peek() == Some(&'1') {
                                chars.next(); // consume '1'
                                current.push('&');
                                current.push('1');
                                tokens.push(current.clone());
                                current.clear();
                                continue;
                            }
                        }
                        tokens.push(current.clone());
                        current.clear();
                        continue;
                    }
                    tokens.push(current.clone());
                    current.clear();
                }
                // Check for >>
                if chars.peek() == Some(&'>') {
                    chars.next();
                    tokens.push(">>".to_string());
                } else {
                    tokens.push(">".to_string());
                }
            }
            '<' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(current.clone());
                    current.clear();
                }
                tokens.push("<".to_string());
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

// --- ShellInstance: per-session shell state ---

/// Where shell output goes.
pub enum OutputSink {
    /// Write to the physical terminal via terminal_write() host function.
    Terminal,
    /// Capture output into a buffer (for SSH sessions).
    Buffer(Vec<u8>),
}

/// OS state for a shell instance.
#[derive(Clone, Copy, PartialEq)]
pub enum OsState {
    /// Normal shell mode
    Shell,
    /// Running the editor
    Editor,
    /// Running the Python REPL
    Python,
}

/// A background job entry.
pub struct Job {
    pub id: usize,
    pub pid: i32,
    pub command: String,
    pub done: bool,
}

/// SSH I/O context for TCP-polling read_line.
/// Stores raw pointers to the SSH transport and connection state that live
/// in handle_connection's stack frame. This is safe because:
/// - ShellInstance is created and destroyed within handle_connection
/// - SshTransport lives for the same duration
/// - Single-threaded: no concurrent mutation
pub struct SshIoContext {
    pub conn_idx: usize,
    pub transport_ptr: *mut ssh::transport::SshTransport,
    pub remote_channel_id: u32,
}

/// An independent shell instance with its own state.
/// The local terminal gets one, each SSH session gets one.
pub struct ShellInstance {
    pub state: OsState,
    pub input_buf: [u8; 256],
    pub input_len: usize,
    pub cwd: String,
    pub is_ssh: bool,
    pub output: OutputSink,
    pub job_table: Vec<Option<Job>>,
    pub next_job_id: usize,
    pub pending_input: Vec<u8>,
    pub ssh_io: Option<SshIoContext>,
}

impl ShellInstance {
    pub fn new_terminal() -> Self {
        let mut job_table = Vec::with_capacity(16);
        for _ in 0..16 {
            job_table.push(None);
        }
        Self {
            state: OsState::Shell,
            input_buf: [0u8; 256],
            input_len: 0,
            cwd: String::new(),
            is_ssh: false,
            output: OutputSink::Terminal,
            job_table,
            next_job_id: 1,
            pending_input: Vec::new(),
            ssh_io: None,
        }
    }

    pub fn new_ssh() -> Self {
        let mut job_table = Vec::with_capacity(16);
        for _ in 0..16 {
            job_table.push(None);
        }
        Self {
            state: OsState::Shell,
            input_buf: [0u8; 256],
            input_len: 0,
            cwd: String::new(),
            is_ssh: true,
            output: OutputSink::Buffer(Vec::new()),
            job_table,
            next_job_id: 1,
            pending_input: Vec::new(),
            ssh_io: None,
        }
    }

    pub fn print(&mut self, s: &str) {
        match &mut self.output {
            OutputSink::Terminal => {
                terminal::print(s);
            }
            OutputSink::Buffer(buf) => {
                buf.extend_from_slice(s.as_bytes());
            }
        }
    }

    pub fn println(&mut self, s: &str) {
        self.print(s);
        self.print("\n");
    }

    pub fn clear(&mut self) {
        match &mut self.output {
            OutputSink::Terminal => {
                terminal::clear();
            }
            OutputSink::Buffer(buf) => {
                buf.extend_from_slice(b"\x1b[2J\x1b[H");
            }
        }
    }

    /// Drain the output buffer (for SSH). Returns empty vec for Terminal sink.
    pub fn drain_output(&mut self) -> Vec<u8> {
        match &mut self.output {
            OutputSink::Terminal => Vec::new(),
            OutputSink::Buffer(buf) => {
                let data = buf.clone();
                buf.clear();
                data
            }
        }
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn set_cwd(&mut self, path: &str) {
        self.cwd = path.to_string();
    }

    /// Run a closure with the global fs CWD set to this shell's CWD.
    /// Restores the old global CWD afterwards and saves any changes.
    pub fn with_cwd<F, R>(&mut self, f: F) -> R
        where F: FnOnce(&mut Self) -> R
    {
        let saved = crate::fs::get_cwd().to_string();
        crate::fs::set_cwd(&self.cwd);
        let result = f(self);
        self.cwd = crate::fs::get_cwd().to_string();
        crate::fs::set_cwd(&saved);
        result
    }

    // --- Job management methods ---

    /// Add a background job. Returns the job ID.
    pub fn add_job(&mut self, pid: i32, command: &str) -> usize {
        let id = self.next_job_id;
        self.next_job_id += 1;
        for slot in self.job_table.iter_mut() {
            if slot.is_none() {
                *slot = Some(Job {
                    id,
                    pid,
                    command: command.to_string(),
                    done: false,
                });
                return id;
            }
        }
        id // Table full, return id anyway
    }

    /// Check for completed background jobs and print notifications.
    pub fn check_completed_jobs(&mut self) {
        // Collect notifications first to avoid borrow conflict
        let mut notifications: Vec<String> = Vec::new();
        unsafe {
            for slot in self.job_table.iter_mut() {
                if let Some(job) = slot {
                    if !job.done {
                        let state = process_state(job.pid);
                        if state == 2 || state == -1 {
                            // zombie or not found => done
                            job.done = true;
                            notifications.push(format!("[{}]+ Done    {}", job.id, job.command));
                        }
                    }
                }
            }
            // Clean up done jobs
            for slot in self.job_table.iter_mut() {
                if let Some(job) = slot {
                    if job.done {
                        *slot = None;
                    }
                }
            }
        }
        for msg in &notifications {
            self.println(msg);
        }
    }

    /// List active jobs.
    pub fn list_jobs(&mut self) {
        // Collect output first to avoid borrow conflict
        let mut lines: Vec<String> = Vec::new();
        unsafe {
            for slot in self.job_table.iter() {
                if let Some(job) = slot {
                    if !job.done {
                        let state = process_state(job.pid);
                        let state_str = match state {
                            0 => "Running",
                            1 => "Stopped",
                            2 => "Done",
                            _ => "Unknown",
                        };
                        lines.push(format!("[{}]+ {} {}", job.id, state_str, job.command));
                    }
                }
            }
        }
        for msg in &lines {
            self.println(msg);
        }
    }

    // --- PTY-like input methods ---

    /// Read a line of input. For terminal mode, calls host function.
    /// For SSH mode, polls TCP for CHANNEL_DATA until a newline is received.
    pub fn read_line(&mut self, prompt: &str) -> String {
        self.print(prompt);
        if !self.is_ssh {
            // Terminal mode: use the host's blocking read_line
            terminal::read_line_raw("")
        } else {
            // SSH mode: poll TCP for input
            self.read_line_ssh()
        }
    }

    /// SSH read_line: polls TCP for CHANNEL_DATA packets until a complete line
    /// (terminated by \r or \n) is available in pending_input.
    fn read_line_ssh(&mut self) -> String {
        let io = match self.ssh_io.as_ref() {
            Some(io) => io,
            None => return String::new(),
        };
        let conn_idx = io.conn_idx;
        let remote_channel_id = io.remote_channel_id;
        let transport_ptr = io.transport_ptr;

        // Flush any buffered output (e.g., the prompt) BEFORE blocking on input.
        // Without this, prompts like "New password: " don't appear until the
        // user types something, because the output sits in the buffer.
        self.flush_ssh_output(conn_idx, transport_ptr, remote_channel_id);

        loop {
            // Check for a complete line in pending_input
            if let Some(pos) = self.pending_input.iter().position(|&b| b == b'\n' || b == b'\r') {
                let line_bytes: Vec<u8> = self.pending_input.drain(..pos).collect();
                // Remove the newline character
                if !self.pending_input.is_empty() {
                    let removed = self.pending_input.remove(0);
                    // Also remove \n after \r (CRLF)
                    if removed == b'\r' && !self.pending_input.is_empty() && self.pending_input[0] == b'\n' {
                        self.pending_input.remove(0);
                    }
                }
                self.print("\r\n");

                // Flush any buffered output to SSH client
                self.flush_ssh_output(conn_idx, transport_ptr, remote_channel_id);

                return String::from_utf8_lossy(&line_bytes).to_string();
            }

            // No complete line — poll TCP for more data
            let stack = match net::NetStack::get() {
                Some(s) => s,
                None => return String::new(),
            };
            stack.poll_rx();
            stack.poll_timers();

            let mut buf = [0u8; 4096];
            let transport = unsafe { &mut *transport_ptr };
            match stack.tcp_recv(conn_idx, &mut buf, 500) {
                Ok(n) if n > 0 => {
                    let payloads = transport.feed(&buf[..n]);
                    for payload in payloads {
                        if payload.is_empty() { continue; }
                        match payload[0] {
                            ssh::packet::msg::CHANNEL_DATA => {
                                if let Some((_, data)) = ssh::channel::parse_channel_data(&payload) {
                                    for &byte in data {
                                        match byte {
                                            8 | 127 => {
                                                // Backspace
                                                if !self.pending_input.is_empty() {
                                                    self.pending_input.pop();
                                                    self.print("\x08 \x08");
                                                }
                                            }
                                            b'\r' | b'\n' => {
                                                self.pending_input.push(b'\n');
                                            }
                                            b if b >= 32 && b < 127 => {
                                                self.pending_input.push(b);
                                                // Echo the character
                                                let ch = [b];
                                                let s = unsafe { core::str::from_utf8_unchecked(&ch) };
                                                self.print(s);
                                            }
                                            _ => {} // Ignore other control chars
                                        }
                                    }
                                }
                            }
                            ssh::packet::msg::CHANNEL_CLOSE | ssh::packet::msg::DISCONNECT => {
                                return String::new();
                            }
                            _ => {}
                        }
                    }

                    // Flush echo output
                    self.flush_ssh_output(conn_idx, transport_ptr, remote_channel_id);
                }
                _ => {
                    // Timeout or error — try decoding from buffer
                    let payloads = transport.feed(&[]);
                    for payload in payloads {
                        if payload.is_empty() { continue; }
                        if payload[0] == ssh::packet::msg::CHANNEL_DATA {
                            if let Some((_, data)) = ssh::channel::parse_channel_data(&payload) {
                                self.pending_input.extend_from_slice(data);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Flush buffered output to SSH client via TCP.
    fn flush_ssh_output(&mut self, conn_idx: usize, transport_ptr: *mut ssh::transport::SshTransport, remote_channel_id: u32) {
        let output = self.drain_output();
        if output.is_empty() { return; }

        let stack = match net::NetStack::get() {
            Some(s) => s,
            None => return,
        };
        let transport = unsafe { &mut *transport_ptr };
        let converted = convert_lf_to_crlf_bytes(&output);
        let data_pkt = ssh::channel::build_channel_data(remote_channel_id, &converted);
        let pkt = transport.encode_packet(&data_pkt);
        let _ = stack.tcp_send(conn_idx, &pkt);
    }

    /// Feed raw input bytes (from SSH CHANNEL_DATA).
    pub fn feed_input(&mut self, data: &[u8]) {
        self.pending_input.extend_from_slice(data);
    }

    /// Look up the PID for a job by its job ID.
    pub fn get_job_pid(&self, job_id: usize) -> i32 {
        for slot in self.job_table.iter() {
            if let Some(job) = slot {
                if job.id == job_id && !job.done {
                    return job.pid;
                }
            }
        }
        -1
    }
}

/// Convert a pipeline back to a display string.
fn pipeline_to_string(pipeline: &Pipeline) -> String {
    let mut s = String::new();
    for (i, stage) in pipeline.stages.iter().enumerate() {
        if i > 0 {
            s.push_str(" | ");
        }
        s.push_str(&stage.command);
        for arg in &stage.args {
            s.push(' ');
            s.push_str(arg);
        }
    }
    if pipeline.background {
        s.push_str(" &");
    }
    s
}

/// Convert \n to \r\n in a byte slice (for SSH terminal output).
fn convert_lf_to_crlf_bytes(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len() + data.len() / 10);
    for &byte in data {
        if byte == b'\n' {
            result.push(b'\r');
        }
        result.push(byte);
    }
    result
}

// --- Pipeline execution engine ---

use crate::fd;
use crate::fs;

extern "C" {
    fn process_spawn(
        path_ptr: *const u8,
        path_len: usize,
        argv_ptr: *const u8,
        argv_len: usize,
        stdin_fd: i32,
        stdout_fd: i32,
        stderr_fd: i32,
    ) -> i32;
    fn process_wait(pid: i32) -> i32;
    fn process_state(pid: i32) -> i32;
}

const O_RDONLY: i32 = 0;
const O_WRONLY: i32 = 1;
const O_CREAT: i32 = 4;
const O_TRUNC: i32 = 8;
const O_APPEND: i32 = 16;

/// Check if a command is a built-in (not a .wasm program).
pub fn is_builtin(cmd: &str) -> bool {
    matches!(
        cmd,
        "echo"
            | "ls"
            | "cat"
            | "cd"
            | "pwd"
            | "mkdir"
            | "rm"
            | "cp"
            | "mv"
            | "touch"
            | "edit"
            | "help"
            | "clear"
            | "python"
            | "visual"
            | "peripherals"
            | "git"
            | "ifconfig"
            | "ip"
            | "ping"
            | "nslookup"
            | "resolvectl"
            | "httpd"
            | "curl"
            | "crypto_test"
            | "ssh-keygen"
            | "fd_test"
            | "ps"
            | "kill"
            | "jobs"
            | "fg"
            | "bg"
            | "passwd"
            | "sshd"
            | "ssh"
            | "sleep"
            | "tty_test"
    )
}

/// Resolve a command to a .wasm path. Returns None for builtins.
pub fn resolve_wasm_path(cmd: &str) -> Option<String> {
    if is_builtin(cmd) {
        return None;
    }

    if cmd.ends_with(".wasm") {
        if fs::exists(cmd) {
            return Some(cmd.to_string());
        }
    }

    // Try cmd.wasm
    let with_ext = format!("{}.wasm", cmd);
    if fs::exists(&with_ext) {
        return Some(with_ext);
    }

    // Try bin/cmd.wasm
    let in_bin = format!("bin/{}.wasm", cmd);
    if fs::exists(&in_bin) {
        return Some(in_bin);
    }

    // Try bin/cmd
    let in_bin_no_ext = format!("bin/{}", cmd);
    if fs::exists(&in_bin_no_ext) {
        return Some(in_bin_no_ext);
    }

    None
}

/// Execute a parsed pipeline.
/// Returns true if the pipeline was handled (even if it failed).
/// Returns false if the first command is a builtin (caller should handle).
pub fn execute_pipeline(shell: &mut ShellInstance, pipeline: &Pipeline) -> bool {
    // Single-stage builtins are handled by the caller
    if pipeline.stages.len() == 1 && is_builtin(&pipeline.stages[0].command) {
        return false;
    }

    let num_stages = pipeline.stages.len();

    if num_stages == 1 {
        // Single .wasm command with possible redirects
        let stage = &pipeline.stages[0];
        let wasm_path = match resolve_wasm_path(&stage.command) {
            Some(p) => p,
            None => return false,
        };

        let stdin_fd = open_redirect_in(&stage.stdin_redirect);
        let stdout_fd = open_redirect_out(&stage.stdout_redirect);
        let stderr_fd = open_redirect_out(&stage.stderr_redirect);

        let argv = build_argv(&wasm_path, &stage.args);

        unsafe {
            let pid = process_spawn(
                wasm_path.as_ptr(),
                wasm_path.len(),
                argv.as_ptr(),
                argv.len(),
                stdin_fd,
                stdout_fd,
                stderr_fd,
            );

            // Close redirect FDs that belong to us
            if stdin_fd >= 0 {
                fd::fd_close(stdin_fd);
            }
            if stdout_fd >= 0 {
                fd::fd_close(stdout_fd);
            }
            if stderr_fd >= 0 {
                fd::fd_close(stderr_fd);
            }

            if pid > 0 {
                if !pipeline.background {
                    let exit_code = process_wait(pid);
                    if exit_code != 0 {
                        shell.print("Process exited with code ");
                        shell.println(&exit_code.to_string());
                    }
                } else {
                    let job_id = shell.add_job(pid, &pipeline_to_string(pipeline));
                    let msg = format!("[{}] {}", job_id, pid);
                    shell.println(&msg);
                }
            } else {
                shell.print("Failed to execute: ");
                shell.println(&wasm_path);
            }
        }

        return true;
    }

    // Multi-stage pipeline: create pipes between stages
    let mut pipe_read_fds = Vec::new();
    let mut pipe_write_fds = Vec::new();

    for _ in 0..num_stages - 1 {
        let mut read_fd: i32 = 0;
        let mut write_fd: i32 = 0;
        unsafe {
            if fd::pipe_create(&mut read_fd, &mut write_fd) != 0 {
                shell.println("Failed to create pipe");
                return true;
            }
        }
        pipe_read_fds.push(read_fd);
        pipe_write_fds.push(write_fd);
    }

    let mut pids = Vec::new();

    for (i, stage) in pipeline.stages.iter().enumerate() {
        let wasm_path = match resolve_wasm_path(&stage.command) {
            Some(p) => p,
            None => {
                shell.print(&stage.command);
                shell.println(": not a WASM program (builtins can't be piped yet)");
                // Clean up pipes
                unsafe {
                    for &f in &pipe_read_fds {
                        fd::fd_close(f);
                    }
                    for &f in &pipe_write_fds {
                        fd::fd_close(f);
                    }
                }
                return true;
            }
        };

        // Determine stdin for this stage
        let stdin_fd = if stage.stdin_redirect.is_some() {
            open_redirect_in(&stage.stdin_redirect)
        } else if i == 0 {
            -1 // inherit terminal stdin
        } else {
            pipe_read_fds[i - 1] // read from previous pipe
        };

        // Determine stdout for this stage
        let stdout_fd = if stage.stdout_redirect.is_some() {
            open_redirect_out(&stage.stdout_redirect)
        } else if i == num_stages - 1 {
            -1 // inherit terminal stdout
        } else {
            pipe_write_fds[i] // write to next pipe
        };

        let stderr_fd = open_redirect_out(&stage.stderr_redirect);

        let argv = build_argv(&wasm_path, &stage.args);

        unsafe {
            let pid = process_spawn(
                wasm_path.as_ptr(),
                wasm_path.len(),
                argv.as_ptr(),
                argv.len(),
                stdin_fd,
                stdout_fd,
                stderr_fd,
            );
            if pid > 0 {
                pids.push(pid);
            }
        }
    }

    // Close all pipe FDs in the shell (child processes have their own copies)
    unsafe {
        for &f in &pipe_read_fds {
            fd::fd_close(f);
        }
        for &f in &pipe_write_fds {
            fd::fd_close(f);
        }
    }

    // Wait for all processes (or report background jobs)
    if !pipeline.background {
        for &pid in &pids {
            unsafe {
                process_wait(pid);
            }
        }
    } else if let Some(&last_pid) = pids.last() {
        let job_id = shell.add_job(last_pid, &pipeline_to_string(pipeline));
        let msg = format!("[{}] {}", job_id, last_pid);
        shell.println(&msg);
    }

    true
}

/// Open an input redirect, returning an FD or -1 for terminal.
fn open_redirect_in(redirect: &Option<Redirect>) -> i32 {
    match redirect {
        Some(Redirect::File { path, .. }) => unsafe {
            fd::fd_open(path.as_ptr(), path.len(), O_RDONLY)
        },
        _ => -1,
    }
}

/// Open an output redirect, returning an FD or -1 for terminal.
fn open_redirect_out(redirect: &Option<Redirect>) -> i32 {
    match redirect {
        Some(Redirect::File { path, append }) => unsafe {
            let flags = if *append {
                O_WRONLY | O_CREAT | O_APPEND
            } else {
                O_WRONLY | O_CREAT | O_TRUNC
            };
            fd::fd_open(path.as_ptr(), path.len(), flags)
        },
        _ => -1,
    }
}

/// Build a newline-delimited argv string: "program\narg1\narg2"
fn build_argv(wasm_path: &str, args: &[String]) -> String {
    let mut argv = wasm_path.to_string();
    for arg in args {
        argv.push('\n');
        argv.push_str(arg);
    }
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let p = parse_pipeline("echo hello world");
        assert_eq!(p.stages.len(), 1);
        assert_eq!(p.stages[0].command, "echo");
        assert_eq!(p.stages[0].args, vec!["hello", "world"]);
        assert!(!p.background);
    }

    #[test]
    fn test_pipe() {
        let p = parse_pipeline("cat file.txt | grep hello");
        assert_eq!(p.stages.len(), 2);
        assert_eq!(p.stages[0].command, "cat");
        assert_eq!(p.stages[1].command, "grep");
        assert_eq!(p.stages[1].args, vec!["hello"]);
    }

    #[test]
    fn test_redirect_out() {
        let p = parse_pipeline("echo test > out.txt");
        assert_eq!(p.stages.len(), 1);
        assert_eq!(p.stages[0].command, "echo");
        assert!(matches!(
            &p.stages[0].stdout_redirect,
            Some(Redirect::File {
                path,
                append: false
            }) if path == "out.txt"
        ));
    }

    #[test]
    fn test_redirect_append() {
        let p = parse_pipeline("echo test >> out.txt");
        assert!(matches!(
            &p.stages[0].stdout_redirect,
            Some(Redirect::File { path, append: true }) if path == "out.txt"
        ));
    }

    #[test]
    fn test_redirect_in() {
        let p = parse_pipeline("cat < input.txt");
        assert!(matches!(
            &p.stages[0].stdin_redirect,
            Some(Redirect::File { path, .. }) if path == "input.txt"
        ));
    }

    #[test]
    fn test_background() {
        let p = parse_pipeline("httpd 8080 &");
        assert!(p.background);
        assert_eq!(p.stages[0].command, "httpd");
        assert_eq!(p.stages[0].args, vec!["8080"]);
    }

    #[test]
    fn test_multi_pipe() {
        let p = parse_pipeline("cat file | grep test | wc -l");
        assert_eq!(p.stages.len(), 3);
        assert_eq!(p.stages[0].command, "cat");
        assert_eq!(p.stages[1].command, "grep");
        assert_eq!(p.stages[2].command, "wc");
    }

    #[test]
    fn test_stderr_redirect() {
        let p = parse_pipeline("cmd 2>&1");
        assert!(matches!(
            &p.stages[0].stderr_redirect,
            Some(Redirect::MergeWith(1))
        ));
    }

    #[test]
    fn test_quoted_args() {
        let p = parse_pipeline("echo \"hello world\" foo");
        assert_eq!(p.stages[0].args, vec!["hello world", "foo"]);
    }
}
