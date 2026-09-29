//! Child socket syscalls → kernel `handle_sock_ipc`, the `NetIpcBridge`
//! model: a child thread enqueues a request and blocks (idle for the
//! scheduler); the kernel loop dispatches it and, while the kernel answers
//! `IPC_PENDING`, keeps it queued and retries after the next tick or frame.
//! The kernel owns every socket timeout.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::proc::Killed;
use crate::sched::{ActorId, Sched};

pub const IPC_PENDING: i32 = -11;

#[derive(Clone, Debug)]
pub struct IpcResult {
    pub status: i32,
    pub payload: Vec<u8>,
}

impl IpcResult {
    pub fn error() -> Self {
        IpcResult { status: -1, payload: Vec::new() }
    }
}

pub struct IpcReq {
    pub session: i32,
    pub syscall: i32,
    pub args: Vec<u8>,
    slot: Arc<Mutex<Option<IpcResult>>>,
    abandoned: Arc<AtomicBool>,
    actor: ActorId,
}

impl IpcReq {
    /// The child gave up waiting (it was killed); never dispatch this.
    pub fn is_abandoned(&self) -> bool {
        self.abandoned.load(Ordering::SeqCst)
    }
}

pub struct IpcBridge {
    incoming: Mutex<Vec<IpcReq>>,
    sched: Arc<Sched>,
}

impl IpcBridge {
    pub fn new(sched: Arc<Sched>) -> Arc<Self> {
        Arc::new(IpcBridge { incoming: Mutex::new(Vec::new()), sched })
    }

    /// Child side: perform one syscall and wait for the kernel's answer.
    pub fn call(&self, session: i32, syscall: i32, args: Vec<u8>, actor: ActorId, killed: &AtomicBool) -> Result<IpcResult, Killed> {
        if killed.load(Ordering::SeqCst) {
            return Err(Killed);
        }
        let slot = Arc::new(Mutex::new(None));
        let abandoned = Arc::new(AtomicBool::new(false));
        self.incoming
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(IpcReq { session, syscall, args, slot: slot.clone(), abandoned: abandoned.clone(), actor });
        self.sched.notify_event();
        self.sched.wait(actor, None, || {
            killed.load(Ordering::SeqCst) || slot.lock().unwrap_or_else(|e| e.into_inner()).is_some()
        });
        let r = slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        match r {
            Some(r) => Ok(r),
            None => {
                abandoned.store(true, Ordering::SeqCst);
                Err(Killed)
            }
        }
    }

    /// Kernel loop: new requests, in a deterministic order (each session
    /// has at most one request in flight, so sorting by session is total).
    pub fn take_incoming(&self) -> Vec<IpcReq> {
        let mut v = std::mem::take(&mut *self.incoming.lock().unwrap_or_else(|e| e.into_inner()));
        v.sort_by_key(|r| r.session);
        v
    }

    pub fn complete(&self, req: IpcReq, r: IpcResult) {
        *req.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
        self.sched.wake(req.actor);
    }


}
