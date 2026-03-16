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
pub mod peripheral;

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
    
    /// Sleeps for the specified duration in milliseconds.
    /// Blocks the terminal but not the game server.
    pub fn sleep(ms: u32) {
        unsafe { sleep_ms(ms as i32); }
    }

    /// Opens the visual programming editor on the client.
    pub fn open_visual() {
        unsafe { open_visual_editor(); }
    }
}

/// Redstone host functions for controlling redstone signals
pub mod redstone {
    extern "C" {
        /// Sets the redstone output power for a specific side.
        /// side: 0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT
        /// power: 0-15
        /// Returns 0 on success, -1 on failure.
        fn redstone_set_output(side: i32, power: i32) -> i32;
    }
    
    /// Relative side constants (relative to the terminal's facing direction)
    pub const DOWN: i32 = 0;   // Bottom of the terminal
    pub const UP: i32 = 1;     // Top of the terminal
    pub const FRONT: i32 = 2;  // Front of the terminal (the screen side)
    pub const BACK: i32 = 3;   // Back of the terminal
    pub const LEFT: i32 = 4;   // Left side of the terminal (when facing the front)
    pub const RIGHT: i32 = 5;  // Right side of the terminal (when facing the front)
    
    /// Sets the redstone output power for a specific side.
    /// 
    /// # Arguments
    /// * `side` - The relative side to output to (use constants: DOWN, UP, FRONT, BACK, LEFT, RIGHT)
    /// * `power` - The power level (0-15)
    /// 
    /// # Returns
    /// `true` on success, `false` on failure
    pub fn set_output(side: i32, power: i32) -> bool {
        unsafe { redstone_set_output(side, power) == 0 }
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

/// Prints the shell prompt
fn print_prompt() {
    print("> ");
}

/// Resets the OS to shell mode, clearing any running programs.
/// Called when Ctrl+T is pressed to terminate current program.
fn reset_to_shell() {
    unsafe {
        // Clear any running program state
        EDITOR = None;
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
        "ls" => cmd_ls(),
        "cat" => cmd_cat(args),
        "edit" => cmd_edit(args),
        "rm" => cmd_rm(args),
        "echo" => cmd_echo(args),
        "python" => cmd_python(args),
        "peripherals" => cmd_peripherals(args),
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
    println("  help           - Display this help message");
    println("  clear          - Clear the screen");
    println("  ls             - List files");
    println("  cat <file>     - Display file contents");
    println("  edit <file>    - Edit a file");
    println("  rm <file>      - Delete a file");
    println("  echo <text>    - Print text");
    println("  python         - Start Python REPL");
    println("  python <f>     - Run a Python script");
    println("  peripherals    - List CC peripherals");
    println("  peripherals <n>- Show methods for peripheral");
    println("  visual         - Open visual programming editor");
    println("");
    println("Python REPL:");
    println("  import terminal  - Access terminal functions");
    println("  import peripheral - Access CC peripherals");
    println("  terminal.write(s)      - Write text (no newline)");
    println("  terminal.println(s)    - Print line");
    println("  terminal.clear()       - Clear screen");
    println("  terminal.read_file(p)  - Read file contents");
    println("  terminal.write_file(p,c) - Write to file");
    println("  terminal.list_files()  - List all files");
    println("  peripheral.list()      - List peripherals");
    println("  peripheral.call(n,m,a) - Call method");
    println("  exit() or Ctrl+D       - Exit Python");
    println("");
    println("System shortcuts:");
    println("  Ctrl+T      - Terminate current program (kill)");
    println("");
    println("Editor shortcuts:");
    println("  Arrow keys  - Move cursor");
    println("  Ctrl+S      - Save file");
    println("  Ctrl+E      - Exit (prompts if unsaved)");
    println("  Ctrl+R      - Save and run file");
    println("  Ctrl+F      - Find text");
    println("  Ctrl+X      - Cut line");
    println("  Ctrl+C      - Copy line");
    println("  Ctrl+V      - Paste line");
    println("  Ctrl+D      - Delete line");
    println("  Ctrl+K      - Clear line");
    println("  Ctrl+A      - Select all");
}

/// Command: clear - Clear the screen
fn cmd_clear() {
    clear();
}

/// Command: ls - List files
fn cmd_ls() {
    println("");
    println("Files:");
    
    let files = fs::list_files();
    if files.is_empty() {
        println("  (no files)");
    } else {
        for line in files.split('\n') {
            if !line.is_empty() {
                print("  ");
                println(line);
            }
        }
    }
}

/// Command: cat - Display file contents
fn cmd_cat(args: &str) {
    if args.is_empty() {
        println("Usage: cat <filename>");
        return;
    }
    
    let filename = args.trim();
    
    if !fs::exists(filename) {
        print("File not found: ");
        println(filename);
        return;
    }
    
    if let Some(content) = fs::read_file(filename) {
        println("");
        println(content);
    } else {
        println("Error reading file.");
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

/// Command: rm - Delete a file
fn cmd_rm(args: &str) {
    if args.is_empty() {
        println("Usage: rm <filename>");
        return;
    }
    
    let filename = args.trim();
    
    if !fs::exists(filename) {
        print("File not found: ");
        println(filename);
        return;
    }
    
    if fs::delete_file(filename) {
        print("Deleted: ");
        println(filename);
    } else {
        println("Error deleting file.");
    }
}

/// Command: echo - Print text
fn cmd_echo(args: &str) {
    println("");
    println(args);
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

// Keep the original add function for backwards compatibility
#[unsafe(no_mangle)]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
