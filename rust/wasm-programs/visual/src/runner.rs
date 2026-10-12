//! Running programs: Run starts `python <file>` as an ordinary process and
//! shows its output. On a computer the process runs in a private command
//! session (`ipc_spawn_command`: the kernel's shell runs one command line
//! with its own terminal and ends when it exits), so nothing is typed into
//! the player's terminal. Tests use [`MockRunner`].

/// What a [`Runner`] produced since the last poll.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Poll {
    /// Output text (terminal control sequences removed).
    pub output: String,
    /// The command finished.
    pub finished: bool,
}

pub trait Runner {
    /// Start a command line. Fails if one is already running.
    fn start(&mut self, command: &str) -> Result<(), String>;
    fn running(&self) -> bool;
    /// New output, and whether the command ended.
    fn poll(&mut self) -> Poll;
    /// Send a line to the program's input.
    fn write(&mut self, text: &str);
    /// Kill the program.
    fn stop(&mut self);
}

/// Strip what a text panel can't show: carriage returns, ANSI escape
/// sequences (CSI and OSC) and other control characters except newlines and tabs.
pub fn clean_output(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut it = raw.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\u{1b}' => match it.peek() {
                Some('[') => {
                    it.next();
                    for d in it.by_ref() {
                        if ('@'..='~').contains(&d) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    it.next();
                    while let Some(d) = it.next() {
                        if d == '\u{7}' {
                            break;
                        }
                        if d == '\u{1b}' && it.peek() == Some(&'\\') {
                            it.next();
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            },
            '\n' | '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// A scripted runner for tests: records commands and input, plays back output.
#[derive(Default, Debug)]
pub struct MockRunner {
    pub commands: Vec<String>,
    pub input: Vec<String>,
    /// Output returned by the next polls, one chunk each; the run ends after the last.
    pub script: Vec<String>,
    pub active: bool,
}

impl Runner for MockRunner {
    fn start(&mut self, command: &str) -> Result<(), String> {
        if self.active {
            return Err("a program is already running".into());
        }
        self.commands.push(command.to_string());
        self.active = true;
        Ok(())
    }
    fn running(&self) -> bool {
        self.active
    }
    fn poll(&mut self) -> Poll {
        if !self.active {
            return Poll::default();
        }
        if self.script.is_empty() {
            self.active = false;
            return Poll { output: String::new(), finished: true };
        }
        let chunk = self.script.remove(0);
        Poll { output: clean_output(&chunk), finished: false }
    }
    fn write(&mut self, text: &str) {
        self.input.push(text.to_string());
    }
    fn stop(&mut self) {
        self.active = false;
    }
}

/// A shared mock, so a test keeps a handle while the editor owns another.
impl Runner for std::rc::Rc<std::cell::RefCell<MockRunner>> {
    fn start(&mut self, command: &str) -> Result<(), String> {
        self.borrow_mut().start(command)
    }
    fn running(&self) -> bool {
        self.borrow().running()
    }
    fn poll(&mut self) -> Poll {
        self.borrow_mut().poll()
    }
    fn write(&mut self, text: &str) {
        self.borrow_mut().write(text)
    }
    fn stop(&mut self) {
        self.borrow_mut().stop()
    }
}

/// A command session on this computer.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct SessionRunner {
    session: Option<i32>,
}

#[cfg(target_arch = "wasm32")]
impl Runner for SessionRunner {
    fn start(&mut self, command: &str) -> Result<(), String> {
        if self.session.is_some() {
            return Err("a program is already running".into());
        }
        let id = ecm_host_abi::ipc::spawn_command("visual", command);
        if id < 0 {
            return Err("can't start a process (too many sessions?)".into());
        }
        ecm_host_abi::ipc::session_resize(id, 120, 40);
        self.session = Some(id);
        Ok(())
    }
    fn running(&self) -> bool {
        self.session.is_some()
    }
    fn poll(&mut self) -> Poll {
        let Some(id) = self.session else { return Poll::default() };
        let mut raw = Vec::new();
        let mut buf = vec![0u8; 4096];
        loop {
            let n = ecm_host_abi::ipc::session_read(id, &mut buf);
            if n <= 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n as usize]);
            if raw.len() > 64 * 1024 {
                break;
            }
        }
        let mut finished = false;
        if raw.is_empty() && ecm_host_abi::ipc::session_status(id) != 0 {
            ecm_host_abi::ipc::session_close(id);
            self.session = None;
            finished = true;
        }
        Poll { output: clean_output(&String::from_utf8_lossy(&raw)), finished }
    }
    fn write(&mut self, text: &str) {
        if let Some(id) = self.session {
            ecm_host_abi::ipc::session_write(id, text.as_bytes());
        }
    }
    fn stop(&mut self) {
        if let Some(id) = self.session.take() {
            ecm_host_abi::ipc::session_close(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_loses_control_sequences() {
        assert_eq!(clean_output("a\r\nb\u{1b}[2J\u{1b}[1;31mred\u{1b}[0m\u{7}\tc\u{1b}]0;title\u{7}d"), "a\nbred\tcd");
    }

    #[test]
    fn the_mock_plays_its_script() {
        let mut r = MockRunner { script: vec!["hi\r\n".into()], ..Default::default() };
        r.start("python x.py").unwrap();
        assert!(r.start("again").is_err());
        assert_eq!(r.poll().output, "hi\n");
        assert!(r.poll().finished);
        assert!(!r.running());
    }
}
