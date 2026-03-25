//! Python REPL integration using RustPython.
//!
//! Provides an embedded Python interpreter with a custom `terminal` module
//! that exposes the host functions for terminal I/O and file system access.

use rustpython_vm::{
    Interpreter,
    Settings,
    builtins::PyStrRef,
    function::OptionalArg,
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
use crate::interrupt;
use crate::modules;

/// Bootstrap code for setting up virtual filesystem imports
const PYTHON_BOOTSTRAP: &str = include_str!("python_bootstrap.py");

/// Static storage for Python interrupt handler callables (indexed by IRQ number).
/// These are PyObjectRef values that are only valid while the Python interpreter is alive.
static mut PYTHON_INTERRUPT_HANDLERS: [Option<rustpython_vm::PyObjectRef>; 16] = {
    // Can't use [None; 16] directly because PyObjectRef isn't Copy,
    // so we use a const initializer block.
    const NONE: Option<rustpython_vm::PyObjectRef> = None;
    [NONE; 16]
};

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
    /// Interrupt handlers are dispatched during sleep (every 10ms).
    ///
    /// Args:
    ///     seconds: Time to sleep (can be fractional, e.g., 0.5 for 500ms)
    ///
    /// Example:
    ///     terminal.sleep(1.0)   # Sleep for 1 second
    ///     terminal.sleep(0.5)   # Sleep for 500ms
    #[pyfunction]
    fn sleep(seconds: f64, vm: &VirtualMachine) {
        let mut remaining = (seconds * 1000.0) as i32;
        while remaining > 0 {
            let chunk = if remaining > 10 { 10 } else { remaining };
            terminal::raw_sleep_ms(chunk);
            remaining -= chunk;
            // Poll and dispatch interrupts with VM access
            dispatch_pending_interrupts(vm);
        }
    }
    
    // ==================== Input Function ====================

    /// Read a line of text input from the user.
    /// Displays the prompt, waits for the user to type and press Enter.
    ///
    /// Args:
    ///     prompt: Text to display before input (default: "")
    ///
    /// Returns:
    ///     str: The text the user entered
    ///
    /// Example:
    ///     name = terminal.input("Enter your name: ")
    ///     terminal.println(f"Hello, {name}!")
    #[pyfunction]
    fn input(prompt: OptionalArg<PyStrRef>) -> String {
        let prompt_str = match &prompt {
            OptionalArg::Present(s) => s.as_str(),
            OptionalArg::Missing => "",
        };
        terminal::read_line(prompt_str)
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

    // ==================== Redstone Input Functions ====================

    /// Read the redstone input power level for a specific side.
    ///
    /// Args:
    ///     side: The relative side (use terminal.DOWN, UP, FRONT, BACK, LEFT, RIGHT)
    ///
    /// Returns:
    ///     int: Power level (0-15)
    ///
    /// Example:
    ///     power = terminal.get_redstone(terminal.BACK)
    #[pyfunction]
    fn get_redstone(side: i32) -> i32 {
        redstone::get_input(side)
    }

    /// Read all 6 redstone input power levels at once.
    ///
    /// Returns:
    ///     list[int]: Power levels for [DOWN, UP, FRONT, BACK, LEFT, RIGHT]
    ///
    /// Example:
    ///     levels = terminal.get_all_redstone()
    ///     print(f"Back power: {levels[3]}")
    #[pyfunction]
    fn get_all_redstone(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let levels = redstone::get_all_input();
        let list = vm.ctx.new_list(
            levels.iter().map(|&v| vm.new_pyobj(v)).collect()
        );
        Ok(list.into())
    }

    // ==================== Interrupt Functions ====================

    /// IRQ number for keyboard input interrupts.
    #[pyattr]
    const IRQ_KEYBOARD: i32 = 1;

    /// IRQ number for redstone input change interrupts.
    #[pyattr]
    const IRQ_REDSTONE: i32 = 2;

    /// Register an interrupt handler for the given IRQ number.
    /// The handler function will be called with a dict containing event data.
    ///
    /// Args:
    ///     irq: Interrupt number (terminal.IRQ_KEYBOARD or terminal.IRQ_REDSTONE)
    ///     handler: Callable that takes one argument (event data dict)
    ///
    /// Example:
    ///     def on_redstone(data):
    ///         print(f"Redstone changed: {data}")
    ///     terminal.on_interrupt(terminal.IRQ_REDSTONE, on_redstone)
    #[pyfunction]
    fn on_interrupt(irq: i32, handler: rustpython_vm::PyObjectRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        if irq < 0 || irq >= 16 || irq == 15 {
            return Err(vm.new_value_error("Invalid IRQ number (must be 0-14)".to_owned()));
        }
        // Verify handler is callable by checking if it has __call__
        if handler.get_attr("__call__", vm).is_err() {
            return Err(vm.new_type_error("handler must be callable".to_owned()));
        }
        // Store the Python handler
        unsafe {
            PYTHON_INTERRUPT_HANDLERS[irq as usize] = Some(handler);
        }
        // Mark the IRQ as having a Python handler in the interrupt module
        interrupt::register_python(irq);
        Ok(())
    }

    /// Clear the interrupt handler for the given IRQ number.
    ///
    /// Args:
    ///     irq: Interrupt number to clear
    ///
    /// Example:
    ///     terminal.clear_interrupt(terminal.IRQ_REDSTONE)
    #[pyfunction]
    fn clear_interrupt(irq: i32) {
        if irq >= 0 && (irq as usize) < 16 {
            unsafe {
                PYTHON_INTERRUPT_HANDLERS[irq as usize] = None;
            }
            interrupt::unregister(irq);
        }
    }

    /// Cooperative interrupt yield point.
    /// Call this in tight loops to allow pending interrupts to be delivered.
    ///
    /// Returns:
    ///     int: Number of interrupts delivered
    ///
    /// Example:
    ///     while True:
    ///         # do work...
    ///         terminal.check_interrupts()
    #[pyfunction]
    fn check_interrupts(vm: &VirtualMachine) -> i32 {
        dispatch_pending_interrupts(vm)
    }
}

/// Polls all pending interrupts from the host and dispatches them using the Python VM.
/// This is the correct way to dispatch Python interrupt handlers — it uses the VM
/// reference directly, avoiding the scope ownership issue in PythonRepl.handle_interrupt().
fn dispatch_pending_interrupts(vm: &VirtualMachine) -> i32 {
    let mut count = 0;
    while let Some((irq, data)) = terminal::poll_interrupt() {
        // Handle terminate interrupt
        if irq == interrupt::IRQ_TERMINATE {
            crate::dispatch_interrupt(irq, &data);
            count += 1;
            continue;
        }

        // Dispatch Rust-level handlers
        interrupt::dispatch_rust(irq, &data);

        // Dispatch Python-level handler if registered
        if interrupt::has_python_handler(irq) {
            let handler = unsafe {
                PYTHON_INTERRUPT_HANDLERS[irq as usize].clone()
            };
            if let Some(handler) = handler {
                // Parse JSON payload into a Python dict
                let data_obj = parse_json_to_pyobj(&data, vm);

                // Call the handler
                if let Err(e) = handler.call((data_obj,), vm) {
                    let type_name = e.class().name().to_string();
                    terminal::print("[Interrupt handler error] ");
                    terminal::print(&type_name);
                    terminal::print(": ");
                    if let Ok(msg) = e.as_object().str(vm) {
                        terminal::println(msg.as_str());
                    } else {
                        terminal::println("(unknown error)");
                    }
                }
            }
        }
        count += 1;
    }
    count
}

/// Parses a simple JSON string into a Python dict using the VM context directly.
/// Handles the known interrupt payload formats:
///   - Keyboard: {"key":"X"}
///   - Redstone: {"sides":[0,0,0,15,0,0],"old_sides":[0,0,0,0,0,0]}
fn parse_json_to_pyobj(data: &str, vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
    let dict = vm.ctx.new_dict();
    let trimmed = data.trim();

    // Strip outer braces
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return vm.new_pyobj(data.to_string());
    }
    let inner = &trimmed[1..trimmed.len() - 1];

    // Parse key-value pairs from simple JSON
    // We iterate through the inner string, handling nested arrays
    let mut pos = 0;
    let bytes = inner.as_bytes();
    let len = bytes.len();

    while pos < len {
        // Skip whitespace and commas
        while pos < len && (bytes[pos] == b' ' || bytes[pos] == b',' || bytes[pos] == b'\n') {
            pos += 1;
        }
        if pos >= len { break; }

        // Expect a quoted key
        if bytes[pos] != b'"' { break; }
        pos += 1;
        let key_start = pos;
        while pos < len && bytes[pos] != b'"' { pos += 1; }
        let key = &inner[key_start..pos];
        pos += 1; // skip closing quote

        // Skip colon and whitespace
        while pos < len && (bytes[pos] == b':' || bytes[pos] == b' ') { pos += 1; }
        if pos >= len { break; }

        // Parse value
        if bytes[pos] == b'"' {
            // String value
            pos += 1;
            let val_start = pos;
            while pos < len && bytes[pos] != b'"' {
                if bytes[pos] == b'\\' { pos += 1; } // skip escaped char
                pos += 1;
            }
            let val = &inner[val_start..pos];
            // Unescape basic sequences
            let val = val.replace("\\n", "\n").replace("\\r", "\r")
                .replace("\\t", "\t").replace("\\\"", "\"").replace("\\\\", "\\");
            pos += 1; // skip closing quote
            let _ = dict.set_item(key, vm.new_pyobj(val), vm);
        } else if bytes[pos] == b'[' {
            // Array value (we only have arrays of integers)
            pos += 1; // skip [
            let mut items: Vec<rustpython_vm::PyObjectRef> = Vec::new();
            while pos < len && bytes[pos] != b']' {
                // Skip whitespace and commas
                while pos < len && (bytes[pos] == b' ' || bytes[pos] == b',' || bytes[pos] == b'\n') {
                    pos += 1;
                }
                if pos >= len || bytes[pos] == b']' { break; }
                // Parse integer
                let num_start = pos;
                if bytes[pos] == b'-' { pos += 1; }
                while pos < len && bytes[pos] >= b'0' && bytes[pos] <= b'9' { pos += 1; }
                if let Ok(n) = inner[num_start..pos].parse::<i32>() {
                    items.push(vm.new_pyobj(n));
                }
            }
            if pos < len { pos += 1; } // skip ]
            let list = vm.ctx.new_list(items);
            let _ = dict.set_item(key, list.into(), vm);
        } else if bytes[pos] >= b'0' && bytes[pos] <= b'9' || bytes[pos] == b'-' {
            // Number value
            let num_start = pos;
            if bytes[pos] == b'-' { pos += 1; }
            while pos < len && bytes[pos] >= b'0' && bytes[pos] <= b'9' { pos += 1; }
            if let Ok(n) = inner[num_start..pos].parse::<i32>() {
                let _ = dict.set_item(key, vm.new_pyobj(n), vm);
            }
        } else {
            // Unknown value type, skip
            break;
        }
    }

    dict.into()
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

/// The _modules bridge module exposed to Python.
/// Provides functions to call Java-registered computer modules and query their metadata.
/// Uses binary protocol for method calls (no JSON serialization overhead).
/// Uses JSON only for `get_metadata()` (called once at startup).
#[pymodule]
pub mod modules_module {
    use super::*;
    use crate::modules::BinaryValue;

    /// Call a method on a registered computer module.
    /// Uses binary protocol — no JSON serialization.
    ///
    /// Example:
    ///     _modules.call("golem", "summon", "iron", 5)
    #[pyfunction]
    fn call(args: rustpython_vm::function::FuncArgs, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        if args.args.len() < 2 {
            return Err(vm.new_type_error("call() requires at least 2 arguments: module, method, [args...]".to_owned()));
        }

        let module_name = args.args[0].str(vm)?.as_str().to_string();
        let method_name = args.args[1].str(vm)?.as_str().to_string();

        // Serialize remaining args to binary
        let args_binary = serialize_args_binary(&args.args[2..], vm)?;

        match modules::call(&module_name, &method_name, &args_binary) {
            Ok(value) => binary_value_to_pyobj(value, vm),
            Err(e) => Err(vm.new_runtime_error(e)),
        }
    }

    /// Get metadata about all registered computer modules.
    /// Returns a Python dict describing available modules and their functions.
    /// Still uses JSON since metadata is complex nested data called once at startup.
    #[pyfunction]
    fn get_metadata(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let json = modules::list_modules();
        parse_json_value_to_pyobj(&json, vm)
    }

    // ==================== Binary Serialization ====================

    /// Serializes Python arguments to binary format.
    /// Format: [u8 arg_count] ([u8 type_tag] [payload])*
    fn serialize_args_binary(args: &[rustpython_vm::PyObjectRef], vm: &VirtualMachine) -> rustpython_vm::PyResult<Vec<u8>> {
        use rustpython_vm::builtins::{PyInt, PyFloat, PyStr};

        let mut values = Vec::with_capacity(args.len());

        for arg in args {
            if vm.is_none(arg) {
                values.push(BinaryValue::Null);
            } else if &*arg.class().name() == "bool" {
                let b = arg.clone().try_to_bool(vm).unwrap_or(false);
                values.push(BinaryValue::Bool(b));
            } else if let Some(i) = arg.payload::<PyInt>() {
                if let Ok(val) = i.try_to_primitive::<i32>(vm) {
                    values.push(BinaryValue::I32(val));
                } else if let Ok(val) = i.try_to_primitive::<i64>(vm) {
                    values.push(BinaryValue::I64(val));
                } else {
                    // Very large int — fall back to string representation
                    values.push(BinaryValue::Str(i.as_bigint().to_string()));
                }
            } else if let Some(f) = arg.payload::<PyFloat>() {
                values.push(BinaryValue::F64(f.to_f64()));
            } else if let Some(s) = arg.payload::<PyStr>() {
                values.push(BinaryValue::Str(s.as_str().to_string()));
            } else {
                // Fallback: convert to string
                let s = arg.str(vm)?;
                values.push(BinaryValue::Str(s.as_str().to_string()));
            }
        }

        Ok(modules::encode_args(&values))
    }

    /// Converts a BinaryValue to a Python object.
    fn binary_value_to_pyobj(value: BinaryValue, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        match value {
            BinaryValue::Null => Ok(vm.ctx.none()),
            BinaryValue::Str(s) => Ok(vm.new_pyobj(s)),
            BinaryValue::I32(i) => Ok(vm.new_pyobj(i)),
            BinaryValue::I64(i) => Ok(vm.new_pyobj(i)),
            BinaryValue::F64(f) => Ok(vm.new_pyobj(f)),
            BinaryValue::Bool(b) => Ok(vm.ctx.new_bool(b).into()),
        }
    }

    // ==================== JSON Parsing (for get_metadata only) ====================

    /// Parses a JSON value string into a Python object.
    /// Used only by get_metadata() for startup module discovery.
    fn parse_json_value_to_pyobj(json: &str, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let json = json.trim();

        if json == "null" { return Ok(vm.ctx.none()); }
        if json == "true" { return Ok(vm.ctx.new_bool(true).into()); }
        if json == "false" { return Ok(vm.ctx.new_bool(false).into()); }

        // String
        if json.starts_with('"') && json.ends_with('"') {
            let inner = &json[1..json.len()-1];
            let unescaped = unescape_json_string(inner);
            return Ok(vm.new_pyobj(unescaped));
        }

        // Number
        if json.starts_with('-') || json.starts_with(|c: char| c.is_ascii_digit()) {
            if json.contains('.') || json.contains('e') || json.contains('E') {
                if let Ok(f) = json.parse::<f64>() { return Ok(vm.new_pyobj(f)); }
            } else {
                if let Ok(i) = json.parse::<i64>() { return Ok(vm.new_pyobj(i)); }
            }
        }

        // Array
        if json.starts_with('[') && json.ends_with(']') {
            let inner = &json[1..json.len()-1].trim();
            if inner.is_empty() { return Ok(vm.ctx.new_list(vec![]).into()); }
            let elements = split_json_values(inner);
            let mut py_list = Vec::new();
            for elem in elements {
                py_list.push(parse_json_value_to_pyobj(elem.trim(), vm)?);
            }
            return Ok(vm.ctx.new_list(py_list).into());
        }

        // Object
        if json.starts_with('{') && json.ends_with('}') {
            let dict = vm.ctx.new_dict();
            let inner = &json[1..json.len()-1].trim();
            if !inner.is_empty() {
                let pairs = split_json_values(inner);
                for pair in pairs {
                    let pair = pair.trim();
                    if let Some(colon_pos) = find_colon(pair) {
                        let key = pair[..colon_pos].trim();
                        let value = pair[colon_pos+1..].trim();
                        if key.starts_with('"') && key.ends_with('"') {
                            let key_str = unescape_json_string(&key[1..key.len()-1]);
                            let py_val = parse_json_value_to_pyobj(value, vm)?;
                            dict.set_item(&*key_str, py_val, vm)?;
                        }
                    }
                }
            }
            return Ok(dict.into());
        }

        Ok(vm.new_pyobj(json.to_string()))
    }

    fn unescape_json_string(s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('"') => result.push('"'),
                    Some('\\') => result.push('\\'),
                    Some('n') => result.push('\n'),
                    Some('r') => result.push('\r'),
                    Some('t') => result.push('\t'),
                    Some(other) => { result.push('\\'); result.push(other); }
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    fn split_json_values(s: &str) -> Vec<&str> {
        let mut result = Vec::new();
        let mut depth = 0;
        let mut start = 0;
        let mut in_string = false;
        let mut escaped = false;
        let chars: Vec<char> = s.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if escaped { escaped = false; continue; }
            if c == '\\' { escaped = true; continue; }
            if c == '"' { in_string = !in_string; continue; }
            if !in_string {
                match c {
                    '{' | '[' => depth += 1,
                    '}' | ']' => depth -= 1,
                    ',' if depth == 0 => { result.push(&s[start..i]); start = i + 1; }
                    _ => {}
                }
            }
        }
        if start < s.len() { result.push(&s[start..]); }
        result
    }

    fn find_colon(s: &str) -> Option<usize> {
        let mut in_string = false;
        let mut escaped = false;
        for (i, c) in s.chars().enumerate() {
            if escaped { escaped = false; continue; }
            if c == '\\' { escaped = true; continue; }
            if c == '"' { in_string = !in_string; continue; }
            if !in_string && c == ':' { return Some(i); }
        }
        None
    }
}

/// Network module — exposes the TCP/IP stack to Python.
#[pymodule]
pub mod net_module {
    use super::*;

    /// List all interfaces. Returns list of dicts.
    #[pyfunction]
    fn interfaces(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let list = vm.ctx.new_list(Vec::new());
        for i in 0..stack.iface_count {
            let iface = &stack.interfaces[i];
            let dict = vm.ctx.new_dict();
            dict.set_item("name", vm.new_pyobj(iface.name_str().to_string()), vm)?;
            dict.set_item("mac", vm.new_pyobj(format!("{}", iface.mac)), vm)?;
            dict.set_item("ip", vm.new_pyobj(format!("{}", iface.ip)), vm)?;
            dict.set_item("prefix_len", vm.new_pyobj(iface.prefix_len as i32), vm)?;
            dict.set_item("link_up", vm.new_pyobj(iface.link_up), vm)?;
            match iface.vlan {
                Some(vid) => dict.set_item("vlan", vm.new_pyobj(vid as i32), vm)?,
                None => dict.set_item("vlan", vm.ctx.none(), vm)?,
            }
            list.borrow_vec_mut().push(dict.into());
        }
        Ok(list.into())
    }

    /// Configure interface IP with CIDR: net.iface_set("eth0", "10.0.0.1/24")
    #[pyfunction]
    fn iface_set(name: PyStrRef, cidr: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        use crate::net::types::Ipv4Addr;
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let idx = stack.find_iface(name.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", name.as_str())))?;
        let (ip, prefix) = Ipv4Addr::parse_cidr(cidr.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid CIDR (e.g. 10.0.0.1/24)".to_string()))?;
        stack.configure_iface(idx, ip, prefix);
        Ok(())
    }

    /// Bring interface up.
    #[pyfunction]
    fn iface_up(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let idx = stack.find_iface(name.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", name.as_str())))?;
        stack.interfaces[idx].link_up = true;
        Ok(())
    }

    /// Bring interface down.
    #[pyfunction]
    fn iface_down(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let idx = stack.find_iface(name.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", name.as_str())))?;
        stack.interfaces[idx].link_up = false;
        Ok(())
    }

    /// Set/clear VLAN on interface: net.iface_vlan("eth0", 100) or net.iface_vlan("eth0", None)
    #[pyfunction]
    fn iface_vlan(name: PyStrRef, vid: rustpython_vm::PyObjectRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let idx = stack.find_iface(name.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", name.as_str())))?;
        if vm.is_none(&vid) {
            stack.interfaces[idx].vlan = None;
        } else {
            let v: i32 = vid.try_into_value(vm)?;
            if v < 0 || v > 4094 {
                return Err(vm.new_value_error("VLAN ID must be 0-4094".to_string()));
            }
            stack.interfaces[idx].vlan = Some(v as u16);
        }
        Ok(())
    }

    /// List routes: returns list of dicts.
    #[pyfunction]
    fn routes(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let list = vm.ctx.new_list(Vec::new());
        for e in &stack.routing.entries {
            if !e.active { continue; }
            let dict = vm.ctx.new_dict();
            dict.set_item("destination", vm.new_pyobj(format!("{}", e.destination)), vm)?;
            dict.set_item("prefix_len", vm.new_pyobj(e.prefix_len as i32), vm)?;
            dict.set_item("gateway", vm.new_pyobj(format!("{}", e.gateway)), vm)?;
            let dev_name = if e.iface_index < stack.iface_count {
                stack.interfaces[e.iface_index].name_str().to_string()
            } else { "?".to_string() };
            dict.set_item("dev", vm.new_pyobj(dev_name), vm)?;
            list.borrow_vec_mut().push(dict.into());
        }
        Ok(list.into())
    }

    /// Add route: net.route_add("default", "10.0.0.1", "eth0") or net.route_add("192.168.1.0/24", "10.0.0.1", "eth0")
    #[pyfunction]
    fn route_add(dest: PyStrRef, gw: PyStrRef, dev: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        use crate::net::types::Ipv4Addr;
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let iface_idx = stack.find_iface(dev.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", dev.as_str())))?;
        let gw_ip = Ipv4Addr::parse(gw.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid gateway IP".to_string()))?;
        if dest.as_str() == "default" {
            stack.routing.add_route(Ipv4Addr::ZERO, 0, gw_ip, iface_idx)
                .map_err(|e| vm.new_runtime_error(format!("{}", e)))
        } else {
            let (d, p) = Ipv4Addr::parse_cidr(dest.as_str())
                .ok_or_else(|| vm.new_value_error("Invalid CIDR destination".to_string()))?;
            stack.routing.add_route(d, p, gw_ip, iface_idx)
                .map_err(|e| vm.new_runtime_error(format!("{}", e)))
        }
    }

    /// Delete route: net.route_del("default") or net.route_del("192.168.1.0/24")
    #[pyfunction]
    fn route_del(dest: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        use crate::net::types::Ipv4Addr;
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        if dest.as_str() == "default" {
            if !stack.routing.del_route(Ipv4Addr::ZERO, 0) {
                return Err(vm.new_runtime_error("No default route".to_string()));
            }
        } else {
            let (d, p) = Ipv4Addr::parse_cidr(dest.as_str())
                .ok_or_else(|| vm.new_value_error("Invalid CIDR destination".to_string()))?;
            if !stack.routing.del_route(d, p) {
                return Err(vm.new_runtime_error("Route not found".to_string()));
            }
        }
        Ok(())
    }

    /// Set DNS server: net.dns_set("8.8.8.8")
    #[pyfunction]
    fn dns_set(ip: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        use crate::net::types::Ipv4Addr;
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let dns = Ipv4Addr::parse(ip.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid IP address".to_string()))?;
        stack.dns_server = dns;
        Ok(())
    }

    /// Get DNS server.
    #[pyfunction]
    fn dns_get(vm: &VirtualMachine) -> rustpython_vm::PyResult<String> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        Ok(format!("{}", stack.dns_server))
    }

    /// Send ICMP ping. Returns RTT in ms or raises on timeout.
    #[pyfunction]
    fn ping(target: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<u32> {
        use crate::net::types::Ipv4Addr;
        let ip = Ipv4Addr::parse(target.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid IP address".to_string()))?;
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        stack.ping(ip, 2000)
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))
    }

    /// Resolve a hostname via DNS. Returns IP string.
    #[pyfunction]
    fn resolve(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<String> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let ip = stack.dns_resolve(name.as_str(), 5000)
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))?;
        Ok(format!("{}", ip))
    }

    /// Open a UDP socket bound to a port.
    #[pyfunction]
    fn udp_open(port: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let idx = stack.udp_sockets.bind(port as u16)
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))?;
        Ok(idx as i32)
    }

    /// Send a UDP datagram.
    #[pyfunction]
    fn udp_send(
        sock: i32,
        dst_ip: PyStrRef,
        dst_port: i32,
        data: rustpython_vm::builtins::PyBytesRef,
        vm: &VirtualMachine,
    ) -> rustpython_vm::PyResult<()> {
        use crate::net::types::{Ipv4Addr, SocketAddr};
        let ip = Ipv4Addr::parse(dst_ip.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid IP address".to_string()))?;
        let dst = SocketAddr { ip, port: dst_port as u16 };
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        stack.udp_send(sock as usize, dst, data.as_bytes())
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))
    }

    /// Receive a UDP datagram. Returns (data, ip, port) or None.
    #[pyfunction]
    fn udp_recv(sock: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;

        // Poll until data or timeout
        let deadline = chrono::Utc::now().timestamp_millis() + 5000;
        let mut buf = [0u8; 2048];
        loop {
            if let Some((src, len)) = stack.udp_sockets.recv(sock as usize, &mut buf) {
                let data = vm.ctx.new_bytes(buf[..len].to_vec());
                let ip = vm.new_pyobj(format!("{}", src.ip));
                let port = vm.new_pyobj(src.port as i32);
                let tuple = vm.ctx.new_tuple(vec![data.into(), ip, port]);
                return Ok(tuple.into());
            }
            if chrono::Utc::now().timestamp_millis() >= deadline {
                return Ok(vm.ctx.none());
            }
            stack.poll_rx();
            stack.poll_timers();
            crate::terminal::raw_sleep_ms(10);
        }
    }

    /// Close a UDP socket.
    #[pyfunction]
    fn udp_close(sock: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        stack.udp_sockets.close(sock as usize);
        Ok(())
    }

    /// Connect to a TCP server. Returns connection handle.
    #[pyfunction]
    fn tcp_connect(host: PyStrRef, port: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        use crate::net::types::{Ipv4Addr, SocketAddr};
        let ip = Ipv4Addr::parse(host.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid IP address".to_string()))?;
        let remote = SocketAddr { ip, port: port as u16 };
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let idx = stack.tcp_connect(remote, 10000)
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))?;
        Ok(idx as i32)
    }

    /// Listen on a TCP port. Returns listener handle.
    #[pyfunction]
    fn tcp_listen(port: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let idx = stack.tcp_connections.listen(crate::net::types::Ipv4Addr::ZERO, port as u16)
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))?;
        Ok(idx as i32)
    }

    /// Accept a TCP connection. Blocks until a connection arrives.
    #[pyfunction]
    fn tcp_accept(listener: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let idx = stack.tcp_accept(listener as usize, 30000)
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))?;
        Ok(idx as i32)
    }

    /// Send data on a TCP connection.
    #[pyfunction]
    fn tcp_send(
        conn: i32,
        data: rustpython_vm::builtins::PyBytesRef,
        vm: &VirtualMachine,
    ) -> rustpython_vm::PyResult<i32> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let written = stack.tcp_send(conn as usize, data.as_bytes())
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))?;
        Ok(written as i32)
    }

    /// Receive data from a TCP connection. Returns bytes.
    #[pyfunction]
    fn tcp_recv(conn: i32, max_len: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        let mut buf = vec![0u8; max_len as usize];
        let n = stack.tcp_recv(conn as usize, &mut buf, 10000)
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))?;
        buf.truncate(n);
        Ok(vm.ctx.new_bytes(buf).into())
    }

    /// Close a TCP connection.
    #[pyfunction]
    fn tcp_close(conn: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;
        stack.tcp_close(conn as usize);
        Ok(())
    }

    /// HTTP GET request. Returns dict with 'status', 'body', 'headers'.
    #[pyfunction]
    fn http_get(url: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;

        let (host, port, path) = crate::net::http::parse_url(url.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid URL".to_string()))?;

        let resp = crate::net::http::http_request(stack, "GET", &host, port, &path, None, &[])
            .map_err(|e| vm.new_runtime_error(format!("{}", e)))?;

        http_response_to_dict(resp, vm)
    }

    /// HTTP POST request. Returns dict with 'status', 'body', 'headers'.
    #[pyfunction(name = "http_post")]
    fn http_post_fn(
        url: PyStrRef,
        body: rustpython_vm::builtins::PyBytesRef,
        vm: &VirtualMachine,
    ) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let stack = crate::net::NetStack::get()
            .ok_or_else(|| vm.new_runtime_error("Network stack not initialized".to_string()))?;

        let (host, port, path) = crate::net::http::parse_url(url.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid URL".to_string()))?;

        let resp = crate::net::http::http_request(
            stack, "POST", &host, port, &path,
            Some(body.as_bytes()), &[],
        ).map_err(|e| vm.new_runtime_error(format!("{}", e)))?;

        http_response_to_dict(resp, vm)
    }

    fn http_response_to_dict(
        resp: crate::net::http::HttpResponse,
        vm: &VirtualMachine,
    ) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let dict = vm.ctx.new_dict();
        dict.set_item("status", vm.new_pyobj(resp.status_code as i32), vm)?;
        let body_str = String::from_utf8_lossy(&resp.body).to_string();
        dict.set_item("body", vm.new_pyobj(body_str), vm)?;

        let headers_dict = vm.ctx.new_dict();
        for (k, v) in &resp.headers {
            headers_dict.set_item(k.as_str(), vm.new_pyobj(v.clone()), vm)?;
        }
        dict.set_item("headers", headers_dict.into(), vm)?;

        Ok(dict.into())
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
            // Add the modules bridge for annotation-driven auto-registration
            vm.add_native_module("_modules".to_owned(), Box::new(modules_module::make_module));
            // Add the networking module
            vm.add_native_module("net".to_owned(), Box::new(net_module::make_module));
        });
        
        // Create a persistent scope that will maintain imports and variables
        let scope = interpreter.enter(|vm| {
            vm.new_scope_with_builtins()
        });
        
        // Execute bootstrap code to set up virtual filesystem import hooks
        // This allows `import mymodule` to work for .py files in the virtual filesystem
        interpreter.enter(|vm| {
            match vm.compile(PYTHON_BOOTSTRAP, Mode::Exec, "<bootstrap>".to_owned()) {
                Ok(code_obj) => {
                    if let Err(e) = vm.run_code_obj(code_obj, scope.clone()) {
                        // Bootstrap failed - log the error so we can debug
                        terminal::println("[Bootstrap Error]");
                        let type_name = e.class().name().to_string();
                        terminal::print(&type_name);
                        terminal::print(": ");
                        if let Ok(msg) = e.as_object().str(vm) {
                            terminal::println(msg.as_str());
                        } else {
                            terminal::println("(unknown error)");
                        }
                    }
                }
                Err(e) => {
                    // Compilation error in bootstrap
                    terminal::println("[Bootstrap Compile Error]");
                    terminal::println(&format!("{}", e));
                }
            }
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
                    // Use the persistent scope so imports and variables are preserved.
                    // Mode::Single automatically prints expression results via
                    // sys.displayhook -> sys.stdout, so we don't need to print
                    // the result manually.
                    match vm.run_code_obj(code_obj, scope.clone()) {
                        Ok(_result) => {
                            // Result printing is handled by Mode::Single via sys.stdout
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

    /// Handles an interrupt delivered from the host by calling the registered Python handler.
    /// The data string is a JSON payload that gets parsed into a Python dict.
    pub fn handle_interrupt(&mut self, irq: i32, data: &str) {
        let handler = unsafe {
            PYTHON_INTERRUPT_HANDLERS[irq as usize].clone()
        };
        let handler = match handler {
            Some(h) => h,
            None => return,
        };

        let scope = match self.scope.take() {
            Some(s) => s,
            None => return,
        };

        let data_str = data.to_string();
        let scope = self.interpreter.enter(|vm| {
            // Parse JSON data string into a Python dict using json.loads
            let json_code = format!("__import__('json').loads('{}')", data_str.replace('\'', "\\'"));
            let data_obj = match vm.compile(&json_code, Mode::Eval, "<interrupt>".to_owned()) {
                Ok(code) => {
                    match vm.run_code_obj(code, scope.clone()) {
                        Ok(obj) => obj,
                        Err(_) => {
                            // Fallback: pass raw string
                            vm.new_pyobj(data_str.clone())
                        }
                    }
                }
                Err(_) => vm.new_pyobj(data_str.clone()),
            };

            // Call the handler with the data
            if let Err(e) = handler.call((data_obj,), vm) {
                terminal::print("[Interrupt handler error] ");
                self.print_exception(vm, &e);
            }

            scope
        });

        self.scope = Some(scope);
    }

    /// Clears all Python interrupt handlers. Call when exiting Python mode.
    pub fn clear_interrupt_handlers() {
        unsafe {
            for slot in PYTHON_INTERRUPT_HANDLERS.iter_mut() {
                *slot = None;
            }
        }
        interrupt::clear_all_python();
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

// ==================== Tests ====================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    
    // Thread-local mock filesystem for tests
    thread_local! {
        static MOCK_FILES: std::cell::RefCell<HashMap<String, String>> = std::cell::RefCell::new(HashMap::new());
        static MOCK_OUTPUT: std::cell::RefCell<String> = std::cell::RefCell::new(String::new());
    }
    
    /// Mock terminal module for testing (doesn't require WASM host functions)
    #[pymodule]
    mod mock_terminal {
        use super::*;
        
        #[pyfunction]
        fn write(s: PyStrRef) {
            MOCK_OUTPUT.with(|out| {
                out.borrow_mut().push_str(s.as_str());
            });
        }
        
        #[pyfunction]
        fn println(s: PyStrRef) {
            MOCK_OUTPUT.with(|out| {
                out.borrow_mut().push_str(s.as_str());
                out.borrow_mut().push('\n');
            });
        }
        
        #[pyfunction]
        fn clear() {
            MOCK_OUTPUT.with(|out| {
                out.borrow_mut().clear();
            });
        }
        
        #[pyfunction]
        fn file_exists(path: PyStrRef) -> bool {
            MOCK_FILES.with(|files| {
                files.borrow().contains_key(path.as_str())
            })
        }
        
        #[pyfunction]
        fn read_file(path: PyStrRef) -> Option<String> {
            MOCK_FILES.with(|files| {
                files.borrow().get(path.as_str()).cloned()
            })
        }
        
        #[pyfunction]
        fn write_file(path: PyStrRef, content: PyStrRef) -> bool {
            MOCK_FILES.with(|files| {
                files.borrow_mut().insert(path.as_str().to_string(), content.as_str().to_string());
            });
            true
        }
        
        #[pyfunction]
        fn list_files() -> String {
            MOCK_FILES.with(|files| {
                files.borrow().keys().cloned().collect::<Vec<_>>().join("\n")
            })
        }
        
        #[pyfunction]
        fn set_cursor(_x: i32, _y: i32) {}
        
        #[pyfunction]
        fn get_width() -> i32 { 80 }
        
        #[pyfunction]
        fn get_height() -> i32 { 24 }
        
        #[pyfunction]
        fn sleep(_seconds: f64) {}
        
        #[pyfunction]
        fn set_redstone(_side: i32, _power: i32) -> bool { true }
        
        #[pyfunction]
        fn file_size(path: PyStrRef) -> Option<usize> {
            MOCK_FILES.with(|files| {
                files.borrow().get(path.as_str()).map(|s| s.len())
            })
        }
        
        #[pyfunction]
        fn delete_file(path: PyStrRef) -> bool {
            MOCK_FILES.with(|files| {
                files.borrow_mut().remove(path.as_str()).is_some()
            })
        }
    }
    
    /// Helper to clear mock state between tests
    fn reset_mock_state() {
        MOCK_FILES.with(|files| files.borrow_mut().clear());
        MOCK_OUTPUT.with(|out| out.borrow_mut().clear());
    }
    
    /// Helper to get mock output (useful for debugging)
    #[allow(dead_code)]
    fn get_mock_output() -> String {
        MOCK_OUTPUT.with(|out| out.borrow().clone())
    }
    
    /// Helper to add a mock file
    fn add_mock_file(name: &str, content: &str) {
        MOCK_FILES.with(|files| {
            files.borrow_mut().insert(name.to_string(), content.to_string());
        });
    }
    
    #[test]
    fn test_bootstrap_runs() {
        reset_mock_state();
        
        let settings = Settings::default();
        let interpreter = Interpreter::with_init(settings, |vm| {
            vm.add_native_module("terminal".to_owned(), Box::new(mock_terminal::make_module));
        });
        
        let scope = interpreter.enter(|vm| {
            vm.new_scope_with_builtins()
        });
        
        // Run bootstrap
        let result = interpreter.enter(|vm| {
            match vm.compile(PYTHON_BOOTSTRAP, Mode::Exec, "<bootstrap>".to_owned()) {
                Ok(code_obj) => {
                    match vm.run_code_obj(code_obj, scope.clone()) {
                        Ok(_) => Ok(()),
                        Err(e) => {
                            let msg = e.as_object().str(vm).map(|s| s.to_string()).unwrap_or_default();
                            Err(format!("{}: {}", e.class().name(), msg))
                        }
                    }
                }
                Err(e) => Err(format!("Compile error: {}", e))
            }
        });
        
        assert!(result.is_ok(), "Bootstrap failed: {:?}", result.err());
        
        // Verify custom __import__ is installed
        let has_custom_import = interpreter.enter(|vm| {
            let check_code = r#"
import builtins
_result = builtins.__import__.__name__ == '_virtual_fs_import'
"#;
            match vm.compile(check_code, Mode::Exec, "<test>".to_owned()) {
                Ok(code_obj) => {
                    match vm.run_code_obj(code_obj, scope.clone()) {
                        Ok(_) => {
                            if let Some(result) = scope.globals.get_item_opt("_result", vm).ok().flatten() {
                                result.try_to_bool(vm).unwrap_or(false)
                            } else {
                                false
                            }
                        }
                        Err(e) => {
                            let msg = e.as_object().str(vm).map(|s| s.to_string()).unwrap_or_default();
                            panic!("Check code failed: {}: {}", e.class().name(), msg);
                        }
                    }
                }
                Err(e) => panic!("Check code compile error: {}", e)
            }
        });
        
        assert!(has_custom_import, "Custom __import__ not installed");
    }
    
    #[test]
    fn test_import_from_virtual_fs() {
        reset_mock_state();
        
        // Add a test module to the mock filesystem
        add_mock_file("mymath.py", "def add(a, b):\n    return a + b\n\nPI = 3.14159\n");
        
        let settings = Settings::default();
        let interpreter = Interpreter::with_init(settings, |vm| {
            vm.add_native_module("terminal".to_owned(), Box::new(mock_terminal::make_module));
        });
        
        // Do everything in a single enter() call
        let result: Result<Option<i32>, String> = interpreter.enter(|vm| {
            let scope = vm.new_scope_with_builtins();
            
            // Run bootstrap
            let bootstrap_code = vm.compile(PYTHON_BOOTSTRAP, Mode::Exec, "<bootstrap>".to_owned())
                .map_err(|e| format!("Bootstrap compile error: {}", e))?;
            vm.run_code_obj(bootstrap_code, scope.clone())
                .map_err(|e| {
                    let msg = e.as_object().str(vm).map(|s| s.to_string()).unwrap_or_default();
                    format!("Bootstrap run error: {}: {}", e.class().name(), msg)
                })?;
            
            // Test importing from virtual filesystem using actual import statement
            let import_code = vm.compile(r#"
import mymath
_result = mymath.add(2, 3)
"#, Mode::Exec, "<test>".to_owned())
                .map_err(|e| format!("Import compile error: {}", e))?;
            vm.run_code_obj(import_code, scope.clone())
                .map_err(|e| {
                    let msg = e.as_object().str(vm).map(|s| s.to_string()).unwrap_or_default();
                    format!("Import run error: {}: {}", e.class().name(), msg)
                })?;
            
            // Get result
            if let Some(result) = scope.globals.get_item_opt("_result", vm).ok().flatten() {
                Ok(result.try_to_value::<i32>(vm).ok())
            } else {
                Ok(None)
            }
        });
        
        match result {
            Ok(Some(5)) => (),
            Ok(Some(n)) => panic!("Expected 5, got {}", n),
            Ok(None) => panic!("Result was None"),
            Err(e) => panic!("Error: {}", e),
        }
    }
    
    #[test]
    fn test_import_nonexistent_module_fails() {
        reset_mock_state();
        
        let settings = Settings::default();
        let interpreter = Interpreter::with_init(settings, |vm| {
            vm.add_native_module("terminal".to_owned(), Box::new(mock_terminal::make_module));
        });
        
        let scope = interpreter.enter(|vm| {
            vm.new_scope_with_builtins()
        });
        
        // Run bootstrap
        interpreter.enter(|vm| {
            let code_obj = vm.compile(PYTHON_BOOTSTRAP, Mode::Exec, "<bootstrap>".to_owned()).unwrap();
            vm.run_code_obj(code_obj, scope.clone()).unwrap();
        });
        
        // Try to import a nonexistent module - should fail
        let import_failed = interpreter.enter(|vm| {
            let import_code = "import nonexistent_module";
            match vm.compile(import_code, Mode::Exec, "<test>".to_owned()) {
                Ok(code_obj) => {
                    vm.run_code_obj(code_obj, scope.clone()).is_err()
                }
                Err(_) => false
            }
        });
        
        assert!(import_failed, "Expected import to fail for nonexistent module");
    }
}
