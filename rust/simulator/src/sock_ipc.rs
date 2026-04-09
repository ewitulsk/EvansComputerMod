//! Socket IPC bridge — thread-safe request/response queue connecting child
//! WASI processes to the kernel's handle_sock_ipc export.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A single IPC request from a child WASI thread.
pub struct SockIpcRequest {
    pub session_id: i32,
    pub syscall_id: i32,
    pub args: Vec<u8>,
    pub response: Arc<(Mutex<Option<Vec<u8>>>, Condvar)>,
}

/// Thread-safe bridge for socket IPC between child processes and kernel.
pub struct SockIpcBridge {
    pending: Mutex<VecDeque<SockIpcRequest>>,
}

impl SockIpcBridge {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(VecDeque::new()),
        }
    }

    /// Called from child thread: enqueue request and block until response.
    pub fn call_blocking(
        &self,
        session_id: i32,
        syscall_id: i32,
        args: Vec<u8>,
        timeout_ms: u64,
    ) -> Vec<u8> {
        let response = Arc::new((Mutex::new(None), Condvar::new()));
        let req = SockIpcRequest {
            session_id,
            syscall_id,
            args,
            response: response.clone(),
        };

        // Enqueue
        self.pending.lock().unwrap().push_back(req);

        // Wait for response
        let (lock, cvar) = &*response;
        let mut guard = lock.lock().unwrap();
        let timeout = Duration::from_millis(timeout_ms);
        loop {
            if let Some(ref data) = *guard {
                return data.clone();
            }
            let (new_guard, timeout_result) = cvar.wait_timeout(guard, timeout).unwrap();
            guard = new_guard;
            if guard.is_some() {
                return guard.take().unwrap_or_default();
            }
            if timeout_result.timed_out() {
                return Vec::new();
            }
        }
    }

    /// Called from kernel thread: dequeue all pending requests.
    /// Returns requests to be serviced.
    pub fn drain_pending(&self) -> Vec<SockIpcRequest> {
        let mut pending = self.pending.lock().unwrap();
        pending.drain(..).collect()
    }

    /// Check if there are pending requests.
    pub fn has_pending(&self) -> bool {
        !self.pending.lock().unwrap().is_empty()
    }
}

/// Complete a request by setting the response and notifying the waiter.
pub fn complete_request(req: &SockIpcRequest, data: Vec<u8>) {
    let (lock, cvar) = &*req.response;
    let mut guard = lock.lock().unwrap();
    *guard = Some(data);
    cvar.notify_one();
}
