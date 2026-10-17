//! One event queue for a program: keys and typed characters, timers, in-game
//! alarms, peripheral events and events the program queued itself, in the
//! order they happened. ComputerCraft programs are written around this loop
//! (`os.pullEvent`); Python and Lua read it through `events.pull` and
//! `peripheral.pull_event` too.
//!
//! Input (keys and characters) joins the queue once the program asks for it
//! ([`Events::want_input`], done by `events.pull` and `os.pullEvent`), so a
//! program that reads lines with `shell.input` and waits for peripheral
//! events keeps its stdin.

use crate::keys::{self, Input, StdinDecoder};
use ecm_host_abi::peripheral::Value;
use std::collections::VecDeque;

/// A raw key from the Terminal: a USB HID usage, the modifier bits, and what
/// happened (1 pressed, 2 auto-repeat, 3 released, 4 a character typed: `hid`
/// is its byte). Keys and characters come in the order they were typed.
#[derive(Clone, Copy, Debug)]
pub struct RawKey {
    pub hid: u32,
    pub mods: u8,
    pub kind: u8,
}

/// Where events come from: the host, or stand-ins in tests.
pub trait Sources {
    /// Milliseconds on a steady clock.
    fn now_ms(&mut self) -> f64;
    /// The world's day time in ticks, None outside a world.
    fn day_ticks(&mut self) -> Option<i64>;
    /// Start delivering raw keys to [`Sources::raw_key`]; false if this console has none.
    fn capture_keys(&mut self) -> bool;
    /// The next raw key, if any.
    fn raw_key(&mut self) -> Option<RawKey>;
    /// Bytes typed since the last call, without waiting (0 if none).
    fn read_stdin(&mut self, buf: &mut [u8]) -> usize;
    /// The next peripheral event as [name, args...], waiting up to
    /// `timeout_ms` (0 = don't wait); sleeps the timeout when there is none.
    fn wait(&mut self, timeout_ms: i32) -> Option<Vec<Value>>;
    /// The six redstone inputs (shell's side order), None without redstone.
    fn redstone_inputs(&mut self) -> Option<[i32; 6]> {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub name: String,
    pub args: Vec<Value>,
}

impl Event {
    pub fn new(name: &str, args: Vec<Value>) -> Event {
        Event { name: name.to_string(), args }
    }
}

/// Longest wait between looks at the inputs (one game tick).
const POLL_MS: f64 = 50.0;
/// How long typed bytes wait for the raw keys that go with them, while it
/// isn't known yet whether this program gets raw keys (an SSH session on a
/// Terminal can capture them but never sees any).
const HOLD_MS: f64 = 100.0;
/// Timers count in game ticks: a duration is rounded up to a whole tick.
const TICK_S: f64 = 0.05;
const TICKS_PER_DAY: i64 = 24000;

pub struct Events {
    queue: VecDeque<Event>,
    timers: Vec<(u32, f64)>,
    /// (id, absolute in-game tick) of each alarm.
    alarms: Vec<(u32, i64)>,
    next_id: u32,
    input: bool,
    raw: bool,
    raw_seen: bool,
    decoder: StdinDecoder,
    /// Typed bytes waiting for their raw keys, and since when.
    held: Vec<u8>,
    held_since: f64,
    started_ms: Option<f64>,
    /// The redstone inputs last seen, for "redstone" events.
    redstone: Option<[i32; 6]>,
}

impl Default for Events {
    fn default() -> Self {
        Events {
            queue: VecDeque::new(),
            timers: Vec::new(),
            alarms: Vec::new(),
            next_id: 1,
            input: false,
            raw: false,
            raw_seen: false,
            decoder: StdinDecoder::default(),
            held: Vec::new(),
            held_since: 0.0,
            started_ms: None,
            redstone: None,
        }
    }
}

impl Events {
    /// Seconds since the program first used the clock or the queue (`os.clock`).
    pub fn uptime(&mut self, src: &mut dyn Sources) -> f64 {
        let now = src.now_ms();
        let start = *self.started_ms.get_or_insert(now);
        ((now - start) / 1000.0).max(0.0)
    }

    /// Remember the redstone inputs now, so a change before the program
    /// first waits for events still makes a "redstone" event.
    pub fn watch_redstone(&mut self, src: &mut dyn Sources) {
        self.redstone = src.redstone_inputs();
    }

    /// Take keys and typed characters into the queue from now on.
    pub fn want_input(&mut self, src: &mut dyn Sources) {
        if !self.input {
            self.input = true;
            self.raw = src.capture_keys();
        }
    }

    pub fn queue(&mut self, ev: Event) {
        self.queue.push_back(ev);
    }

    /// A timer firing after `seconds` (rounded up to a whole tick, at least
    /// one): a "timer" event with the returned id.
    pub fn start_timer(&mut self, src: &mut dyn Sources, seconds: f64) -> Result<u32, String> {
        if !(seconds >= 0.0) {
            return Err("Expected a non-negative number of seconds".into());
        }
        let ticks = (seconds / TICK_S - 1e-9).ceil().max(1.0);
        let id = self.id();
        let due = src.now_ms() + ticks * 50.0;
        self.timers.push((id, due));
        Ok(id)
    }

    pub fn cancel_timer(&mut self, id: u32) {
        self.timers.retain(|(t, _)| *t != id);
    }

    /// An alarm at in-game hour `hour` (0 <= hour < 24), the next time the
    /// world's clock reaches it: an "alarm" event with the returned id.
    pub fn set_alarm(&mut self, src: &mut dyn Sources, hour: f64) -> Result<u32, String> {
        if !(0.0..24.0).contains(&hour) {
            return Err("Number out of range".into());
        }
        let now = src.day_ticks().unwrap_or(0) + 6000;
        let mut at = now - now.rem_euclid(TICKS_PER_DAY) + (hour * 1000.0) as i64;
        if at <= now {
            at += TICKS_PER_DAY;
        }
        let id = self.id();
        self.alarms.push((id, at));
        Ok(id)
    }

    pub fn cancel_alarm(&mut self, id: u32) {
        self.alarms.retain(|(a, _)| *a != id);
    }

    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        id
    }

    /// Move everything that has happened into the queue, oldest first.
    fn gather(&mut self, src: &mut dyn Sources) {
        if self.input {
            let mut inputs = Vec::new();
            if self.raw {
                while let Some(k) = src.raw_key() {
                    self.raw_seen = true;
                    if k.kind == 4 {
                        inputs.push(Input::Char(k.hid as u8));
                        continue;
                    }
                    let Some(code) = keys::from_hid(k.hid) else { continue };
                    inputs.push(match k.kind {
                        3 => Input::KeyUp(code),
                        2 => Input::Key(code, true),
                        _ => Input::Key(code, false),
                    });
                }
            }
            let mut buf = [0u8; 256];
            loop {
                let n = src.read_stdin(&mut buf);
                if n == 0 {
                    break;
                }
                if self.held.is_empty() {
                    self.held_since = src.now_ms();
                }
                self.held.extend_from_slice(&buf[..n]);
            }
            // With raw keys, the characters come with them, in order: stdin's copy
            // is dropped. Until a raw key has come, typed bytes wait a moment for
            // theirs; without any (an SSH session), keys are worked out from stdin.
            if !self.held.is_empty() {
                if self.raw && self.raw_seen {
                    self.held.clear();
                } else if !self.raw || src.now_ms() - self.held_since >= HOLD_MS {
                    let held = std::mem::take(&mut self.held);
                    self.decoder.feed(&held, false, &mut inputs);
                }
            }
            for i in inputs {
                self.queue.push_back(match i {
                    Input::Key(k, held) => Event::new("key", vec![Value::Int(k as i64), Value::Bool(held)]),
                    Input::KeyUp(k) => Event::new("key_up", vec![Value::Int(k as i64)]),
                    Input::Char(c) => Event::new("char", vec![Value::Bytes(vec![c])]),
                });
            }
        }
        // A "redstone" event when any input changed since the last look (or
        // since the program started, see watch_redstone).
        if self.input || self.redstone.is_some() {
            if let Some(now_in) = src.redstone_inputs() {
                if self.redstone.is_some_and(|before| before != now_in) {
                    self.queue.push_back(Event::new("redstone", vec![]));
                }
                self.redstone = Some(now_in);
            }
        }
        let now = src.now_ms();
        if !self.timers.is_empty() {
            let mut due: Vec<(u32, f64)> = self.timers.iter().copied().filter(|(_, t)| *t <= now).collect();
            due.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            self.timers.retain(|(_, t)| *t > now);
            for (id, _) in due {
                self.queue.push_back(Event::new("timer", vec![Value::Int(id as i64)]));
            }
        }
        if !self.alarms.is_empty() {
            if let Some(ticks) = src.day_ticks() {
                let now = ticks + 6000;
                let due: Vec<u32> = self.alarms.iter().filter(|(_, at)| *at <= now).map(|(id, _)| *id).collect();
                self.alarms.retain(|(_, at)| *at > now);
                for id in due {
                    self.queue.push_back(Event::new("alarm", vec![Value::Int(id as i64)]));
                }
            }
        }
    }

    fn take(&mut self, filter: Option<&str>) -> Option<Event> {
        while let Some(ev) = self.queue.pop_front() {
            if filter.is_none_or(|f| f == ev.name || ev.name == "terminate") {
                return Some(ev);
            }
        }
        None
    }

    /// The next event named `filter` (any event if None; "terminate" always
    /// comes through), discarding the others. `timeout` in seconds: None
    /// waits forever, 0 only looks. None on timeout.
    pub fn pull(&mut self, src: &mut dyn Sources, filter: Option<&str>, timeout: Option<f64>) -> Option<Event> {
        let deadline = timeout.map(|t| src.now_ms() + t.max(0.0) * 1000.0);
        loop {
            self.gather(src);
            if let Some(ev) = self.take(filter) {
                return Some(ev);
            }
            let now = src.now_ms();
            if deadline.is_some_and(|d| now >= d) {
                return None;
            }
            let mut until = now + POLL_MS;
            if let Some(d) = deadline {
                until = until.min(d);
            }
            for (_, t) in &self.timers {
                until = until.min(*t);
            }
            if !self.held.is_empty() {
                until = until.min(self.held_since + HOLD_MS);
            }
            let ms = (until - now).ceil().max(0.0) as i32;
            if let Some(mut items) = src.wait(ms) {
                if !items.is_empty() {
                    let name = match items.remove(0) {
                        Value::Str(s) => s,
                        Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
                        _ => continue,
                    };
                    // Earlier input goes first.
                    self.gather(src);
                    self.queue.push_back(Event { name, args: items });
                }
            }
        }
    }

    /// Wait `seconds` (rounded up to a tick, at least one), discarding the
    /// events that arrive meanwhile, as `sleep` does.
    pub fn sleep(&mut self, src: &mut dyn Sources, seconds: f64) {
        let Ok(id) = self.start_timer(src, seconds.max(0.0)) else { return };
        loop {
            match self.pull(src, Some("timer"), None) {
                Some(ev) if ev.args.first() == Some(&Value::Int(id as i64)) => return,
                Some(ev) if ev.name == "terminate" => {
                    self.queue.push_front(ev);
                    return;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A console on virtual time: `wait` advances the clock.
    #[derive(Default)]
    pub struct Fake {
        pub now: f64,
        pub ticks: Option<i64>,
        pub raw_available: bool,
        pub raw: VecDeque<RawKey>,
        pub stdin: VecDeque<Vec<u8>>,
        /// Peripheral events, each delivered once the clock reaches its time.
        pub periph: VecDeque<(f64, Vec<Value>)>,
        /// Typed bytes, each arriving once the clock reaches its time.
        pub typed: VecDeque<(f64, Vec<u8>)>,
    }

    impl Sources for Fake {
        fn now_ms(&mut self) -> f64 {
            self.now
        }
        fn day_ticks(&mut self) -> Option<i64> {
            self.ticks
        }
        fn capture_keys(&mut self) -> bool {
            self.raw_available
        }
        fn raw_key(&mut self) -> Option<RawKey> {
            self.raw.pop_front()
        }
        fn read_stdin(&mut self, buf: &mut [u8]) -> usize {
            while self.typed.front().is_some_and(|(t, _)| *t <= self.now) {
                let (_, b) = self.typed.pop_front().unwrap();
                self.stdin.push_back(b);
            }
            let Some(b) = self.stdin.pop_front() else { return 0 };
            buf[..b.len()].copy_from_slice(&b);
            b.len()
        }
        fn wait(&mut self, timeout_ms: i32) -> Option<Vec<Value>> {
            if self.periph.front().is_some_and(|(t, _)| *t <= self.now + timeout_ms as f64) {
                let (t, ev) = self.periph.pop_front().unwrap();
                self.now = self.now.max(t);
                return Some(ev);
            }
            self.now += timeout_ms as f64;
            if let Some(t) = self.ticks.as_mut() {
                *t += (timeout_ms / 50) as i64;
            }
            None
        }
    }

    fn ints(ev: &Event) -> Vec<i64> {
        ev.args.iter().filter_map(|v| if let Value::Int(i) = v { Some(*i) } else { None }).collect()
    }

    #[test]
    fn timers_fire_in_order_after_whole_ticks() {
        let mut src = Fake::default();
        let mut ev = Events::default();
        let slow = ev.start_timer(&mut src, 0.12).unwrap(); // 3 ticks = 150 ms
        let fast = ev.start_timer(&mut src, 0.0).unwrap(); // next tick
        let cancelled = ev.start_timer(&mut src, 0.05).unwrap();
        ev.cancel_timer(cancelled);
        let a = ev.pull(&mut src, None, None).unwrap();
        assert_eq!((a.name.as_str(), ints(&a)), ("timer", vec![fast as i64]));
        assert!((src.now - 50.0).abs() < 1.0, "fired at one tick, now {}", src.now);
        let b = ev.pull(&mut src, None, None).unwrap();
        assert_eq!(ints(&b), vec![slow as i64]);
        assert!((src.now - 150.0).abs() < 1.0);
        assert!(ev.start_timer(&mut src, -1.0).is_err());
        assert_eq!(ev.pull(&mut src, None, Some(0.2)), None, "the cancelled timer never fires");
    }

    #[test]
    fn filters_discard_other_events_but_not_terminate() {
        let mut src = Fake::default();
        let mut ev = Events::default();
        ev.queue(Event::new("a", vec![]));
        ev.queue(Event::new("terminate", vec![]));
        ev.queue(Event::new("b", vec![Value::Int(7)]));
        assert_eq!(ev.pull(&mut src, Some("b"), Some(0.0)).unwrap().name, "terminate");
        assert_eq!(ints(&ev.pull(&mut src, Some("b"), Some(0.0)).unwrap()), vec![7]);
        assert_eq!(ev.pull(&mut src, None, Some(0.0)), None, "\"a\" was discarded");
    }

    #[test]
    fn input_peripheral_events_and_queued_events_share_one_queue() {
        let mut src = Fake::default();
        src.typed.push_back((10.0, b"x".to_vec()));
        src.periph.push_back((30.0, vec![Value::Str("redstone_link".into()), Value::Str("left".into())]));
        let mut ev = Events::default();
        // Before a program asks for input, typed bytes stay on stdin.
        assert_eq!(ev.pull(&mut src, None, None).unwrap().name, "redstone_link");
        assert_eq!(src.typed.len(), 1);
        ev.want_input(&mut src);
        let names: Vec<String> = (0..3).map(|_| ev.pull(&mut src, None, None).unwrap().name).collect();
        assert_eq!(names, ["key", "char", "key_up"]);
        ev.queue(Event::new("mine", vec![Value::Str("hi".into())]));
        assert_eq!(ev.pull(&mut src, None, Some(0.0)).unwrap().name, "mine");
    }

    #[test]
    fn raw_keys_give_presses_and_stdin_the_characters() {
        let mut src = Fake { raw_available: true, ..Default::default() };
        let mut ev = Events::default();
        ev.want_input(&mut src);
        src.raw.push_back(RawKey { hid: 0xE0, mods: 2, kind: 1 }); // left ctrl
        src.raw.push_back(RawKey { hid: 0x04, mods: 0, kind: 1 });
        src.raw.push_back(RawKey { hid: 0x04, mods: 0, kind: 2 });
        src.raw.push_back(RawKey { hid: 0x04, mods: 0, kind: 3 });
        src.raw.push_back(RawKey { hid: b'a' as u32, mods: 0, kind: 4 });
        src.raw.push_back(RawKey { hid: 0x28, mods: 0, kind: 1 }); // Enter, after the "a"
        src.stdin.push_back(b"a\n".to_vec()); // stdin's copy is dropped
        let got: Vec<(String, Vec<Value>)> = std::iter::from_fn(|| ev.pull(&mut src, None, Some(0.0)))
            .map(|e| (e.name, e.args))
            .collect();
        assert_eq!(
            got,
            [
                ("key".into(), vec![Value::Int(341), Value::Bool(false)]),
                ("key".into(), vec![Value::Int(65), Value::Bool(false)]),
                ("key".into(), vec![Value::Int(65), Value::Bool(true)]),
                ("key_up".into(), vec![Value::Int(65)]),
                ("char".into(), vec![Value::Bytes(b"a".to_vec())]),
                ("key".into(), vec![Value::Int(257), Value::Bool(false)]),
            ]
        );
    }

    #[test]
    fn without_raw_keys_typed_bytes_give_keys_after_a_moment() {
        // Capture is on, but no raw keys ever come (an SSH session on a Terminal).
        let mut src = Fake { raw_available: true, ..Default::default() };
        let mut ev = Events::default();
        ev.want_input(&mut src);
        src.stdin.push_back(b"
".to_vec());
        let e = ev.pull(&mut src, None, None).unwrap();
        assert_eq!(e, Event::new("key", vec![Value::Int(keys::ENTER as i64), Value::Bool(false)]));
        assert!(src.now >= HOLD_MS, "it waited for raw keys first ({} ms)", src.now);
    }

    #[test]
    fn a_changed_redstone_input_is_an_event() {
        struct Rs(Fake, [i32; 6]);
        impl Sources for Rs {
            fn now_ms(&mut self) -> f64 { self.0.now_ms() }
            fn day_ticks(&mut self) -> Option<i64> { None }
            fn capture_keys(&mut self) -> bool { false }
            fn raw_key(&mut self) -> Option<RawKey> { None }
            fn read_stdin(&mut self, _: &mut [u8]) -> usize { 0 }
            fn wait(&mut self, ms: i32) -> Option<Vec<Value>> {
                let r = self.0.wait(ms);
                if self.0.now >= 200.0 { self.1[4] = 15; } // a lever on the left, at 200 ms
                r
            }
            fn redstone_inputs(&mut self) -> Option<[i32; 6]> { Some(self.1) }
        }
        let mut src = Rs(Fake::default(), [0; 6]);
        let mut ev = Events::default();
        ev.want_input(&mut src);
        assert_eq!(ev.pull(&mut src, None, Some(0.1)), None, "nothing changed yet");
        assert_eq!(ev.pull(&mut src, None, None).unwrap().name, "redstone");
        assert!(src.0.now >= 200.0);
    }

    #[test]
    fn sleep_waits_and_discards_and_alarms_follow_the_world() {
        let mut src = Fake { ticks: Some(0), ..Default::default() };
        let mut ev = Events::default();
        ev.queue(Event::new("noise", vec![]));
        ev.sleep(&mut src, 0.1);
        assert!((src.now - 100.0).abs() < 1.0);
        assert_eq!(ev.pull(&mut src, None, Some(0.0)), None, "sleep discarded the event");
        // 06:00 + 0.1 h = 100 ticks = 5 s of waiting.
        let id = ev.set_alarm(&mut src, 6.1).unwrap();
        let a = ev.pull(&mut src, None, Some(10.0)).unwrap();
        assert_eq!((a.name.as_str(), ints(&a)), ("alarm", vec![id as i64]));
        assert!(ev.set_alarm(&mut src, 24.0).is_err());
        assert_eq!(ev.uptime(&mut src), 0.0);
    }
}
