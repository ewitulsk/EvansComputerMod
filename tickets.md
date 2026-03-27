# OS-to-Kernel Transformation: Jira Tickets

> **Goal**: Transform the Rust OS into a kernel that supports multiprocessing, WASI user programs, pipes/redirection, job control, virtual TTYs, and SSH — all verified through the simulator.
>
> **Testing Strategy**: Every ticket includes simulator-based acceptance tests. The simulator (`simulator/src/`) is the primary development target. Java (`src/main/java/.../wasm/ComputerInstance.java`) gets parallel implementation but simulator comes first.
>
> **Dependency Notation**: `Blocked by: KERN-XX` means that ticket must be complete before this one can start. Tickets without blockers or with different blockers can run in parallel.

---

## Epic 1: Foundation — Linker Migration & Infrastructure

*These tickets establish the plumbing that everything else depends on.*

---

### KERN-001: Add `ProcessManager` Scaffold to Simulator

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: None
**Parallel with**: KERN-002, KERN-003

**Description**:
Create the `ProcessManager` struct in the simulator that will own the process table, PID allocation, and the shared `Engine`. Today, `WasmHost::new()` in `simulator/src/wasm_host.rs:91` creates its own `Engine` per instance. The `ProcessManager` will own a single `Engine` (they're cheap to share and expensive to duplicate) and hand it to each spawned process.

This ticket is scaffolding only — no process spawning yet. Just the data structures and integration point.

**Acceptance Criteria**:
- New file `simulator/src/process.rs` with:
  ```rust
  pub struct ProcessManager {
      engine: Engine,
      processes: HashMap<u32, ProcessEntry>,
      next_pid: AtomicU32,
  }

  pub struct ProcessEntry {
      pub pid: u32,
      pub parent_pid: u32,
      pub name: String,
      pub state: ProcessState,
      pub exit_code: Option<i32>,
  }

  pub enum ProcessState {
      Running,
      Stopped,
      Zombie,
  }
  ```
- `ProcessManager::new()` creates a shared `Engine`
- `ProcessManager::register_kernel(pid=1, name="kernel")` registers PID 1
- `WasmHost::new()` accepts an `&Engine` instead of creating its own (update signature)
- Kernel (existing OS) is registered as PID 1 on startup
- All existing simulator tests still pass (no behavioral change)

**Files to modify**:
- `simulator/src/process.rs` (new)
- `simulator/src/wasm_host.rs` — change `Engine::default()` to accept shared engine
- `simulator/src/main.rs` — create `ProcessManager`, pass engine to `WasmHost`

**Simulator test**:
```bash
# Existing tests must pass unchanged
printf 'echo hello\n' | timeout 10 cargo run --release -- --headless
# Should print "hello" — no regression
```

---

### KERN-002: Implement `FdTable` and `FileDescriptor` Trait

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: None
**Parallel with**: KERN-001, KERN-003

**Description**:
Create the file descriptor abstraction layer. This is the single most important abstraction — pipes, redirection, sockets, and TTYs are all FDs.

Today, terminal I/O goes through `terminal_write`/`terminal_read_line` host functions (simulator: `host/terminal.rs`), and file I/O goes through path-based `file_read`/`file_write` (simulator: `host/filesystem.rs`). These continue to work for the kernel. The FD layer sits alongside them for user processes.

**Acceptance Criteria**:
- New file `simulator/src/fd.rs` with:
  ```rust
  pub trait FileDescriptor: Send {
      fn read(&mut self, buf: &mut [u8]) -> io::Result<usize>;  // 0 = EOF
      fn write(&mut self, buf: &[u8]) -> io::Result<usize>;
      fn close(&mut self) {}
      fn is_readable(&self) -> bool;
      fn is_writable(&self) -> bool;
  }

  pub struct FdTable {
      fds: HashMap<i32, Box<dyn FileDescriptor>>,
      next_fd: i32,  // starts at 3 (0,1,2 = stdio)
  }
  ```
- `FdTable` methods: `allocate(fd) -> i32`, `get(fd)`, `get_mut(fd)`, `close(fd)`, `dup(old) -> new`, `dup2(old, new)`
- `NullFd` implementation (discards writes, returns EOF on read)
- Unit tests for FdTable operations (allocate, close, dup, dup2)

**Files to create**:
- `simulator/src/fd.rs`

**Simulator test**:
```bash
# Unit tests only — no integration yet
cargo test -p terminal-simulator fd
```

---

### KERN-003: Implement `PipeFd`

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-002
**Parallel with**: KERN-001 (after KERN-002)

**Description**:
Implement a pipe backed by a bounded buffer. Pipes are the mechanism for `cmd1 | cmd2`. The pipe has a read end and a write end. Reading from an empty pipe blocks. Writing to a full pipe blocks. Closing the write end causes the read end to return EOF.

**Acceptance Criteria**:
- `PipeFd` struct in `simulator/src/fd.rs` (or new `simulator/src/fd/pipe.rs`):
  ```rust
  pub struct PipeFd {
      buffer: Arc<Mutex<PipeBuffer>>,
      is_read_end: bool,
  }

  struct PipeBuffer {
      data: VecDeque<u8>,
      capacity: usize,         // default 4096
      write_closed: bool,
      read_closed: bool,
  }
  ```
- `PipeFd::create_pair() -> (PipeFd, PipeFd)` — returns (read_end, write_end)
- Read blocks (via Condvar) when buffer empty, returns 0 (EOF) when write end closed and buffer empty
- Write blocks when buffer full, returns error when read end closed (broken pipe)
- Thread-safe (read end and write end used from different threads)
- Unit tests: write then read, EOF on close, blocking behavior, broken pipe

**Files to modify**:
- `simulator/src/fd.rs`

**Simulator test**:
```bash
cargo test -p terminal-simulator pipe
```

---

### KERN-004: Implement `VfsFileFd`

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-002
**Parallel with**: KERN-003

**Description**:
Implement a file descriptor backed by the computer's virtual filesystem. Used for `< file` and `> file` redirection. Wraps the existing `FileSystem` struct from `simulator/src/filesystem.rs`.

**Acceptance Criteria**:
- `VfsFileFd` struct:
  ```rust
  pub struct VfsFileFd {
      file: std::fs::File,  // opened host file within computerStoragePath
      readable: bool,
      writable: bool,
  }
  ```
- `VfsFileFd::open(path, flags)` where flags support: read-only, write-only (truncate), write-only (append)
- Uses `FileSystem::resolve_path()` for path sanitization (reuse existing logic from `simulator/src/filesystem.rs:24-45`)
- Read returns bytes read, 0 on EOF
- Write returns bytes written
- Unit tests

**Files to modify**:
- `simulator/src/fd.rs`
- `simulator/src/filesystem.rs` — extract `resolve_path()` as a public function if not already

**Simulator test**:
```bash
cargo test -p terminal-simulator vfs_file_fd
```

---

### KERN-005: Implement `TerminalFd`

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-002
**Parallel with**: KERN-003, KERN-004

**Description**:
Implement a file descriptor that reads from the terminal input queue and writes to the `TerminalBuffer`. This is what PID 1 (kernel) and foreground user processes without redirection will use for stdio.

Today, `terminal_read_line` in `host/terminal.rs` blocks on an mpsc channel for input. `TerminalFd` wraps similar logic — it reads raw bytes from an input channel and writes to the `TerminalBuffer`.

**Acceptance Criteria**:
- `TerminalFd` struct:
  ```rust
  pub struct TerminalFd {
      input_rx: Arc<ChannelReceiver>,  // for read
      terminal: Arc<Mutex<TerminalBuffer>>,  // for write
      is_read: bool,   // stdin instance
      is_write: bool,  // stdout/stderr instance
  }
  ```
- Read: pulls bytes from input channel (blocks with timeout)
- Write: calls `terminal.write()` on the shared `TerminalBuffer`
- Can create separate read-only and write-only instances sharing same underlying resources
- Unit tests

**Files to modify**:
- `simulator/src/fd.rs`

---

### KERN-006: Register FD Host Functions for the Kernel

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-002, KERN-003, KERN-004
**Parallel with**: KERN-005

**Description**:
Add new host functions in the `"env"` namespace that let the kernel (Rust OS) create and manipulate file descriptors. These are the syscalls the kernel uses to set up pipes and redirects before spawning user processes.

The kernel itself continues using `terminal_write`/`file_read` etc. for its own I/O. These FD functions are for orchestrating child process I/O.

**Acceptance Criteria**:
- New file `simulator/src/host/fd_ops.rs` following existing pattern (see `host/sleep.rs` for template)
- Host functions registered in `"env"` namespace:
  | Function | Signature | Description |
  |----------|-----------|-------------|
  | `fd_open` | `(path_ptr, path_len, flags) -> i32` | Open VFS file as FD. Flags: O_RDONLY=0, O_WRONLY=1, O_RDWR=2, O_CREAT=4, O_TRUNC=8, O_APPEND=16. Returns fd or -1 |
  | `fd_read` | `(fd, buf_ptr, buf_len) -> i32` | Read from FD. Returns bytes read, 0=EOF, -1=error |
  | `fd_write` | `(fd, buf_ptr, buf_len) -> i32` | Write to FD. Returns bytes written, -1=error |
  | `fd_close` | `(fd) -> i32` | Close FD. 0=ok, -1=error |
  | `fd_dup` | `(old_fd) -> i32` | Duplicate FD, returns new fd |
  | `fd_dup2` | `(old_fd, new_fd) -> i32` | Dup to specific number |
  | `pipe_create` | `(read_fd_ptr, write_fd_ptr) -> i32` | Create pipe, writes two i32 FDs to WASM memory. 0=ok |
- FdTable stored in `HostState.custom` via `insert_custom::<FdTable>()`
- Wired into `host/mod.rs` `known_names()` + `register_all()`

**Rust OS declarations** (new extern functions in `operating-system/rust/src/lib.rs`):
```rust
extern "C" {
    fn fd_open(path_ptr: *const u8, path_len: usize, flags: i32) -> i32;
    fn fd_read(fd: i32, buf_ptr: *mut u8, buf_len: usize) -> i32;
    fn fd_write(fd: i32, buf_ptr: *const u8, buf_len: usize) -> i32;
    fn fd_close(fd: i32) -> i32;
    fn fd_dup(old_fd: i32) -> i32;
    fn fd_dup2(old_fd: i32, new_fd: i32) -> i32;
    fn pipe_create(read_fd_ptr: *mut i32, write_fd_ptr: *mut i32) -> i32;
}
```

**Files to create/modify**:
- `simulator/src/host/fd_ops.rs` (new)
- `simulator/src/host/mod.rs` — add module + register
- `operating-system/rust/src/lib.rs` — add extern declarations

**Simulator test**:
```bash
# Test pipe_create + fd_write + fd_read round-trip via headless commands
# (Requires kernel-side test command — see KERN-007)
printf 'fd_test\n' | timeout 10 cargo run --release -- --headless
# Should print "FD test: PASS"
```

---

### KERN-007: Add `fd_test` Kernel Built-in Command

**Type**: Feature
**Priority**: P1 — Testing
**Blocked by**: KERN-006

**Description**:
Add a `fd_test` shell command to the Rust OS kernel that exercises the FD host functions. This is a diagnostic tool that validates the FD layer works end-to-end through WASM.

**Acceptance Criteria**:
- New command `fd_test` in `process_command()` match arm (`lib.rs:492`)
- Tests:
  1. `pipe_create()` → gets read_fd and write_fd
  2. `fd_write(write_fd, "hello pipe")` → returns 10
  3. `fd_close(write_fd)`
  4. `fd_read(read_fd, buf, 256)` → returns 10, buf = "hello pipe"
  5. `fd_read(read_fd, buf, 256)` → returns 0 (EOF after close)
  6. `fd_close(read_fd)`
  7. `fd_open("test.txt", O_WRONLY | O_CREAT | O_TRUNC)` → returns fd
  8. `fd_write(fd, "file content")` → returns 12
  9. `fd_close(fd)`
  10. `fd_open("test.txt", O_RDONLY)` → returns fd
  11. `fd_read(fd, buf, 256)` → returns 12, buf = "file content"
  12. `fd_close(fd)`
- Prints "FD test: PASS" if all assertions hold, "FD test: FAIL at step N" otherwise

**Files to modify**:
- `operating-system/rust/src/lib.rs` — add command + extern declarations

**Simulator test**:
```bash
printf 'fd_test\n' | timeout 10 cargo run --release -- --headless | grep "FD test: PASS"
```

---

## Epic 2: Process Spawning — Loading and Running WASI Binaries

*Depends on: Epic 1 (FdTable + FD host functions). These tickets enable spawning external .wasm files as child processes.*

---

### KERN-008: Implement WASI Tier 1 Host Functions

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-002
**Parallel with**: KERN-006, KERN-009

**Description**:
Implement the minimal set of WASI `wasi_snapshot_preview1` functions as custom host functions in the simulator. These are required for any WASI binary to start (even a simple `println!("hello")`).

**Critical constraint**: wasmtime 29's built-in WASI support exists but we need custom stream backing for pipes. Implement these manually to maintain full control over FD routing.

WASI functions use a different calling convention than your kernel host functions — they return `errno` codes (0 = success) and write results through pointer parameters.

**Acceptance Criteria**:
- New file `simulator/src/wasi.rs` with WASI errno constants and helpers
- New file `simulator/src/host/wasi_io.rs`:
  | WASI Function | What it does |
  |---------------|-------------|
  | `fd_write(fd, iovs_ptr, iovs_len, nwritten_ptr) -> errno` | Scatter-gather write. Each iov is (buf_ptr: i32, buf_len: i32). Sums writes, stores total in nwritten_ptr |
  | `fd_read(fd, iovs_ptr, iovs_len, nread_ptr) -> errno` | Scatter-gather read |
  | `fd_close(fd) -> errno` | Close FD |
  | `fd_seek(fd, offset_i64, whence, newoffset_ptr) -> errno` | Seek (for file FDs, return EBADF for pipes/terminal) |
  | `fd_fdstat_get(fd, buf_ptr) -> errno` | Return FD type (filetype + flags). Required by Rust's libstd |
  | `fd_prestat_get(fd, buf_ptr) -> errno` | Return preopened dir info. FD 3 = "/" (root). Return EBADF for fd > 3 |
  | `fd_prestat_dir_name(fd, path_ptr, path_len) -> errno` | Write "/" to memory for fd 3 |
  | `proc_exit(code)` | Terminate the calling WASM instance (trap) |
- All functions registered in `"wasi_snapshot_preview1"` namespace (NOT `"env"`)
- WASI errno codes: ESUCCESS=0, EBADF=8, EINVAL=28, ENOSYS=52, ENOENT=44

**Key implementation detail**: The WASI functions read from a per-process `FdTable` stored in the `HostState.custom`. For the kernel process (PID 1), WASI functions are never called (kernel uses `"env"` namespace). For user processes, the `FdTable` is populated during `process_spawn`.

**Files to create**:
- `simulator/src/wasi.rs` — constants, errno, helper types
- `simulator/src/host/wasi_io.rs` — WASI I/O functions
- `simulator/src/host/mod.rs` — add module

**Simulator test**: Tested via KERN-013 (first WASI binary execution)

---

### KERN-009: Implement WASI Args + Environment Functions

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-008

**Description**:
Implement WASI functions for command-line arguments and environment variables. Without these, most WASI programs will abort on startup when they try to read argv.

**Acceptance Criteria**:
- Add to `simulator/src/host/wasi_io.rs` (or separate `wasi_args.rs`):
  | WASI Function | What it does |
  |---------------|-------------|
  | `args_sizes_get(argc_ptr, argv_buf_size_ptr) -> errno` | Write arg count and total buffer size |
  | `args_get(argv_ptr, argv_buf_ptr) -> errno` | Write null-terminated arg strings + pointer array |
  | `environ_sizes_get(count_ptr, buf_size_ptr) -> errno` | Write env var count and buffer size |
  | `environ_get(environ_ptr, environ_buf_ptr) -> errno` | Write KEY=VALUE strings + pointer array |
  | `clock_time_get(clock_id, precision, time_ptr) -> errno` | Return current time as u64 nanoseconds |
  | `random_get(buf_ptr, buf_len) -> errno` | Fill buffer with random bytes |
- Args and env stored in `HostState.custom` as `WasiArgs { argv: Vec<String>, env: Vec<(String, String)> }`
- Default env includes `HOME=/`, `TERM=dumb`, `PATH=/bin`

**Files to modify**:
- `simulator/src/host/wasi_io.rs` or new `simulator/src/host/wasi_args.rs`
- `simulator/src/host/mod.rs`

---

### KERN-010: Implement WASI Filesystem Functions

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-008
**Parallel with**: KERN-009

**Description**:
Implement WASI filesystem functions so user programs can open/read/write/list files. These wrap the existing `FileSystem` struct.

**Acceptance Criteria**:
- New file `simulator/src/host/wasi_fs.rs`:
  | WASI Function | What it does |
  |---------------|-------------|
  | `path_open(dirfd, dirflags, path_ptr, path_len, oflags, fs_rights_base, fs_rights_inheriting, fdflags, fd_ptr) -> errno` | Open file relative to preopened dir (fd 3). Returns new FD |
  | `path_create_directory(dirfd, path_ptr, path_len) -> errno` | mkdir |
  | `path_remove_directory(dirfd, path_ptr, path_len) -> errno` | rmdir |
  | `path_unlink_file(dirfd, path_ptr, path_len) -> errno` | delete file |
  | `path_filestat_get(dirfd, flags, path_ptr, path_len, buf_ptr) -> errno` | stat file (size, type) |
  | `fd_readdir(fd, buf_ptr, buf_len, cookie, bufused_ptr) -> errno` | List directory entries |
- Map WASI oflags to VfsFileFd open modes:
  - `O_CREAT` (bit 0) → create if not exists
  - `O_DIRECTORY` (bit 1) → must be dir
  - `O_EXCL` (bit 2) → fail if exists
  - `O_TRUNC` (bit 3) → truncate
- Path resolution: `dirfd=3` maps to computer root `/`, combine with path_ptr string

**Files to create**:
- `simulator/src/host/wasi_fs.rs`
- `simulator/src/host/mod.rs` — wire in

---

### KERN-011: WASI Stub Functions (Tier 3 — Return ENOSYS)

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-008
**Parallel with**: KERN-009, KERN-010

**Description**:
Stub out all remaining WASI functions with `ENOSYS` (errno 52). Well-behaved WASI programs check the return code and handle gracefully. Without these stubs, wasmtime will fail to link any WASI binary that imports them.

**Acceptance Criteria**:
- New file `simulator/src/host/wasi_stubs.rs`
- Register all unstubbed `wasi_snapshot_preview1` functions returning errno 52:
  - `poll_oneoff`, `sock_accept`, `sock_recv`, `sock_send`, `sock_shutdown`
  - `path_link`, `path_symlink`, `path_readlink`, `path_rename`
  - `fd_advise`, `fd_allocate`, `fd_datasync`, `fd_sync`, `fd_renumber`
  - `fd_pread`, `fd_pwrite`, `fd_filestat_get`, `fd_filestat_set_size`, `fd_filestat_set_times`
  - `path_filestat_set_times`
  - `fd_fdstat_set_flags`
  - `sched_yield` (return 0 — this one's harmless)
- Use a macro or loop to reduce boilerplate
- Register all in `"wasi_snapshot_preview1"` namespace

**Files to create**:
- `simulator/src/host/wasi_stubs.rs`
- `simulator/src/host/mod.rs` — wire in

---

### KERN-012: Process Spawn Infrastructure in Simulator

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-001, KERN-008, KERN-009, KERN-010, KERN-011

**Description**:
Implement `ProcessManager::spawn()` — the core function that loads a `.wasm` file from the virtual filesystem, creates a new wasmtime `Store` + `Instance` with WASI host functions, wires up the FD table, and runs the process on a new thread.

This is the most complex ticket in the project. It ties together the Engine, Linker, WASI functions, FdTable, and thread management.

**Acceptance Criteria**:
- `ProcessManager::spawn()` method:
  ```rust
  pub fn spawn(
      &mut self,
      wasm_bytes: &[u8],       // .wasm file contents from VFS
      name: String,             // process name for ps
      argv: Vec<String>,        // command-line args
      env: Vec<(String, String)>, // environment variables
      stdin: Box<dyn FileDescriptor>,
      stdout: Box<dyn FileDescriptor>,
      stderr: Box<dyn FileDescriptor>,
      filesystem: FileSystem,   // shared filesystem access
  ) -> Result<u32>  // returns PID
  ```
- Implementation:
  1. Allocate PID (atomic increment)
  2. Create new `Store<HostState>` with the shared engine
  3. Populate `HostState` with: FdTable (fd 0=stdin, 1=stdout, 2=stderr, 3=preopened root), WasiArgs, InterruptQueue, filesystem, shutdown flag
  4. Create new `Linker`, register WASI functions (NOT kernel `"env"` functions)
  5. Load WASM module from `wasm_bytes`
  6. Instantiate via `linker.instantiate(&mut store, &module)`
  7. Spawn thread that calls `_start()` export
  8. On `_start()` return or trap, set `ProcessEntry.exit_code` and state to `Zombie`
  9. Store `ProcessEntry` in process table
- Thread-safety: process table access via `Arc<Mutex<...>>`
- Process cleanup: join handle stored, zombie reaped on `wait()`

**Files to modify**:
- `simulator/src/process.rs` — implement `spawn()`, thread management
- `simulator/src/wasm_host.rs` — may need to expose linker setup as reusable function

**Simulator test**: Tested via KERN-013

---

### KERN-013: Process Host Functions for the Kernel

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-012

**Description**:
Add host functions in the `"env"` namespace that let the kernel spawn, wait for, and manage child processes. These are the syscalls the Rust OS shell calls to execute user programs.

**Acceptance Criteria**:
- New file `simulator/src/host/process.rs`:
  | Function | Signature | Description |
  |----------|-----------|-------------|
  | `process_spawn` | `(path_ptr, path_len, argv_ptr, argv_len, stdin_fd, stdout_fd, stderr_fd) -> i32` | Read .wasm from VFS, call ProcessManager::spawn(). argv is newline-delimited string. Returns PID or -1 |
  | `process_wait` | `(pid) -> i32` | Block until process exits, return exit code. -1 if no such PID |
  | `process_wait_any` | `(status_ptr) -> i32` | Block until any child exits. Write exit code to ptr, return PID |
  | `process_kill` | `(pid, signal) -> i32` | Send signal (currently just terminate). 0=ok, -1=error |
  | `process_list` | `(buf_ptr, buf_len) -> i32` | Write JSON process list to buffer, return bytes written |
  | `process_state` | `(pid) -> i32` | 0=running, 1=stopped, 2=zombie, -1=not found |
  | `process_exit` | `(code)` | Current process exits with code (trap) |
- ProcessManager stored in `HostState.custom` via `Arc<Mutex<ProcessManager>>`
- `process_spawn` reads wasm bytes from filesystem, passes to `ProcessManager::spawn()`
- `process_wait` uses condvar to block until child exits (NOT busy-wait)
- argv serialization: kernel writes `"arg0\narg1\narg2"` to WASM memory, host splits on `\n`

**Rust OS declarations** (new extern functions in `operating-system/rust/src/lib.rs`):
```rust
extern "C" {
    fn process_spawn(path_ptr: *const u8, path_len: usize,
                     argv_ptr: *const u8, argv_len: usize,
                     stdin_fd: i32, stdout_fd: i32, stderr_fd: i32) -> i32;
    fn process_wait(pid: i32) -> i32;
    fn process_wait_any(status_ptr: *mut i32) -> i32;
    fn process_kill(pid: i32, signal: i32) -> i32;
    fn process_list(buf_ptr: *mut u8, buf_len: usize) -> i32;
    fn process_state(pid: i32) -> i32;
    fn process_exit(code: i32);
}
```

**Files to create/modify**:
- `simulator/src/host/process.rs` (new)
- `simulator/src/host/mod.rs` — wire in
- `operating-system/rust/src/lib.rs` — add extern declarations

**Simulator test**: Tested via KERN-014

---

### KERN-014: Build and Execute First WASI Binary ("hello.wasm")

**Type**: Feature + Test
**Priority**: P0 — Critical Path
**Blocked by**: KERN-013

**Description**:
Create a minimal WASI test program, add an `exec` shell command to the kernel that loads and runs `.wasm` files, and verify the complete spawn→execute→wait pipeline works.

This is the first end-to-end milestone: **drop a .wasm file into the filesystem and run it from the shell**.

**Acceptance Criteria**:
- New directory `wasm-programs/hello/` with:
  ```rust
  // src/main.rs
  fn main() {
      println!("Hello from WASI!");
      let args: Vec<String> = std::env::args().collect();
      if args.len() > 1 {
          println!("Args: {:?}", &args[1..]);
      }
  }
  ```
  - `Cargo.toml` targeting `wasm32-wasip1`
  - Build script or Makefile that compiles to `hello.wasm`
- Shell command `exec <path>` or automatic `.wasm` detection in `process_command()`:
  - If command ends in `.wasm` or a file `<command>.wasm` exists, spawn it as WASI process
  - Stdin = terminal, stdout = terminal, stderr = terminal (foreground execution)
  - Wait for process to exit, print exit code if non-zero
- Build infrastructure: `scripts/build-wasm-programs.sh` that compiles all programs in `wasm-programs/`
- Pre-built `hello.wasm` checked into `wasm-bin/` for CI

**Files to create**:
- `wasm-programs/hello/Cargo.toml`
- `wasm-programs/hello/src/main.rs`
- `scripts/build-wasm-programs.sh`
- `operating-system/rust/src/lib.rs` — modify `process_command()` to detect .wasm

**Simulator test**:
```bash
# Copy hello.wasm into simulator storage, then run it
mkdir -p simulator-data/bin
cp wasm-bin/hello.wasm simulator-data/bin/
printf 'exec bin/hello.wasm\n' | timeout 15 cargo run --release -- --headless | grep "Hello from WASI"
```

---

### KERN-015: `ps` and `kill` Shell Built-in Commands

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-013
**Parallel with**: KERN-014

**Description**:
Add `ps` and `kill` commands to the kernel shell for process visibility and management.

**Acceptance Criteria**:
- `ps` command:
  - Calls `process_list()` host function
  - Parses JSON response
  - Prints formatted table:
    ```
    PID  STATE    NAME
      1  running  kernel
      2  running  httpd
      3  zombie   cat
    ```
- `kill <pid>` command:
  - Calls `process_kill(pid, SIGTERM)`
  - Prints confirmation or error
- `kill -9 <pid>` — force kill variant

**Files to modify**:
- `operating-system/rust/src/lib.rs` — add `ps` and `kill` to `process_command()`

**Simulator test**:
```bash
# Run hello.wasm in background (requires KERN-020), then ps
# For now, just test ps shows kernel
printf 'ps\n' | timeout 10 cargo run --release -- --headless | grep "kernel"
```

---

## Epic 3: Pipes & Redirection — Shell Pipeline Engine

*Depends on: Epic 1 (FDs + pipes) + Epic 2 (process spawning). These tickets add the `|`, `>`, `<` operators.*

---

### KERN-016: Shell Pipeline Parser

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-006 (FD host functions exist)
**Parallel with**: KERN-012 (process spawning)

**Description**:
Rewrite the shell's command parser in `lib.rs` to handle pipes, redirects, and backgrounding. Today, `parse_command()` at line 710 just splits on the first space. It needs to produce a structured `Pipeline`.

**Acceptance Criteria**:
- New file `operating-system/rust/src/shell.rs` (or module within lib.rs):
  ```rust
  pub struct PipelineStage {
      pub command: String,
      pub args: Vec<String>,
      pub stdin_redirect: Option<Redirect>,
      pub stdout_redirect: Option<Redirect>,
      pub stderr_redirect: Option<Redirect>,
  }

  pub enum Redirect {
      File { path: String, append: bool },
      MergeWith(i32),  // e.g., 2>&1
  }

  pub struct Pipeline {
      pub stages: Vec<PipelineStage>,
      pub background: bool,  // trailing &
  }
  ```
- Parser handles:
  - `cmd arg1 arg2` — basic command
  - `cmd > file` — stdout to file (truncate)
  - `cmd >> file` — stdout to file (append)
  - `cmd < file` — stdin from file
  - `cmd 2> file` — stderr to file
  - `cmd 2>&1` — stderr to stdout
  - `cmd1 | cmd2` — pipe stdout→stdin
  - `cmd1 | cmd2 | cmd3` — multi-stage pipeline
  - `cmd &` — background execution
  - Quoted strings: `echo "hello world"` treats "hello world" as one arg
- Existing `tokenize_args()` function (lib.rs:2198) can be adapted for quote handling
- Unit tests for parser (parse various command lines, verify Pipeline structure)

**Files to create/modify**:
- `operating-system/rust/src/shell.rs` (new)
- `operating-system/rust/src/lib.rs` — use new parser in `process_command()`

**Simulator test**:
```bash
# Parser is internal — tested via integration in KERN-018
cargo test -p terminal-os shell_parse  # if unit tests added
```

---

### KERN-017: Shell Pipeline Execution Engine

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-006, KERN-013, KERN-016

**Description**:
Implement the pipeline executor that takes a parsed `Pipeline` and orchestrates FD creation, process spawning, and waiting.

**Acceptance Criteria**:
- New function `execute_pipeline(pipeline: &Pipeline)` in `shell.rs`:
  1. For single-stage pipelines with builtins (ls, cd, echo, etc.): execute in-process as today
  2. For single-stage .wasm commands: create FDs per redirects, spawn process, wait
  3. For multi-stage pipelines:
     a. Create N-1 pipes for N stages
     b. For each stage, determine stdin/stdout/stderr FDs:
        - First stage: stdin = terminal (or redirect), stdout = pipe write end
        - Middle stages: stdin = pipe read end, stdout = next pipe write end
        - Last stage: stdout = terminal (or redirect)
     c. Spawn all stages as processes
     d. Close pipe ends the kernel doesn't need
     e. Wait for all processes (or just the last one)
  4. For background pipelines (`&`): skip the wait, print `[job_id] pid`
- Built-in commands in pipelines:
  - Some builtins (`echo`, `cat`) should be able to participate in pipes
  - For now, builtins write to terminal — only .wasm commands support piping
  - Long term: builtins could write to an FD too (deferred to future ticket)

**Files to modify**:
- `operating-system/rust/src/shell.rs` — add `execute_pipeline()`
- `operating-system/rust/src/lib.rs` — replace old dispatch with pipeline executor

**Simulator test**: Tested via KERN-018

---

### KERN-018: Build WASI Utility Programs (cat, grep, wc, tee, head, uppercase)

**Type**: Feature
**Priority**: P0 — Critical Path
**Blocked by**: KERN-014 (first WASI binary works)
**Parallel with**: KERN-016, KERN-017

**Description**:
Build a set of standard Unix-like utilities as WASI programs. These are the building blocks for testing pipes and redirection.

**Acceptance Criteria**:
- `wasm-programs/cat/` — concatenate files or stdin to stdout
  ```rust
  fn main() {
      let args: Vec<String> = std::env::args().collect();
      if args.len() > 1 {
          for file in &args[1..] {
              let content = std::fs::read_to_string(file).unwrap();
              print!("{}", content);
          }
      } else {
          // Read stdin, write to stdout
          std::io::copy(&mut std::io::stdin(), &mut std::io::stdout()).unwrap();
      }
  }
  ```
- `wasm-programs/grep/` — filter lines matching a pattern (simple substring match)
- `wasm-programs/wc/` — count lines, words, bytes
- `wasm-programs/tee/` — copy stdin to both stdout and a file
- `wasm-programs/head/` — first N lines of stdin
- `wasm-programs/uppercase/` — uppercase all input (demo/test program)
- All compile to `wasm32-wasip1`, pre-built binaries in `wasm-bin/`
- Build script updated

**Files to create**:
- `wasm-programs/{cat,grep,wc,tee,head,uppercase}/Cargo.toml + src/main.rs`
- `scripts/build-wasm-programs.sh` — updated

---

### KERN-019: End-to-End Pipeline Integration Test

**Type**: Test
**Priority**: P0 — Milestone
**Blocked by**: KERN-017, KERN-018

**Description**:
Comprehensive integration test verifying pipes and redirection work through the simulator.

**Acceptance Criteria**:
- New test script `scripts/test-processes.sh` following the pattern of `scripts/test-networking.sh`
- Test cases:
  1. `hello.wasm` runs and prints output
  2. `echo "test data" > test.txt` then `cat.wasm test.txt` prints "test data"
  3. `cat.wasm test.txt | grep.wasm "data"` prints "test data"
  4. `cat.wasm test.txt | uppercase.wasm` prints "TEST DATA"
  5. `cat.wasm test.txt | grep.wasm "data" | wc.wasm -l` prints "1"
  6. `cat.wasm test.txt | tee.wasm copy.txt | uppercase.wasm` — both prints uppercase AND creates copy.txt
  7. `cat.wasm < input.txt > output.txt` — redirect both stdin and stdout
  8. `ps` shows running/zombie processes
  9. Long-running process can be killed with `kill`

**Files to create**:
- `scripts/test-processes.sh`

**Simulator test**: This IS the test.

---

## Epic 4: Job Control — Background Processes

*Depends on: Epic 2 (process spawning) + Epic 3 (pipeline parser). These tickets add &, jobs, fg, bg.*

---

### KERN-020: Background Execution (& Operator)

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-017

**Description**:
When a command line ends with `&`, the shell should spawn the process(es) but not wait for them. Instead, print a job number and PID, and return to the prompt immediately.

**Acceptance Criteria**:
- Pipeline parser already handles `&` (KERN-016)
- When `pipeline.background == true`:
  - Spawn process(es) as normal
  - Don't call `process_wait()`
  - Print `[1] 42` (job number, PID of last process in pipeline)
  - Store job in a job table (new `JobTable` struct in `shell.rs`)
- When a background job exits, print `[1]+ Done  command` on next prompt
  - Check via `process_state()` in the shell prompt loop

**Files to modify**:
- `operating-system/rust/src/shell.rs` — job table, background execution
- `operating-system/rust/src/lib.rs` — check jobs on prompt display

**Simulator test**:
```bash
# Run hello.wasm in background
printf 'hello.wasm &\nps\n' | timeout 15 cargo run --release -- --headless
# ps should show hello.wasm
```

---

### KERN-021: `jobs`, `fg`, `bg` Shell Built-in Commands

**Type**: Feature
**Priority**: P2
**Blocked by**: KERN-020

**Description**:
Add job control builtins matching bash behavior.

**Acceptance Criteria**:
- `jobs`:
  - Lists all background jobs with status
  - Format: `[1]+ Running  httpd 8080`
- `fg %1` or `fg <pid>`:
  - Brings job to foreground
  - Waits for it to exit
  - Re-routes terminal I/O to the process
- `bg %1`:
  - Resumes a stopped job in the background
  - (Stopped jobs require Ctrl+Z — deferred to KERN-022)

**Files to modify**:
- `operating-system/rust/src/shell.rs`
- `operating-system/rust/src/lib.rs`

**Simulator test**:
```bash
printf 'hello.wasm &\njobs\n' | timeout 15 cargo run --release -- --headless | grep "Running"
```

---

### KERN-022: Ctrl+Z (SIGTSTP) — Stop Foreground Process

**Type**: Feature
**Priority**: P2
**Blocked by**: KERN-021

**Description**:
Add Ctrl+Z support to stop (suspend) the foreground process and return to the shell prompt.

**Acceptance Criteria**:
- New IRQ: `IRQ_SIGTSTP = 4` (or reuse keyboard IRQ with special payload)
- Ctrl+Z in simulator: sends IRQ_SIGTSTP to kernel
- Kernel handler:
  - Calls `process_kill(foreground_pid, SIGTSTP)`
  - Java/simulator side: sets process state to `Stopped`, suspends worker thread
  - Shell prints `[1]+ Stopped  command`
  - Returns to prompt
- `fg %1` resumes: calls `process_kill(pid, SIGCONT)` → resumes thread

**Files to modify**:
- `simulator/src/main.rs` — Ctrl+Z key handler
- `simulator/src/process.rs` — stop/continue logic
- `simulator/src/host/process.rs` — SIGTSTP/SIGCONT handling
- `operating-system/rust/src/lib.rs` — IRQ handler
- `operating-system/rust/src/interrupt.rs` — new IRQ constant

---

### KERN-023: Convert `httpd` to Background-Capable Daemon

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-020

**Description**:
Today, `httpd 8080` in `lib.rs:1866` blocks the entire shell in a loop. Convert it so `httpd 8080 &` works — the HTTP server runs in the background while the shell remains interactive.

Two approaches:
1. **Keep httpd as kernel builtin** but use the cooperative interrupt system to yield periodically (easier, but httpd still blocks the kernel's single thread during processing)
2. **Move httpd to a WASI binary** that uses socket host functions (requires KERN-045+)

For now, approach 1: make httpd check for a "stop" signal and yield to the shell between connections.

**Acceptance Criteria**:
- `httpd 8080 &` starts the server and returns to shell
- `httpd` registers itself as a background task in the kernel's cooperative scheduler
- The kernel's main loop periodically gives httpd CPU time to poll for TCP connections
- `kill <httpd_pid>` stops the server
- `ps` shows httpd as running

**Files to modify**:
- `operating-system/rust/src/lib.rs` — refactor httpd loop

**Simulator test**:
```bash
# Start httpd in background on instance 1, curl from instance 0
# Reuse existing networking test pattern
printf 'curl http://10.0.0.2:8080/\n' | timeout 20 cargo run --release -- \
    --headless --instances 2 --auto-net --script '1:/tmp/test-httpd-bg.sh'
# Where test-httpd-bg.sh contains: "httpd 8080 &\nps"
```

---

### KERN-024: IRQ_SIGCHLD — Child Exit Notification

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-012
**Parallel with**: KERN-020

**Description**:
When a child process exits, the simulator should deliver an interrupt to the kernel so it can update the job table and print "[N] Done" without polling.

**Acceptance Criteria**:
- New IRQ constant: `IRQ_SIGCHLD = 4`
- When `ProcessEntry` transitions to Zombie state, push IRQ_SIGCHLD to kernel's interrupt queue
- Payload: `{"pid": 42, "exit_code": 0}`
- Kernel handler in `interrupt.rs` updates job table
- Shell prints done notification on next prompt

**Files to modify**:
- `simulator/src/process.rs` — fire IRQ on exit
- `simulator/src/interrupts.rs` — add IRQ_SIGCHLD constant
- `operating-system/rust/src/interrupt.rs` — add IRQ_SIGCHLD
- `operating-system/rust/src/shell.rs` — handler

---

## Epic 5: Virtual TTY Layer

*Depends on: Epic 2 (process spawning). Required for SSH (multiple terminals per computer).*

---

### KERN-025: `VirtualTty` Struct and TTY Registry

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-002 (FdTable)

**Description**:
Create a virtual TTY abstraction that decouples terminal I/O from the physical Minecraft terminal. Each TTY has its own input queue and output buffer. The physical terminal is just one "display" that can be attached to any TTY.

**Acceptance Criteria**:
- New file `simulator/src/tty.rs`:
  ```rust
  pub struct VirtualTty {
      id: u32,
      input_queue: Arc<Mutex<VecDeque<u8>>>,
      output_buffer: Arc<Mutex<TerminalBuffer>>,
      size: (u16, u16),  // width, height
  }

  pub struct TtyRegistry {
      ttys: HashMap<u32, VirtualTty>,
      next_id: u32,
      foreground_tty: u32,  // which TTY is shown on physical display
  }
  ```
- `VirtualTty::new(width, height) -> VirtualTty`
- `TtyRegistry::create() -> u32` — allocate new TTY
- `TtyRegistry::set_foreground(id)` — connect TTY to physical display
- `VirtualTty::to_fd_read() -> TtyReadFd` — creates an FD that reads from input queue
- `VirtualTty::to_fd_write() -> TtyWriteFd` — creates an FD that writes to output buffer
- TTY 0 = physical terminal (kernel's existing terminal)

**Files to create**:
- `simulator/src/tty.rs`

---

### KERN-026: TTY Host Functions

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-025

**Description**:
Add host functions for the kernel to manage TTYs.

**Acceptance Criteria**:
- New file `simulator/src/host/tty.rs`:
  | Function | Signature | Description |
  |----------|-----------|-------------|
  | `tty_create` | `(width, height) -> i32` | Allocate new TTY, returns tty_id or -1 |
  | `tty_attach_fd` | `(tty_id, mode) -> i32` | Get FD connected to TTY. mode: 0=read, 1=write. Returns fd |
  | `tty_set_foreground` | `(tty_id) -> i32` | Connect TTY to physical display. 0=ok |
  | `tty_get_size` | `(tty_id, width_ptr, height_ptr) -> i32` | Get dimensions |
  | `tty_write_input` | `(tty_id, buf_ptr, buf_len) -> i32` | Push bytes to TTY's input queue (for SSH to inject remote input) |
- Registered in `"env"` namespace

**Rust OS extern declarations**:
```rust
extern "C" {
    fn tty_create(width: i32, height: i32) -> i32;
    fn tty_attach_fd(tty_id: i32, mode: i32) -> i32;
    fn tty_set_foreground(tty_id: i32) -> i32;
    fn tty_get_size(tty_id: i32, width_ptr: *mut i32, height_ptr: *mut i32) -> i32;
    fn tty_write_input(tty_id: i32, buf_ptr: *const u8, buf_len: usize) -> i32;
}
```

**Files to create/modify**:
- `simulator/src/host/tty.rs` (new)
- `simulator/src/host/mod.rs`
- `operating-system/rust/src/lib.rs` — extern declarations

---

### KERN-027: Wire Physical Terminal Through TTY 0

**Type**: Refactor
**Priority**: P1
**Blocked by**: KERN-025, KERN-026

**Description**:
Refactor the simulator so the existing physical terminal is backed by TTY 0. This ensures the TTY abstraction works for the existing single-terminal case before adding multiple TTYs.

**Acceptance Criteria**:
- `WasmHost::new()` creates `TtyRegistry` with TTY 0 backed by existing `TerminalBuffer`
- Physical keyboard input routes through TTY 0's input queue
- Terminal rendering reads from TTY 0's output buffer
- All existing tests pass with no behavioral change

**Files to modify**:
- `simulator/src/wasm_host.rs` — integrate TtyRegistry
- `simulator/src/main.rs` — route keyboard through TTY 0
- `simulator/src/host/terminal.rs` — optionally redirect through TTY

---

### KERN-028: TTY Integration Test — Multiple TTYs

**Type**: Test
**Priority**: P1
**Blocked by**: KERN-027

**Description**:
Test that multiple TTYs work: create TTY 1, spawn a process on it, switch foreground, switch back.

**Acceptance Criteria**:
- Add test to `scripts/test-processes.sh`:
  1. Create TTY 1 via kernel command `tty_test`
  2. Spawn `hello.wasm` with stdin/stdout on TTY 1
  3. Switch foreground to TTY 1 — see hello output
  4. Switch back to TTY 0 — see kernel shell
- Test kernel command `tty_test` exercises the full flow

---

## Epic 6: Socket Layer — Network Access for User Processes

*Depends on: Epic 1 (FDs). These tickets bridge the kernel's raw Ethernet networking with an FD-based socket API.*

---

### KERN-029: Socket Abstraction in Kernel

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-006 (FD host functions)

**Description**:
Add socket host functions that wrap the kernel's existing TCP stack (`net/tcp.rs`) behind the FD interface. Sockets become FDs — readable and writable like pipes and files.

**Important**: The existing TCP stack lives in the Rust OS WASM (compiled into `terminal_os.wasm`). Socket host functions will call back into the kernel's TCP implementation. This means the kernel mediates all network access for child processes.

Architecture:
```
User Process → sock_connect(fd) → Host Function → Kernel's TCP Stack → net_tx_frame_on → Network
```

**Acceptance Criteria**:
- New host functions in `"env"` namespace:
  | Function | Signature | Description |
  |----------|-----------|-------------|
  | `sock_tcp_connect` | `(ip_ptr, ip_len, port) -> i32` | Create TCP connection via kernel stack. Returns fd or -1 |
  | `sock_tcp_listen` | `(port, backlog) -> i32` | Create TCP listener via kernel stack. Returns listener fd or -1 |
  | `sock_tcp_accept` | `(listener_fd, remote_addr_ptr, remote_addr_len_ptr) -> i32` | Accept connection, returns new fd or -1 |
  | `sock_send` | `(fd, buf_ptr, buf_len) -> i32` | Send data on socket fd |
  | `sock_recv` | `(fd, buf_ptr, buf_len) -> i32` | Receive data from socket fd |
  | `sock_shutdown` | `(fd, how) -> i32` | Shutdown read(0)/write(1)/both(2) |

**Design decision**: These host functions need to invoke the kernel's TCP state machine. The host function implementation will:
1. Queue a request in a shared channel
2. The kernel's networking poll loop processes the request (connect, send, etc.)
3. Data flows through the kernel's existing TCP/IP stack
4. Received data is placed in the socket's FD buffer

This is complex and will be refined during implementation.

**Files to create/modify**:
- `simulator/src/host/socket.rs` (new)
- `simulator/src/fd.rs` — add `SocketFd` implementation
- `simulator/src/host/mod.rs` — wire in
- `operating-system/rust/src/lib.rs` — extern declarations

---

### KERN-030: `SocketFd` Implementation

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-029

**Description**:
Implement the `SocketFd` type that bridges FD read/write operations to the kernel's TCP stack.

**Acceptance Criteria**:
- `SocketFd` struct:
  ```rust
  pub struct SocketFd {
      rx_buffer: Arc<Mutex<VecDeque<u8>>>,  // data received from network
      tx_buffer: Arc<Mutex<VecDeque<u8>>>,  // data to send to network
      state: Arc<AtomicU8>,                  // connected, closed, error
      notify: Arc<Condvar>,                  // wake on data arrival
  }
  ```
- Read: pulls from rx_buffer, blocks if empty, returns 0 on connection closed
- Write: pushes to tx_buffer, kernel's poll loop drains it through TCP
- Close: initiates TCP FIN sequence via kernel
- Integrates with kernel's `net::tcp::TcpConnectionTable`

**Files to modify**:
- `simulator/src/fd.rs` — add SocketFd
- `operating-system/rust/src/net/tcp.rs` — expose hooks for host-mediated connections

---

### KERN-031: Socket Integration Test

**Type**: Test
**Priority**: P1
**Blocked by**: KERN-030

**Description**:
Test that socket FDs work for TCP communication between processes and between computers.

**Acceptance Criteria**:
- Test 1: Kernel opens socket listener, WASI client connects, data flows both ways
- Test 2: Two-instance test — instance 0 runs a WASI TCP client, instance 1 runs httpd, client fetches a page
- Added to `scripts/test-processes.sh`

---

## Epic 7: Cryptography — Foundation for SSH

*Depends on: Nothing (pure Rust crate additions). Can be done in parallel with earlier epics.*

---

### KERN-032: Add Crypto Dependencies to Rust OS

**Type**: Feature
**Priority**: P1
**Blocked by**: None
**Parallel with**: Everything in Epics 1-6

**Description**:
Add pure-Rust cryptography crates to `operating-system/rust/Cargo.toml` that compile to `wasm32-unknown-unknown`. These are needed for SSH key exchange, encryption, and authentication.

**Critical**: `ring` does NOT compile to WASM. Use the pure-Rust alternatives.

**Acceptance Criteria**:
- Add to `operating-system/rust/Cargo.toml`:
  ```toml
  # SSH crypto
  ed25519-dalek = { version = "2", default-features = false, features = ["alloc", "rand_core"] }
  x25519-dalek = { version = "2", default-features = false, features = ["static_secrets"] }
  chacha20poly1305 = { version = "0.10", default-features = false, features = ["alloc"] }
  sha2 = { version = "0.10", default-features = false }
  hmac = { version = "0.12", default-features = false }
  curve25519-dalek = { version = "4", default-features = false, features = ["alloc"] }
  rand_core = { version = "0.6", default-features = false }
  ```
- Verify they all compile to `wasm32-unknown-unknown`:
  ```bash
  cd operating-system/rust
  cargo build --target wasm32-unknown-unknown --release
  ```
- The resulting `terminal_os.wasm` still loads and runs in the simulator
- No functional changes — just dependency additions

**Files to modify**:
- `operating-system/rust/Cargo.toml`

**Simulator test**:
```bash
# Existing tests pass — no regression
printf 'echo hello\n' | timeout 10 cargo run --release -- --headless | grep "hello"
```

---

### KERN-033: SSH Crypto Module — Key Generation and Signing

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-032

**Description**:
Create a crypto module in the Rust OS that provides key generation, signing, and verification using Ed25519.

**Acceptance Criteria**:
- New file `operating-system/rust/src/crypto.rs`:
  - `generate_ed25519_keypair() -> (PublicKey, SecretKey)` — using getrandom for entropy
  - `sign(secret_key, message) -> Signature`
  - `verify(public_key, message, signature) -> bool`
  - `ed25519_public_key_to_bytes(key) -> [u8; 32]`
  - `ed25519_public_key_from_bytes(bytes) -> PublicKey`
- New file `operating-system/rust/src/crypto/kex.rs`:
  - `x25519_generate_keypair() -> (PublicKey, SecretKey)`
  - `x25519_diffie_hellman(our_secret, their_public) -> SharedSecret`
- New file `operating-system/rust/src/crypto/cipher.rs`:
  - `chacha20_poly1305_encrypt(key, nonce, aad, plaintext) -> ciphertext`
  - `chacha20_poly1305_decrypt(key, nonce, aad, ciphertext) -> plaintext`
- `sha256(data) -> [u8; 32]`
- `hmac_sha256(key, data) -> [u8; 32]`
- Unit tests for sign/verify round-trip
- Shell command `crypto_test` to verify crypto works in WASM

**Files to create**:
- `operating-system/rust/src/crypto.rs`
- `operating-system/rust/src/crypto/kex.rs`
- `operating-system/rust/src/crypto/cipher.rs`

**Simulator test**:
```bash
printf 'crypto_test\n' | timeout 15 cargo run --release -- --headless | grep "Crypto test: PASS"
```

---

### KERN-034: SSH Host Key Generation and Storage

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-033

**Description**:
Generate and persist an Ed25519 host key for each computer. Stored in the filesystem at `/etc/ssh/ssh_host_ed25519_key`.

**Acceptance Criteria**:
- On first boot (or when `sshd` starts), check if `/etc/ssh/ssh_host_ed25519_key` exists
- If not, generate a new Ed25519 keypair and save:
  - Private key: `/etc/ssh/ssh_host_ed25519_key` (raw 64 bytes, or OpenSSH format)
  - Public key: `/etc/ssh/ssh_host_ed25519_key.pub` (raw 32 bytes + comment)
- Load key on subsequent boots
- `ssh-keygen` shell command to manually regenerate

**Files to modify**:
- `operating-system/rust/src/crypto.rs` — key serialization
- `operating-system/rust/src/lib.rs` — `ssh-keygen` command

---

## Epic 8: SSH Implementation

*Depends on: Epic 5 (TTYs), Epic 6 (sockets), Epic 7 (crypto). This is the culmination of everything.*

---

### KERN-035: SSH Binary Packet Protocol

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-033
**Parallel with**: KERN-034

**Description**:
Implement the SSH binary packet protocol (RFC 4253 §6). This is the framing layer that all SSH messages use.

**Acceptance Criteria**:
- New file `operating-system/rust/src/ssh/packet.rs`:
  - `SshPacket { payload: Vec<u8>, padding: Vec<u8>, mac: Option<Vec<u8>> }`
  - `encode_packet(payload, cipher_state) -> Vec<u8>` — packs: [u32 packet_length] [u8 padding_length] [payload] [padding] [mac]
  - `decode_packet(data, cipher_state) -> Result<(SshPacket, usize), SshError>` — returns packet + bytes consumed
  - Before key exchange: no encryption, no MAC
  - After key exchange: ChaCha20-Poly1305 encryption + Poly1305 MAC
- Packet length includes padding_length + payload + padding (not mac)
- Minimum padding: 4 bytes, total (payload + padding + 1) must be multiple of cipher block size (or 8 before encryption)

**Files to create**:
- `operating-system/rust/src/ssh/mod.rs`
- `operating-system/rust/src/ssh/packet.rs`

---

### KERN-036: SSH Transport — Version Exchange + Key Exchange

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-035, KERN-033

**Description**:
Implement the SSH transport layer: version string exchange and key exchange using curve25519-sha256.

**Acceptance Criteria**:
- SSH version string: `SSH-2.0-TerminalOS_1.0\r\n`
- Key exchange algorithm: `curve25519-sha256` (RFC 8731)
- Key exchange flow:
  1. Both sides send `SSH_MSG_KEXINIT` (type 20) with supported algorithms
  2. Client sends `SSH_MSG_KEX_ECDH_INIT` (type 30) with ephemeral public key
  3. Server sends `SSH_MSG_KEX_ECDH_REPLY` (type 31) with:
     - Server host public key
     - Server ephemeral public key
     - Signature of exchange hash (H) using host key
  4. Both compute shared secret K via X25519
  5. Both compute exchange hash H = SHA256(V_C || V_S || I_C || I_S || K_S || Q_C || Q_S || K)
  6. Derive session keys from K + H
  7. Both send `SSH_MSG_NEWKEYS` (type 21)
- After key exchange, all packets encrypted with ChaCha20-Poly1305
- Algorithm negotiation lists:
  - kex: curve25519-sha256
  - host key: ssh-ed25519
  - cipher: chacha20-poly1305@openssh.com
  - mac: (implicit with AEAD)
  - compression: none

**Files to create**:
- `operating-system/rust/src/ssh/transport.rs`
- `operating-system/rust/src/ssh/kex.rs`

---

### KERN-037: SSH Authentication (Password + Public Key)

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-036

**Description**:
Implement SSH user authentication (RFC 4252).

**Acceptance Criteria**:
- `SSH_MSG_SERVICE_REQUEST` (type 5) for "ssh-userauth"
- `SSH_MSG_SERVICE_ACCEPT` (type 6)
- Authentication methods:
  1. **Password** (simpler, implement first):
     - `SSH_MSG_USERAUTH_REQUEST` with method "password"
     - Check against `/etc/passwd` (simple format: `username:password_hash`)
     - Default user `root` with no password (or configurable)
  2. **Public key** (implement second):
     - `SSH_MSG_USERAUTH_REQUEST` with method "publickey"
     - Check against `~/.ssh/authorized_keys` (one public key per line)
     - Verify signature
- `SSH_MSG_USERAUTH_SUCCESS` (type 52) or `SSH_MSG_USERAUTH_FAILURE` (type 51)
- Shell commands: `passwd` to set password, `ssh-copy-id` equivalent

**Files to create**:
- `operating-system/rust/src/ssh/auth.rs`
- `operating-system/rust/src/ssh/userdb.rs`

---

### KERN-038: SSH Connection — Channels and PTY

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-037, KERN-026 (TTY host functions)

**Description**:
Implement the SSH connection protocol (RFC 4254): channels, session requests, and PTY allocation.

**Acceptance Criteria**:
- Channel management:
  - `SSH_MSG_CHANNEL_OPEN` (type 90) — "session" channel type
  - `SSH_MSG_CHANNEL_OPEN_CONFIRMATION` (type 91)
  - `SSH_MSG_CHANNEL_DATA` (type 94) — payload data
  - `SSH_MSG_CHANNEL_EOF` (type 96)
  - `SSH_MSG_CHANNEL_CLOSE` (type 97)
  - `SSH_MSG_CHANNEL_WINDOW_ADJUST` (type 93) — flow control
- Session requests (`SSH_MSG_CHANNEL_REQUEST`, type 98):
  - `pty-req`: allocate virtual TTY via `tty_create()` host function
  - `shell`: spawn a new kernel shell on the TTY
  - `exec`: spawn a command on the TTY
  - `window-change`: resize TTY
- Data flow:
  ```
  Remote SSH Client → TCP → SSH Channel → VirtualTty → Spawned Shell
  Spawned Shell → VirtualTty → SSH Channel → TCP → Remote SSH Client
  ```
- Window size: initial 80x24, adjustable via window-change

**Files to create**:
- `operating-system/rust/src/ssh/channel.rs`
- `operating-system/rust/src/ssh/session.rs`

---

### KERN-039: SSH Server (sshd) — Background Daemon in Kernel

**Type**: Feature
**Priority**: P0 — Final Goal
**Blocked by**: KERN-038, KERN-029 (socket layer), KERN-023 (background daemon support)

**Description**:
Implement `sshd` as a built-in kernel command that listens on port 22 and handles SSH connections. Built into the kernel (not a standalone WASI binary) because it needs deep access to the networking stack, process spawning, and TTY management.

**Acceptance Criteria**:
- `sshd` command (or auto-start on boot):
  - Binds TCP listener on port 22
  - Runs as background daemon
- On new TCP connection:
  1. SSH version exchange
  2. Key exchange (curve25519-sha256)
  3. Authentication (password or public key)
  4. Channel open + PTY request
  5. Create new VirtualTty via `tty_create()`
  6. Spawn new shell/kernel instance with stdio on the TTY
  7. Bridge: SSH channel data ↔ VirtualTty input/output
- Multiple concurrent sessions (up to `MAX_TCP_CONNECTIONS` limit of 8)
- Clean disconnect: close channel → close TTY → kill shell process
- `sshd` shows in `ps` output

**Files to create/modify**:
- `operating-system/rust/src/ssh/server.rs` (new)
- `operating-system/rust/src/lib.rs` — add `sshd` command

---

### KERN-040: SSH Client (ssh) — Connect to Other Computers

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-039

**Description**:
Implement an `ssh` command that connects to another in-game computer running sshd.

**Acceptance Criteria**:
- `ssh user@host` command:
  1. DNS resolve host (or use IP directly)
  2. TCP connect to port 22
  3. SSH version exchange
  4. Key exchange
  5. Host key verification (first use: ask "accept? (yes/no)", store in `~/.ssh/known_hosts`)
  6. Authentication (password prompt or public key)
  7. Open session channel, request PTY, request shell
  8. Bridge: local terminal ↔ SSH channel
  9. `~.` escape sequence to disconnect
- `ssh user@host command` — run single command, print output, exit
- `ssh -p <port>` — custom port

**Files to create**:
- `operating-system/rust/src/ssh/client.rs`
- `operating-system/rust/src/lib.rs` — add `ssh` command

---

### KERN-041: SSH End-to-End Integration Test

**Type**: Test
**Priority**: P0 — Final Milestone
**Blocked by**: KERN-040

**Description**:
Comprehensive test of SSH between two simulator instances.

**Acceptance Criteria**:
- New test script `scripts/test-ssh.sh`
- Test cases:
  1. Instance 1 starts `sshd`
  2. Instance 0 connects via `ssh root@10.0.0.2`
  3. Remote shell works (run `echo hello`, verify output)
  4. Remote `ls`, `cat`, `ps` work
  5. File created on remote is visible via `cat` locally (via curl or ssh)
  6. `exit` cleanly disconnects
  7. Host key is persisted in `~/.ssh/known_hosts`
  8. Password authentication works
  9. Multiple sequential connections work (connect, disconnect, reconnect)

**Files to create**:
- `scripts/test-ssh.sh`

---

## Epic 9: Java Side Parity

*These tickets bring the Minecraft Java mod in sync with the simulator. Can be done after simulator implementation is stable.*

---

### KERN-042: Java — Linker Migration

**Type**: Feature
**Priority**: P2
**Blocked by**: KERN-001 (simulator version proves the pattern)

**Description**:
Migrate `ComputerInstance.java` from the manual `hostFunctionMap` + `createImportsForModule()` pattern to wasmtime-java's `Linker` API. This is required for WASI namespace support.

Today (`ComputerInstance.java:1295-1326`): imports are matched by name only, ignoring the module namespace. The `Linker` handles namespaces properly.

**Acceptance Criteria**:
- Replace `Map<String, Extern> hostFunctionMap` with `Linker` usage
- `linker.define(store, "env", "terminal_write", ...)` for all kernel host functions
- `Instance` created via `linker.instantiate(store, module)` instead of `new Instance(store, module, imports)`
- All existing functionality preserved
- Java unit test `WasmImportTest` still passes

**Files to modify**:
- `src/main/java/com/example/evanscomputermod/wasm/ComputerInstance.java`

---

### KERN-043: Java — FdTable and Pipe Implementation

**Type**: Feature
**Priority**: P2
**Blocked by**: KERN-042

**Description**:
Port the FdTable, PipeFd, VfsFileFd, and TerminalFd from the simulator to Java.

**Acceptance Criteria**:
- Java interfaces and classes mirroring simulator's `fd.rs`:
  - `IFileDescriptor` interface with `read()`, `write()`, `close()`
  - `FdTable` class with allocate/get/close/dup/dup2
  - `PipeFd` backed by `ArrayBlockingQueue<byte[]>`
  - `VfsFileFd` backed by `Path` within `computerStoragePath`
  - `TerminalFd` backed by `inputQueue` and `TerminalDisplay`
  - `NullFd`
- FD host functions registered in `"env"` namespace via Linker

**Files to create**:
- `src/main/java/com/example/evanscomputermod/wasm/fd/IFileDescriptor.java`
- `src/main/java/com/example/evanscomputermod/wasm/fd/FdTable.java`
- `src/main/java/com/example/evanscomputermod/wasm/fd/PipeFd.java`
- `src/main/java/com/example/evanscomputermod/wasm/fd/VfsFileFd.java`
- `src/main/java/com/example/evanscomputermod/wasm/fd/TerminalFd.java`
- `src/main/java/com/example/evanscomputermod/wasm/fd/NullFd.java`

---

### KERN-044: Java — ProcessManager and WASI Functions

**Type**: Feature
**Priority**: P2
**Blocked by**: KERN-043

**Description**:
Port the ProcessManager and WASI host function implementations from the simulator to Java.

**Acceptance Criteria**:
- `ProcessManager` class managing multiple WASM instances
- WASI `wasi_snapshot_preview1` functions implemented as Java host functions
- `process_spawn/wait/kill/list` host functions
- Can spawn a `.wasm` file from the computer's filesystem
- Verified via `WasmImportTest` extension

**Files to create**:
- `src/main/java/com/example/evanscomputermod/wasm/ProcessManager.java`
- `src/main/java/com/example/evanscomputermod/wasm/ProcessEntry.java`
- `src/main/java/com/example/evanscomputermod/wasm/wasi/WasiFunctions.java`

---

### KERN-045: Java — TTY and Socket Layers

**Type**: Feature
**Priority**: P2
**Blocked by**: KERN-044

**Description**:
Port VirtualTty, TtyRegistry, and Socket host functions to Java.

**Acceptance Criteria**:
- VirtualTty and TtyRegistry classes
- TTY host functions in "env" namespace
- Socket host functions in "env" namespace
- All SSH functionality works in-game

**Files to create**:
- `src/main/java/com/example/evanscomputermod/wasm/tty/VirtualTty.java`
- `src/main/java/com/example/evanscomputermod/wasm/tty/TtyRegistry.java`

---

## Epic 10: Comprehensive Test Suite

*Integration tests that validate the complete system.*

---

### KERN-046: Master Test Script

**Type**: Test
**Priority**: P1
**Blocked by**: KERN-019 (process tests), KERN-041 (SSH tests)

**Description**:
Create a master test script that runs all test suites in order.

**Acceptance Criteria**:
- New script `scripts/test-all.sh` that runs:
  1. `scripts/test-networking.sh` — existing networking tests
  2. `scripts/test-processes.sh` — FD, pipe, redirect, process tests
  3. `scripts/test-ssh.sh` — SSH tests
- Summary report at end: total pass/fail across all suites
- Exit code 1 if any test fails

**Files to create**:
- `scripts/test-all.sh`

---

### KERN-047: CI Build Script for WASI Programs

**Type**: Infrastructure
**Priority**: P1
**Blocked by**: KERN-018

**Description**:
Ensure `scripts/build-wasm-programs.sh` can be run in CI to compile all WASI programs. Add `wasm32-wasip1` target installation.

**Acceptance Criteria**:
- `scripts/build-wasm-programs.sh`:
  - Installs `wasm32-wasip1` target if missing (`rustup target add wasm32-wasip1`)
  - Builds every `wasm-programs/*/` crate
  - Copies `.wasm` binaries to `wasm-bin/`
  - Prints success/failure summary
- Script is idempotent (safe to run multiple times)

**Files to create/modify**:
- `scripts/build-wasm-programs.sh`

---

### KERN-048: WASM Binary Auto-Deploy in Simulator

**Type**: Feature
**Priority**: P1
**Blocked by**: KERN-014

**Description**:
Add a simulator flag `--bin-dir <path>` that copies WASI binaries from a directory into every instance's `/bin/` virtual filesystem on startup. This makes testing easier — you don't have to manually copy `.wasm` files into `simulator-data/`.

**Acceptance Criteria**:
- New CLI flag: `--bin-dir ../wasm-bin`
- On startup, for each instance:
  - Create `bin/` in the instance's storage directory
  - Copy all `.wasm` files from `--bin-dir` into `bin/`
- Shell can then run `bin/hello.wasm` or (with PATH support) just `hello.wasm`

**Files to modify**:
- `simulator/src/main.rs` — add CLI flag, copy logic

---

### KERN-049: PATH-Based Command Resolution

**Type**: Feature
**Priority**: P2
**Blocked by**: KERN-014

**Description**:
Support a `PATH` environment variable in the kernel shell so users can type `hello` instead of `bin/hello.wasm`.

**Acceptance Criteria**:
- Kernel maintains a `PATH` variable (default: `/bin`)
- When a command is not a builtin:
  1. Check if `<command>` exists as a file → run it
  2. Check if `<command>.wasm` exists → run it
  3. For each directory in PATH: check `<dir>/<command>` and `<dir>/<command>.wasm`
  4. If none found: "command not found"
- `export PATH=/bin:/usr/bin` to modify (or simplify: just support `PATH` as a kernel variable)

**Files to modify**:
- `operating-system/rust/src/shell.rs` — command resolution
- `operating-system/rust/src/lib.rs`

---

### KERN-050: SSH as a Standalone WASI Binary (Stretch Goal)

**Type**: Feature
**Priority**: P3 — Stretch
**Blocked by**: KERN-039, KERN-029, KERN-030

**Description**:
Once the socket layer exposes TCP to WASI programs, port the SSH server and client from kernel builtins to standalone WASI binaries that users can drop into `/bin/`.

This requires:
- WASI programs can call `sock_tcp_listen`, `sock_tcp_accept`, `sock_send`, `sock_recv`
- WASI programs can call `tty_create`, `tty_attach_fd` (for PTY allocation)
- WASI programs can call `process_spawn` (to spawn shell per connection)

These are custom host functions beyond standard WASI — a "TerminalOS syscall" extension in a custom namespace like `"terminalos"`.

**Acceptance Criteria**:
- `wasm-programs/sshd/` — standalone SSH server
- `wasm-programs/ssh/` — standalone SSH client
- Can be dropped into any computer's `/bin/` and run
- Works identically to the kernel builtin versions

---

## Dependency Graph & Parallelism

### What can run in parallel

```
Phase 1 (all parallel):
  KERN-001 (ProcessManager scaffold)
  KERN-002 (FdTable + FileDescriptor trait)
  KERN-032 (Crypto dependencies)              ← independent, can start day 1

Phase 1b (after KERN-002):
  KERN-003 (PipeFd)      ┐
  KERN-004 (VfsFileFd)   ├── all parallel
  KERN-005 (TerminalFd)  ┘

Phase 2 (after Phase 1b):
  KERN-006 (FD host functions)   → KERN-007 (fd_test)
  KERN-008 (WASI Tier 1)        ┐
  KERN-009 (WASI Args)          ├── after KERN-008, parallel with each other
  KERN-010 (WASI FS)            │
  KERN-011 (WASI Stubs)         ┘

Phase 3 (after KERN-008-011 + KERN-001):
  KERN-012 (Process spawn infra)  → KERN-013 (Process host fns)  → KERN-014 (First WASI binary!)
  KERN-016 (Pipeline parser)      ← can start in parallel with KERN-012

Phase 4 (after KERN-014):
  KERN-015 (ps/kill)          ┐
  KERN-018 (WASI utilities)   ├── all parallel
  KERN-024 (SIGCHLD)          ┘
  KERN-017 (Pipeline executor)    ← after KERN-016 + KERN-013

Phase 5 (after KERN-017 + KERN-018):
  KERN-019 (Pipeline integration test)  ← MILESTONE: pipes work
  KERN-020 (Background &)
  KERN-023 (httpd background)

Phase 6 (parallel with Phase 4-5):
  KERN-025 (VirtualTty)  → KERN-026 (TTY host fns)  → KERN-027 (Wire TTY 0)  → KERN-028 (TTY test)
  KERN-029 (Socket abstraction)  → KERN-030 (SocketFd)  → KERN-031 (Socket test)

Phase 7 (after crypto + KERN-032):
  KERN-033 (Crypto module)  → KERN-034 (Host keys)  ← parallel with everything
  KERN-035 (SSH packet)     → KERN-036 (SSH transport)  → KERN-037 (SSH auth)

Phase 8 (after Phase 6 + Phase 7):
  KERN-038 (SSH channels)  → KERN-039 (sshd)  → KERN-040 (ssh client)  → KERN-041 (SSH test!)  ← FINAL MILESTONE

Phase 9 (after Phase 8 stable):
  KERN-042 → KERN-043 → KERN-044 → KERN-045  (Java parity, sequential)

Phase 10:
  KERN-046 (Master test)
  KERN-047 (CI build)
  KERN-048 (Auto-deploy)
  KERN-049 (PATH resolution)
  KERN-050 (SSH as WASI — stretch)
```

### Critical Path

The longest dependency chain (determines minimum time to SSH):

```
KERN-002 → KERN-003 → KERN-006 → KERN-008 → KERN-012 → KERN-013 → KERN-014 → KERN-017 → KERN-039 → KERN-041
(FdTable)  (PipeFd)   (FD host)  (WASI T1)  (Spawn)    (Proc host) (1st WASI)  (Pipeline)  (sshd)     (SSH test)
```

With crypto (KERN-032→033→035→036→037→038) running in parallel, it merges at KERN-038→039.

### Estimated ticket count by epic

| Epic | Tickets | Description |
|------|---------|-------------|
| 1: Foundation | 7 | FDs, pipes, host functions |
| 2: Process Spawning | 8 | WASI, ProcessManager, first binary |
| 3: Pipes & Redirection | 4 | Parser, executor, utilities, test |
| 4: Job Control | 5 | Background, jobs, fg/bg, signals |
| 5: Virtual TTY | 4 | TTY struct, host fns, wiring |
| 6: Sockets | 3 | Socket abstraction, FD, test |
| 7: Crypto | 3 | Dependencies, module, host keys |
| 8: SSH | 7 | Packet, transport, auth, channels, server, client, test |
| 9: Java Parity | 4 | Linker, FDs, ProcessManager, TTY/Socket |
| 10: Test/Infra | 5 | Master test, CI, auto-deploy, PATH, stretch |
| **Total** | **50** | |
