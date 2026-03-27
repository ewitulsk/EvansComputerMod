//! Terminal OS - A simple operating system for the WASM terminal.
//!
//! Provides a shell interface with built-in programs including:
//! - help: List available commands
//! - edit: A CC:Tweaked-style text editor
//! - python: Python REPL with terminal module
//! - ls: List files
//! - clear: Clear the screen
//! - cat: Display file contents

mod fs;
mod editor;
mod python;
mod git;
mod crypto;
mod ssh;
pub mod shell;
pub mod peripheral;
pub mod interrupt;
pub mod modules;
pub mod net;

// Custom random implementation for WASM
// Uses a simple xorshift PRNG seeded with a fixed value
// (In a real implementation, you'd want to seed from the host)
use getrandom::register_custom_getrandom;

fn custom_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    // Simple xorshift64* PRNG
    static mut STATE: u64 = 0x853c_49e6_748f_ea9b;

    unsafe {
        for byte in buf.iter_mut() {
            STATE ^= STATE >> 12;
            STATE ^= STATE << 25;
            STATE ^= STATE >> 27;
            *byte = (STATE.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8;
        }
    }
    Ok(())
}

register_custom_getrandom!(custom_getrandom);

/// Global pointer to the currently active ShellInstance.
/// Set before running Python code, git commands, ssh client, etc.
/// This lets deeply-nested code route output through the correct shell
/// (local terminal or SSH session) without threading `&mut ShellInstance`
/// through every function signature.
pub(crate) static mut ACTIVE_SHELL: Option<*mut shell::ShellInstance> = None;

/// Terminal host functions provided by the Minecraft mod.
/// This module is kernel-internal — user-facing code should use
/// ShellInstance methods (print/println/clear/read_line) instead.
pub(crate) mod terminal {
    extern "C" {
        /// Writes a string to the terminal.
        fn terminal_write(ptr: *const u8, len: usize) -> i32;

        /// Clears the terminal screen.
        fn terminal_clear();

        /// Sets the cursor position.
        fn terminal_set_cursor(x: i32, y: i32);

        /// Gets the terminal width (80 columns).
        fn terminal_get_width() -> i32;

        /// Gets the terminal height (24 rows).
        fn terminal_get_height() -> i32;

        /// Sleeps for the specified number of milliseconds.
        /// Blocks the terminal but not the game server.
        fn sleep_ms(milliseconds: i32);

        /// Opens the visual programming editor on the client.
        fn open_visual_editor();

        /// Reads a line of text input from the user. Blocks until Enter is pressed.
        /// The prompt is displayed by the Rust side before calling this.
        /// Returns number of bytes written to buf, or -1 on error.
        fn terminal_read_line(prompt_ptr: *const u8, prompt_len: i32, buf_ptr: *mut u8, buf_len: i32) -> i32;

        /// Polls for the next pending interrupt.
        /// Writes the interrupt payload to buf_ptr (up to buf_len bytes).
        /// Returns the IRQ number (>= 0) if an interrupt was available, or -1 if none pending.
        fn interrupt_poll(buf_ptr: *mut u8, buf_len: i32) -> i32;

        /// Gets the length of the last polled interrupt's payload.
        fn interrupt_poll_len() -> i32;
    }

    /// Helper function to write a string to the terminal.
    pub fn print(s: &str) {
        unsafe {
            terminal_write(s.as_ptr(), s.len());
        }
    }

    /// Helper function to print a line (with newline).
    pub fn println(s: &str) {
        print(s);
        print("\n");
    }

    /// Clears the terminal screen.
    pub fn clear() {
        unsafe { terminal_clear(); }
    }

    /// Sets the cursor position.
    pub fn set_cursor(x: i32, y: i32) {
        unsafe { terminal_set_cursor(x, y); }
    }

    /// Gets the terminal width.
    pub fn get_width() -> i32 {
        unsafe { terminal_get_width() }
    }

    /// Gets the terminal height.
    pub fn get_height() -> i32 {
        unsafe { terminal_get_height() }
    }

    /// Sleeps for the specified duration in milliseconds (raw, no interrupt polling).
    /// Use this when you need to sleep without automatic interrupt dispatch.
    pub fn raw_sleep_ms(ms: i32) {
        unsafe { sleep_ms(ms); }
    }

    /// Sleeps for the specified duration in milliseconds.
    /// Blocks the terminal but not the game server.
    /// Sleeps are chunked in 10ms intervals to allow interrupt delivery.
    /// Note: This dispatches via Rust-level handlers only. Python code should
    /// use the Python-level sleep() which has VM access for Python handler dispatch.
    pub fn sleep(ms: u32) {
        let mut remaining = ms as i32;
        while remaining > 0 {
            let chunk = if remaining > 10 { 10 } else { remaining };
            raw_sleep_ms(chunk);
            remaining -= chunk;
            // Poll and dispatch interrupts between sleep chunks (Rust handlers only)
            yield_interrupts();
        }
    }

    /// Raw read_line: calls the host terminal_read_line function directly.
    /// For SSH sessions, use ShellInstance::read_line() instead which polls TCP.
    pub fn read_line_raw(prompt: &str) -> String {
        static mut READ_BUF_RAW: [u8; 1024] = [0u8; 1024];
        let len = unsafe {
            terminal_read_line(
                prompt.as_ptr(), prompt.len() as i32,
                READ_BUF_RAW.as_mut_ptr(), READ_BUF_RAW.len() as i32,
            )
        };
        if len <= 0 {
            return String::new();
        }
        unsafe {
            std::str::from_utf8_unchecked(&READ_BUF_RAW[..len as usize]).to_string()
        }
    }

    /// Displays a prompt and reads a line of text input from the user.
    /// Blocks until Enter is pressed. Returns the entered string.
    /// If interrupted (Ctrl+T), returns empty string. The interrupted flag
    /// remains set on the Java side so the next host function call will
    /// trigger the normal interrupt/reset flow via WasmInterruptedException.
    pub fn read_line(prompt: &str) -> String {
        static mut READ_BUF: [u8; 1024] = [0u8; 1024];
        print(prompt);
        let len = unsafe {
            terminal_read_line(
                prompt.as_ptr(), prompt.len() as i32,
                READ_BUF.as_mut_ptr(), READ_BUF.len() as i32,
            )
        };
        if len <= 0 {
            // -2 = interrupted, -1 = shutdown, 0 = empty
            // For -2: the interrupted flag is still set on Java side;
            // the next host function call (e.g. terminal_write from println)
            // will throw WasmInterruptedException and trigger reset_to_shell.
            return String::new();
        }
        unsafe {
            std::str::from_utf8_unchecked(&READ_BUF[..len as usize]).to_string()
        }
    }

    /// Opens the visual programming editor on the client.
    pub fn open_visual() {
        unsafe { open_visual_editor(); }
    }

    /// Polls one pending interrupt from the host.
    /// Returns `Some((irq, payload))` if an interrupt was available, `None` otherwise.
    /// Does NOT dispatch — the caller is responsible for dispatching.
    pub fn poll_interrupt() -> Option<(i32, String)> {
        static mut INTERRUPT_BUF: [u8; 4096] = [0u8; 4096];

        let irq = unsafe { interrupt_poll(INTERRUPT_BUF.as_mut_ptr(), INTERRUPT_BUF.len() as i32) };
        if irq < 0 {
            return None;
        }
        let payload_len = unsafe { interrupt_poll_len() } as usize;
        let data = unsafe {
            std::str::from_utf8_unchecked(&INTERRUPT_BUF[..payload_len]).to_string()
        };
        Some((irq, data))
    }

    /// Cooperative interrupt yield point.
    /// Polls and dispatches any pending interrupts via Rust-level handlers.
    /// For Python handlers, use the VM-aware dispatch in python.rs instead.
    /// Returns the number of interrupts delivered.
    pub fn yield_interrupts() -> i32 {
        let mut count = 0;
        while let Some((irq, data)) = poll_interrupt() {
            crate::dispatch_interrupt(irq, &data);
            count += 1;
        }
        count
    }
}

/// Redstone host functions for controlling and reading redstone signals
pub mod redstone {
    extern "C" {
        /// Sets the redstone output power for a specific side.
        fn redstone_set_output(side: i32, power: i32) -> i32;
        /// Gets the redstone input power for a specific side.
        fn redstone_get_input(side: i32) -> i32;
        /// Gets all 6 redstone input levels into a buffer (6 x i32).
        fn redstone_get_all_input(buf_ptr: *mut i32) -> i32;
    }

    /// Relative side constants (relative to the terminal's facing direction)
    pub const DOWN: i32 = 0;
    pub const UP: i32 = 1;
    pub const FRONT: i32 = 2;
    pub const BACK: i32 = 3;
    pub const LEFT: i32 = 4;
    pub const RIGHT: i32 = 5;

    /// Sets the redstone output power for a specific side (0-15).
    pub fn set_output(side: i32, power: i32) -> bool {
        unsafe { redstone_set_output(side, power) == 0 }
    }

    /// Gets the redstone input power for a specific relative side (0-15).
    pub fn get_input(side: i32) -> i32 {
        unsafe { redstone_get_input(side) }
    }

    /// Gets all 6 redstone input levels as an array, in relative side order.
    pub fn get_all_input() -> [i32; 6] {
        let mut buf = [0i32; 6];
        unsafe { redstone_get_all_input(buf.as_mut_ptr()); }
        buf
    }
}

/// File descriptor host functions for pipes and file I/O
pub(crate) mod fd {
    extern "C" {
        pub fn fd_open(path_ptr: *const u8, path_len: usize, flags: i32) -> i32;
        pub fn fd_read(fd: i32, buf_ptr: *mut u8, buf_len: usize) -> i32;
        pub fn fd_write(fd: i32, buf_ptr: *const u8, buf_len: usize) -> i32;
        pub fn fd_close(fd: i32) -> i32;
        pub fn pipe_create(read_fd_ptr: *mut i32, write_fd_ptr: *mut i32) -> i32;
    }
}

#[allow(dead_code)]
mod tty {
    extern "C" {
        pub fn tty_create(width: i32, height: i32) -> i32;
        pub fn tty_attach_fd(tty_id: i32, mode: i32) -> i32;
        pub fn tty_set_foreground(tty_id: i32) -> i32;
        pub fn tty_get_size(tty_id: i32, width_ptr: *mut i32, height_ptr: *mut i32) -> i32;
        pub fn tty_write_input(tty_id: i32, buf_ptr: *const u8, buf_len: usize) -> i32;
    }
}

use editor::{Editor, ExitResult};
use python::PythonRepl;
use shell::{ShellInstance, OsState};

/// The local terminal's shell instance.
static mut LOCAL_SHELL: Option<ShellInstance> = None;

/// Global editor instance (needed because we can't allocate per-shell yet)
/// TODO: Move into ShellInstance
static mut EDITOR: Option<Editor> = None;

/// Global Python REPL instance
/// TODO: Move into ShellInstance
static mut PYTHON_REPL: Option<PythonRepl> = None;

/// Main entry point - called when the terminal is opened.
#[cfg(all(target_arch = "wasm32", not(test)))]
#[unsafe(no_mangle)]
pub fn main() {
    terminal::clear();

    // Initialize the local shell instance
    unsafe {
        LOCAL_SHELL = Some(ShellInstance::new_terminal());
    }

    // Initialize networking
    net::NetStack::init();
    interrupt::register(interrupt::IRQ_NETWORK, |_irq, _data| {
        if let Some(stack) = net::NetStack::get() {
            stack.poll_rx();
        }
    });

    // Load saved network config
    if let Some(stack) = net::NetStack::get() {
        if let Some(config) = fs::read_file_absolute("network.cfg") {
            for line in config.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') { continue; }
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.is_empty() { continue; }
                match parts[0] {
                    "iface" if parts.len() >= 3 => {
                        // iface eth0 10.0.0.1/24 [vlan=100]
                        if let Some(idx) = stack.find_iface(parts[1]) {
                            if let Some((ip, prefix)) = net::types::Ipv4Addr::parse_cidr(parts[2]) {
                                stack.configure_iface(idx, ip, prefix);
                            }
                            for p in &parts[3..] {
                                if let Some(vid_str) = p.strip_prefix("vlan=") {
                                    if let Ok(vid) = vid_str.parse::<u16>() {
                                        stack.interfaces[idx].vlan = Some(vid);
                                    }
                                }
                            }
                        }
                    }
                    "dns" if parts.len() >= 2 => {
                        if let Some(dns) = net::types::Ipv4Addr::parse(parts[1]) {
                            stack.dns_server = dns;
                        }
                    }
                    "route" if parts.len() >= 5 => {
                        // route default via 10.0.0.1 dev eth0
                        // route 192.168.1.0/24 via 10.0.0.1 dev eth0
                        let dest_str = parts[1];
                        let mut gw = net::types::Ipv4Addr::ZERO;
                        let mut dev = "";
                        let mut i = 2;
                        while i < parts.len() {
                            match parts[i] {
                                "via" if i + 1 < parts.len() => {
                                    gw = net::types::Ipv4Addr::parse(parts[i+1]).unwrap_or(net::types::Ipv4Addr::ZERO);
                                    i += 2;
                                }
                                "dev" if i + 1 < parts.len() => {
                                    dev = parts[i+1];
                                    i += 2;
                                }
                                _ => { i += 1; }
                            }
                        }
                        if let Some(iface_idx) = stack.find_iface(dev) {
                            if dest_str == "default" {
                                let _ = stack.routing.add_route(net::types::Ipv4Addr::ZERO, 0, gw, iface_idx);
                            } else if let Some((dest, prefix)) = net::types::Ipv4Addr::parse_cidr(dest_str) {
                                let _ = stack.routing.add_route(dest, prefix, gw, iface_idx);
                            }
                        }
                    }
                    _ => {}
                }
            }
            terminal::println("Network config restored.");
        }
    }

    terminal::println("================================================================================");
    terminal::println("                         TERMINAL OS v1.0                                      ");
    terminal::println("================================================================================");
    terminal::println("");
    terminal::println("Welcome to Terminal OS!");
    terminal::println("Type 'help' for a list of available commands.");
    terminal::println("");
    unsafe {
        if let Some(ref mut shell) = LOCAL_SHELL {
            print_prompt(shell);
        }
    }
}

/// Prints the shell prompt with CWD, checking for completed background jobs first.
fn print_prompt(shell: &mut ShellInstance) {
    shell.check_completed_jobs();
    let cwd = shell.cwd().to_string();
    if cwd.is_empty() {
        shell.print("/ > ");
    } else {
        shell.print("/");
        shell.print(&cwd);
        shell.print(" > ");
    }
}

/// Resets the OS to shell mode, clearing any running programs.
/// Called when Ctrl+T is pressed to terminate current program.
fn reset_to_shell() {
    unsafe {
        // Clear any running program state
        EDITOR = None;
        PythonRepl::clear_interrupt_handlers();
        PYTHON_REPL = None;

        if let Some(ref mut shell) = LOCAL_SHELL {
            shell.input_len = 0;

            // Clean up SSH client if active
            if shell.ssh_client.is_some() {
                if let Some(ref ctx) = shell.ssh_client {
                    if let Some(stack) = net::NetStack::get() {
                        stack.tcp_close_immediate(ctx.conn_idx);
                    }
                }
                shell.ssh_client = None;
            }

            // Reset to shell mode
            shell.state = OsState::Shell;

            // Clear screen and show message
            shell.clear();
            shell.println("^T - Program terminated");
            shell.println("");
            print_prompt(shell);
        }
    }
}

/// Called when the user enters a line of input.
#[unsafe(no_mangle)]
pub fn on_input(ptr: *const u8, len: usize) {
    // Read the input string from memory
    let input = unsafe {
        let slice = std::slice::from_raw_parts(ptr, len);
        std::str::from_utf8_unchecked(slice)
    };

    unsafe {
        if let Some(ref mut shell) = LOCAL_SHELL {
            match shell.state {
                OsState::Shell => handle_shell_input(shell, input),
                OsState::Editor => handle_editor_input(shell, input),
                OsState::Python => handle_python_input(shell, input),
                OsState::Ssh => ssh::client::handle_ssh_client_input(shell, input),
            }
        }
    }
}

/// Internal interrupt dispatch logic for Rust-level handlers.
/// Python-level handlers are dispatched separately via VM-aware functions in python.rs,
/// since they need the VirtualMachine reference that's only available inside #[pyfunction] calls.
fn dispatch_interrupt(irq: i32, data: &str) {
    // IRQ_TERMINATE is non-maskable — always resets to shell
    if irq == interrupt::IRQ_TERMINATE {
        reset_to_shell();
        return;
    }

    // Dispatch to Rust-level handler if registered
    interrupt::dispatch_rust(irq, data);

    // Note: Python-level handlers are NOT dispatched here.
    // They are dispatched by terminal_module::sleep() and terminal_module::check_interrupts()
    // which have access to the Python VirtualMachine.
}

/// Called by the host to deliver an interrupt event.
/// The host writes the payload data to WASM memory and calls this with the IRQ number.
#[cfg(all(target_arch = "wasm32", not(test)))]
#[unsafe(no_mangle)]
pub fn on_interrupt(irq: i32, data_ptr: *const u8, data_len: usize) {
    let data = unsafe {
        let slice = std::slice::from_raw_parts(data_ptr, data_len);
        std::str::from_utf8_unchecked(slice)
    };
    dispatch_interrupt(irq, data);
}

/// Handles input in shell mode - buffers characters until Enter is pressed
pub fn handle_shell_input(shell: &mut ShellInstance, input: &str) {
    let bytes = input.as_bytes();

    for &byte in bytes {
        // Check for Ctrl+T (0x14) - terminate/reset
        if byte == 0x14 {
            reset_to_shell();
            return;
        }

        match byte {
            b'\n' | b'\r' => {
                // Enter pressed - process the buffered command
                shell.println("");

                if shell.input_len > 0 {
                    // Get the command string
                    let cmd = unsafe {
                        std::str::from_utf8_unchecked(&shell.input_buf[..shell.input_len])
                    }.to_string();
                    process_command(shell, &cmd);
                }

                // Clear the buffer
                shell.input_len = 0;

                // Print prompt if still in shell mode
                if shell.state == OsState::Shell {
                    print_prompt(shell);
                }
            }
            8 | 127 => {
                // Backspace - delete last character
                if shell.input_len > 0 {
                    shell.input_len -= 1;
                    // Erase character on screen: move back, print space, move back
                    shell.print("\x08 \x08");
                }
            }
            _ if byte >= 32 && byte < 127 => {
                // Printable character - add to buffer and echo
                if shell.input_len < shell.input_buf.len() {
                    shell.input_buf[shell.input_len] = byte;
                    shell.input_len += 1;
                    // Echo the character
                    let char_slice = unsafe { std::slice::from_raw_parts(&byte, 1) };
                    if let Ok(s) = std::str::from_utf8(char_slice) {
                        shell.print(s);
                    }
                }
            }
            _ => {
                // Ignore other characters (escape sequences, etc.)
            }
        }
    }
}

/// Processes a complete command line
pub fn process_command(shell: &mut ShellInstance, input: &str) {
    let input = input.trim();

    if input.is_empty() {
        return;
    }

    // Block commands that can't work over SSH
    // visual requires client-side GUI, sshd/httpd would nest blocking loops
    if shell.is_ssh {
        let (command, _) = parse_command(input);
        match command {
            "visual" | "sshd" | "httpd" => {
                let msg = format!("{}: not available over SSH", command);
                shell.println(&msg);
                return;
            }
            _ => {}
        }
    }

    // Try executing as a pipeline (handles .wasm programs, pipes, redirects)
    // Use with_cwd to set the global CWD for fs operations
    let pipeline = shell::parse_pipeline(input);
    let handled = shell.with_cwd(|shell| {
        shell::execute_pipeline(shell, &pipeline)
    });
    if handled {
        return;
    }

    // Fall through to builtins
    let (command, args) = parse_command(input);

    // Wrap all builtin execution in with_cwd so fs operations use this shell's CWD
    shell.with_cwd(|shell| {
        match command {
            "help" => cmd_help(shell),
            "exit" => {
                if shell.is_ssh {
                    shell.println("Connection closed.");
                    shell.exited = true;
                } else {
                    shell.println("Use Ctrl+T to exit.");
                }
            }
            "clear" => cmd_clear(shell),
            "ls" => cmd_ls(shell, args),
            "cat" => cmd_cat(shell, args),
            "edit" => cmd_edit(shell, args),
            "rm" => cmd_rm(shell, args),
            "echo" => cmd_echo(shell, args),
            "sleep" => {
                if let Ok(ms) = args.trim().parse::<u32>() {
                    terminal::sleep(ms);
                } else {
                    shell.println("Usage: sleep <milliseconds>");
                }
            }
            "python" => cmd_python(shell, args),
            "peripherals" => cmd_peripherals(shell, args),
            "cd" => cmd_cd(shell, args),
            "pwd" => cmd_pwd(shell),
            "mkdir" => cmd_mkdir(shell, args),
            "cp" => cmd_cp(shell, args),
            "mv" => cmd_mv(shell, args),
            "touch" => cmd_touch(shell, args),
            "git" => cmd_git(shell, args),
            "ifconfig" => cmd_ifconfig(shell, args),
            "ip" => cmd_ip(shell, args),
            "ping" => cmd_ping(shell, args),
            "nslookup" => cmd_nslookup(shell, args),
            "resolvectl" => cmd_resolvectl(shell, args),
            "httpd" => cmd_httpd(shell, args),
            "curl" => cmd_curl(shell, args),
            "ssh" => {
                let arg = args.trim();
                if arg.is_empty() {
                    shell.println("Usage: ssh [user[:password]@]host[:port]");
                } else {
                    // Parse: [user[:password]@]host[:port]
                    let (username, password, hostport) = if let Some(at_pos) = arg.find('@') {
                        let userpart = &arg[..at_pos];
                        let hostpart = &arg[at_pos+1..];
                        if let Some(colon_pos) = userpart.find(':') {
                            (&userpart[..colon_pos], Some(&userpart[colon_pos+1..]), hostpart)
                        } else {
                            (userpart, None, hostpart)
                        }
                    } else {
                        ("root", None, arg)
                    };

                    let (host, port) = if let Some(colon_pos) = hostport.find(':') {
                        (&hostport[..colon_pos], hostport[colon_pos+1..].parse().unwrap_or(22u16))
                    } else {
                        (hostport, 22u16)
                    };

                    ssh::client::ssh_connect(shell, host, port, username, password);
                }
            }
            "passwd" => {
                let password = shell.read_line("New password: ");
                if password.is_empty() {
                    shell.println("Password not changed.");
                } else {
                    let confirm = shell.read_line("Confirm password: ");
                    if password == confirm {
                        ssh::auth::set_password("root", &password);
                        shell.println("Password updated.");
                    } else {
                        shell.println("Passwords don't match.");
                    }
                }
            }
            "fd_test" => {
                shell.println("Running FD tests...");

                // Test 1: pipe_create + fd_write + fd_read
                unsafe {
                    let mut read_fd: i32 = 0;
                    let mut write_fd: i32 = 0;
                    let status = fd::pipe_create(&mut read_fd, &mut write_fd);
                    if status != 0 {
                        shell.println("  FAIL step 1: pipe_create");
                        return;
                    }
                    let msg = format!("  pipe_create: read_fd={}, write_fd={}", read_fd, write_fd);
                    shell.println(&msg);

                    // Write to pipe
                    let msg_bytes = b"hello pipe";
                    let written = fd::fd_write(write_fd, msg_bytes.as_ptr(), msg_bytes.len());
                    if written != 10 {
                        shell.println("  FAIL step 2: fd_write to pipe");
                        return;
                    }
                    shell.println("  PASS: fd_write to pipe");

                    // Close write end
                    fd::fd_close(write_fd);
                    shell.println("  PASS: fd_close write end");

                    // Read from pipe
                    let mut buf = [0u8; 256];
                    let n = fd::fd_read(read_fd, buf.as_mut_ptr(), buf.len());
                    if n != 10 || &buf[..10] != b"hello pipe" {
                        shell.println("  FAIL step 4: fd_read from pipe");
                        return;
                    }
                    shell.println("  PASS: fd_read from pipe");

                    // Read again should get EOF (0)
                    let n2 = fd::fd_read(read_fd, buf.as_mut_ptr(), buf.len());
                    if n2 != 0 {
                        let msg = format!("  FAIL step 5: expected EOF, got {}", n2);
                        shell.println(&msg);
                        return;
                    }
                    shell.println("  PASS: fd_read EOF after close");

                    fd::fd_close(read_fd);

                    // Test 2: fd_open + fd_write + fd_read (file)
                    let path = b"fd_test_file.txt";
                    // O_WRONLY | O_CREAT | O_TRUNC = 1 | 4 | 8 = 13
                    let wfd = fd::fd_open(path.as_ptr(), path.len(), 13);
                    if wfd < 0 {
                        shell.println("  FAIL step 6: fd_open for write");
                        return;
                    }

                    let content = b"file content";
                    let written = fd::fd_write(wfd, content.as_ptr(), content.len());
                    if written != 12 {
                        shell.println("  FAIL step 7: fd_write to file");
                        return;
                    }
                    fd::fd_close(wfd);
                    shell.println("  PASS: fd_open + fd_write to file");

                    // Read it back (O_RDONLY = 0)
                    let rfd = fd::fd_open(path.as_ptr(), path.len(), 0);
                    if rfd < 0 {
                        shell.println("  FAIL step 8: fd_open for read");
                        return;
                    }
                    let mut buf2 = [0u8; 256];
                    let n3 = fd::fd_read(rfd, buf2.as_mut_ptr(), buf2.len());
                    if n3 != 12 {
                        let msg = format!("  FAIL step 9: fd_read from file, got {}", n3);
                        shell.println(&msg);
                        return;
                    }
                    if &buf2[..12] != b"file content" {
                        shell.println("  FAIL step 9: fd_read content mismatch");
                        return;
                    }
                    fd::fd_close(rfd);
                    shell.println("  PASS: fd_open + fd_read from file");
                }

                shell.println("FD test: PASS");
            }
            "tty_test" => {
                shell.println("Running TTY tests...");
                unsafe {
                    // Create a new TTY
                    let tty_id = tty::tty_create(80, 24);
                    if tty_id < 0 {
                        shell.println("  FAIL: tty_create");
                        return;
                    }
                    let msg = format!("  PASS: tty_create -> tty_id={}", tty_id);
                    shell.println(&msg);

                    // Get its size
                    let mut w: i32 = 0;
                    let mut h: i32 = 0;
                    let result = tty::tty_get_size(tty_id, &mut w, &mut h);
                    if result != 0 || w != 80 || h != 24 {
                        shell.println("  FAIL: tty_get_size");
                        return;
                    }
                    shell.println("  PASS: tty_get_size 80x24");

                    // Attach write FD
                    let write_fd = tty::tty_attach_fd(tty_id, 1);
                    if write_fd < 0 {
                        shell.println("  FAIL: tty_attach_fd write");
                        return;
                    }
                    shell.println("  PASS: tty_attach_fd write");

                    // Write to TTY
                    let msg_bytes = b"TTY test output";
                    let n = fd::fd_write(write_fd, msg_bytes.as_ptr(), msg_bytes.len());
                    if n != msg_bytes.len() as i32 {
                        shell.println("  FAIL: fd_write to TTY");
                        return;
                    }
                    shell.println("  PASS: fd_write to TTY");

                    fd::fd_close(write_fd);

                    // Set foreground to new TTY and back
                    let result = tty::tty_set_foreground(tty_id);
                    if result != 0 {
                        shell.println("  FAIL: tty_set_foreground");
                        return;
                    }
                    tty::tty_set_foreground(0); // back to physical
                    shell.println("  PASS: tty_set_foreground switch");

                    shell.println("TTY test: PASS");
                }
            }
            "crypto_test" => {
                shell.println("Running crypto tests...");

                // Test 1: Ed25519 sign/verify
                let (pub_key, priv_key) = crypto::ed25519_generate_keypair();
                let message = b"hello from minecraft";
                let sig = crypto::ed25519_sign(&priv_key, message);
                let valid = crypto::ed25519_verify(&pub_key, message, &sig);
                if !valid {
                    shell.println("  FAIL: Ed25519 sign/verify");
                    return;
                }
                shell.println("  PASS: Ed25519 sign/verify");

                // Test 2: Ed25519 wrong message fails
                let invalid = crypto::ed25519_verify(&pub_key, b"wrong message", &sig);
                if invalid {
                    shell.println("  FAIL: Ed25519 wrong message should fail");
                    return;
                }
                shell.println("  PASS: Ed25519 wrong message rejected");

                // Test 3: Key serialization round-trip
                let pub_bytes = crypto::ed25519_public_key_bytes(&pub_key);
                let pub_key2 = crypto::ed25519_public_key_from_bytes(&pub_bytes).unwrap();
                let valid2 = crypto::ed25519_verify(&pub_key2, message, &sig);
                if !valid2 {
                    shell.println("  FAIL: Key serialization round-trip");
                    return;
                }
                shell.println("  PASS: Key serialization round-trip");

                // Test 4: X25519 DH
                let (pub_a, sec_a) = crypto::x25519_generate_keypair();
                let (pub_b, sec_b) = crypto::x25519_generate_keypair();
                let shared_a = crypto::x25519_diffie_hellman(&sec_a, &pub_b);
                let shared_b = crypto::x25519_diffie_hellman(&sec_b, &pub_a);
                if shared_a != shared_b {
                    shell.println("  FAIL: X25519 DH shared secret mismatch");
                    return;
                }
                shell.println("  PASS: X25519 DH key agreement");

                // Test 5: SHA-256
                let hash = crypto::sha256(b"test");
                if hash.len() != 32 {
                    shell.println("  FAIL: SHA-256 output length");
                    return;
                }
                shell.println("  PASS: SHA-256");

                // Test 6: HMAC-SHA-256
                let mac = crypto::hmac_sha256(b"key", b"data");
                if mac.len() != 32 {
                    shell.println("  FAIL: HMAC-SHA-256 output length");
                    return;
                }
                shell.println("  PASS: HMAC-SHA-256");

                // Test 7: ChaCha20-Poly1305 encrypt/decrypt
                let key = crypto::sha256(b"encryption key");
                let nonce = [0u8; 12];
                let plaintext = b"secret message";
                match crypto::chacha20_poly1305_encrypt(&key, &nonce, plaintext) {
                    Ok(ciphertext) => {
                        match crypto::chacha20_poly1305_decrypt(&key, &nonce, &ciphertext) {
                            Ok(decrypted) => {
                                if decrypted != plaintext {
                                    shell.println("  FAIL: ChaCha20-Poly1305 decrypt mismatch");
                                    return;
                                }
                                shell.println("  PASS: ChaCha20-Poly1305 encrypt/decrypt");
                            }
                            Err(e) => {
                                let msg = format!("  FAIL: ChaCha20-Poly1305 decrypt: {}", e);
                                shell.println(&msg);
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("  FAIL: ChaCha20-Poly1305 encrypt: {}", e);
                        shell.println(&msg);
                        return;
                    }
                }

                shell.println("Crypto test: PASS");
            }
            "ssh-keygen" => {
                shell.println("Generating new SSH host key...");
                let (pub_key, _priv_key) = crypto::generate_and_save_host_key();
                let pub_bytes = crypto::ed25519_public_key_bytes(&pub_key);
                // Print fingerprint (SHA-256 of public key, first 16 bytes as hex)
                let fingerprint = crypto::sha256(&pub_bytes);
                let hex: String = fingerprint[..16].iter().map(|b| format!("{:02x}", b)).collect();
                let msg = format!("Host key fingerprint: SHA256:{}", hex);
                shell.println(&msg);
                shell.println("Key saved to /etc/ssh/ssh_host_ed25519_key");
            }
            "sshd" => cmd_sshd(shell, args),
            "visual" => {
                shell.println("Opening visual editor...");
                terminal::open_visual();
            }
            "ps" => {
                unsafe {
                    let mut buf = [0u8; 4096];
                    let n = process_list(buf.as_mut_ptr(), buf.len());
                    if n > 0 {
                        let json = core::str::from_utf8(&buf[..n as usize]).unwrap_or("[]");
                        shell.println("PID  STATE    NAME");
                        for entry in json.split('{').skip(1) {
                            let pid = extract_json_int(entry, "pid").unwrap_or(0);
                            let name = extract_json_str(entry, "name").unwrap_or("?");
                            let state = extract_json_str(entry, "state").unwrap_or("?");
                            let line = format!("{:>3}  {:<8} {}", pid, state, name);
                            shell.println(&line);
                        }
                    } else {
                        shell.println("No processes.");
                    }
                }
            }
            "jobs" => {
                shell.list_jobs();
            }
            "fg" => {
                let target = args.trim();
                if target.is_empty() {
                    shell.println("Usage: fg %<job_id> or fg <pid>");
                } else {
                    let pid = if target.starts_with('%') {
                        shell.get_job_pid(target[1..].parse().unwrap_or(0))
                    } else {
                        target.parse().unwrap_or(-1)
                    };
                    if pid > 0 {
                        unsafe {
                            let exit_code = process_wait(pid);
                            if exit_code != 0 {
                                let msg = format!("Process exited with code {}", exit_code);
                                shell.println(&msg);
                            }
                        }
                    } else {
                        shell.println("No such job.");
                    }
                }
            }
            "bg" => {
                // bg acknowledges that a job continues in background.
                // Since our background jobs already run independently, this is a no-op.
                shell.println("Job continues in background.");
            }
            "kill" => {
                if let Ok(pid) = args.trim().parse::<i32>() {
                    unsafe {
                        let result = process_kill(pid, 15); // SIGTERM
                        if result == 0 {
                            shell.println("Process killed.");
                        } else {
                            shell.println("Failed to kill process.");
                        }
                    }
                } else {
                    shell.println("Usage: kill <pid>");
                }
            }
            _ => {
                // Check if it's a .wasm file or a program in bin/
                let cmd = command;
                let wasm_path = if cmd.ends_with(".wasm") {
                    if fs::exists(cmd) {
                        Some(cmd.to_string())
                    } else {
                        None
                    }
                } else if fs::exists(&format!("{}.wasm", cmd)) {
                    Some(format!("{}.wasm", cmd))
                } else if fs::exists(&format!("bin/{}.wasm", cmd)) {
                    Some(format!("bin/{}.wasm", cmd))
                } else {
                    None
                };

                if let Some(wasm_path) = wasm_path {
                    // Build argv: program name followed by arguments separated by newlines
                    let mut argv_str = wasm_path.clone();
                    if !args.is_empty() {
                        argv_str.push('\n');
                        argv_str.push_str(&args.replace(' ', "\n"));
                    }

                    unsafe {
                        let pid = process_spawn(
                            wasm_path.as_ptr(), wasm_path.len(),
                            argv_str.as_ptr(), argv_str.len(),
                            -1, -1, -1,  // use terminal for stdio
                        );
                        if pid > 0 {
                            let exit_code = process_wait(pid);
                            if exit_code != 0 {
                                let msg = format!("Process exited with code {}", exit_code);
                                shell.println(&msg);
                            }
                        } else {
                            shell.print("Failed to execute: ");
                            shell.println(&wasm_path);
                        }
                    }
                } else {
                    shell.print("Unknown command: ");
                    shell.println(command);
                    shell.println("Type 'help' for a list of commands.");
                }
            }
        }
    });

    // Check if git rebase -i requested the editor to open
    if let Some(todo_path) = git::take_rebase_edit_request() {
        unsafe {
            let mut editor = Editor::new();
            editor.open(&todo_path);
            EDITOR = Some(editor);
            shell.state = OsState::Editor;
            if let Some(ref editor) = EDITOR {
                editor.render();
            }
        }
        return; // Don't print blank line or prompt — editor is now active
    }

    // Print blank line after command output (if still in shell mode)
    if shell.state == OsState::Shell {
        shell.println("");
    }
}

/// Handles input in editor mode (works for both local terminal and SSH shells)
pub fn handle_editor_input(shell: &mut ShellInstance, input: &str) {
    // Set ACTIVE_SHELL so editor I/O routes through the correct shell
    unsafe { ACTIVE_SHELL = Some(shell as *mut ShellInstance); }

    // Check for Ctrl+T (0x14) - terminate/reset
    for &byte in input.as_bytes() {
        if byte == 0x14 {
            if !shell.is_ssh {
                reset_to_shell();
            } else {
                shell.state = OsState::Shell;
                unsafe { EDITOR = None; }
                shell.clear();
                shell.println("^T - Program terminated");
                shell.println("");
                print_prompt(shell);
            }
            return;
        }
    }

    unsafe {
        if let Some(ref mut editor) = EDITOR {
            editor.handle_input(input);

            if editor.should_exit() {
                let exit_result = editor.get_exit_result();
                let run_filename = if exit_result == ExitResult::ExitAndRun {
                    // Copy filename for running after editor closes
                    Some(copy_filename(editor.get_run_filename()))
                } else {
                    None
                };

                // Exit editor, return to shell
                shell.state = OsState::Shell;
                EDITOR = None;
                shell.clear();

                match exit_result {
                    ExitResult::ExitAndRun => {
                        if let Some(filename) = run_filename {
                            shell.print("Running: ");
                            shell.println(filename);
                            shell.println("");
                            // TODO: Actually run the file when script execution is implemented
                            shell.println("(Script execution not yet implemented)");
                            shell.println("");
                        }
                    }
                    _ => {
                        // Check if this was a rebase todo edit
                        if git::has_rebase_in_progress() {
                            shell.println("Rebase todo saved.");
                            shell.println("Run 'git rebase --continue' to execute or 'git rebase --abort' to cancel.");
                            shell.println("");
                        } else {
                            shell.println("Exited editor.");
                            shell.println("");
                        }
                    }
                }

                print_prompt(shell);
            }
        }
    }
}

/// Handles input in Python REPL mode (works for both local terminal and SSH shells)
pub fn handle_python_input(shell: &mut ShellInstance, input: &str) {
    // Set ACTIVE_SHELL so Python I/O routes through the correct shell
    unsafe { ACTIVE_SHELL = Some(shell as *mut ShellInstance); }
    // Buffer for Python input line
    static mut PYTHON_INPUT: [u8; 1024] = [0u8; 1024];
    static mut PYTHON_INPUT_LEN: usize = 0;

    let bytes = input.as_bytes();

    unsafe {
        for &byte in bytes {
            // Check for Ctrl+T (0x14) - terminate/reset
            if byte == 0x14 {
                PYTHON_INPUT_LEN = 0;
                if !shell.is_ssh {
                    reset_to_shell();
                } else {
                    shell.state = OsState::Shell;
                    PythonRepl::clear_interrupt_handlers();
                    PYTHON_REPL = None;
                    shell.clear();
                    shell.println("^T - Program terminated");
                    shell.println("");
                    print_prompt(shell);
                }
                return;
            }

            match byte {
                b'\n' | b'\r' => {
                    // Enter pressed - send line to Python REPL
                    shell.println("");

                    if let Some(ref mut repl) = PYTHON_REPL {
                        let line = std::str::from_utf8_unchecked(&PYTHON_INPUT[..PYTHON_INPUT_LEN]);
                        let should_exit = repl.handle_input(line);

                        if should_exit {
                            // Exit Python, return to shell
                            shell.state = OsState::Shell;
                            PythonRepl::clear_interrupt_handlers();
                            PYTHON_REPL = None;
                            shell.println("");
                            shell.println("Exited Python.");
                            shell.println("");
                            print_prompt(shell);
                        } else {
                            repl.print_prompt();
                        }
                    }

                    PYTHON_INPUT_LEN = 0;
                }
                8 | 127 => {
                    // Backspace
                    if PYTHON_INPUT_LEN > 0 {
                        PYTHON_INPUT_LEN -= 1;
                        shell.print("\x08 \x08");
                    }
                }
                4 => {
                    // Ctrl+D - exit Python
                    if let Some(ref mut repl) = PYTHON_REPL {
                        repl.handle_input("\x04");
                    }
                    shell.state = OsState::Shell;
                    PythonRepl::clear_interrupt_handlers();
                    PYTHON_REPL = None;
                    shell.println("");
                    shell.println("Exited Python.");
                    shell.println("");
                    print_prompt(shell);
                }
                _ if byte >= 32 && byte < 127 => {
                    // Printable character
                    if PYTHON_INPUT_LEN < PYTHON_INPUT.len() {
                        PYTHON_INPUT[PYTHON_INPUT_LEN] = byte;
                        PYTHON_INPUT_LEN += 1;
                        // Echo the character
                        let char_slice = std::slice::from_raw_parts(&byte, 1);
                        if let Ok(s) = std::str::from_utf8(char_slice) {
                            shell.print(s);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Copies a filename to a static buffer (needed because editor will be dropped)
fn copy_filename(filename: &str) -> &'static str {
    static mut FILENAME_BUFFER: [u8; 64] = [0u8; 64];

    unsafe {
        let bytes = filename.as_bytes();
        let len = bytes.len().min(FILENAME_BUFFER.len());
        FILENAME_BUFFER[..len].copy_from_slice(&bytes[..len]);
        std::str::from_utf8_unchecked(&FILENAME_BUFFER[..len])
    }
}

// Process management host functions
extern "C" {
    fn process_spawn(path_ptr: *const u8, path_len: usize,
                     argv_ptr: *const u8, argv_len: usize,
                     stdin_fd: i32, stdout_fd: i32, stderr_fd: i32) -> i32;
    fn process_wait(pid: i32) -> i32;
    fn process_list(buf_ptr: *mut u8, buf_len: usize) -> i32;
    #[allow(dead_code)]
    fn process_state(pid: i32) -> i32;
    fn process_kill(pid: i32, signal: i32) -> i32;
}

// Socket host functions — kernel-mediated TCP for user processes.
// The kernel's sshd uses the raw TCP stack directly; these are for
// future user-process network access.
#[allow(dead_code)]
extern "C" {
    fn sock_tcp_connect(ip_ptr: *const u8, ip_len: usize, port: i32) -> i32;
    fn sock_tcp_listen(port: i32, backlog: i32) -> i32;
    fn sock_tcp_accept(fd: i32, addr_ptr: *mut u8, addr_len_ptr: *mut i32) -> i32;
    fn sock_send(fd: i32, buf_ptr: *const u8, buf_len: usize) -> i32;
    fn sock_recv(fd: i32, buf_ptr: *mut u8, buf_len: usize) -> i32;
    fn sock_shutdown(fd: i32, how: i32) -> i32;
}

fn extract_json_int(json: &str, key: &str) -> Option<i32> {
    let search = format!("\"{}\":", key);
    let start = json.find(&search)? + search.len();
    let rest = &json[start..];
    let end = rest.find(|c: char| !c.is_ascii_digit() && c != '-').unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

fn extract_json_str<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let search = format!("\"{}\":\"", key);
    let start = json.find(&search)? + search.len();
    let rest = &json[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

/// Parses a command line into command and arguments
fn parse_command(input: &str) -> (&str, &str) {
    let input = input.trim();

    if let Some(space_idx) = input.find(' ') {
        let command = &input[..space_idx];
        let args = input[space_idx + 1..].trim();
        (command, args)
    } else {
        (input, "")
    }
}

/// Command: help - Display available commands
fn cmd_help(shell: &mut ShellInstance) {
    shell.println("");
    shell.println("Available commands:");
    shell.println("");
    shell.println("  help              - Display this help message");
    shell.println("  clear             - Clear the screen");
    shell.println("  ls [path] [-l]    - List directory contents");
    shell.println("  cd [dir]          - Change directory (no arg = root)");
    shell.println("  pwd               - Print working directory");
    shell.println("  mkdir [-p] <dir>  - Create directory");
    shell.println("  cat <file> ...    - Display file contents");
    shell.println("  edit <file>       - Edit a file");
    shell.println("  touch <file>      - Create empty file");
    shell.println("  cp <src> <dst>    - Copy file");
    shell.println("  mv <src> <dst>    - Move/rename file");
    shell.println("  rm [-r] <file>... - Delete files or directories");
    shell.println("  echo [-n|-e] text - Print text");
    shell.println("  python [file]     - Start Python REPL or run script");
    shell.println("  git <command>     - Version control (init/add/commit/log/...)");
    shell.println("  peripherals [name]- List peripherals or methods");
    shell.println("  visual            - Open visual programming editor");
    shell.println("");
    shell.println("Networking:");
    shell.println("  ifconfig <iface>   - Show/set interface configuration");
    shell.println("  ifconfig vlan <id> - Set 802.1Q VLAN (0-4094) or 'off'");
    shell.println("  ip addr            - Show/manage interface addresses");
    shell.println("  ip route           - Show/manage routing table");
    shell.println("  ip link            - Show/manage link-layer info");
    shell.println("  ping <ip> [count]  - Send ICMP echo requests");
    shell.println("  nslookup <host>    - DNS lookup");
    shell.println("  resolvectl status  - Show DNS configuration");
    shell.println("  resolvectl dns ..  - Set DNS server");
    shell.println("  httpd <port>       - Start HTTP server (blocks shell)");
    shell.println("  sshd [port]        - Start SSH server (default: 22)");
    shell.println("  curl <url>         - HTTP client (GET/POST)");
    shell.println("  ssh [user@]host    - SSH client (connect to remote)");
    shell.println("");
    shell.println("Process management:");
    shell.println("  ps                - List running processes");
    shell.println("  kill <pid>        - Kill a process by PID");
    shell.println("  jobs              - List background jobs");
    shell.println("  fg %<id>|<pid>    - Bring job to foreground (wait for exit)");
    shell.println("  bg                - Continue job in background");
    shell.println("  <program>         - Run a .wasm program (searches bin/)");
    shell.println("  <program> &       - Run program in background");
    shell.println("");
    shell.println("System shortcuts:");
    shell.println("  Ctrl+T      - Terminate current program (kill)");
    shell.println("");
    shell.println("Editor shortcuts:");
    shell.println("  Ctrl+S  Save   Ctrl+E  Exit   Ctrl+R  Save & run");
    shell.println("  Ctrl+F  Find   Ctrl+X  Cut    Ctrl+C  Copy");
    shell.println("  Ctrl+V  Paste  Ctrl+D  Delete Ctrl+K  Clear line");
}

/// Command: clear - Clear the screen
fn cmd_clear(shell: &mut ShellInstance) {
    shell.clear();
}

/// Command: ls - List directory contents
fn cmd_ls(shell: &mut ShellInstance, args: &str) {
    let mut show_long = false;
    let mut target = "";

    // Parse args: look for -l flag and optional path
    for arg in args.split_whitespace() {
        if arg == "-l" {
            show_long = true;
        } else if target.is_empty() {
            target = arg;
        }
    }

    let entries = fs::list_dir(target);

    if entries.is_empty() {
        // Check if target exists but is empty vs doesn't exist
        if !target.is_empty() && !fs::exists(target) && !fs::is_dir(target) {
            shell.print("ls: cannot access '");
            shell.print(target);
            shell.println("': No such file or directory");
            return;
        }
        shell.println("  (empty)");
        return;
    }

    for entry in &entries {
        if show_long {
            if entry.is_dir {
                shell.print("  d  ---     ");
            } else {
                shell.print("  f  ");
                // Show file size
                let full_path = if target.is_empty() {
                    entry.name.clone()
                } else {
                    let mut p = target.to_string();
                    p.push('/');
                    p.push_str(&entry.name);
                    p
                };
                match fs::get_size(&full_path) {
                    Some(size) => {
                        let size_str = format_size(size);
                        // Right-align size in 7 chars
                        for _ in 0..(7usize.saturating_sub(size_str.len())) {
                            shell.print(" ");
                        }
                        shell.print(&size_str);
                        shell.print(" ");
                    }
                    None => shell.print("      ? "),
                }
            }
        } else {
            shell.print("  ");
        }
        shell.print(&entry.name);
        if entry.is_dir {
            shell.print("/");
        }
        shell.println("");
    }
}

/// Format a file size in human-readable form
fn format_size(size: usize) -> String {
    if size < 1024 {
        format!("{}B", size)
    } else if size < 1024 * 1024 {
        format!("{:.1}K", size as f64 / 1024.0)
    } else {
        format!("{:.1}M", size as f64 / (1024.0 * 1024.0))
    }
}

/// Command: cat - Display file contents (supports multiple files)
fn cmd_cat(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("Usage: cat <file> [file2] ...");
        return;
    }

    for filename in args.split_whitespace() {
        if !fs::exists(filename) {
            shell.print("cat: ");
            shell.print(filename);
            shell.println(": No such file or directory");
            continue;
        }
        if fs::is_dir(filename) {
            shell.print("cat: ");
            shell.print(filename);
            shell.println(": Is a directory");
            continue;
        }
        if let Some(content) = fs::read_file(filename) {
            shell.print(content);
            // Add newline if content doesn't end with one
            if !content.ends_with('\n') {
                shell.println("");
            }
        } else {
            shell.print("cat: ");
            shell.print(filename);
            shell.println(": Error reading file");
        }
    }
}

/// Command: edit - Open file in editor
fn cmd_edit(shell: &mut ShellInstance, args: &str) {
    let filename = args.trim();

    unsafe {
        // Set ACTIVE_SHELL so editor rendering routes through the correct shell
        ACTIVE_SHELL = Some(shell as *mut ShellInstance);

        // Create a new editor
        let mut editor = Editor::new();

        if !filename.is_empty() {
            editor.open(filename);
        }

        // Switch to editor mode
        EDITOR = Some(editor);
        shell.state = OsState::Editor;

        // Render the editor
        if let Some(ref editor) = EDITOR {
            editor.render();
        }
    }
}

/// Command: rm - Delete files or directories
fn cmd_rm(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("Usage: rm [-r] <file> [file2] ...");
        return;
    }

    let mut recursive = false;
    let mut targets: Vec<&str> = Vec::new();

    for arg in args.split_whitespace() {
        if arg == "-r" || arg == "-rf" {
            recursive = true;
        } else {
            targets.push(arg);
        }
    }

    if targets.is_empty() {
        shell.println("Usage: rm [-r] <file> [file2] ...");
        return;
    }

    for target in &targets {
        if !fs::exists(target) && !fs::is_dir(target) {
            shell.print("rm: ");
            shell.print(target);
            shell.println(": No such file or directory");
            continue;
        }

        if fs::is_dir(target) {
            if !recursive {
                shell.print("rm: ");
                shell.print(target);
                shell.println(": Is a directory (use -r to remove)");
                continue;
            }
            // Recursively delete directory contents
            rm_recursive(target);
        } else {
            if !fs::delete_file(target) {
                shell.print("rm: cannot remove '");
                shell.print(target);
                shell.println("'");
            }
        }
    }
}

/// Recursively delete a directory and its contents.
/// `path` is already resolved (absolute from storage root).
fn rm_recursive(path: &str) {
    let resolved = fs::resolve_path(path);
    let entries = fs::list_dir_absolute(&resolved);
    for entry in &entries {
        let child_path = if resolved.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", resolved, entry.name)
        };
        if entry.is_dir {
            // Recurse with the already-resolved child path
            rm_recursive_absolute(&child_path);
        } else {
            fs::delete_absolute(&child_path);
        }
    }
    fs::delete_absolute(&resolved);
}

/// Internal recursive delete with absolute paths (no CWD resolution).
fn rm_recursive_absolute(path: &str) {
    let entries = fs::list_dir_absolute(path);
    for entry in &entries {
        let child_path = format!("{}/{}", path, entry.name);
        if entry.is_dir {
            rm_recursive_absolute(&child_path);
        } else {
            fs::delete_absolute(&child_path);
        }
    }
    fs::delete_absolute(path);
}

/// Command: echo - Print text with optional flags
fn cmd_echo(shell: &mut ShellInstance, args: &str) {
    let mut no_newline = false;
    let mut interpret_escapes = false;
    let mut text_start = 0;

    // Parse leading flags
    let mut remaining = args;
    loop {
        let trimmed = remaining.trim_start();
        if trimmed.starts_with("-n") && (trimmed.len() == 2 || trimmed.as_bytes().get(2) == Some(&b' ')) {
            no_newline = true;
            remaining = if trimmed.len() > 2 { &trimmed[3..] } else { "" };
        } else if trimmed.starts_with("-e") && (trimmed.len() == 2 || trimmed.as_bytes().get(2) == Some(&b' ')) {
            interpret_escapes = true;
            remaining = if trimmed.len() > 2 { &trimmed[3..] } else { "" };
        } else if trimmed.starts_with("-ne") || trimmed.starts_with("-en") {
            no_newline = true;
            interpret_escapes = true;
            let skip = if trimmed.starts_with("-ne") { 3 } else { 3 };
            remaining = if trimmed.len() > skip { &trimmed[skip + 1..] } else { "" };
        } else {
            text_start = args.len() - remaining.len();
            break;
        }
    }

    let text = &args[text_start..];

    if interpret_escapes {
        let mut chars = text.chars();
        let mut output = String::new();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => output.push('\n'),
                    Some('t') => output.push('\t'),
                    Some('r') => output.push('\r'),
                    Some('\\') => output.push('\\'),
                    Some('0') => output.push('\0'),
                    Some(other) => {
                        output.push('\\');
                        output.push(other);
                    }
                    None => output.push('\\'),
                }
            } else {
                output.push(c);
            }
        }
        shell.print(&output);
    } else {
        shell.print(text);
    }

    if !no_newline {
        shell.println("");
    }
}

/// Command: python - Start Python REPL or run a Python file
fn cmd_python(shell: &mut ShellInstance, args: &str) {
    let filename = args.trim();

    // Set ACTIVE_SHELL so Python I/O routes through the correct shell
    unsafe { ACTIVE_SHELL = Some(shell as *mut ShellInstance); }

    if filename.is_empty() {
        // No filename - start interactive REPL
        unsafe {
            // Create a new Python REPL
            let repl = PythonRepl::new();

            // Show the banner
            repl.show_banner();

            // Store the REPL and switch to Python mode
            PYTHON_REPL = Some(repl);
            shell.state = OsState::Python;

            // Print the initial prompt
            if let Some(ref repl) = PYTHON_REPL {
                repl.print_prompt();
            }
        }
    } else {
        // Filename provided - execute the file
        if !fs::exists(filename) {
            shell.print("File not found: ");
            shell.println(filename);
            unsafe { ACTIVE_SHELL = None; }
            return;
        }

        if let Some(code) = fs::read_file(filename) {
            shell.print("Running: ");
            shell.println(filename);
            shell.println("");

            // Create a temporary Python interpreter and run the file
            let mut repl = PythonRepl::new();
            repl.run_file(code, filename);

            // Interpreter is dropped here, returning to shell
        } else {
            shell.print("Error reading file: ");
            shell.println(filename);
        }
        unsafe { ACTIVE_SHELL = None; }
    }
}

/// Command: peripherals - List CC peripherals or show methods
fn cmd_peripherals(shell: &mut ShellInstance, args: &str) {
    let arg = args.trim();

    if arg.is_empty() {
        // List all peripherals
        let peripherals = peripheral::list();

        if peripherals.is_empty() {
            shell.println("");
            shell.println("No peripherals connected.");
            shell.println("Place CC:Tweaked peripheral blocks adjacent to this terminal.");
        } else {
            shell.println("");
            shell.println("Connected peripherals:");
            shell.println("");
            for p in &peripherals {
                shell.print("  ");
                shell.print(&p.name);
                shell.print(" (");
                shell.print(&p.peripheral_type);
                shell.print(") - ");
                shell.println(&p.side);
            }
        }
    } else {
        // Show methods for a specific peripheral
        match peripheral::get_methods(arg) {
            Ok(methods) => {
                shell.println("");
                shell.print("Methods for ");
                shell.print(arg);
                shell.println(":");
                shell.println("");
                for method in &methods {
                    shell.print("  ");
                    shell.println(method);
                }
                if methods.is_empty() {
                    shell.println("  (no methods)");
                }
            }
            Err(e) => {
                shell.print("Error: ");
                shell.println(&e);
            }
        }
    }
}

/// Command: cd - Change directory
fn cmd_cd(shell: &mut ShellInstance, args: &str) {
    let target = args.trim();
    if target.is_empty() || target == "/" {
        fs::set_cwd("");
        return;
    }

    // Handle absolute paths (starting with /)
    let resolved = if target.starts_with('/') {
        target[1..].to_string()
    } else {
        fs::resolve_path(target)
    };

    // Verify it's a directory — use is_dir_absolute to avoid double-resolving
    if !fs::is_dir_absolute(&resolved) {
        shell.print("cd: ");
        shell.print(target);
        shell.println(": No such directory");
        return;
    }

    fs::set_cwd(&resolved);
}

/// Command: pwd - Print working directory
fn cmd_pwd(shell: &mut ShellInstance) {
    let cwd = fs::get_cwd();
    if cwd.is_empty() {
        shell.println("/");
    } else {
        shell.print("/");
        shell.println(cwd);
    }
}

/// Command: mkdir - Create directory
fn cmd_mkdir(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("Usage: mkdir [-p] <dir>");
        return;
    }

    // -p flag is implicitly supported since host creates parents
    let dirname = args.trim().trim_start_matches("-p").trim();
    if dirname.is_empty() {
        shell.println("Usage: mkdir [-p] <dir>");
        return;
    }

    if fs::mkdir(dirname) {
        // silent success (like Unix mkdir)
    } else {
        shell.print("mkdir: cannot create directory '");
        shell.print(dirname);
        shell.println("'");
    }
}

/// Command: cp - Copy file
fn cmd_cp(shell: &mut ShellInstance, args: &str) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() != 2 {
        shell.println("Usage: cp <source> <destination>");
        return;
    }

    let src = parts[0];
    let dst = parts[1];

    if !fs::exists(src) {
        shell.print("cp: ");
        shell.print(src);
        shell.println(": No such file or directory");
        return;
    }

    if let Some(content) = fs::read_file(src) {
        if !fs::write_file(dst, content) {
            shell.print("cp: cannot create '");
            shell.print(dst);
            shell.println("'");
        }
    } else {
        shell.print("cp: error reading '");
        shell.print(src);
        shell.println("'");
    }
}

/// Command: mv - Move/rename file
fn cmd_mv(shell: &mut ShellInstance, args: &str) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() != 2 {
        shell.println("Usage: mv <source> <destination>");
        return;
    }

    let src = parts[0];
    let dst = parts[1];

    if !fs::exists(src) {
        shell.print("mv: ");
        shell.print(src);
        shell.println(": No such file or directory");
        return;
    }

    if let Some(content) = fs::read_file(src) {
        if fs::write_file(dst, content) {
            fs::delete_file(src);
        } else {
            shell.print("mv: cannot create '");
            shell.print(dst);
            shell.println("'");
        }
    } else {
        shell.print("mv: error reading '");
        shell.print(src);
        shell.println("'");
    }
}

/// Command: touch - Create empty file
fn cmd_touch(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("Usage: touch <file>");
        return;
    }

    let filename = args.trim();
    if !fs::exists(filename) {
        fs::write_file(filename, "");
    }
    // If file exists, touch does nothing (we don't have timestamps)
}

/// Command: git - Version control
fn cmd_git(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("usage: git <command> [args]");
        shell.println("");
        shell.println("Commands:");
        shell.println("  init             Initialize a new repository");
        shell.println("  add <file>       Stage files for commit");
        shell.println("  status           Show working tree status");
        shell.println("  commit -m <msg>  Record changes");
        shell.println("  log              Show commit history");
        shell.println("  branch [name]    List or create branches");
        shell.println("  checkout <branch> Switch branches");
        shell.println("  rebase <branch>  Rebase current branch onto target");
        shell.println("  diff             Show unstaged changes");
        return;
    }

    // Set ACTIVE_SHELL so git functions can route output through the correct shell
    unsafe { ACTIVE_SHELL = Some(shell as *mut ShellInstance); }

    let (subcmd, rest) = parse_command(args);
    match subcmd {
        "init" => git::cmd_init(),
        "add" => git::cmd_add(rest),
        "status" => git::cmd_status(),
        "commit" => git::cmd_commit(rest),
        "log" => git::cmd_log(),
        "branch" => git::cmd_branch(rest),
        "checkout" => git::cmd_checkout(rest),
        "diff" => git::cmd_diff(),
        "rebase" => git::cmd_rebase(rest),
        "debug" => git::cmd_debug(),
        _ => {
            shell.print("git: '");
            shell.print(subcmd);
            shell.println("' is not a git command");
        }
    }

    unsafe { ACTIVE_SHELL = None; }
}

fn cmd_ifconfig(shell: &mut ShellInstance, args: &str) {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => { shell.println("Network stack not initialized"); return; }
    };

    let parts: Vec<&str> = args.split_whitespace().collect();

    if parts.is_empty() {
        // Show all interfaces
        for i in 0..stack.iface_count {
            show_interface(shell, &stack.interfaces[i]);
        }
        return;
    }

    let iface_name = parts[0];
    let iface_idx = match stack.find_iface(iface_name) {
        Some(idx) => idx,
        None => {
            let msg = format!("Unknown interface: {}", iface_name);
            shell.println(&msg);
            return;
        }
    };

    if parts.len() == 1 {
        // Show specific interface
        show_interface(shell, &stack.interfaces[iface_idx]);
        return;
    }

    match parts[1] {
        "up" => {
            stack.set_link_state(iface_idx, true);
            shell.println("Link up.");
        }
        "down" => {
            stack.set_link_state(iface_idx, false);
            shell.println("Link down.");
        }
        "vlan" if parts.len() >= 3 => {
            let vlan_arg = parts[2];
            if vlan_arg == "off" || vlan_arg == "none" {
                stack.interfaces[iface_idx].vlan = None;
                shell.println("VLAN disabled.");
            } else if let Ok(vid) = vlan_arg.parse::<u16>() {
                if vid <= 4094 {
                    stack.interfaces[iface_idx].vlan = Some(vid);
                    let msg = format!("VLAN set to {}.", vid);
                    shell.println(&msg);
                } else {
                    shell.println("VLAN ID must be 0-4094.");
                }
            } else {
                shell.println("Invalid VLAN ID.");
            }
        }
        cidr if cidr.contains('/') => {
            // Set IP with CIDR notation: ifconfig eth0 10.0.0.1/24
            match net::types::Ipv4Addr::parse_cidr(cidr) {
                Some((ip, prefix)) => {
                    stack.configure_iface(iface_idx, ip, prefix);
                    let msg = format!("{}: inet {}/{}", iface_name, ip, prefix);
                    shell.println(&msg);
                }
                None => shell.println("Invalid CIDR address (e.g. 10.0.0.1/24)."),
            }
        }
        _ => {
            shell.println("Usage: ifconfig <iface> [<ip>/<prefix> | up | down | vlan <id|off>]");
        }
    }
    save_network_config();
}

fn show_interface(shell: &mut ShellInstance, iface: &net::NetworkInterface) {
    let name = iface.name_str();
    let flags = if iface.link_up { "UP" } else { "DOWN" };
    let msg = format!("{}: flags=<{}>  mtu 1500", name, flags);
    shell.println(&msg);
    let mac_str = format!("      ether {}", iface.mac);
    shell.println(&mac_str);
    if iface.configured() {
        let addr_str = format!("      inet {}/{}", iface.ip, iface.prefix_len);
        shell.println(&addr_str);
    }
    if let Some(vid) = iface.vlan {
        let vlan_str = format!("      vlan {}", vid);
        shell.println(&vlan_str);
    }
    shell.println("");
}

fn cmd_ip(shell: &mut ShellInstance, args: &str) {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => { shell.println("Network stack not initialized"); return; }
    };

    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        shell.println("Usage: ip addr | ip route | ip link");
        return;
    }

    match parts[0] {
        "addr" | "address" => cmd_ip_addr(shell, stack, &parts[1..]),
        "route" => cmd_ip_route(shell, stack, &parts[1..]),
        "link" => cmd_ip_link(shell, stack, &parts[1..]),
        _ => shell.println("Usage: ip addr | ip route | ip link"),
    }
}

fn cmd_ip_addr(shell: &mut ShellInstance, stack: &mut net::NetStack, args: &[&str]) {
    if args.is_empty() || args[0] == "show" {
        // ip addr [show [dev ethN]]
        let dev = if args.len() >= 3 && args[1] == "dev" { Some(args[2]) } else if args.len() >= 2 && args[0] == "show" && args.len() >= 4 && args[2] == "dev" { Some(args[3]) } else { None };
        for i in 0..stack.iface_count {
            let iface = &stack.interfaces[i];
            if let Some(d) = dev {
                if iface.name_str() != d { continue; }
            }
            show_interface(shell, iface);
        }
    } else if args[0] == "add" {
        // ip addr add 10.0.0.1/24 dev eth0
        if args.len() < 4 || args[2] != "dev" {
            shell.println("Usage: ip addr add <ip>/<prefix> dev <iface>");
            return;
        }
        let cidr = args[1];
        let dev = args[3];
        match (net::types::Ipv4Addr::parse_cidr(cidr), stack.find_iface(dev)) {
            (Some((ip, prefix)), Some(idx)) => {
                stack.configure_iface(idx, ip, prefix);
                let msg = format!("Added {}/{} to {}", ip, prefix, dev);
                shell.println(&msg);
                save_network_config();
            }
            (None, _) => shell.println("Invalid CIDR address."),
            (_, None) => shell.println("Unknown interface."),
        }
    } else if args[0] == "del" {
        // ip addr del 10.0.0.1/24 dev eth0
        if args.len() < 4 || args[2] != "dev" {
            shell.println("Usage: ip addr del <ip>/<prefix> dev <iface>");
            return;
        }
        let dev = args[3];
        if let Some(idx) = stack.find_iface(dev) {
            stack.deconfigure_iface(idx);
            let msg = format!("Removed address from {}", dev);
            shell.println(&msg);
            save_network_config();
        } else {
            shell.println("Unknown interface.");
        }
    } else {
        shell.println("Usage: ip addr [show|add|del]");
    }
}

fn cmd_ip_route(shell: &mut ShellInstance, stack: &mut net::NetStack, args: &[&str]) {
    if args.is_empty() || args[0] == "show" {
        // ip route [show]
        let mut found = false;
        for e in &stack.routing.entries {
            if !e.active { continue; }
            found = true;
            let iface_name = if e.iface_index < stack.iface_count {
                stack.interfaces[e.iface_index].name_str()
            } else { "?" };
            if e.prefix_len == 0 && e.destination == net::types::Ipv4Addr::ZERO {
                let msg = format!("default via {} dev {}", e.gateway, iface_name);
                shell.println(&msg);
            } else if e.gateway == net::types::Ipv4Addr::ZERO {
                let msg = format!("{}/{} dev {} scope link", e.destination, e.prefix_len, iface_name);
                shell.println(&msg);
            } else {
                let msg = format!("{}/{} via {} dev {}", e.destination, e.prefix_len, e.gateway, iface_name);
                shell.println(&msg);
            }
        }
        if !found { shell.println("No routes configured."); }
    } else if args[0] == "add" {
        // ip route add default via 10.0.0.1 dev eth0
        // ip route add 192.168.1.0/24 via 10.0.0.1 dev eth0
        if args.len() < 2 {
            shell.println("Usage: ip route add <dest>/<prefix>|default via <gw> dev <iface>");
            return;
        }
        let dest_str = args[1];
        let mut gw = net::types::Ipv4Addr::ZERO;
        let mut dev = "";
        let mut i = 2;
        while i < args.len() {
            match args[i] {
                "via" if i + 1 < args.len() => {
                    gw = net::types::Ipv4Addr::parse(args[i+1]).unwrap_or(net::types::Ipv4Addr::ZERO);
                    i += 2;
                }
                "dev" if i + 1 < args.len() => {
                    dev = args[i+1];
                    i += 2;
                }
                _ => { i += 1; }
            }
        }
        let iface_idx = match stack.find_iface(dev) {
            Some(idx) => idx,
            None => { shell.println("Unknown interface."); return; }
        };
        if dest_str == "default" {
            match stack.routing.add_route(net::types::Ipv4Addr::ZERO, 0, gw, iface_idx) {
                Ok(()) => { shell.println("Default route added."); save_network_config(); }
                Err(_) => shell.println("Routing table full."),
            }
        } else if let Some((dest, prefix)) = net::types::Ipv4Addr::parse_cidr(dest_str) {
            match stack.routing.add_route(dest, prefix, gw, iface_idx) {
                Ok(()) => {
                    let msg = format!("Route {}/{} added.", dest, prefix);
                    shell.println(&msg);
                    save_network_config();
                }
                Err(_) => shell.println("Routing table full."),
            }
        } else {
            shell.println("Invalid destination. Use CIDR (e.g. 192.168.1.0/24) or 'default'.");
        }
    } else if args[0] == "del" {
        // ip route del default
        // ip route del 192.168.1.0/24
        if args.len() < 2 {
            shell.println("Usage: ip route del <dest>/<prefix>|default");
            return;
        }
        let dest_str = args[1];
        if dest_str == "default" {
            if stack.routing.del_route(net::types::Ipv4Addr::ZERO, 0) {
                shell.println("Default route deleted.");
                save_network_config();
            } else {
                shell.println("No default route.");
            }
        } else if let Some((dest, prefix)) = net::types::Ipv4Addr::parse_cidr(dest_str) {
            if stack.routing.del_route(dest, prefix) {
                shell.println("Route deleted.");
                save_network_config();
            } else {
                shell.println("Route not found.");
            }
        } else {
            shell.println("Invalid destination.");
        }
    } else {
        shell.println("Usage: ip route [show|add|del]");
    }
}

fn cmd_ip_link(shell: &mut ShellInstance, stack: &mut net::NetStack, args: &[&str]) {
    if args.is_empty() || args[0] == "show" {
        let dev = if args.len() >= 3 && args[1] == "dev" { Some(args[2]) } else { None };
        for i in 0..stack.iface_count {
            let iface = &stack.interfaces[i];
            if let Some(d) = dev {
                if iface.name_str() != d { continue; }
            }
            let flags = if iface.link_up { "UP" } else { "DOWN" };
            let msg = format!("{}: <{}> mtu 1500", iface.name_str(), flags);
            shell.println(&msg);
            let mac_str = format!("    link/ether {}", iface.mac);
            shell.println(&mac_str);
        }
    } else if args[0] == "set" && args.len() >= 3 {
        // ip link set eth0 up/down
        let dev = args[1];
        let state = args[2];
        match stack.find_iface(dev) {
            Some(idx) => {
                match state {
                    "up" => { stack.set_link_state(idx, true); shell.println("Link up."); }
                    "down" => { stack.set_link_state(idx, false); shell.println("Link down."); }
                    _ => shell.println("Usage: ip link set <iface> up|down"),
                }
            }
            None => shell.println("Unknown interface."),
        }
    } else {
        shell.println("Usage: ip link [show|set <iface> up|down]");
    }
}

fn save_network_config() {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => return,
    };
    let mut config = String::new();
    // Save interface configs
    for i in 0..stack.iface_count {
        let iface = &stack.interfaces[i];
        if iface.configured() {
            config.push_str("iface ");
            config.push_str(iface.name_str());
            config.push(' ');
            let addr = format!("{}/{}", iface.ip, iface.prefix_len);
            config.push_str(&addr);
            if let Some(vid) = iface.vlan {
                let vs = format!(" vlan={}", vid);
                config.push_str(&vs);
            }
            config.push('\n');
        }
    }
    // Save DNS
    if stack.dns_server != net::types::Ipv4Addr::ZERO {
        let dns = format!("dns {}\n", stack.dns_server);
        config.push_str(&dns);
    }
    // Save non-connected routes (connected routes are auto-generated from interface config)
    for e in &stack.routing.entries {
        if !e.active { continue; }
        if e.gateway == net::types::Ipv4Addr::ZERO { continue; } // skip connected routes
        let iface_name = if e.iface_index < stack.iface_count {
            stack.interfaces[e.iface_index].name_str()
        } else { "eth0" };
        if e.prefix_len == 0 && e.destination == net::types::Ipv4Addr::ZERO {
            let line = format!("route default via {} dev {}\n", e.gateway, iface_name);
            config.push_str(&line);
        } else {
            let line = format!("route {}/{} via {} dev {}\n", e.destination, e.prefix_len, e.gateway, iface_name);
            config.push_str(&line);
        }
    }
    fs::write_file_absolute("network.cfg", &config);
}

fn cmd_ping(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("Usage: ping <ip> [count]");
        return;
    }

    let parts: Vec<&str> = args.split_whitespace().collect();
    let target = match net::types::Ipv4Addr::parse(parts[0]) {
        Some(ip) => ip,
        None => {
            shell.println("Invalid IP address");
            return;
        }
    };
    let count: u32 = if parts.len() >= 2 {
        parts[1].parse().unwrap_or(4)
    } else {
        4
    };

    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => {
            shell.println("Network stack not initialized");
            return;
        }
    };

    if !stack.configured() {
        shell.println("Network not configured. Use: ifconfig <iface> <ip>/<prefix>");
        return;
    }

    let target_str = format!("{}", target);
    let msg = format!("PING {} - {} packets", target_str, count);
    shell.println(&msg);

    let mut sent = 0u32;
    let mut received = 0u32;

    for seq in 0..count {
        sent += 1;
        match stack.ping(target, 2000) {
            Ok(rtt) => {
                received += 1;
                let msg = format!("Reply from {}: time={}ms seq={}", target_str, rtt, seq);
                shell.println(&msg);
            }
            Err(e) => {
                let msg = format!("Request timed out: {}", e);
                shell.println(&msg);
            }
        }
        if seq + 1 < count {
            terminal::raw_sleep_ms(1000);
        }
    }

    let msg = format!("--- {} ping statistics ---", target_str);
    shell.println(&msg);
    let msg = format!("{} packets sent, {} received", sent, received);
    shell.println(&msg);
}

fn cmd_nslookup(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("Usage: nslookup <hostname>");
        return;
    }

    let name = args.trim();
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => {
            shell.println("Network stack not initialized");
            return;
        }
    };

    if !stack.configured() {
        shell.println("Network not configured. Use: ifconfig <iface> <ip>/<prefix>");
        return;
    }

    let dns_str = format!("{}", stack.dns_server);
    let msg = format!("Server: {}", dns_str);
    shell.println(&msg);

    match stack.dns_resolve(name, 5000) {
        Ok(ip) => {
            let ip_str = format!("{}", ip);
            let msg = format!("Name:    {}", name);
            shell.println(&msg);
            let msg = format!("Address: {}", ip_str);
            shell.println(&msg);
        }
        Err(e) => {
            let msg = format!("DNS lookup failed: {}", e);
            shell.println(&msg);
        }
    }
}

fn cmd_resolvectl(shell: &mut ShellInstance, args: &str) {
    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => { shell.println("Network stack not initialized"); return; }
    };

    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        shell.println("Usage: resolvectl status | dns [iface] <server> | query <hostname>");
        return;
    }

    match parts[0] {
        "status" => {
            // Show global DNS configuration
            if stack.dns_server == net::types::Ipv4Addr::ZERO {
                shell.println("Global DNS: (none)");
            } else {
                let msg = format!("Global DNS: {}", stack.dns_server);
                shell.println(&msg);
            }
            shell.println("");
            // Show per-link info
            for i in 0..stack.iface_count {
                let iface = &stack.interfaces[i];
                let flags = if iface.link_up { "UP" } else { "DOWN" };
                let msg = format!("Link {} ({}):", iface.name_str(), flags);
                shell.println(&msg);
                if iface.configured() {
                    let addr = format!("    Address: {}/{}", iface.ip, iface.prefix_len);
                    shell.println(&addr);
                }
                if stack.dns_server != net::types::Ipv4Addr::ZERO {
                    let dns_line = format!("    DNS: {}", stack.dns_server);
                    shell.println(&dns_line);
                } else {
                    shell.println("    DNS: (none)");
                }
            }
        }
        "dns" => {
            // resolvectl dns [iface] <server> [server2 ...]
            // Find the first valid IP in the args (skip interface name if present)
            let mut server_ip = None;
            for &part in &parts[1..] {
                if let Some(ip) = net::types::Ipv4Addr::parse(part) {
                    server_ip = Some(ip);
                    break;
                }
            }
            match server_ip {
                Some(ip) => {
                    stack.dns_server = ip;
                    save_network_config();
                    let msg = format!("DNS server set to {}", ip);
                    shell.println(&msg);
                }
                None => {
                    if stack.dns_server == net::types::Ipv4Addr::ZERO {
                        shell.println("Global DNS: (none)");
                    } else {
                        let msg = format!("Global DNS: {}", stack.dns_server);
                        shell.println(&msg);
                    }
                }
            }
        }
        "query" => {
            if parts.len() < 2 {
                shell.println("Usage: resolvectl query <hostname>");
                return;
            }
            let name = parts[1];
            if stack.dns_server == net::types::Ipv4Addr::ZERO {
                shell.println("No DNS server configured. Use: resolvectl dns <iface> <server>");
                return;
            }
            if !stack.configured() {
                shell.println("Network not configured.");
                return;
            }
            let msg = format!("Resolving {} via {}...", name, stack.dns_server);
            shell.println(&msg);
            match stack.dns_resolve(name, 5000) {
                Ok(ip) => {
                    let result = format!("{} -> {}", name, ip);
                    shell.println(&result);
                }
                Err(e) => {
                    let msg = format!("Resolution failed: {}", e);
                    shell.println(&msg);
                }
            }
        }
        _ => {
            shell.println("Usage: resolvectl status | dns [iface] <server> | query <hostname>");
        }
    }
}

fn cmd_sshd(shell: &mut ShellInstance, args: &str) {
    let port: u16 = if args.trim().is_empty() {
        22
    } else {
        match args.trim().parse() {
            Ok(p) => p,
            Err(_) => {
                shell.println("Invalid port number. Usage: sshd [port]");
                return;
            }
        }
    };
    ssh::server::run_sshd(port);
}

// KERN-023: httpd is a builtin command, so `httpd 8080 &` does NOT background it.
// Builtins execute inline in the kernel's main loop. To support background httpd,
// the server would need to be refactored into a WASI binary that runs as a spawned
// process, or the kernel would need cooperative multitasking for builtins.
fn cmd_httpd(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("Usage: httpd <port>");
        return;
    }

    let port: u16 = match args.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            shell.println("Invalid port number");
            return;
        }
    };

    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => {
            shell.println("Network stack not initialized");
            return;
        }
    };

    if !stack.configured() {
        shell.println("Network not configured. Use: ifconfig <iface> <ip>/<prefix>");
        return;
    }

    let listener = match stack.tcp_connections.listen(net::types::Ipv4Addr::ZERO, port) {
        Ok(idx) => idx,
        Err(e) => {
            let msg = format!("Failed to listen: {}", e);
            shell.println(&msg);
            return;
        }
    };

    let msg = format!("HTTP server listening on port {}. Ctrl+T to stop.", port);
    shell.println(&msg);

    // Accept loop
    loop {
        // Poll network (process any pending frames)
        stack.poll_rx();
        stack.poll_timers();

        // Try to accept (short timeout so we can check interrupts)
        match stack.tcp_accept(listener, 500) {
            Ok(conn) => {
                httpd_handle_connection(stack, conn);
            }
            Err(net::types::NetError::TimedOut) => {
                // No connection yet, loop and check interrupts
                continue;
            }
            Err(e) => {
                let msg = format!("Accept error: {}", e);
                shell.println(&msg);
                break;
            }
        }
    }

    // Close listener
    stack.tcp_close(listener);
    shell.println("HTTP server stopped.");
}

fn httpd_handle_connection(stack: &mut net::NetStack, conn: usize) {
    // Read request
    let mut request_data = Vec::new();
    let mut buf = [0u8; 1460];

    // Read until we have complete headers (look for \r\n\r\n)
    let deadline_ms = chrono::Utc::now().timestamp_millis() + 5000;
    loop {
        match stack.tcp_recv(conn, &mut buf, 1000) {
            Ok(0) => break,
            Ok(n) => {
                request_data.extend_from_slice(&buf[..n]);
                // Check if we have complete headers
                if request_data.windows(4).any(|w| w == b"\r\n\r\n") {
                    // Check Content-Length for body
                    if let Ok(text) = core::str::from_utf8(&request_data) {
                        if let Some(header_end) = text.find("\r\n\r\n") {
                            let headers = &text[..header_end];
                            let body_start = header_end + 4;
                            let mut content_length = 0usize;
                            for line in headers.split("\r\n") {
                                if line.to_lowercase().starts_with("content-length:") {
                                    if let Some(val) = line.split(':').nth(1) {
                                        content_length = val.trim().parse().unwrap_or(0);
                                    }
                                }
                            }
                            let body_received = request_data.len() - body_start;
                            if body_received >= content_length {
                                break; // full request received
                            }
                        }
                    }
                }
                if request_data.len() > 16384 {
                    break; // safety limit
                }
            }
            Err(_) => break,
        }
        if chrono::Utc::now().timestamp_millis() >= deadline_ms {
            break;
        }
    }

    if request_data.is_empty() {
        stack.tcp_close(conn);
        return;
    }

    // Parse request
    let request = match net::http::HttpRequest::parse(&request_data) {
        Some(r) => r,
        None => {
            let resp = net::http::HttpResponse::bad_request("Malformed request");
            let data = resp.serialize();
            let _ = stack.tcp_send(conn, &data);
            terminal::raw_sleep_ms(50);
            stack.tcp_close(conn);
            return;
        }
    };

    let msg = format!("{} {}", request.method, request.path);
    terminal::println(&msg);

    // Route request
    let response = match request.method.as_str() {
        "GET" => httpd_handle_get(&request.path),
        "POST" => httpd_handle_post(&request.path, &request.body),
        _ => net::http::HttpResponse::bad_request("Method not supported"),
    };

    let data = response.serialize();
    let _ = stack.tcp_send(conn, &data);

    // Give TCP time to flush
    terminal::raw_sleep_ms(100);
    stack.poll_timers();
    stack.poll_rx();

    stack.tcp_close(conn);
}

fn httpd_handle_get(path: &str) -> net::http::HttpResponse {
    if path == "/" {
        // Directory listing
        let entries = fs::list_dir(fs::get_cwd());
        let mut body = String::from("<html><head><title>Terminal OS File Server</title></head><body>\n");
        body.push_str("<h1>Files</h1>\n<ul>\n");
        for entry in &entries {
            if entry.is_dir {
                body.push_str(&format!("<li><a href=\"/{}\">{}/</a></li>\n", entry.name, entry.name));
            } else {
                body.push_str(&format!("<li><a href=\"/{}\">{}</a></li>\n", entry.name, entry.name));
            }
        }
        body.push_str("</ul>\n</body></html>");
        net::http::HttpResponse::ok(&body, "text/html")
    } else {
        // Serve file
        let filename = path.trim_start_matches('/');
        if let Some(content) = fs::read_file(filename) {
            let content_type = if filename.ends_with(".html") || filename.ends_with(".htm") {
                "text/html"
            } else if filename.ends_with(".json") {
                "application/json"
            } else if filename.ends_with(".py") {
                "text/x-python"
            } else {
                "text/plain"
            };
            net::http::HttpResponse::ok(content, content_type)
        } else {
            net::http::HttpResponse::not_found()
        }
    }
}

fn httpd_handle_post(path: &str, body: &[u8]) -> net::http::HttpResponse {
    let filename = path.trim_start_matches('/');
    if filename.is_empty() {
        return net::http::HttpResponse::bad_request("No filename specified");
    }
    let resolved = fs::resolve_path(filename);
    if fs::write_file_bytes_absolute(&resolved, body) {
        net::http::HttpResponse::ok("OK", "text/plain")
    } else {
        net::http::HttpResponse::new(500, "Internal Server Error")
            .with_header("Content-Type", "text/plain")
            .with_body(b"Failed to write file".to_vec())
    }
}

fn cmd_curl(shell: &mut ShellInstance, args: &str) {
    if args.is_empty() {
        shell.println("Usage: curl [-v] [-X METHOD] [-d DATA] [-H HEADER] <url>");
        return;
    }

    let stack = match net::NetStack::get() {
        Some(s) => s,
        None => {
            shell.println("Network stack not initialized");
            return;
        }
    };

    if !stack.configured() {
        shell.println("Network not configured. Use: ifconfig <iface> <ip>/<prefix>");
        return;
    }

    // Parse arguments — handle quoted strings for -d
    let mut verbose = false;
    let mut method_str = String::from("GET");
    let mut body_string: Option<String> = None;
    let mut extra_header_strings: Vec<(String, String)> = Vec::new();
    let mut url_string: Option<String> = None;

    // Simple tokenizer that handles quoted strings
    let tokens = tokenize_args(args);
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "-v" => verbose = true,
            "-X" => {
                i += 1;
                if i < tokens.len() {
                    method_str = tokens[i].clone();
                }
            }
            "-d" => {
                i += 1;
                if i < tokens.len() {
                    body_string = Some(tokens[i].clone());
                    if method_str == "GET" {
                        method_str = "POST".to_string();
                    }
                }
            }
            "-H" => {
                i += 1;
                if i < tokens.len() {
                    if let Some(colon) = tokens[i].find(':') {
                        let key = tokens[i][..colon].trim().to_string();
                        let value = tokens[i][colon + 1..].trim().to_string();
                        extra_header_strings.push((key, value));
                    }
                }
            }
            _ => {
                url_string = Some(tokens[i].clone());
            }
        }
        i += 1;
    }

    let method = method_str.as_str();
    let body_data = body_string.as_deref();

    let url = match &url_string {
        Some(u) => u.as_str(),
        None => {
            shell.println("No URL specified");
            return;
        }
    };

    let (host, port, path) = match net::http::parse_url(url) {
        Some(v) => v,
        None => {
            shell.println("Invalid URL");
            return;
        }
    };

    if verbose {
        let msg = format!("> {} {} HTTP/1.0", method, path);
        shell.println(&msg);
        let msg = format!("> Host: {}:{}", host, port);
        shell.println(&msg);
        shell.println(">");
    }

    let body_bytes = body_data.map(|s| s.as_bytes());
    let header_refs: Vec<(&str, &str)> = extra_header_strings.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();

    match net::http::http_request(
        stack,
        method,
        &host,
        port,
        &path,
        body_bytes,
        &header_refs,
    ) {
        Ok(response) => {
            if verbose {
                let msg = format!("< HTTP/1.0 {} {}", response.status_code, response.status_text);
                shell.println(&msg);
                for (k, v) in &response.headers {
                    let msg = format!("< {}: {}", k, v);
                    shell.println(&msg);
                }
                shell.println("<");
            }
            if let Ok(body_str) = core::str::from_utf8(&response.body) {
                shell.print(body_str);
                if !body_str.ends_with('\n') {
                    shell.println("");
                }
            } else {
                let msg = format!("[binary data, {} bytes]", response.body.len());
                shell.println(&msg);
            }
        }
        Err(e) => {
            let msg = format!("curl: {}", e);
            shell.println(&msg);
        }
    }
}

/// Tokenize a command-line string, handling double-quoted strings.
fn tokenize_args(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '"' {
            in_quotes = !in_quotes;
        } else if ch == ' ' && !in_quotes {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

// Keep the original add function for backwards compatibility
#[unsafe(no_mangle)]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
