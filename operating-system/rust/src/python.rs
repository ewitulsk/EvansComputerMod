//! Python REPL integration using RustPython.
//!
//! Provides an embedded Python interpreter with a custom `terminal` module
//! that exposes the host functions for terminal I/O and file system access.

use rustpython_vm::{
    Interpreter,
    Settings,
    builtins::PyStrRef,
    pymodule,
    VirtualMachine,
    AsObject,
    compiler::Mode,
    scope::Scope,
};

use crate::terminal;
use crate::fs;
use crate::redstone;
use crate::peripheral;

/// The terminal module exposed to Python.
/// Provides functions for terminal I/O and file system access.
#[pymodule]
pub mod terminal_module {
    use super::*;

    /// Write text to the terminal (no newline).
    /// 
    /// Example:
    ///     terminal.write("Hello ")
    ///     terminal.write("World!")
    #[pyfunction]
    fn write(s: PyStrRef) {
        terminal::print(s.as_str());
    }

    /// Print text to the terminal with a newline.
    /// 
    /// Example:
    ///     terminal.println("Hello World!")
    #[pyfunction]
    fn println(s: PyStrRef) {
        terminal::println(s.as_str());
    }

    /// Clear the terminal screen.
    /// 
    /// Example:
    ///     terminal.clear()
    #[pyfunction]
    fn clear() {
        terminal::clear();
    }

    /// Set the cursor position.
    /// 
    /// Args:
    ///     x: Column (0-based)
    ///     y: Row (0-based)
    /// 
    /// Example:
    ///     terminal.set_cursor(0, 0)  # Move to top-left
    #[pyfunction]
    fn set_cursor(x: i32, y: i32) {
        terminal::set_cursor(x, y);
    }

    /// Get the terminal width in characters.
    /// 
    /// Returns:
    ///     int: Terminal width (usually 80)
    /// 
    /// Example:
    ///     width = terminal.get_width()
    #[pyfunction]
    fn get_width() -> i32 {
        terminal::get_width()
    }

    /// Get the terminal height in characters.
    /// 
    /// Returns:
    ///     int: Terminal height (usually 24)
    /// 
    /// Example:
    ///     height = terminal.get_height()
    #[pyfunction]
    fn get_height() -> i32 {
        terminal::get_height()
    }

    /// Read a file's contents.
    /// 
    /// Args:
    ///     path: Filename to read
    /// 
    /// Returns:
    ///     str or None: File contents, or None if file doesn't exist
    /// 
    /// Example:
    ///     content = terminal.read_file("test.txt")
    ///     if content is not None:
    ///         print(content)
    #[pyfunction]
    fn read_file(path: PyStrRef) -> Option<String> {
        fs::read_file(path.as_str()).map(|s| s.to_string())
    }

    /// Write content to a file.
    /// 
    /// Args:
    ///     path: Filename to write
    ///     content: Content to write
    /// 
    /// Returns:
    ///     bool: True if successful, False otherwise
    /// 
    /// Example:
    ///     success = terminal.write_file("test.txt", "Hello!")
    #[pyfunction]
    fn write_file(path: PyStrRef, content: PyStrRef) -> bool {
        fs::write_file(path.as_str(), content.as_str())
    }

    /// Check if a file exists.
    /// 
    /// Args:
    ///     path: Filename to check
    /// 
    /// Returns:
    ///     bool: True if file exists
    /// 
    /// Example:
    ///     if terminal.file_exists("test.txt"):
    ///         print("File exists!")
    #[pyfunction]
    fn file_exists(path: PyStrRef) -> bool {
        fs::exists(path.as_str())
    }

    /// Delete a file.
    /// 
    /// Args:
    ///     path: Filename to delete
    /// 
    /// Returns:
    ///     bool: True if successful
    /// 
    /// Example:
    ///     terminal.delete_file("test.txt")
    #[pyfunction]
    fn delete_file(path: PyStrRef) -> bool {
        fs::delete_file(path.as_str())
    }

    /// List all files in the computer's storage.
    /// 
    /// Returns:
    ///     str: Newline-separated list of filenames
    /// 
    /// Example:
    ///     files = terminal.list_files()
    ///     print(files)
    #[pyfunction]
    fn list_files() -> String {
        fs::list_files().to_string()
    }

    /// Get the size of a file in bytes.
    /// 
    /// Args:
    ///     path: Filename
    /// 
    /// Returns:
    ///     int or None: File size, or None if file doesn't exist
    /// 
    /// Example:
    ///     size = terminal.file_size("test.txt")
    #[pyfunction]
    fn file_size(path: PyStrRef) -> Option<usize> {
        fs::get_size(path.as_str())
    }
    
    // ==================== Sleep Function ====================
    
    /// Sleep for the specified number of seconds.
    /// Blocks the terminal but not the game server.
    /// 
    /// Args:
    ///     seconds: Time to sleep (can be fractional, e.g., 0.5 for 500ms)
    /// 
    /// Example:
    ///     terminal.sleep(1.0)   # Sleep for 1 second
    ///     terminal.sleep(0.5)   # Sleep for 500ms
    #[pyfunction]
    fn sleep(seconds: f64) {
        let ms = (seconds * 1000.0) as u32;
        terminal::sleep(ms);
    }
    
    // ==================== Redstone Functions ====================
    
    /// Relative side constants for redstone output.
    /// These are relative to the terminal's facing direction.
    /// Use these with set_redstone() to specify which side to output to.
    #[pyattr]
    const DOWN: i32 = 0;   // Bottom of the terminal
    #[pyattr]
    const UP: i32 = 1;     // Top of the terminal
    #[pyattr]
    const FRONT: i32 = 2;  // Front of the terminal (the screen side)
    #[pyattr]
    const BACK: i32 = 3;   // Back of the terminal
    #[pyattr]
    const LEFT: i32 = 4;   // Left side of the terminal
    #[pyattr]
    const RIGHT: i32 = 5;  // Right side of the terminal
    
    /// Set the redstone output power for a specific side of the computer.
    /// Directions are relative to the terminal's facing direction.
    /// 
    /// Args:
    ///     side: The relative side to output to (use terminal.DOWN, UP, FRONT, BACK, LEFT, RIGHT)
    ///     power: Power level from 0 (off) to 15 (full power)
    /// 
    /// Returns:
    ///     bool: True if successful, False otherwise
    /// 
    /// Example:
    ///     terminal.set_redstone(terminal.BACK, 15)   # Full power behind terminal
    ///     terminal.set_redstone(terminal.UP, 8)      # Half power on top
    ///     
    ///     # Blink redstone on back of terminal
    ///     for i in range(5):
    ///         terminal.set_redstone(terminal.BACK, 15)
    ///         terminal.sleep(0.5)
    ///         terminal.set_redstone(terminal.BACK, 0)
    ///         terminal.sleep(0.5)
    #[pyfunction]
    fn set_redstone(side: i32, power: i32) -> bool {
        redstone::set_output(side, power)
    }
}

/// The peripheral module exposed to Python.
/// Provides functions for interacting with CC:Tweaked peripherals.
#[pymodule]
pub mod peripheral_module {
    use super::*;

    /// List all connected peripherals.
    /// 
    /// Returns:
    ///     list: List of dictionaries with 'name', 'type', and 'side' keys
    /// 
    /// Example:
    ///     import peripheral
    ///     for p in peripheral.list():
    ///         print(f"{p['name']} ({p['type']}) - {p['side']}")
    #[pyfunction]
    fn list(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let peripherals = peripheral::list();
        
        let list = vm.ctx.new_list(Vec::new());
        for p in peripherals {
            let dict = vm.ctx.new_dict();
            dict.set_item("name", vm.new_pyobj(p.name), vm)?;
            dict.set_item("type", vm.new_pyobj(p.peripheral_type), vm)?;
            dict.set_item("side", vm.new_pyobj(p.side), vm)?;
            list.borrow_vec_mut().push(dict.into());
        }
        
        Ok(list.into())
    }

    /// Get the names of all connected peripherals.
    /// 
    /// Returns:
    ///     list: List of peripheral names
    /// 
    /// Example:
    ///     names = peripheral.get_names()
    #[pyfunction]
    fn get_names(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let names = peripheral::get_names();
        let list = vm.ctx.new_list(
            names.into_iter().map(|n| vm.new_pyobj(n)).collect()
        );
        Ok(list.into())
    }

    /// Get the methods available on a peripheral.
    /// 
    /// Args:
    ///     name: Name of the peripheral (e.g., "chat_box_0")
    /// 
    /// Returns:
    ///     list: List of method names, or None if peripheral not found
    /// 
    /// Example:
    ///     methods = peripheral.get_methods("chat_box_0")
    ///     for m in methods:
    ///         print(m)
    #[pyfunction]
    fn get_methods(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        match peripheral::get_methods(name.as_str()) {
            Ok(methods) => {
                let list = vm.ctx.new_list(
                    methods.into_iter().map(|m| vm.new_pyobj(m)).collect()
                );
                Ok(list.into())
            }
            Err(e) => {
                Err(vm.new_runtime_error(e))
            }
        }
    }

    /// Call a method on a peripheral.
    /// 
    /// Args:
    ///     name: Name of the peripheral (e.g., "chat_box_0")
    ///     method: Name of the method to call
    ///     args: JSON string of arguments (default: "[]")
    /// 
    /// Returns:
    ///     str: JSON string of the result
    /// 
    /// Example:
    ///     # Call with no arguments
    ///     result = peripheral.call("environment_detector_0", "getBiome", "[]")
    ///     
    ///     # Call with arguments
    ///     result = peripheral.call("chat_box_0", "sendMessage", '["Hello!"]')
    #[pyfunction]
    fn call(name: PyStrRef, method: PyStrRef, args: Option<PyStrRef>) -> String {
        let args_str = args.map(|s| s.as_str().to_string()).unwrap_or_else(|| "[]".to_string());
        match peripheral::call(name.as_str(), method.as_str(), &args_str) {
            Ok(result) => result,
            Err(e) => format!("{{\"ok\":false,\"error\":\"{}\"}}", e),
        }
    }

    /// Check if a peripheral exists by name.
    /// 
    /// Args:
    ///     name: Name of the peripheral
    /// 
    /// Returns:
    ///     bool: True if peripheral exists
    /// 
    /// Example:
    ///     if peripheral.is_present("chat_box_0"):
    ///         print("Chat box is connected!")
    #[pyfunction]
    fn is_present(name: PyStrRef) -> bool {
        peripheral::list().iter().any(|p| p.name == name.as_str())
    }

    /// Find a peripheral by type.
    /// 
    /// Args:
    ///     peripheral_type: Type of peripheral to find (e.g., "chat_box")
    /// 
    /// Returns:
    ///     str or None: Name of the first peripheral of that type, or None
    /// 
    /// Example:
    ///     name = peripheral.find("chat_box")
    ///     if name:
    ///         peripheral.call(name, "sendMessage", '["Hello!"]')
    #[pyfunction]
    fn find(peripheral_type: PyStrRef) -> Option<String> {
        peripheral::find_by_type(peripheral_type.as_str()).map(|p| p.name)
    }

    /// Wrap a peripheral for easier method calling.
    /// Returns None if the peripheral doesn't exist.
    /// 
    /// Args:
    ///     name: Name of the peripheral
    /// 
    /// Returns:
    ///     dict or None: A dictionary with the peripheral info, or None
    /// 
    /// Example:
    ///     chat = peripheral.wrap("chat_box_0")
    ///     if chat:
    ///         print(f"Found: {chat['name']}")
    #[pyfunction]
    fn wrap(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        match peripheral::Peripheral::wrap(name.as_str()) {
            Some(p) => {
                let dict = vm.ctx.new_dict();
                dict.set_item("name", vm.new_pyobj(p.name.clone()), vm)?;
                dict.set_item("type", vm.new_pyobj(p.peripheral_type.clone()), vm)?;
                Ok(dict.into())
            }
            None => Ok(vm.ctx.none())
        }
    }
}

/// Python REPL state
pub struct PythonRepl {
    /// Input buffer for multi-line statements
    input_buffer: String,
    /// Whether we're in a multi-line input mode
    continuation: bool,
    /// The RustPython interpreter
    interpreter: Interpreter,
    /// Persistent scope for maintaining imports and variables across commands
    scope: Option<Scope>,
}

impl PythonRepl {
    /// Creates a new Python REPL instance.
    pub fn new() -> Self {
        let settings = Settings::default();
        
        let interpreter = Interpreter::with_init(settings, |vm| {
            // Add our custom terminal module
            vm.add_native_module("terminal".to_owned(), Box::new(terminal_module::make_module));
            // Add the peripheral module for CC:Tweaked integration
            vm.add_native_module("peripheral".to_owned(), Box::new(peripheral_module::make_module));
        });
        
        // Create a persistent scope that will maintain imports and variables
        let scope = interpreter.enter(|vm| {
            vm.new_scope_with_builtins()
        });
        
        Self {
            input_buffer: String::new(),
            continuation: false,
            interpreter,
            scope: Some(scope),
        }
    }

    /// Displays the REPL banner.
    pub fn show_banner(&self) {
        terminal::println("Python 3.11 (RustPython)");
        terminal::println("Type 'exit()' or Ctrl+D to exit.");
        terminal::println("Use 'import terminal' for terminal functions.");
        terminal::println("Use 'import peripheral' for CC peripherals.");
        terminal::println("");
    }

    /// Prints the appropriate prompt.
    pub fn print_prompt(&self) {
        if self.continuation {
            terminal::print("... ");
        } else {
            terminal::print(">>> ");
        }
    }

    /// Handles a line of input from the user.
    /// Returns true if the REPL should exit.
    pub fn handle_input(&mut self, input: &str) -> bool {
        // Check for exit commands
        let trimmed = input.trim();
        if trimmed == "exit()" || trimmed == "quit()" {
            return true;
        }
        
        // Check for Ctrl+D (EOF)
        if input.contains('\x04') {
            terminal::println("");
            return true;
        }

        // Add input to buffer
        if !self.input_buffer.is_empty() {
            self.input_buffer.push('\n');
        }
        self.input_buffer.push_str(input);

        // Try to compile and execute
        let code = self.input_buffer.clone();
        
        // Check if the input is complete
        if self.is_complete(&code) {
            self.continuation = false;
            self.execute(&code);
            self.input_buffer.clear();
        } else {
            self.continuation = true;
        }

        false
    }

    /// Checks if the input is a complete Python statement.
    fn is_complete(&self, code: &str) -> bool {
        // Simple heuristics for incomplete statements:
        // - Ends with colon (if, for, while, def, class, etc.)
        // - Unclosed brackets/parens
        // - Trailing backslash
        let trimmed = code.trim();
        
        if trimmed.is_empty() {
            return true;
        }
        
        if trimmed.ends_with('\\') {
            return false;
        }
        
        if trimmed.ends_with(':') {
            return false;
        }
        
        // Count brackets
        let mut parens = 0i32;
        let mut brackets = 0i32;
        let mut braces = 0i32;
        let mut in_string = false;
        let mut string_char = ' ';
        
        for c in code.chars() {
            if in_string {
                if c == string_char {
                    in_string = false;
                }
            } else {
                match c {
                    '"' | '\'' => {
                        in_string = true;
                        string_char = c;
                    }
                    '(' => parens += 1,
                    ')' => parens -= 1,
                    '[' => brackets += 1,
                    ']' => brackets -= 1,
                    '{' => braces += 1,
                    '}' => braces -= 1,
                    _ => {}
                }
            }
        }
        
        parens == 0 && brackets == 0 && braces == 0 && !in_string
    }

    /// Executes Python code.
    fn execute(&mut self, code: &str) {
        // Take the scope out temporarily (we'll put it back after)
        let scope = self.scope.take().expect("scope should always exist");
        
        let scope = self.interpreter.enter(|vm| {
            // Compile and execute the code
            match vm.compile(code, Mode::Single, "<stdin>".to_owned()) {
                Ok(code_obj) => {
                    // Use the persistent scope so imports and variables are preserved
                    match vm.run_code_obj(code_obj, scope.clone()) {
                        Ok(result) => {
                            // Print the result if it's not None
                            if !vm.is_none(&result) {
                                match result.repr(vm) {
                                    Ok(repr) => {
                                        terminal::println(repr.as_str());
                                    }
                                    Err(e) => {
                                        self.print_exception(vm, &e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            self.print_exception(vm, &e);
                        }
                    }
                }
                Err(e) => {
                    terminal::print("SyntaxError: ");
                    terminal::println(&format!("{}", e));
                }
            }
            // Return the scope so we can store it again
            scope
        });
        
        // Put the scope back
        self.scope = Some(scope);
    }

    /// Executes a Python file (script mode).
    /// Unlike execute(), this uses Mode::Exec and doesn't print the result.
    pub fn run_file(&mut self, code: &str, filename: &str) {
        // Take the scope out temporarily (we'll put it back after)
        let scope = self.scope.take().expect("scope should always exist");
        
        let scope = self.interpreter.enter(|vm| {
            // Compile with Mode::Exec for script execution
            match vm.compile(code, Mode::Exec, filename.to_owned()) {
                Ok(code_obj) => {
                    // Use the persistent scope so imports and variables are preserved
                    match vm.run_code_obj(code_obj, scope.clone()) {
                        Ok(_) => {
                            // Scripts don't print their result value
                        }
                        Err(e) => {
                            self.print_exception(vm, &e);
                        }
                    }
                }
                Err(e) => {
                    terminal::print("SyntaxError: ");
                    terminal::println(&format!("{}", e));
                }
            }
            // Return the scope so we can store it again
            scope
        });
        
        // Put the scope back
        self.scope = Some(scope);
    }

    /// Prints a Python exception.
    fn print_exception(&self, vm: &VirtualMachine, exc: &rustpython_vm::PyRef<rustpython_vm::builtins::PyBaseException>) {
        // Get the exception type name
        let type_name = exc.class().name().to_string();
        terminal::print(&type_name);
        terminal::print(": ");
        
        // Get the exception message - try to get string representation
        if let Ok(msg) = exc.as_object().str(vm) {
            terminal::println(msg.as_str());
        } else {
            terminal::println("(unknown error)");
        }
    }
}
