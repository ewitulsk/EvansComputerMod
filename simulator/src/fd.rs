//! File descriptor abstraction layer.
//!
//! Provides the `FileDescriptor` trait and `FdTable` for managing per-process
//! file descriptors. Pipes, files, terminals, and sockets all implement
//! `FileDescriptor`.

use std::collections::HashMap;
use std::io;

/// A file descriptor that can be read from and/or written to.
pub trait FileDescriptor: Send {
    /// Read up to `buf.len()` bytes. Returns bytes read, 0 = EOF.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// Write bytes. Returns bytes written.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
    /// Close the descriptor (called on drop or explicit close).
    fn close(&mut self) {}
    /// Whether this FD supports reading.
    fn is_readable(&self) -> bool;
    /// Whether this FD supports writing.
    fn is_writable(&self) -> bool;
}

/// /dev/null — discards writes, returns EOF on read.
pub struct NullFd;

impl FileDescriptor for NullFd {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Ok(0) // EOF
    }
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len()) // discard
    }
    fn is_readable(&self) -> bool { true }
    fn is_writable(&self) -> bool { true }
}

/// Per-process file descriptor table.
pub struct FdTable {
    fds: HashMap<i32, Box<dyn FileDescriptor>>,
    next_fd: i32,
}

impl FdTable {
    /// Create a new empty FdTable. FDs 0, 1, 2 are NOT pre-allocated —
    /// the caller must set them up (stdin, stdout, stderr).
    pub fn new() -> Self {
        Self {
            fds: HashMap::new(),
            next_fd: 3,
        }
    }

    /// Allocate a new FD number and insert the descriptor.
    pub fn allocate(&mut self, fd: Box<dyn FileDescriptor>) -> i32 {
        let num = self.next_fd;
        self.next_fd += 1;
        self.fds.insert(num, fd);
        num
    }

    /// Insert a descriptor at a specific FD number.
    pub fn insert_at(&mut self, num: i32, fd: Box<dyn FileDescriptor>) {
        self.fds.insert(num, fd);
        if num >= self.next_fd {
            self.next_fd = num + 1;
        }
    }

    /// Get a reference to a descriptor.
    pub fn get(&self, fd: i32) -> Option<&dyn FileDescriptor> {
        self.fds.get(&fd).map(|f| &**f)
    }

    /// Get a mutable reference to a descriptor.
    pub fn get_mut(&mut self, fd: i32) -> Option<&mut (dyn FileDescriptor + 'static)> {
        self.fds.get_mut(&fd).map(|f| &mut **f)
    }

    /// Close a descriptor and remove it from the table.
    pub fn close(&mut self, fd: i32) -> bool {
        if let Some(mut desc) = self.fds.remove(&fd) {
            desc.close();
            true
        } else {
            false
        }
    }

    /// Check if an FD exists.
    pub fn contains(&self, fd: i32) -> bool {
        self.fds.contains_key(&fd)
    }

    /// Number of open FDs.
    pub fn len(&self) -> usize {
        self.fds.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.fds.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_fd_read_eof() {
        let mut fd = NullFd;
        let mut buf = [0u8; 64];
        assert_eq!(fd.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn test_null_fd_write_discards() {
        let mut fd = NullFd;
        assert_eq!(fd.write(b"hello").unwrap(), 5);
    }

    #[test]
    fn test_fd_table_allocate() {
        let mut table = FdTable::new();
        let fd1 = table.allocate(Box::new(NullFd));
        let fd2 = table.allocate(Box::new(NullFd));
        assert_eq!(fd1, 3);
        assert_eq!(fd2, 4);
        assert_eq!(table.len(), 2);
    }

    #[test]
    fn test_fd_table_insert_at() {
        let mut table = FdTable::new();
        table.insert_at(0, Box::new(NullFd));
        table.insert_at(1, Box::new(NullFd));
        table.insert_at(2, Box::new(NullFd));
        assert_eq!(table.len(), 3);
        assert!(table.contains(0));
        assert!(table.contains(1));
        assert!(table.contains(2));
    }

    #[test]
    fn test_fd_table_close() {
        let mut table = FdTable::new();
        let fd = table.allocate(Box::new(NullFd));
        assert!(table.contains(fd));
        assert!(table.close(fd));
        assert!(!table.contains(fd));
        // Closing again returns false
        assert!(!table.close(fd));
    }

    #[test]
    fn test_fd_table_get_mut() {
        let mut table = FdTable::new();
        table.insert_at(0, Box::new(NullFd));
        let fd = table.get_mut(0).unwrap();
        let mut buf = [0u8; 8];
        assert_eq!(fd.read(&mut buf).unwrap(), 0); // NullFd returns EOF
    }
}
