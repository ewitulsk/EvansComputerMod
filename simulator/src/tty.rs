//! Virtual TTY layer.
//!
//! Decouples terminal I/O from the physical Minecraft terminal.
//! Each TTY has its own input queue and output buffer.
//! The physical terminal is attached to one TTY at a time.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::fd::{TerminalReadFd, TerminalWriteFd};
use crate::terminal_io::TerminalBuffer;

/// A virtual terminal with its own I/O buffers.
pub struct VirtualTty {
    pub id: u32,
    pub input_queue: Arc<Mutex<VecDeque<u8>>>,
    pub output_buffer: Arc<Mutex<TerminalBuffer>>,
    pub width: u16,
    pub height: u16,
}

impl VirtualTty {
    pub fn new(id: u32, width: u16, height: u16) -> Self {
        let mut tb = TerminalBuffer::new(width as usize, height as usize);
        tb.headless = true; // Don't try to render to real terminal
        Self {
            id,
            input_queue: Arc::new(Mutex::new(VecDeque::new())),
            output_buffer: Arc::new(Mutex::new(tb)),
            width,
            height,
        }
    }

    /// Create a read FD connected to this TTY's input queue.
    pub fn create_read_fd(&self) -> TerminalReadFd {
        TerminalReadFd::new(self.input_queue.clone())
    }

    /// Create a write FD connected to this TTY's output buffer.
    pub fn create_write_fd(&self) -> TerminalWriteFd {
        let buf = self.output_buffer.clone();
        TerminalWriteFd::new(Box::new(move |data: &[u8]| {
            if let Ok(s) = std::str::from_utf8(data) {
                if let Ok(mut tb) = buf.lock() {
                    tb.write_text(s);
                }
            }
        }))
    }

    /// Push input bytes to this TTY (from keyboard or SSH).
    pub fn push_input(&self, data: &[u8]) {
        let mut q = self.input_queue.lock().unwrap();
        q.extend(data);
    }

    /// Get the current output as a string.
    pub fn get_output(&self) -> String {
        let tb = self.output_buffer.lock().unwrap();
        let mut result = String::new();
        for row in &tb.buffer {
            let line: String = row.iter().collect();
            result.push_str(&line);
            result.push('\n');
        }
        result
    }
}

/// Registry of all virtual TTYs for a computer.
pub struct TtyRegistry {
    ttys: HashMap<u32, VirtualTty>,
    next_id: u32,
    pub foreground_tty: u32,
}

impl TtyRegistry {
    pub fn new() -> Self {
        Self {
            ttys: HashMap::new(),
            next_id: 1, // TTY 0 is the physical terminal
            foreground_tty: 0,
        }
    }

    /// Create a new virtual TTY. Returns the TTY ID.
    pub fn create(&mut self, width: u16, height: u16) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.ttys.insert(id, VirtualTty::new(id, width, height));
        id
    }

    /// Get a TTY by ID.
    pub fn get(&self, id: u32) -> Option<&VirtualTty> {
        self.ttys.get(&id)
    }

    /// Get a mutable TTY by ID.
    pub fn get_mut(&mut self, id: u32) -> Option<&mut VirtualTty> {
        self.ttys.get_mut(&id)
    }

    /// Set the foreground TTY (which one is displayed on the physical terminal).
    pub fn set_foreground(&mut self, id: u32) -> bool {
        if id == 0 || self.ttys.contains_key(&id) {
            self.foreground_tty = id;
            true
        } else {
            false
        }
    }

    /// Remove a TTY.
    pub fn remove(&mut self, id: u32) -> Option<VirtualTty> {
        if id == 0 { return None; } // Can't remove physical terminal
        self.ttys.remove(&id)
    }

    /// List all TTY IDs.
    pub fn list_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = std::iter::once(0).chain(self.ttys.keys().copied()).collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tty_create() {
        let mut registry = TtyRegistry::new();
        let id = registry.create(80, 24);
        assert_eq!(id, 1);
        assert!(registry.get(id).is_some());
    }

    #[test]
    fn test_tty_io() {
        let tty = VirtualTty::new(1, 80, 24);
        tty.push_input(b"hello");

        let mut read_fd = tty.create_read_fd();
        let mut buf = [0u8; 64];
        use crate::fd::FileDescriptor;
        let n = read_fd.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[test]
    fn test_tty_write() {
        let tty = VirtualTty::new(1, 80, 24);
        let mut write_fd = tty.create_write_fd();
        use crate::fd::FileDescriptor;
        write_fd.write(b"test output").unwrap();
        // Output should be in the terminal buffer
        let output = tty.get_output();
        assert!(output.contains("test output"));
    }

    #[test]
    fn test_tty_registry() {
        let mut registry = TtyRegistry::new();
        let id1 = registry.create(80, 24);
        let id2 = registry.create(80, 24);
        assert_eq!(registry.list_ids().len(), 3); // 0 + id1 + id2
        assert!(registry.set_foreground(id1));
        assert_eq!(registry.foreground_tty, id1);
        registry.remove(id2);
        assert_eq!(registry.list_ids().len(), 2);
    }
}
