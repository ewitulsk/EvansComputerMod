//! The command shell: builtins and program launching.
//!
//! `execute` never blocks. Launching a program returns
//! `Outcome::Foreground`/`Background`; the kernel's job control takes it from
//! there. Kernel-level commands (switch, gfxtest) are returned as outcomes so
//! the kernel can route them to their services.

use crate::console::Term;
use crate::fs;
use crate::hal;
use crate::parse::{self, Pipeline, Redirect};

pub enum Outcome {
    /// Command finished; print a prompt.
    Done,
    /// A foreground job was started with these pids.
    Foreground(Vec<i32>, String),
    /// A background job was started.
    Background(Vec<i32>, String),
    /// `fg [spec]`: bring a background or stopped job forward.
    Resume(String),
    /// `bg [spec]`: a stopped job runs on in the background.
    ResumeBackground(String),
    /// `jobs`
    ListJobs,
    /// `switch` / `switch on` / `switch off`
    Switch(SwitchCmd),
    Router(SwitchCmd),
    /// `gfxtest ...`
    GfxTest(Vec<String>),
    /// `exit`: ends a remote (SSH) session; ignored on the local console.
    Exit,
    /// `history [N|-c]`: the line editor's history.
    History(Vec<String>),
}

pub enum SwitchCmd {
    EnterCli,
    StartDetached,
    Stop,
}

pub struct Shell {
    pub cwd: String,
    /// Running over SSH: no local-only commands (graphics).
    pub remote: bool,
    /// The last command's exit status (`$?`).
    pub status: i32,
    /// Shell variables (`NAME=value`, `$NAME`).
    pub vars: std::collections::BTreeMap<String, String>,
}

pub const BUILTINS: &[&str] = &[
    "cd", "exit", "ps", "kill", "jobs", "fg", "bg", "gfxtest", "switch", "router", "history", "set", "unset",
];

/// `NAME=value` as a whole line (a variable assignment).
fn assignment(line: &str) -> Option<(String, String)> {
    let (name, value) = line.split_once('=')?;
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return None;
    }
    let v = value.trim();
    let unq = if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
        &v[1..v.len() - 1]
    } else if v.contains(' ') {
        return None;
    } else {
        v
    };
    Some((name.to_string(), unq.to_string()))
}

/// Replace `$?`, `$NAME` and `${NAME}` outside single quotes (an unknown
/// variable is empty).
pub fn expand(line: &str, status: i32, vars: &std::collections::BTreeMap<String, String>) -> String {
    let b = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut single = false;
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c == '\'' {
            single = !single;
        }
        if c != '$' || single || i + 1 >= b.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let rest = &line[i + 1..];
        if rest.starts_with('?') {
            out.push_str(&status.to_string());
            i += 2;
        } else if let Some(inner) = rest.strip_prefix('{').and_then(|r| r.split_once('}')) {
            out.push_str(vars.get(inner.0).map(String::as_str).unwrap_or(""));
            i += 3 + inner.0.len();
        } else {
            let n = rest.bytes().take_while(|c| c.is_ascii_alphanumeric() || *c == b'_').count();
            if n == 0 || rest.as_bytes()[0].is_ascii_digit() {
                out.push('$');
                i += 1;
            } else {
                out.push_str(vars.get(&rest[..n]).map(String::as_str).unwrap_or(""));
                i += 1 + n;
            }
        }
    }
    out
}

pub fn is_builtin(cmd: &str) -> bool {
    BUILTINS.contains(&cmd)
}

const O_RDONLY: i32 = 0;
const O_WRONLY: i32 = 1;
const O_CREAT: i32 = 4;
const O_TRUNC: i32 = 8;
const O_APPEND: i32 = 16;

impl Shell {
    pub fn new() -> Self {
        Self {
            cwd: String::new(),
            remote: false,
            status: 0,
            vars: Default::default(),
        }
    }

    pub fn new_remote() -> Self {
        Self {
            cwd: String::new(),
            remote: true,
            status: 0,
            vars: Default::default(),
        }
    }

    pub fn prompt(&self) -> String {
        format!("/{} > ", self.cwd)
    }

    /// Resolve a command name to a program path.
    fn resolve_program(&self, cmd: &str) -> Option<String> {
        if is_builtin(cmd) || cmd.is_empty() {
            return None;
        }
        // Same search order as the original shell: an explicit .wasm path,
        // <cmd>.wasm in the cwd, then user bin/, then the read-only
        // server-bin/ mount.
        let mut candidates = Vec::with_capacity(6);
        if cmd.ends_with(".wasm") {
            candidates.push(fs::resolve(&self.cwd, cmd));
        }
        candidates.push(fs::resolve(&self.cwd, &format!("{}.wasm", cmd)));
        candidates.push(format!("bin/{}.wasm", cmd));
        candidates.push(format!("bin/{}", cmd));
        candidates.push(format!("server-bin/{}.wasm", cmd));
        candidates.push(format!("server-bin/{}", cmd));
        candidates.into_iter().find(|c| fs::exists(c))
    }

    pub fn execute(&mut self, line: &str, con: &mut dyn Term) -> Outcome {
        let line = line.trim();
        if line.is_empty() {
            return Outcome::Done;
        }
        if let Some((name, value)) = assignment(line) {
            let value = expand(&value, self.status, &self.vars);
            self.vars.insert(name, value);
            self.status = 0;
            return Outcome::Done;
        }
        let line = expand(line, self.status, &self.vars);
        let pipeline = parse::parse_pipeline(&line);
        let Some(first) = pipeline.stages.first() else {
            return Outcome::Done;
        };

        if pipeline.stages.len() == 1 && is_builtin(&first.command) {
            let args: Vec<&str> = first.args.iter().map(|s| s.as_str()).collect();
            self.status = 0;
            return self.builtin(&first.command, &args, con);
        }
        let out = self.launch(&pipeline, con);
        if matches!(out, Outcome::Done) {
            // Nothing started: an unknown command or a failed launch.
            self.status = 127;
        }
        out
    }

    fn builtin(&mut self, cmd: &str, args: &[&str], con: &mut dyn Term) -> Outcome {
        match cmd {
            "exit" => return Outcome::Exit,
            "cd" => self.cd(args.first().copied().unwrap_or(""), con),
            "ps" => ps(con),
            "jobs" => return Outcome::ListJobs,
            "bg" => return Outcome::ResumeBackground(args.first().unwrap_or(&"").to_string()),
            "fg" => return Outcome::Resume(args.first().unwrap_or(&"").to_string()),
            "set" => {
                for (k, v) in &self.vars {
                    con.println(&format!("{k}={v}"));
                }
            }
            "unset" => {
                for a in args {
                    self.vars.remove(*a);
                }
            }
            "kill" => match args.first().and_then(|a| a.parse::<i32>().ok()) {
                Some(pid) if hal::proc::kill(pid) => con.println("Process killed."),
                Some(_) => {
                    con.println("Failed to kill process.");
                    self.status = 1;
                }
                None => {
                    con.println("Usage: kill <pid>");
                    self.status = 1;
                }
            },
            "switch" | "router" => {
                let outcome = if cmd == "router" {
                    Outcome::Router
                } else {
                    Outcome::Switch
                };
                return match args.first().copied() {
                    None => outcome(SwitchCmd::EnterCli),
                    Some("on") => outcome(SwitchCmd::StartDetached),
                    Some("off") => outcome(SwitchCmd::Stop),
                    Some(_) => {
                        con.println(&format!("Usage: {} [on|off]", cmd));
                        Outcome::Done
                    }
                };
            }
            "gfxtest" => return Outcome::GfxTest(args.iter().map(|s| s.to_string()).collect()),
            "history" => return Outcome::History(args.iter().map(|s| s.to_string()).collect()),
            _ => {}
        }
        Outcome::Done
    }

    fn cd(&mut self, target: &str, con: &mut dyn Term) {
        if target.is_empty() || target == "/" {
            self.cwd.clear();
            return;
        }
        let resolved = fs::resolve(&self.cwd, target);
        if fs::is_dir(&resolved) {
            self.cwd = resolved;
        } else {
            con.println(&format!("cd: {}: No such directory", target));
            self.status = 1;
        }
    }

    fn open_in(&self, r: &Option<Redirect>) -> i32 {
        match r {
            Some(Redirect::File { path, .. }) => {
                hal::file::fd_open(&fs::resolve(&self.cwd, path), O_RDONLY)
            }
            _ => -1,
        }
    }

    fn open_out(&self, r: &Option<Redirect>) -> i32 {
        match r {
            Some(Redirect::File { path, append }) => {
                let flags = O_WRONLY | O_CREAT | if *append { O_APPEND } else { O_TRUNC };
                hal::file::fd_open(&fs::resolve(&self.cwd, path), flags)
            }
            _ => -1,
        }
    }

    fn launch(&mut self, pipeline: &Pipeline, con: &mut dyn Term) -> Outcome {
        let n = pipeline.stages.len();
        let mut paths = Vec::with_capacity(n);
        for stage in &pipeline.stages {
            if is_builtin(&stage.command) {
                con.println(&format!(
                    "{}: builtins can't be used in a pipeline",
                    stage.command
                ));
                return Outcome::Done;
            }
            match self.resolve_program(&stage.command) {
                Some(p) => paths.push(p),
                None => {
                    con.println(&format!("Unknown command: {}", stage.command));
                    con.println("Type 'help' for a list of commands.");
                    return Outcome::Done;
                }
            }
        }

        let mut pipes = Vec::new();
        for _ in 1..n {
            match hal::file::pipe() {
                Some(p) => pipes.push(p),
                None => {
                    con.println("Pipes are not supported by this host.");
                    for (r, w) in pipes {
                        hal::file::fd_close(r);
                        hal::file::fd_close(w);
                    }
                    return Outcome::Done;
                }
            }
        }

        let mut pids = Vec::with_capacity(n);
        let mut opened = Vec::new();
        for (i, stage) in pipeline.stages.iter().enumerate() {
            let stdin = if stage.stdin_redirect.is_some() {
                let fd = self.open_in(&stage.stdin_redirect);
                opened.push(fd);
                fd
            } else if i > 0 {
                pipes[i - 1].0
            } else {
                -1
            };
            let stdout = if stage.stdout_redirect.is_some() {
                let fd = self.open_out(&stage.stdout_redirect);
                opened.push(fd);
                fd
            } else if i + 1 < n {
                pipes[i].1
            } else {
                -1
            };
            let stderr = match &stage.stderr_redirect {
                Some(Redirect::MergeWith(1)) => stdout,
                r @ Some(Redirect::File { .. }) => {
                    let fd = self.open_out(r);
                    opened.push(fd);
                    fd
                }
                _ => -1,
            };
            let mut argv = paths[i].clone();
            for a in &stage.args {
                argv.push('\n');
                argv.push_str(a);
            }
            match hal::proc::spawn(&paths[i], &argv, stdin, stdout, stderr) {
                Some(pid) => pids.push(pid),
                None => con.println(&format!("Failed to execute: {}", paths[i])),
            }
        }
        // The children hold their own copies of redirect and pipe fds.
        for fd in opened.into_iter().filter(|&fd| fd >= 0) {
            hal::file::fd_close(fd);
        }
        for (r, w) in pipes {
            hal::file::fd_close(r);
            hal::file::fd_close(w);
        }
        if pids.is_empty() {
            return Outcome::Done;
        }
        let desc = parse::pipeline_to_string(pipeline);
        if pipeline.background {
            Outcome::Background(pids, desc)
        } else {
            Outcome::Foreground(pids, desc)
        }
    }
}

fn ps(con: &mut dyn Term) {
    let json = hal::proc::list();
    if json.is_empty() {
        con.println("No processes.");
        return;
    }
    con.println("PID  STATE    NAME");
    for entry in json.split('{').skip(1) {
        let pid = json_int(entry, "pid").unwrap_or(0);
        let name = json_str(entry, "name").unwrap_or("?");
        let state = json_str(entry, "state").unwrap_or("?");
        con.println(&format!("{:>3}  {:<8} {}", pid, state, name));
    }
}

fn json_int(json: &str, key: &str) -> Option<i32> {
    let pat = format!("\"{}\":", key);
    let rest = &json[json.find(&pat)? + pat.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

fn json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{}\":\"", key);
    let rest = &json[json.find(&pat)? + pat.len()..];
    Some(&rest[..rest.find('"')?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Console;
    use crate::hal::ffi;

    #[test]
    fn launches_programs_without_blocking() {
        let _g = crate::hal::test_lock();
        ffi::FILES.with(|f| f.borrow_mut().insert("bin/hello.wasm".into(), vec![0]));
        let mut con = Console::new();
        let mut sh = Shell::new();
        match sh.execute("hello a b", &mut con) {
            Outcome::Foreground(pids, desc) => {
                assert_eq!(pids.len(), 1);
                assert_eq!(desc, "hello a b");
            }
            _ => panic!("expected foreground job"),
        }
        let spawned = ffi::SPAWNED.with(|s| s.borrow().last().cloned()).unwrap();
        assert_eq!(
            spawned,
            (
                "bin/hello.wasm".to_string(),
                "bin/hello.wasm\na\nb".to_string()
            )
        );
        assert!(matches!(
            sh.execute("hello &", &mut con),
            Outcome::Background(..)
        ));
        assert!(matches!(sh.execute("nosuchcmd", &mut con), Outcome::Done));
        assert!(matches!(
            sh.execute("switch on", &mut con),
            Outcome::Switch(SwitchCmd::StartDetached)
        ));
    }

    #[test]
    fn json_helpers_tolerate_garbage() {
        assert_eq!(json_int("\"pid\":42,", "pid"), Some(42));
        assert_eq!(json_int("nothing", "pid"), None);
        assert_eq!(json_str("\"name\":\"ls\"", "name"), Some("ls"));
        assert_eq!(json_str("\"name\":\"unterminated", "name"), None);
    }
}
