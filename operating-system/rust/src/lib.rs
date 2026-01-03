//! Terminal OS - A simple operating system for the WASM terminal.
//!
//! Provides a shell interface with built-in programs including:
//! - help: List available commands
//! - edit: A CC:Tweaked-style text editor
//! - ls: List files
//! - clear: Clear the screen
//! - cat: Display file contents

#![no_std]

use core::panic::PanicInfo;

mod fs;
mod editor;

/// Panic handler - required for no_std
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {}
}

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
}

use terminal::{print, println, clear};
use editor::{Editor, ExitResult};

/// OS State
#[derive(Clone, Copy, PartialEq)]
enum OsState {
    /// Normal shell mode
    Shell,
    /// Running the editor
    Editor,
}

/// Global OS state
static mut OS_STATE: OsState = OsState::Shell;

/// Global editor instance (needed because we can't allocate)
static mut EDITOR: Option<Editor> = None;

/// Shell input buffer for line-based input
static mut SHELL_INPUT: [u8; 256] = [0u8; 256];
static mut SHELL_INPUT_LEN: usize = 0;

/// Main entry point - called when the terminal is opened.
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

/// Called when the user enters a line of input.
#[unsafe(no_mangle)]
pub fn on_input(ptr: *const u8, len: usize) {
    // Read the input string from memory
    let input = unsafe {
        let slice = core::slice::from_raw_parts(ptr, len);
        core::str::from_utf8_unchecked(slice)
    };
    
    unsafe {
        match OS_STATE {
            OsState::Shell => handle_shell_input(input),
            OsState::Editor => handle_editor_input(input),
        }
    }
}

/// Handles input in shell mode - buffers characters until Enter is pressed
fn handle_shell_input(input: &str) {
    let bytes = input.as_bytes();
    
    unsafe {
        for &byte in bytes {
            match byte {
                b'\n' | b'\r' => {
                    // Enter pressed - process the buffered command
                    println("");
                    
                    if SHELL_INPUT_LEN > 0 {
                        // Get the command string
                        let cmd = core::str::from_utf8_unchecked(&SHELL_INPUT[..SHELL_INPUT_LEN]);
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
                        let char_slice = core::slice::from_raw_parts(&byte, 1);
                        if let Ok(s) = core::str::from_utf8(char_slice) {
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

/// Copies a filename to a static buffer (needed because editor will be dropped)
fn copy_filename(filename: &str) -> &'static str {
    static mut FILENAME_BUFFER: [u8; 64] = [0u8; 64];
    
    unsafe {
        let bytes = filename.as_bytes();
        let len = bytes.len().min(FILENAME_BUFFER.len());
        FILENAME_BUFFER[..len].copy_from_slice(&bytes[..len]);
        core::str::from_utf8_unchecked(&FILENAME_BUFFER[..len])
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
    println("  help        - Display this help message");
    println("  clear       - Clear the screen");
    println("  ls          - List files");
    println("  cat <file>  - Display file contents");
    println("  edit <file> - Edit a file");
    println("  rm <file>   - Delete a file");
    println("  echo <text> - Print text");
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

// Keep the original add function for backwards compatibility
#[unsafe(no_mangle)]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
