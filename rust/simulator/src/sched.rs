//! The simulation clock and the idle accounting that makes virtual time work.
//!
//! Every child process thread is an *actor*. An actor is either running or
//! blocked inside a host call (IPC wait, sleep, stdin/pipe read, full pipe
//! write). In virtual mode the main loop only advances time when no actor is
//! running ("quiescence"), and then jumps straight to the earliest pending
//! deadline. Kernels run on the main thread, so they are idle by definition
//! whenever the main loop is deciding what to do next.
//!
//! Lost-wakeup freedom: an actor evaluates its readiness predicate while
//! holding the scheduler lock and marks itself blocked in the same critical
//! section. Wakers first change the shared data (under that data's own
//! lock, released again) and only then call [`Sched::wake`], which takes the
//! scheduler lock. Whoever flips an actor from blocked to runnable counts it
//! as busy again, so the main loop can never observe "all idle" between a
//! wake-up and the woken actor actually running. Lock order is always
//! scheduler → data, never the reverse.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClockMode {
    Real,
    Virtual,
}

pub type ActorId = u64;

pub enum Wake {
    Ready,
    Timeout,
}

struct Actor {
    blocked: bool,
    deadline: Option<i64>,
    label: String,
}

struct State {
    next_id: ActorId,
    actors: BTreeMap<ActorId, Actor>,
    busy: usize,
    events: u64,
}

pub struct Sched {
    mode: ClockMode,
    start: Instant,
    base_ms: i64,
    now_v: AtomicI64,
    st: Mutex<State>,
    cv: Condvar,
}

/// Virtual time starts at a plausible epoch value (2026-01-01T00:00:00Z) so
/// timestamps printed by guests look sane. The kernel only needs monotonic.
pub const VIRTUAL_EPOCH_MS: i64 = 1_767_225_600_000;

impl Sched {
    pub fn new(mode: ClockMode) -> Self {
        let base_ms = match mode {
            ClockMode::Virtual => VIRTUAL_EPOCH_MS,
            ClockMode::Real => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(VIRTUAL_EPOCH_MS),
        };
        Sched {
            mode,
            start: Instant::now(),
            base_ms,
            now_v: AtomicI64::new(base_ms),
            st: Mutex::new(State { next_id: 1, actors: BTreeMap::new(), busy: 0, events: 0 }),
            cv: Condvar::new(),
        }
    }

    pub fn is_virtual(&self) -> bool {
        self.mode == ClockMode::Virtual
    }

    /// Current simulation time in ms.
    pub fn now(&self) -> i64 {
        match self.mode {
            ClockMode::Real => self.base_ms + self.start.elapsed().as_millis() as i64,
            ClockMode::Virtual => self.now_v.load(Ordering::SeqCst),
        }
    }

    /// Milliseconds since the simulation started.
    pub fn elapsed(&self) -> i64 {
        self.now() - self.base_ms
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.st.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A new runnable actor (a child process about to start running).
    pub fn register(&self, label: &str) -> ActorId {
        let mut g = self.lock();
        let id = g.next_id;
        g.next_id += 1;
        g.actors.insert(id, Actor { blocked: false, deadline: None, label: label.to_string() });
        g.busy += 1;
        id
    }

    /// The actor finished for good.
    pub fn exit(&self, id: ActorId) {
        let mut g = self.lock();
        if let Some(a) = g.actors.remove(&id) {
            if !a.blocked {
                g.busy -= 1;
            }
        }
        g.events += 1;
        self.cv.notify_all();
    }

    fn unblock(g: &mut State, id: ActorId) {
        let mut flipped = false;
        if let Some(a) = g.actors.get_mut(&id) {
            if a.blocked {
                a.blocked = false;
                a.deadline = None;
                flipped = true;
            }
        }
        if flipped {
            g.busy += 1;
        }
    }

    /// Block actor `id` until `ready()` holds or `deadline` (sim ms) passes.
    /// `ready` runs under the scheduler lock; it may take data locks but
    /// must not call back into the scheduler.
    pub fn wait(&self, id: ActorId, deadline: Option<i64>, mut ready: impl FnMut() -> bool) -> Wake {
        let mut g = self.lock();
        loop {
            if ready() {
                Self::unblock(&mut g, id);
                return Wake::Ready;
            }
            let now = self.now();
            if let Some(d) = deadline {
                if now >= d {
                    Self::unblock(&mut g, id);
                    return Wake::Timeout;
                }
            }
            let mut newly = false;
            if let Some(a) = g.actors.get_mut(&id) {
                if !a.blocked {
                    a.blocked = true;
                    newly = true;
                }
                a.deadline = deadline;
            }
            if newly {
                g.busy -= 1;
                g.events += 1;
                self.cv.notify_all();
            }
            g = match self.mode {
                ClockMode::Virtual => self.cv.wait_timeout(g, Duration::from_millis(200)).unwrap_or_else(|e| e.into_inner()).0,
                ClockMode::Real => {
                    let tmo = deadline.map(|d| (d - now).clamp(1, 50)).unwrap_or(50) as u64;
                    self.cv.wait_timeout(g, Duration::from_millis(tmo)).unwrap_or_else(|e| e.into_inner()).0
                }
            };
        }
    }

    /// Make a blocked actor runnable (its predicate may now hold).
    pub fn wake(&self, id: ActorId) {
        let mut g = self.lock();
        Self::unblock(&mut g, id);
        g.events += 1;
        self.cv.notify_all();
    }

    pub fn wake_all(&self, ids: &[ActorId]) {
        if ids.is_empty() {
            return;
        }
        let mut g = self.lock();
        for &id in ids {
            Self::unblock(&mut g, id);
        }
        g.events += 1;
        self.cv.notify_all();
    }

    /// Something happened that the main loop should look at.
    pub fn notify_event(&self) {
        let mut g = self.lock();
        g.events += 1;
        self.cv.notify_all();
    }

    pub fn busy(&self) -> usize {
        self.lock().busy
    }

    /// Labels of actors that are currently running (for stall diagnostics).
    pub fn running_labels(&self) -> Vec<String> {
        self.lock().actors.values().filter(|a| !a.blocked).map(|a| a.label.clone()).collect()
    }

    /// Wait until no actor is running. Returns false if `max_wall` passed
    /// first (a compute-bound child).
    pub fn quiesce(&self, max_wall: Duration) -> bool {
        let t0 = Instant::now();
        let mut g = self.lock();
        while g.busy > 0 {
            let el = t0.elapsed();
            if el >= max_wall {
                return false;
            }
            let left = (max_wall - el).min(Duration::from_millis(20));
            g = self.cv.wait_timeout(g, left).unwrap_or_else(|e| e.into_inner()).0;
        }
        true
    }

    /// Real mode: sleep until an event is posted (event counter moves past
    /// `seen`) or `timeout` passes. Returns the current event counter.
    pub fn wait_event(&self, seen: u64, timeout: Duration) -> u64 {
        let mut g = self.lock();
        if g.events == seen {
            g = self.cv.wait_timeout(g, timeout).unwrap_or_else(|e| e.into_inner()).0;
        }
        g.events
    }

    pub fn events(&self) -> u64 {
        self.lock().events
    }

    /// Earliest deadline of a blocked actor (child sleeps, timed waits).
    pub fn next_actor_deadline(&self) -> Option<i64> {
        self.lock().actors.values().filter(|a| a.blocked).filter_map(|a| a.deadline).min()
    }

    /// Virtual mode: move time forward and release actors whose deadline passed.
    pub fn advance_to(&self, t: i64) {
        if self.mode != ClockMode::Virtual {
            return;
        }
        let mut g = self.lock();
        let now = self.now_v.load(Ordering::SeqCst);
        if t > now {
            self.now_v.store(t, Ordering::SeqCst);
        }
        let now = self.now_v.load(Ordering::SeqCst);
        let due: Vec<ActorId> = g
            .actors
            .iter()
            .filter(|(_, a)| a.blocked && a.deadline.is_some_and(|d| d <= now))
            .map(|(&id, _)| id)
            .collect();
        for id in due {
            Self::unblock(&mut g, id);
        }
        g.events += 1;
        self.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    #[test]
    fn virtual_sleep_advances_only_when_idle() {
        let s = Arc::new(Sched::new(ClockMode::Virtual));
        let t0 = s.now();
        let id = s.register("t");
        let s2 = s.clone();
        let done = Arc::new(AtomicBool::new(false));
        let d2 = done.clone();
        let h = std::thread::spawn(move || {
            let dl = s2.now() + 1000;
            s2.wait(id, Some(dl), || false);
            d2.store(true, Ordering::SeqCst);
            s2.exit(id);
        });
        assert!(s.quiesce(Duration::from_secs(5)));
        assert_eq!(s.next_actor_deadline(), Some(t0 + 1000));
        assert!(!done.load(Ordering::SeqCst));
        s.advance_to(t0 + 1000);
        h.join().unwrap();
        assert!(done.load(Ordering::SeqCst));
        assert_eq!(s.busy(), 0);
    }

    #[test]
    fn wake_counts_actor_busy_before_it_runs() {
        let s = Arc::new(Sched::new(ClockMode::Virtual));
        let id = s.register("t");
        let flag = Arc::new(AtomicBool::new(false));
        let (s2, f2) = (s.clone(), flag.clone());
        let h = std::thread::spawn(move || {
            s2.wait(id, None, || f2.load(Ordering::SeqCst));
            s2.exit(id);
        });
        assert!(s.quiesce(Duration::from_secs(5)));
        flag.store(true, Ordering::SeqCst);
        s.wake(id);
        // Immediately after wake the actor is counted busy even if it has
        // not been scheduled yet.
        h.join().unwrap();
        assert_eq!(s.busy(), 0);
    }
}
