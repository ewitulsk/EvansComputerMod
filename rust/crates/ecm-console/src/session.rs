//! One program's console: its [`Term`], its [`Events`] and the host they
//! run on. The Python and Lua bindings (`_console`) call these; each call
//! that draws sends its escape sequences to the console at once.

use crate::clock::{self, Locale, Now};
use crate::events::{Event, Events, Sources};
use crate::term::Term;

/// Everything a console needs from the computer, on top of the event sources.
pub trait Host: Sources {
    /// Write bytes to the console (stdout), unbuffered.
    fn write_out(&mut self, bytes: &[u8]);
    /// Real time: milliseconds since the Unix epoch.
    fn unix_ms(&mut self) -> i64;
    /// This computer's number, if it has one.
    fn computer_id(&mut self) -> Option<i32>;
    /// The computer's label, if it has one.
    fn label(&mut self) -> Option<String>;
    fn set_label(&mut self, label: Option<&str>);
}

pub struct Session {
    pub term: Term,
    pub events: Events,
    pub host: Box<dyn Host>,
}

/// Where a label is kept on the computer: a file of the computer's own, so
/// it stays with the computer's files.
pub const LABEL_FILE: &str = "etc/label";

impl Session {
    pub fn new(term: Term, mut host: Box<dyn Host>) -> Session {
        let mut events = Events::default();
        events.uptime(&mut *host); // the clock starts with the program
        Session { term, events, host }
    }

    /// Run `f` on the term and send what it drew.
    pub fn draw<R>(&mut self, f: impl FnOnce(&mut Term) -> R) -> R {
        let r = f(&mut self.term);
        let out = self.term.take();
        if !out.is_empty() {
            self.host.write_out(&out);
        }
        r
    }

    /// Restore the console's colours and cursor if the program changed them.
    pub fn finish(&mut self) {
        self.draw(|t| t.finish());
    }

    /// The next event (keys and characters included), as `os.pullEvent` waits for it.
    pub fn pull(&mut self, filter: Option<&str>, timeout: Option<f64>) -> Option<Event> {
        self.events.want_input(&mut *self.host);
        self.events.pull(&mut *self.host, filter, timeout)
    }

    /// The next event without taking the keyboard (`peripheral.pull_event`):
    /// keys only come along once the program pulled them some other way.
    pub fn pull_quiet(&mut self, filter: Option<&str>, timeout: Option<f64>) -> Option<Event> {
        self.events.pull(&mut *self.host, filter, timeout)
    }

    pub fn queue(&mut self, ev: Event) {
        self.events.queue(ev);
    }

    pub fn start_timer(&mut self, seconds: f64) -> Result<u32, String> {
        self.events.start_timer(&mut *self.host, seconds)
    }

    pub fn cancel_timer(&mut self, id: u32) {
        self.events.cancel_timer(id);
    }

    pub fn set_alarm(&mut self, hour: f64) -> Result<u32, String> {
        self.events.set_alarm(&mut *self.host, hour)
    }

    pub fn cancel_alarm(&mut self, id: u32) {
        self.events.cancel_alarm(id);
    }

    pub fn sleep(&mut self, seconds: f64) {
        self.events.sleep(&mut *self.host, seconds);
    }

    pub fn uptime(&mut self) -> f64 {
        self.events.uptime(&mut *self.host)
    }

    fn now(&mut self) -> Now {
        Now { day_ticks: self.host.day_ticks(), unix_ms: self.host.unix_ms() }
    }

    pub fn time(&mut self, locale: Option<&str>) -> Result<f64, String> {
        let l = Locale::parse(locale)?;
        Ok(clock::time(&self.now(), l))
    }

    pub fn day(&mut self, locale: Option<&str>) -> Result<i64, String> {
        let l = Locale::parse(locale)?;
        Ok(clock::day(&self.now(), l))
    }

    pub fn epoch(&mut self, locale: Option<&str>) -> Result<i64, String> {
        let l = Locale::parse(locale)?;
        Ok(clock::epoch(&self.now(), l))
    }
}

/// The real console, for programs on a computer (wasm32-wasip1).
#[cfg(target_os = "wasi")]
pub mod host {
    use super::*;
    use crate::events::RawKey;
    use ecm_host_abi::peripheral::{self, Value};
    use std::io::{Read, Write};

    pub struct Computer {
        start: std::time::Instant,
        has_peripherals: bool,
    }

    impl Computer {
        pub fn new() -> Computer {
            Computer { start: std::time::Instant::now(), has_peripherals: true }
        }
    }

    impl Sources for Computer {
        fn now_ms(&mut self) -> f64 {
            self.start.elapsed().as_secs_f64() * 1000.0
        }
        fn day_ticks(&mut self) -> Option<i64> {
            ecm_host_abi::console::day_ticks()
        }
        fn capture_keys(&mut self) -> bool {
            ecm_host_abi::console::capture_keys()
        }
        fn raw_key(&mut self) -> Option<RawKey> {
            ecm_host_abi::console::poll_key().map(|k| RawKey { hid: k.hid as u32, mods: k.mods, kind: k.kind })
        }
        fn read_stdin(&mut self, buf: &mut [u8]) -> usize {
            ecm_host_abi::console::stdin_nonblocking(true);
            let n = std::io::stdin().lock().read(buf).unwrap_or(0);
            ecm_host_abi::console::stdin_nonblocking(false);
            n
        }
        fn wait(&mut self, timeout_ms: i32) -> Option<Vec<Value>> {
            if self.has_peripherals {
                match peripheral::wait_event(None, timeout_ms) {
                    Ok(ev) => return ev,
                    Err(peripheral::Error::Unavailable) => self.has_peripherals = false,
                    Err(_) => return None,
                }
            }
            if timeout_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(timeout_ms as u64));
            }
            None
        }
    }

    impl Host for Computer {
        fn write_out(&mut self, bytes: &[u8]) {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(bytes);
            let _ = out.flush();
        }
        fn unix_ms(&mut self) -> i64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0)
        }
        fn computer_id(&mut self) -> Option<i32> {
            ecm_host_abi::console::computer_id()
        }
        fn label(&mut self) -> Option<String> {
            let b = ecm_host_abi::fs::read_file_bytes(LABEL_FILE)?;
            let s = String::from_utf8_lossy(&b).trim_end_matches(['\n', '\r']).to_string();
            (!s.is_empty()).then_some(s)
        }
        fn set_label(&mut self, label: Option<&str>) {
            match label.filter(|l| !l.is_empty()) {
                Some(l) => {
                    ecm_host_abi::fs::mkdir("etc");
                    ecm_host_abi::fs::write_file_bytes(LABEL_FILE, l.as_bytes());
                }
                None => {
                    ecm_host_abi::fs::delete(LABEL_FILE);
                }
            }
        }
    }

    /// The session of a program started on the computer (size and cursor from its environment).
    pub fn session() -> Session {
        Session::new(Term::from_env(|k| std::env::var(k).ok()), Box::new(Computer::new()))
    }
}

/// A console for tests: virtual time, scripted input, captured output.
pub mod fake {
    use super::*;
    use crate::events::RawKey;
    use ecm_host_abi::peripheral::Value;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    #[derive(Default)]
    pub struct State {
        pub now: f64,
        pub unix_ms: i64,
        pub ticks: Option<i64>,
        pub raw_available: bool,
        pub raw: VecDeque<RawKey>,
        /// Bytes typed, each arriving once the clock reaches its time (ms).
        pub typed: VecDeque<(f64, Vec<u8>)>,
        /// Peripheral events, each delivered once the clock reaches its time.
        pub periph: VecDeque<(f64, Vec<Value>)>,
        pub out: Vec<u8>,
        pub id: Option<i32>,
        pub label: Option<String>,
    }

    /// Shared with the test, which scripts and inspects it.
    #[derive(Clone, Default)]
    pub struct Fake(pub Rc<RefCell<State>>);

    impl Sources for Fake {
        fn now_ms(&mut self) -> f64 {
            self.0.borrow().now
        }
        fn day_ticks(&mut self) -> Option<i64> {
            self.0.borrow().ticks
        }
        fn capture_keys(&mut self) -> bool {
            self.0.borrow().raw_available
        }
        fn raw_key(&mut self) -> Option<RawKey> {
            self.0.borrow_mut().raw.pop_front()
        }
        fn read_stdin(&mut self, buf: &mut [u8]) -> usize {
            let mut s = self.0.borrow_mut();
            if !s.typed.front().is_some_and(|(t, _)| *t <= s.now) {
                return 0;
            }
            let (_, b) = s.typed.pop_front().unwrap();
            let n = b.len().min(buf.len());
            buf[..n].copy_from_slice(&b[..n]);
            n
        }
        fn wait(&mut self, timeout_ms: i32) -> Option<Vec<Value>> {
            let mut s = self.0.borrow_mut();
            if s.periph.front().is_some_and(|(t, _)| *t <= s.now + timeout_ms as f64) {
                let (t, ev) = s.periph.pop_front().unwrap();
                s.now = s.now.max(t);
                return Some(ev);
            }
            s.now += timeout_ms as f64;
            s.unix_ms += timeout_ms as i64;
            if let Some(t) = s.ticks.as_mut() {
                *t += (timeout_ms / 50) as i64;
            }
            None
        }
    }

    impl Host for Fake {
        fn write_out(&mut self, bytes: &[u8]) {
            self.0.borrow_mut().out.extend_from_slice(bytes);
        }
        fn unix_ms(&mut self) -> i64 {
            self.0.borrow().unix_ms
        }
        fn computer_id(&mut self) -> Option<i32> {
            self.0.borrow().id
        }
        fn label(&mut self) -> Option<String> {
            self.0.borrow().label.clone()
        }
        fn set_label(&mut self, label: Option<&str>) {
            self.0.borrow_mut().label = label.filter(|l| !l.is_empty()).map(str::to_string);
        }
    }

    impl Fake {
        /// What was drawn since the last call, as text.
        pub fn output(&self) -> String {
            String::from_utf8_lossy(&std::mem::take(&mut self.0.borrow_mut().out)).into_owned()
        }
    }
}
