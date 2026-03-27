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

use std::sync::{Arc, Mutex, Condvar};
use std::collections::VecDeque;

/// Internal shared state for a pipe.
struct PipeBuffer {
    data: VecDeque<u8>,
    capacity: usize,
    write_closed: bool,
    read_closed: bool,
}

/// One end of a pipe. Created in pairs via `create_pipe()`.
pub struct PipeFd {
    buffer: Arc<(Mutex<PipeBuffer>, Condvar)>,
    is_read_end: bool,
}

/// Create a pipe pair: (read_end, write_end).
pub fn create_pipe() -> (PipeFd, PipeFd) {
    create_pipe_with_capacity(4096)
}

/// Create a pipe pair with custom capacity.
pub fn create_pipe_with_capacity(capacity: usize) -> (PipeFd, PipeFd) {
    let buffer = Arc::new((
        Mutex::new(PipeBuffer {
            data: VecDeque::with_capacity(capacity),
            capacity,
            write_closed: false,
            read_closed: false,
        }),
        Condvar::new(),
    ));
    let read_end = PipeFd { buffer: buffer.clone(), is_read_end: true };
    let write_end = PipeFd { buffer, is_read_end: false };
    (read_end, write_end)
}

impl FileDescriptor for PipeFd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.is_read_end {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not the read end"));
        }
        let (lock, cvar) = &*self.buffer;
        let mut pipe = lock.lock().unwrap();

        // Wait for data or write-end closure
        while pipe.data.is_empty() && !pipe.write_closed {
            // Use a timeout to avoid deadlock in WASM context
            let result = cvar.wait_timeout(pipe, std::time::Duration::from_millis(100)).unwrap();
            pipe = result.0;
        }

        if pipe.data.is_empty() && pipe.write_closed {
            return Ok(0); // EOF
        }

        let to_read = buf.len().min(pipe.data.len());
        for i in 0..to_read {
            buf[i] = pipe.data.pop_front().unwrap();
        }
        cvar.notify_all(); // Wake writers waiting for space
        Ok(to_read)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.is_read_end {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not the write end"));
        }
        let (lock, cvar) = &*self.buffer;
        let mut pipe = lock.lock().unwrap();

        if pipe.read_closed {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"));
        }

        // Wait for space
        while pipe.data.len() >= pipe.capacity && !pipe.read_closed {
            let result = cvar.wait_timeout(pipe, std::time::Duration::from_millis(100)).unwrap();
            pipe = result.0;
        }

        if pipe.read_closed {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"));
        }

        let space = pipe.capacity - pipe.data.len();
        let to_write = buf.len().min(space);
        for &byte in &buf[..to_write] {
            pipe.data.push_back(byte);
        }
        cvar.notify_all(); // Wake readers waiting for data
        Ok(to_write)
    }

    fn close(&mut self) {
        let (lock, cvar) = &*self.buffer;
        let mut pipe = lock.lock().unwrap();
        if self.is_read_end {
            pipe.read_closed = true;
        } else {
            pipe.write_closed = true;
        }
        cvar.notify_all();
    }

    fn is_readable(&self) -> bool { self.is_read_end }
    fn is_writable(&self) -> bool { !self.is_read_end }
}

impl Drop for PipeFd {
    fn drop(&mut self) {
        self.close();
    }
}

use std::fs::{File, OpenOptions};
use std::io::{Read as IoRead, Write as IoWrite};
use std::path::Path;

/// Flags for opening a VFS file descriptor.
pub const O_RDONLY: i32 = 0;
pub const O_WRONLY: i32 = 1;
pub const O_RDWR: i32 = 2;
pub const O_CREAT: i32 = 4;
pub const O_TRUNC: i32 = 8;
pub const O_APPEND: i32 = 16;

/// File descriptor backed by the computer's virtual filesystem.
pub struct VfsFileFd {
    file: File,
    readable: bool,
    writable: bool,
}

impl VfsFileFd {
    /// Open a file within the storage directory.
    /// `base_dir` is the computer's storage path.
    /// `path` is the relative path within the VFS.
    /// `flags` is a combination of O_RDONLY, O_WRONLY, O_CREAT, O_TRUNC, O_APPEND.
    pub fn open(base_dir: &Path, path: &str, flags: i32) -> io::Result<Self> {
        // Sanitize path - prevent directory traversal
        let sanitized = path.replace('\\', "/");
        if sanitized.contains("..") || sanitized.starts_with('/') {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "invalid path"));
        }

        let full_path = base_dir.join(&sanitized);

        // Verify it stays within base_dir
        let canonical_base = base_dir.canonicalize().unwrap_or_else(|_| base_dir.to_path_buf());
        // For new files, check the parent
        if let Some(parent) = full_path.parent() {
            let canonical_parent = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
            if !canonical_parent.starts_with(&canonical_base) && canonical_parent != canonical_base {
                // Allow if base_dir itself doesn't exist yet (first write)
                if canonical_base.exists() {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "path escapes storage"));
                }
            }
        }

        let readable = flags & 0x3 != O_WRONLY;
        let writable = flags & 0x3 != O_RDONLY;
        let create = flags & O_CREAT != 0;
        let truncate = flags & O_TRUNC != 0;
        let append = flags & O_APPEND != 0;

        // Create parent directories if needed
        if create || writable {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let file = OpenOptions::new()
            .read(readable)
            .write(writable)
            .create(create || writable)
            .truncate(truncate)
            .append(append)
            .open(&full_path)?;

        Ok(Self { file, readable, writable })
    }
}

impl FileDescriptor for VfsFileFd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.readable {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "not readable"));
        }
        self.file.read(buf)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.writable {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "not writable"));
        }
        self.file.write(buf)
    }

    fn is_readable(&self) -> bool { self.readable }
    fn is_writable(&self) -> bool { self.writable }
}

use std::sync::atomic::{AtomicBool, Ordering};

/// File descriptor connected to a terminal (read end).
/// Read end pulls from an input byte queue.
pub struct TerminalReadFd {
    input_buffer: Arc<Mutex<VecDeque<u8>>>,
    closed: Arc<AtomicBool>,
}

/// File descriptor connected to a terminal (write end).
/// Write end pushes text via a callback (e.g. to a TerminalBuffer).
pub struct TerminalWriteFd {
    output_fn: Arc<Mutex<Box<dyn FnMut(&[u8]) + Send>>>,
}

impl TerminalReadFd {
    /// Create a new terminal read FD with the given input buffer.
    pub fn new(input_buffer: Arc<Mutex<VecDeque<u8>>>) -> Self {
        Self {
            input_buffer,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Push input bytes (from keyboard/remote) into this terminal's input buffer.
    pub fn push_input(buffer: &Arc<Mutex<VecDeque<u8>>>, data: &[u8]) {
        let mut buf = buffer.lock().unwrap();
        buf.extend(data);
    }
}

impl FileDescriptor for TerminalReadFd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut input = self.input_buffer.lock().unwrap();
        if input.is_empty() {
            if self.closed.load(Ordering::Relaxed) {
                return Ok(0); // EOF
            }
            return Ok(0); // No data available (non-blocking for now)
        }
        let to_read = buf.len().min(input.len());
        for i in 0..to_read {
            buf[i] = input.pop_front().unwrap();
        }
        Ok(to_read)
    }

    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "terminal read fd is not writable"))
    }

    fn close(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    fn is_readable(&self) -> bool { true }
    fn is_writable(&self) -> bool { false }
}

impl TerminalWriteFd {
    /// Create a new terminal write FD that calls the given function on write.
    pub fn new(output_fn: Box<dyn FnMut(&[u8]) + Send>) -> Self {
        Self {
            output_fn: Arc::new(Mutex::new(output_fn)),
        }
    }
}

impl FileDescriptor for TerminalWriteFd {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::InvalidInput, "terminal write fd is not readable"))
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut f = self.output_fn.lock().unwrap();
        f(buf);
        Ok(buf.len())
    }

    fn is_readable(&self) -> bool { false }
    fn is_writable(&self) -> bool { true }
}

/// File descriptor backed by a TCP socket (kernel-mediated).
///
/// Data flows:
///   user process -> SocketFd tx_buffer -> kernel TCP stack -> network
///   network -> kernel TCP stack -> SocketFd rx_buffer -> user process
///
/// Currently a placeholder — the kernel's sshd uses the raw TCP stack
/// directly. SocketFd will be used when user WASI processes need network access.
pub struct SocketFd {
    pub rx_buffer: Arc<Mutex<VecDeque<u8>>>,
    pub tx_buffer: Arc<Mutex<VecDeque<u8>>>,
    pub closed: Arc<AtomicBool>,
    readable: bool,
    writable: bool,
}

impl SocketFd {
    /// Create a bidirectional socket FD (both readable and writable).
    pub fn new() -> Self {
        Self {
            rx_buffer: Arc::new(Mutex::new(VecDeque::new())),
            tx_buffer: Arc::new(Mutex::new(VecDeque::new())),
            closed: Arc::new(AtomicBool::new(false)),
            readable: true,
            writable: true,
        }
    }

    /// Create a connected pair of socket FDs.
    /// Data written to one appears in the other's rx_buffer.
    pub fn new_pair() -> (SocketFd, SocketFd) {
        let buf_a = Arc::new(Mutex::new(VecDeque::new()));
        let buf_b = Arc::new(Mutex::new(VecDeque::new()));
        let closed = Arc::new(AtomicBool::new(false));

        let fd_a = SocketFd {
            rx_buffer: buf_a.clone(),
            tx_buffer: buf_b.clone(),
            closed: closed.clone(),
            readable: true,
            writable: true,
        };
        let fd_b = SocketFd {
            rx_buffer: buf_b,
            tx_buffer: buf_a,
            closed,
            readable: true,
            writable: true,
        };
        (fd_a, fd_b)
    }
}

impl FileDescriptor for SocketFd {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.readable {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not readable"));
        }
        let mut rx = self.rx_buffer.lock().unwrap();
        if rx.is_empty() {
            if self.closed.load(Ordering::Relaxed) {
                return Ok(0); // EOF
            }
            return Ok(0); // No data available (non-blocking)
        }
        let n = buf.len().min(rx.len());
        for i in 0..n {
            buf[i] = rx.pop_front().unwrap();
        }
        Ok(n)
    }

    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.writable {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not writable"));
        }
        if self.closed.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "socket closed"));
        }
        let mut tx = self.tx_buffer.lock().unwrap();
        tx.extend(buf);
        Ok(buf.len())
    }

    fn close(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    fn is_readable(&self) -> bool { self.readable }
    fn is_writable(&self) -> bool { self.writable }
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

    #[test]
    fn test_pipe_write_then_read() {
        let (mut read_end, mut write_end) = create_pipe();
        write_end.write(b"hello pipe").unwrap();
        let mut buf = [0u8; 64];
        let n = read_end.read(&mut buf).unwrap();
        assert_eq!(n, 10);
        assert_eq!(&buf[..n], b"hello pipe");
    }

    #[test]
    fn test_pipe_eof_on_close() {
        let (mut read_end, write_end) = create_pipe();
        drop(write_end); // Close write end
        let mut buf = [0u8; 64];
        let n = read_end.read(&mut buf).unwrap();
        assert_eq!(n, 0); // EOF
    }

    #[test]
    fn test_pipe_broken_pipe() {
        let (read_end, mut write_end) = create_pipe();
        drop(read_end); // Close read end
        let result = write_end.write(b"hello");
        assert!(result.is_err());
    }

    #[test]
    fn test_pipe_threaded() {
        use std::thread;
        let (mut read_end, mut write_end) = create_pipe();

        let writer = thread::spawn(move || {
            write_end.write(b"threaded data").unwrap();
            drop(write_end);
        });

        let mut buf = [0u8; 64];
        let n = read_end.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"threaded data");

        // After writer closes, should get EOF
        let n = read_end.read(&mut buf).unwrap();
        assert_eq!(n, 0);

        writer.join().unwrap();
    }

    #[test]
    fn test_vfs_file_write_read() {
        let dir = std::env::temp_dir().join("test_vfs_fd");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Write
        {
            let mut fd = VfsFileFd::open(&dir, "test.txt", O_WRONLY | O_CREAT | O_TRUNC).unwrap();
            fd.write(b"file content").unwrap();
        }

        // Read
        {
            let mut fd = VfsFileFd::open(&dir, "test.txt", O_RDONLY).unwrap();
            let mut buf = [0u8; 64];
            let n = fd.read(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"file content");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_vfs_file_path_traversal_blocked() {
        let dir = std::env::temp_dir().join("test_vfs_traverse");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let result = VfsFileFd::open(&dir, "../escape.txt", O_WRONLY | O_CREAT);
        assert!(result.is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_terminal_read_fd() {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let mut fd = TerminalReadFd::new(buffer.clone());

        // Push input
        TerminalReadFd::push_input(&buffer, b"hello");

        let mut buf = [0u8; 64];
        let n = fd.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"hello");
    }

    #[test]
    fn test_terminal_write_fd() {
        let written = Arc::new(Mutex::new(Vec::<u8>::new()));
        let written_clone = written.clone();
        let mut fd = TerminalWriteFd::new(Box::new(move |data: &[u8]| {
            written_clone.lock().unwrap().extend_from_slice(data);
        }));

        fd.write(b"test output").unwrap();
        assert_eq!(&written.lock().unwrap()[..], b"test output");
    }

    #[test]
    fn test_vfs_file_append() {
        let dir = std::env::temp_dir().join("test_vfs_append");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        {
            let mut fd = VfsFileFd::open(&dir, "append.txt", O_WRONLY | O_CREAT | O_TRUNC).unwrap();
            fd.write(b"first").unwrap();
        }
        {
            let mut fd = VfsFileFd::open(&dir, "append.txt", O_WRONLY | O_APPEND).unwrap();
            fd.write(b" second").unwrap();
        }
        {
            let mut fd = VfsFileFd::open(&dir, "append.txt", O_RDONLY).unwrap();
            let mut buf = [0u8; 64];
            let n = fd.read(&mut buf).unwrap();
            assert_eq!(&buf[..n], b"first second");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
