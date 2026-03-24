use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

pub const IRQ_KEYBOARD: i32 = 1;
pub const IRQ_REDSTONE: i32 = 2;
pub const IRQ_NETWORK: i32 = 3;
pub const IRQ_TERMINATE: i32 = 15;

#[derive(Clone, Debug)]
pub struct InterruptEvent {
    pub irq: i32,
    pub payload: String,
}

#[derive(Clone)]
pub struct InterruptQueue {
    queue: Arc<Mutex<VecDeque<InterruptEvent>>>,
}

impl InterruptQueue {
    pub fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn push(&self, irq: i32, payload: String) {
        self.queue.lock().unwrap().push_back(InterruptEvent { irq, payload });
    }

    pub fn pop(&self) -> Option<InterruptEvent> {
        self.queue.lock().unwrap().pop_front()
    }
}
