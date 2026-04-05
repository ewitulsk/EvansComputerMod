//! Terminal OS - A simple operating system for the WASM terminal.
//!
//! Provides a shell interface with built-in programs including:
//! - help: List available commands
//! - ls: List files
//! - clear: Clear the screen
//! - cat: Display file contents

mod fs;
mod git;
mod crypto;
mod ssh;
pub mod shell;
pub mod peripheral;
pub mod interrupt;
pub mod modules;
pub use ecm_net as net;
pub mod framebuffer;
pub mod gfx;
pub mod gfx_test;
pub mod vte;

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
/// Set before running git commands, ssh client, etc.
/// This lets deeply-nested code route output through the correct shell
/// (local terminal or SSH session) without threading `&mut ShellInstance`
/// through every function signature.
pub(crate) static mut ACTIVE_SHELL: Option<*mut shell::ShellInstance> = None;

/// Terminal abstraction layer.
///
/// In the old architecture, these functions called host functions directly
/// (terminal_write, terminal_clear, etc.). Now they route through the
/// physical VTE instance which writes to the memory-mapped framebuffer.
/// The host reads the framebuffer and renders it — no host function needed
/// for display output.
pub(crate) mod terminal {
    extern "C" {
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

        /// Hint to the host to read the framebuffer now (for low-latency sync).
        fn fb_sync();
    }

    /// Write a string to the physical terminal via the VTE.
    pub fn print(s: &str) {
        unsafe {
            if let Some(ref mut vte) = crate::PHYSICAL_VTE {
                vte.write_str(s);
            }
        }
    }

    /// Print a line (with newline) to the physical terminal via the VTE.
    pub fn println(s: &str) {
        print(s);
        print("\n");
    }

    /// Clear the terminal screen (via ANSI escape to VTE).
    pub fn clear() {
        print("\x1b[2J\x1b[H");
    }

    /// Set cursor position (via ANSI escape to VTE).
    /// x = column (0-based), y = row (0-based).
    pub fn set_cursor(x: i32, y: i32) {
        // ANSI CUP is 1-based
        let seq = format!("\x1b[{};{}H", y + 1, x + 1);
        print(&seq);
    }

    /// Gets the terminal width from the framebuffer header.
    pub fn get_width() -> i32 {
        crate::framebuffer::width() as i32
    }

    /// Gets the terminal height from the framebuffer header.
    pub fn get_height() -> i32 {
        crate::framebuffer::height() as i32
    }

    /// Sleeps for the specified duration in milliseconds (raw, no interrupt polling).
    pub fn raw_sleep_ms(ms: i32) {
        unsafe { sleep_ms(ms); }
    }

    /// Sleeps for the specified duration in milliseconds.
    /// Sleeps are chunked in 10ms intervals to allow interrupt delivery.
    pub fn sleep(ms: u32) {
        let mut remaining = ms as i32;
        while remaining > 0 {
            let chunk = if remaining > 10 { 10 } else { remaining };
            raw_sleep_ms(chunk);
            remaining -= chunk;
            yield_interrupts();
        }
    }

    /// Trigger an immediate framebuffer sync to the host.
    pub fn sync() {
        unsafe { fb_sync(); }
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
    /// Blocks until Enter is pressed.
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

use shell::{ShellInstance, OsState};
use vte::Vte;

/// The local terminal's shell instance.
static mut LOCAL_SHELL: Option<ShellInstance> = None;

/// Physical VTE instance — writes to the memory-mapped framebuffer at 0x20000.
/// All terminal output (print, clear, cursor movement) routes through this.
pub(crate) static mut PHYSICAL_VTE: Option<Vte> = None;

/// Main entry point - called when the terminal is opened.
#[cfg(all(target_arch = "wasm32", not(test)))]
#[unsafe(no_mangle)]
pub fn main() {
    // Initialize the memory-mapped framebuffer
    framebuffer::init();
    unsafe {
        PHYSICAL_VTE = Some(Vte::new_physical(
            framebuffer::DEFAULT_WIDTH,
            framebuffer::DEFAULT_HEIGHT,
        ));
    }

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
                OsState::Ssh => ssh::client::handle_ssh_client_input(shell, input),
            }
        }
    }
}

/// Internal interrupt dispatch logic for Rust-level handlers.
fn dispatch_interrupt(irq: i32, data: &str) {
    // IRQ_TERMINATE is non-maskable — always resets to shell
    if irq == interrupt::IRQ_TERMINATE {
        reset_to_shell();
        return;
    }

    // Dispatch to Rust-level handler if registered
    interrupt::dispatch_rust(irq, data);

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
            "visual" | "httpd" => {
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
    // Only shell intrinsics remain as builtins — all programs are now WASI binaries
    shell.with_cwd(|shell| {
        match command {
            "exit" => {
                if shell.is_ssh {
                    shell.println("Connection closed.");
                    shell.exited = true;
                } else {
                    shell.println("Use Ctrl+T to exit.");
                }
            }
            "cd" => cmd_cd(shell, args),
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
            "gfxtest" => {
                let arg_list: Vec<&str> = if args.is_empty() {
                    Vec::new()
                } else {
                    args.split_whitespace().collect()
                };
                gfx_test::run(&arg_list);
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
                            // Resync VTE cursor from framebuffer header
                            // (child process wrote directly to FB, bypassing VTE)
                            if let Some(ref mut vte) = PHYSICAL_VTE {
                                vte.resync_cursor_from_header();
                            }
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

    // Print blank line after command output (if still in shell mode)
    if shell.state == OsState::Shell {
        shell.println("");
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

/// Print raw bytes to the kernel's VTE terminal.
/// Called by the host to display child process output on the framebuffer.
#[unsafe(no_mangle)]
pub fn terminal_print(ptr: *const u8, len: usize) {
    let data = unsafe { core::slice::from_raw_parts(ptr, len) };
    unsafe {
        if let Some(ref mut vte) = PHYSICAL_VTE {
            vte.write(data);
        }
    }
}

// Keep the original add function for backwards compatibility
#[unsafe(no_mangle)]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
