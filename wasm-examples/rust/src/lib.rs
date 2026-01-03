// Terminal host functions provided by the Minecraft mod
extern "C" {
    /// Writes a string to the terminal.
    /// ptr: pointer to the UTF-8 string in WASM memory
    /// len: length of the string in bytes
    /// Returns: number of bytes written, or -1 on error
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
fn print(s: &str) {
    unsafe {
        terminal_write(s.as_ptr(), s.len());
    }
}

/// Helper function to print a line (with newline).
fn println(s: &str) {
    print(s);
    print("\n");
}

/// Main entry point - called when the terminal is opened.
#[unsafe(no_mangle)]
pub fn main() {
    // Clear the screen first
    unsafe { terminal_clear(); }
    
    // Print a welcome message
    println("================================================================================");
    println("                    MINECRAFT WASM TERMINAL v1.0                               ");
    println("================================================================================");
    println("");
    println("Welcome to the WASM-powered terminal!");
    println("");
    println("This terminal is running WebAssembly code inside Minecraft.");
    println("The text you see is being written by Rust code compiled to WASM.");
    println("");
    
    // Show terminal dimensions
    unsafe {
        let width = terminal_get_width();
        let height = terminal_get_height();
        print("Terminal size: ");
        print_int(width);
        print("x");
        print_int(height);
        println("");
    }
    
    println("");
    println("Type something and press Enter to test input handling.");
    println("");
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
    
    // Echo the input back
    print("You typed: ");
    println(input);
    println("");
    
    // Handle some basic commands
    if input == "clear" {
        unsafe { terminal_clear(); }
        println("Screen cleared.");
    } else if input == "help" {
        println("Available commands:");
        println("  clear  - Clear the screen");
        println("  help   - Show this help message");
        println("  hello  - Print a greeting");
    } else if input == "hello" {
        println("Hello from WASM! :)");
    }
    
    println("");
    print("> ");
}

/// Simple integer printing (no std library)
fn print_int(mut n: i32) {
    if n < 0 {
        print("-");
        n = -n;
    }
    if n == 0 {
        print("0");
        return;
    }
    
    let mut digits = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    
    while i > 0 {
        i -= 1;
        let c = [digits[i]];
        unsafe {
            terminal_write(c.as_ptr(), 1);
        }
    }
}

// Keep the original add function for backwards compatibility
#[unsafe(no_mangle)]
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
