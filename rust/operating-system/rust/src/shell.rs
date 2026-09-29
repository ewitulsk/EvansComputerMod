//! The command shell: builtins and program launching.
//!
//! `execute` never blocks. Launching a program returns
//! `Outcome::Foreground`/`Background`; the kernel's job control takes it from
//! there. Kernel-level commands (switch, gfxtest) are returned as outcomes so
//! the kernel can route them to their services.

use crate::console::Console;
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
    /// `fg`: bring a background job forward.
    Resume(String),
    /// `jobs`
    ListJobs,
    /// `switch` / `switch on` / `switch off`
    Switch(SwitchCmd),
    /// `gfxtest ...`
    GfxTest(Vec<String>),
}

pub enum SwitchCmd {
    EnterCli,
    StartDetached,
    Stop,
}

pub struct Shell {
    pub cwd: String,
}

const BUILTINS: &[&str] = &["cd", "exit", "ps", "kill", "jobs", "fg", "bg", "visual", "gfxtest", "switch"];

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
        Self { cwd: String::new() }
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

    pub fn execute(&mut self, line: &str, con: &mut Console) -> Outcome {
        let line = line.trim();
        if line.is_empty() {
            return Outcome::Done;
        }
        let pipeline = parse::parse_pipeline(line);
        let Some(first) = pipeline.stages.first() else { return Outcome::Done };

        if pipeline.stages.len() == 1 && is_builtin(&first.command) {
            let args: Vec<&str> = first.args.iter().map(|s| s.as_str()).collect();
            return self.builtin(&first.command, &args, con);
        }
        self.launch(&pipeline, con)
    }

    fn builtin(&mut self, cmd: &str, args: &[&str], con: &mut Console) -> Outcome {
        match cmd {
            "exit" => con.println("Use Ctrl+T to stop a program; the shell always stays running."),
            "cd" => self.cd(args.first().copied().unwrap_or(""), con),
            "visual" => {
                con.println("Opening visual editor...");
                hal::open_visual_editor();
            }
            "ps" => ps(con),
            "jobs" => return Outcome::ListJobs,
            "bg" => con.println("Background jobs already run independently."),
            "fg" => match args.first() {
                Some(spec) => return Outcome::Resume(spec.to_string()),
                None => con.println("Usage: fg %<job> | fg <pid>"),
            },
            "kill" => match args.first().and_then(|a| a.parse::<i32>().ok()) {
                Some(pid) if hal::proc::kill(pid) => con.println("Process killed."),
                Some(_) => con.println("Failed to kill process."),
                None => con.println("Usage: kill <pid>"),
            },
            "switch" => {
                return match args.first().copied() {
                    None => Outcome::Switch(SwitchCmd::EnterCli),
                    Some("on") => Outcome::Switch(SwitchCmd::StartDetached),
                    Some("off") => Outcome::Switch(SwitchCmd::Stop),
                    Some(_) => {
                        con.println("Usage: switch [on|off]");
                        Outcome::Done
                    }
                }
            }
            "gfxtest" => return Outcome::GfxTest(args.iter().map(|s| s.to_string()).collect()),
            _ => {}
        }
        Outcome::Done
    }

    fn cd(&mut self, target: &str, con: &mut Console) {
        if target.is_empty() || target == "/" {
            self.cwd.clear();
            return;
        }
        let resolved = fs::resolve(&self.cwd, target);
        if fs::is_dir(&resolved) {
            self.cwd = resolved;
        } else {
            con.println(&format!("cd: {}: No such directory", target));
        }
    }

    fn open_in(&self, r: &Option<Redirect>) -> i32 {
        match r {
            Some(Redirect::File { path, .. }) => hal::file::fd_open(&fs::resolve(&self.cwd, path), O_RDONLY),
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

    fn launch(&mut self, pipeline: &Pipeline, con: &mut Console) -> Outcome {
        let n = pipeline.stages.len();
        let mut paths = Vec::with_capacity(n);
        for stage in &pipeline.stages {
            if is_builtin(&stage.command) {
                con.println(&format!("{}: builtins can't be used in a pipeline", stage.command));
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

fn ps(con: &mut Console) {
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
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(rest.len());
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
        assert_eq!(spawned, ("bin/hello.wasm".to_string(), "bin/hello.wasm\na\nb".to_string()));
        assert!(matches!(sh.execute("hello &", &mut con), Outcome::Background(..)));
        assert!(matches!(sh.execute("nosuchcmd", &mut con), Outcome::Done));
        assert!(matches!(sh.execute("switch on", &mut con), Outcome::Switch(SwitchCmd::StartDetached)));
    }

    #[test]
    fn json_helpers_tolerate_garbage() {
        assert_eq!(json_int("\"pid\":42,", "pid"), Some(42));
        assert_eq!(json_int("nothing", "pid"), None);
        assert_eq!(json_str("\"name\":\"ls\"", "name"), Some("ls"));
        assert_eq!(json_str("\"name\":\"unterminated", "name"), None);
    }
}
