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
pub mod peripheral;
pub mod interrupt;

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

/// Terminal host functions provided by the Minecraft mod
pub mod terminal {
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

use terminal::{print, println, clear};
use editor::{Editor, ExitResult};
use python::PythonRepl;

/// OS State
#[derive(Clone, Copy, PartialEq)]
enum OsState {
    /// Normal shell mode
    Shell,
    /// Running the editor
    Editor,
    /// Running the Python REPL
    Python,
}

/// Global OS state
static mut OS_STATE: OsState = OsState::Shell;

/// Global editor instance (needed because we can't allocate)
static mut EDITOR: Option<Editor> = None;

/// Global Python REPL instance
static mut PYTHON_REPL: Option<PythonRepl> = None;

/// Shell input buffer for line-based input
static mut SHELL_INPUT: [u8; 256] = [0u8; 256];
static mut SHELL_INPUT_LEN: usize = 0;

/// Main entry point - called when the terminal is opened.
#[cfg(all(target_arch = "wasm32", not(test)))]
#[unsafe(no_mangle)]
pub fn main() {
    clear();
    
    println("================================================================================");
    println("                         TERMINAL OS v1.0                                      ");
    println("================================================================================");
    println("");
    println("Welcome to Terminal OS!");
    println("Type 'help' for a list of available commands.");
    println("");
    print_prompt();
}

/// Prints the shell prompt with CWD
fn print_prompt() {
    let cwd = fs::get_cwd();
    if cwd.is_empty() {
        print("/ > ");
    } else {
        print("/");
        print(cwd);
        print(" > ");
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
        SHELL_INPUT_LEN = 0;
        
        // Reset to shell mode
        OS_STATE = OsState::Shell;
        
        // Clear screen and show message
        clear();
        println("^T - Program terminated");
        println("");
        print_prompt();
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
        match OS_STATE {
            OsState::Shell => handle_shell_input(input),
            OsState::Editor => handle_editor_input(input),
            OsState::Python => handle_python_input(input),
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
fn handle_shell_input(input: &str) {
    let bytes = input.as_bytes();
    
    unsafe {
        for &byte in bytes {
            // Check for Ctrl+T (0x14) - terminate/reset
            if byte == 0x14 {
                reset_to_shell();
                return;
            }
            
            match byte {
                b'\n' | b'\r' => {
                    // Enter pressed - process the buffered command
                    println("");
                    
                    if SHELL_INPUT_LEN > 0 {
                        // Get the command string
                        let cmd = std::str::from_utf8_unchecked(&SHELL_INPUT[..SHELL_INPUT_LEN]);
                        process_command(cmd);
                    }
                    
                    // Clear the buffer
                    SHELL_INPUT_LEN = 0;
                    
                    // Print prompt if still in shell mode
                    if OS_STATE == OsState::Shell {
                        print_prompt();
                    }
                }
                8 | 127 => {
                    // Backspace - delete last character
                    if SHELL_INPUT_LEN > 0 {
                        SHELL_INPUT_LEN -= 1;
                        // Erase character on screen: move back, print space, move back
                        print("\x08 \x08");
                    }
                }
                _ if byte >= 32 && byte < 127 => {
                    // Printable character - add to buffer and echo
                    if SHELL_INPUT_LEN < SHELL_INPUT.len() {
                        SHELL_INPUT[SHELL_INPUT_LEN] = byte;
                        SHELL_INPUT_LEN += 1;
                        // Echo the character
                        let char_slice = std::slice::from_raw_parts(&byte, 1);
                        if let Ok(s) = std::str::from_utf8(char_slice) {
                            print(s);
                        }
                    }
                }
                _ => {
                    // Ignore other characters (escape sequences, etc.)
                }
            }
        }
    }
}

/// Processes a complete command line
fn process_command(input: &str) {
    let input = input.trim();
    
    if input.is_empty() {
        return;
    }
    
    // Parse command and arguments
    let (command, args) = parse_command(input);
    
    match command {
        "help" => cmd_help(),
        "clear" => cmd_clear(),
        "ls" => cmd_ls(args),
        "cat" => cmd_cat(args),
        "edit" => cmd_edit(args),
        "rm" => cmd_rm(args),
        "echo" => cmd_echo(args),
        "python" => cmd_python(args),
        "peripherals" => cmd_peripherals(args),
        "cd" => cmd_cd(args),
        "pwd" => cmd_pwd(),
        "mkdir" => cmd_mkdir(args),
        "cp" => cmd_cp(args),
        "mv" => cmd_mv(args),
        "touch" => cmd_touch(args),
        "git" => cmd_git(args),
        "visual" => {
            println("Opening visual editor...");
            terminal::open_visual();
        }
        _ => {
            print("Unknown command: ");
            println(command);
            println("Type 'help' for a list of commands.");
        }
    }
    
    // Print blank line after command output (if still in shell mode)
    unsafe {
        if OS_STATE == OsState::Shell {
            println("");
        }
    }
}

/// Handles input in editor mode
fn handle_editor_input(input: &str) {
    // Check for Ctrl+T (0x14) - terminate/reset
    for &byte in input.as_bytes() {
        if byte == 0x14 {
            reset_to_shell();
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
                OS_STATE = OsState::Shell;
                EDITOR = None;
                clear();
                
                match exit_result {
                    ExitResult::ExitAndRun => {
                        if let Some(filename) = run_filename {
                            print("Running: ");
                            println(filename);
                            println("");
                            // TODO: Actually run the file when script execution is implemented
                            println("(Script execution not yet implemented)");
                            println("");
                        }
                    }
                    _ => {
                        println("Exited editor.");
                        println("");
                    }
                }
                
                print_prompt();
            }
        }
    }
}

/// Handles input in Python REPL mode
fn handle_python_input(input: &str) {
    // Buffer for Python input line
    static mut PYTHON_INPUT: [u8; 1024] = [0u8; 1024];
    static mut PYTHON_INPUT_LEN: usize = 0;
    
    let bytes = input.as_bytes();
    
    unsafe {
        for &byte in bytes {
            // Check for Ctrl+T (0x14) - terminate/reset
            if byte == 0x14 {
                PYTHON_INPUT_LEN = 0;
                reset_to_shell();
                return;
            }
            
            match byte {
                b'\n' | b'\r' => {
                    // Enter pressed - send line to Python REPL
                    println("");
                    
                    if let Some(ref mut repl) = PYTHON_REPL {
                        let line = std::str::from_utf8_unchecked(&PYTHON_INPUT[..PYTHON_INPUT_LEN]);
                        let should_exit = repl.handle_input(line);
                        
                        if should_exit {
                            // Exit Python, return to shell
                            OS_STATE = OsState::Shell;
                            PythonRepl::clear_interrupt_handlers();
                            PYTHON_REPL = None;
                            println("");
                            println("Exited Python.");
                            println("");
                            print_prompt();
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
                        print("\x08 \x08");
                    }
                }
                4 => {
                    // Ctrl+D - exit Python
                    if let Some(ref mut repl) = PYTHON_REPL {
                        repl.handle_input("\x04");
                    }
                    OS_STATE = OsState::Shell;
                    PythonRepl::clear_interrupt_handlers();
                    PYTHON_REPL = None;
                    println("");
                    println("Exited Python.");
                    println("");
                    print_prompt();
                }
                _ if byte >= 32 && byte < 127 => {
                    // Printable character
                    if PYTHON_INPUT_LEN < PYTHON_INPUT.len() {
                        PYTHON_INPUT[PYTHON_INPUT_LEN] = byte;
                        PYTHON_INPUT_LEN += 1;
                        // Echo the character
                        let char_slice = std::slice::from_raw_parts(&byte, 1);
                        if let Ok(s) = std::str::from_utf8(char_slice) {
                            print(s);
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
fn cmd_help() {
    println("");
    println("Available commands:");
    println("");
    println("  help              - Display this help message");
    println("  clear             - Clear the screen");
    println("  ls [path] [-l]    - List directory contents");
    println("  cd [dir]          - Change directory (no arg = root)");
    println("  pwd               - Print working directory");
    println("  mkdir [-p] <dir>  - Create directory");
    println("  cat <file> ...    - Display file contents");
    println("  edit <file>       - Edit a file");
    println("  touch <file>      - Create empty file");
    println("  cp <src> <dst>    - Copy file");
    println("  mv <src> <dst>    - Move/rename file");
    println("  rm [-r] <file>... - Delete files or directories");
    println("  echo [-n|-e] text - Print text");
    println("  python [file]     - Start Python REPL or run script");
    println("  git <command>     - Version control (init/add/commit/log/...)");
    println("  peripherals [name]- List peripherals or methods");
    println("  visual            - Open visual programming editor");
    println("");
    println("System shortcuts:");
    println("  Ctrl+T      - Terminate current program (kill)");
    println("");
    println("Editor shortcuts:");
    println("  Ctrl+S  Save   Ctrl+E  Exit   Ctrl+R  Save & run");
    println("  Ctrl+F  Find   Ctrl+X  Cut    Ctrl+C  Copy");
    println("  Ctrl+V  Paste  Ctrl+D  Delete Ctrl+K  Clear line");
}

/// Command: clear - Clear the screen
fn cmd_clear() {
    clear();
}

/// Command: ls - List directory contents
fn cmd_ls(args: &str) {
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
            print("ls: cannot access '");
            print(target);
            println("': No such file or directory");
            return;
        }
        println("  (empty)");
        return;
    }

    for entry in &entries {
        if show_long {
            if entry.is_dir {
                print("  d  ---     ");
            } else {
                print("  f  ");
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
                            print(" ");
                        }
                        print(&size_str);
                        print(" ");
                    }
                    None => print("      ? "),
                }
            }
        } else {
            print("  ");
        }
        print(&entry.name);
        if entry.is_dir {
            print("/");
        }
        println("");
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
fn cmd_cat(args: &str) {
    if args.is_empty() {
        println("Usage: cat <file> [file2] ...");
        return;
    }

    for filename in args.split_whitespace() {
        if !fs::exists(filename) {
            print("cat: ");
            print(filename);
            println(": No such file or directory");
            continue;
        }
        if fs::is_dir(filename) {
            print("cat: ");
            print(filename);
            println(": Is a directory");
            continue;
        }
        if let Some(content) = fs::read_file(filename) {
            print(content);
            // Add newline if content doesn't end with one
            if !content.ends_with('\n') {
                println("");
            }
        } else {
            print("cat: ");
            print(filename);
            println(": Error reading file");
        }
    }
}

/// Command: edit - Open file in editor
fn cmd_edit(args: &str) {
    let filename = args.trim();
    
    unsafe {
        // Create a new editor
        let mut editor = Editor::new();
        
        if !filename.is_empty() {
            editor.open(filename);
        }
        
        // Switch to editor mode
        EDITOR = Some(editor);
        OS_STATE = OsState::Editor;
        
        // Render the editor
        if let Some(ref editor) = EDITOR {
            editor.render();
        }
    }
}

/// Command: rm - Delete files or directories
fn cmd_rm(args: &str) {
    if args.is_empty() {
        println("Usage: rm [-r] <file> [file2] ...");
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
        println("Usage: rm [-r] <file> [file2] ...");
        return;
    }

    for target in &targets {
        if !fs::exists(target) && !fs::is_dir(target) {
            print("rm: ");
            print(target);
            println(": No such file or directory");
            continue;
        }

        if fs::is_dir(target) {
            if !recursive {
                print("rm: ");
                print(target);
                println(": Is a directory (use -r to remove)");
                continue;
            }
            // Recursively delete directory contents
            rm_recursive(target);
        } else {
            if !fs::delete_file(target) {
                print("rm: cannot remove '");
                print(target);
                println("'");
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
fn cmd_echo(args: &str) {
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
        print(&output);
    } else {
        print(text);
    }

    if !no_newline {
        println("");
    }
}

/// Command: python - Start Python REPL or run a Python file
fn cmd_python(args: &str) {
    let filename = args.trim();
    
    if filename.is_empty() {
        // No filename - start interactive REPL
        unsafe {
            // Create a new Python REPL
            let repl = PythonRepl::new();
            
            // Show the banner
            repl.show_banner();
            
            // Store the REPL and switch to Python mode
            PYTHON_REPL = Some(repl);
            OS_STATE = OsState::Python;
            
            // Print the initial prompt
            if let Some(ref repl) = PYTHON_REPL {
                repl.print_prompt();
            }
        }
    } else {
        // Filename provided - execute the file
        if !fs::exists(filename) {
            print("File not found: ");
            println(filename);
            return;
        }
        
        if let Some(code) = fs::read_file(filename) {
            print("Running: ");
            println(filename);
            println("");
            
            // Create a temporary Python interpreter and run the file
            let mut repl = PythonRepl::new();
            repl.run_file(code, filename);
            
            // Interpreter is dropped here, returning to shell
        } else {
            print("Error reading file: ");
            println(filename);
        }
    }
}

/// Command: peripherals - List CC peripherals or show methods
fn cmd_peripherals(args: &str) {
    let arg = args.trim();
    
    if arg.is_empty() {
        // List all peripherals
        let peripherals = peripheral::list();
        
        if peripherals.is_empty() {
            println("");
            println("No peripherals connected.");
            println("Place CC:Tweaked peripheral blocks adjacent to this terminal.");
        } else {
            println("");
            println("Connected peripherals:");
            println("");
            for p in &peripherals {
                print("  ");
                print(&p.name);
                print(" (");
                print(&p.peripheral_type);
                print(") - ");
                println(&p.side);
            }
        }
    } else {
        // Show methods for a specific peripheral
        match peripheral::get_methods(arg) {
            Ok(methods) => {
                println("");
                print("Methods for ");
                print(arg);
                println(":");
                println("");
                for method in &methods {
                    print("  ");
                    println(method);
                }
                if methods.is_empty() {
                    println("  (no methods)");
                }
            }
            Err(e) => {
                print("Error: ");
                println(&e);
            }
        }
    }
}

/// Command: cd - Change directory
fn cmd_cd(args: &str) {
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
        print("cd: ");
        print(target);
        println(": No such directory");
        return;
    }

    fs::set_cwd(&resolved);
}

/// Command: pwd - Print working directory
fn cmd_pwd() {
    let cwd = fs::get_cwd();
    if cwd.is_empty() {
        println("/");
    } else {
        print("/");
        println(cwd);
    }
}

/// Command: mkdir - Create directory
fn cmd_mkdir(args: &str) {
    if args.is_empty() {
        println("Usage: mkdir [-p] <dir>");
        return;
    }

    // -p flag is implicitly supported since host creates parents
    let dirname = args.trim().trim_start_matches("-p").trim();
    if dirname.is_empty() {
        println("Usage: mkdir [-p] <dir>");
        return;
    }

    if fs::mkdir(dirname) {
        // silent success (like Unix mkdir)
    } else {
        print("mkdir: cannot create directory '");
        print(dirname);
        println("'");
    }
}

/// Command: cp - Copy file
fn cmd_cp(args: &str) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() != 2 {
        println("Usage: cp <source> <destination>");
        return;
    }

    let src = parts[0];
    let dst = parts[1];

    if !fs::exists(src) {
        print("cp: ");
        print(src);
        println(": No such file or directory");
        return;
    }

    if let Some(content) = fs::read_file(src) {
        if !fs::write_file(dst, content) {
            print("cp: cannot create '");
            print(dst);
            println("'");
        }
    } else {
        print("cp: error reading '");
        print(src);
        println("'");
    }
}

/// Command: mv - Move/rename file
fn cmd_mv(args: &str) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() != 2 {
        println("Usage: mv <source> <destination>");
        return;
    }

    let src = parts[0];
    let dst = parts[1];

    if !fs::exists(src) {
        print("mv: ");
        print(src);
        println(": No such file or directory");
        return;
    }

    if let Some(content) = fs::read_file(src) {
        if fs::write_file(dst, content) {
            fs::delete_file(src);
        } else {
            print("mv: cannot create '");
            print(dst);
            println("'");
        }
    } else {
        print("mv: error reading '");
        print(src);
        println("'");
    }
}

/// Command: touch - Create empty file
fn cmd_touch(args: &str) {
    if args.is_empty() {
        println("Usage: touch <file>");
        return;
    }

    let filename = args.trim();
    if !fs::exists(filename) {
        fs::write_file(filename, "");
    }
    // If file exists, touch does nothing (we don't have timestamps)
}

/// Command: git - Version control
fn cmd_git(args: &str) {
    if args.is_empty() {
        terminal::println("usage: git <command> [args]");
        terminal::println("");
        terminal::println("Commands:");
        terminal::println("  init             Initialize a new repository");
        terminal::println("  add <file>       Stage files for commit");
        terminal::println("  status           Show working tree status");
        terminal::println("  commit -m <msg>  Record changes");
        terminal::println("  log              Show commit history");
        terminal::println("  branch [name]    List or create branches");
        terminal::println("  checkout <branch> Switch branches");
        terminal::println("  rebase <branch>  Rebase current branch onto target");
        terminal::println("  diff             Show unstaged changes");
        return;
    }

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
        _ => {
            terminal::print("git: '");
            terminal::print(subcmd);
            terminal::println("' is not a git command");
        }
    }
}

// Keep the original add function for backwards compatibility
#[unsafe(no_mangle)]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
