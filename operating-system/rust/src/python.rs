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
    
    // ==================== Player Detector Functions ====================
    // These functions are only available when the AP integration mod is installed.
    
    /// Check if the player detector peripheral is available.
    /// 
    /// Returns:
    ///     bool: True if Advanced Peripherals integration is installed
    /// 
    /// Example:
    ///     if terminal.player_detector_available():
    ///         players = terminal.get_online_players()
    #[pyfunction]
    fn player_detector_available() -> bool {
        crate::peripherals::player_detector::is_available()
    }
    
    /// Get a list of all online players.
    /// 
    /// Returns:
    ///     list or None: List of player names, or None if peripheral not available
    /// 
    /// Example:
    ///     players = terminal.get_online_players()
    ///     if players is not None:
    ///         for name in players:
    ///             terminal.println(name)
    #[pyfunction]
    fn get_online_players(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::player_detector::get_online_players() {
            Some(players) => {
                let list: Vec<rustpython_vm::PyObjectRef> = players
                    .into_iter()
                    .map(|s| vm.ctx.new_str(s).into())
                    .collect();
                vm.ctx.new_list(list).into()
            }
            None => vm.ctx.none()
        }
    }
    
    /// Get players within a certain range of the terminal.
    /// 
    /// Args:
    ///     range: Maximum distance in blocks (-1 for unlimited)
    /// 
    /// Returns:
    ///     list or None: List of player names, or None if peripheral not available
    /// 
    /// Example:
    ///     nearby = terminal.get_players_in_range(50)
    ///     if nearby:
    ///         terminal.println(f"Found {len(nearby)} players nearby")
    #[pyfunction]
    fn get_players_in_range(range: i32, vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::player_detector::get_players_in_range(range) {
            Some(players) => {
                let list: Vec<rustpython_vm::PyObjectRef> = players
                    .into_iter()
                    .map(|s| vm.ctx.new_str(s).into())
                    .collect();
                vm.ctx.new_list(list).into()
            }
            None => vm.ctx.none()
        }
    }
    
    /// Check if a specific player is within range of the terminal.
    /// 
    /// Args:
    ///     range: Maximum distance in blocks (-1 for unlimited)
    ///     name: Player name to check
    /// 
    /// Returns:
    ///     bool or None: True if in range, False if not, None if unavailable
    /// 
    /// Example:
    ///     if terminal.is_player_in_range(100, "Steve"):
    ///         terminal.println("Steve is nearby!")
    #[pyfunction]
    fn is_player_in_range(range: i32, name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::player_detector::is_player_in_range(range, name.as_str()) {
            Some(true) => vm.ctx.new_bool(true).into(),
            Some(false) => vm.ctx.new_bool(false).into(),
            None => vm.ctx.none()
        }
    }
    
    /// Get the total count of online players.
    /// 
    /// Returns:
    ///     int or None: Number of online players, or None if unavailable
    /// 
    /// Example:
    ///     count = terminal.get_player_count()
    ///     if count is not None:
    ///         terminal.println(f"{count} players online")
    #[pyfunction]
    fn get_player_count(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::player_detector::get_player_count() {
            Some(count) => vm.ctx.new_int(count).into(),
            None => vm.ctx.none()
        }
    }
    
    /// Get detailed information about a player.
    /// 
    /// Args:
    ///     name: Player name
    /// 
    /// Returns:
    ///     dict or None: Player info with x, y, z, dimension, health, etc.
    /// 
    /// Example:
    ///     info = terminal.get_player_info("Steve")
    ///     if info:
    ///         terminal.println(f"Steve is at ({info['x']}, {info['y']}, {info['z']})")
    #[pyfunction]
    fn get_player_info(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        use rustpython_vm::builtins::PyDict;
        
        match crate::peripherals::player_detector::get_player_pos(name.as_str()) {
            Some(info) => {
                let dict = PyDict::new_ref(&vm.ctx);
                dict.set_item("x", vm.ctx.new_float(info.x).into(), vm)?;
                dict.set_item("y", vm.ctx.new_float(info.y).into(), vm)?;
                dict.set_item("z", vm.ctx.new_float(info.z).into(), vm)?;
                dict.set_item("dimension", vm.ctx.new_str(info.dimension).into(), vm)?;
                dict.set_item("yaw", vm.ctx.new_float(info.yaw as f64).into(), vm)?;
                dict.set_item("pitch", vm.ctx.new_float(info.pitch as f64).into(), vm)?;
                dict.set_item("health", vm.ctx.new_float(info.health as f64).into(), vm)?;
                dict.set_item("max_health", vm.ctx.new_float(info.max_health as f64).into(), vm)?;
                Ok(dict.into())
            }
            None => Ok(vm.ctx.none())
        }
    }
    
    // ==================== Environment Detector Functions ====================
    
    #[pyfunction]
    fn environment_available() -> bool {
        crate::peripherals::environment_detector::is_available()
    }
    
    #[pyfunction]
    fn get_biome(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::get_biome() {
            Some(s) => vm.ctx.new_str(s).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_time(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::get_time() {
            Some(t) => vm.ctx.new_int(t).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_moon_id(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::get_moon_id() {
            Some(id) => vm.ctx.new_int(id).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_moon_name(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::get_moon_name() {
            Some(s) => vm.ctx.new_str(s).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn is_raining(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::is_raining() {
            Some(b) => vm.ctx.new_bool(b).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn is_thunder(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::is_thunder() {
            Some(b) => vm.ctx.new_bool(b).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn is_sunny(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::is_sunny() {
            Some(b) => vm.ctx.new_bool(b).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_dimension(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::get_dimension() {
            Some(s) => vm.ctx.new_str(s).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn list_dimensions(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::list_dimensions() {
            Some(dims) => {
                let list: Vec<rustpython_vm::PyObjectRef> = dims
                    .into_iter()
                    .map(|s| vm.ctx.new_str(s).into())
                    .collect();
                vm.ctx.new_list(list).into()
            }
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_sky_light_level(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::get_sky_light_level() {
            Some(l) => vm.ctx.new_int(l).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_block_light_level(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::get_block_light_level() {
            Some(l) => vm.ctx.new_int(l).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_day_light_level(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::get_day_light_level() {
            Some(l) => vm.ctx.new_int(l).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn is_slime_chunk(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::is_slime_chunk() {
            Some(b) => vm.ctx.new_bool(b).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn scan_entities(radius: i32, vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::environment_detector::scan_entities(radius) {
            Some(json) => vm.ctx.new_str(json).into(),
            None => vm.ctx.none()
        }
    }
    
    // ==================== Chat Box Functions ====================
    
    #[pyfunction]
    fn chat_available() -> bool {
        crate::peripherals::chat_box::is_available()
    }
    
    #[pyfunction]
    fn send_chat(message: PyStrRef) -> bool {
        crate::peripherals::chat_box::send_message(message.as_str())
    }
    
    #[pyfunction]
    fn send_chat_to_player(player: PyStrRef, message: PyStrRef) -> bool {
        crate::peripherals::chat_box::send_message_to_player(player.as_str(), message.as_str())
    }
    
    #[pyfunction]
    fn send_toast(player: PyStrRef, title: PyStrRef, message: PyStrRef) -> bool {
        crate::peripherals::chat_box::send_toast(player.as_str(), title.as_str(), message.as_str())
    }
    
    // ==================== Geo Scanner Functions ====================
    
    #[pyfunction]
    fn geo_scanner_available() -> bool {
        crate::peripherals::geo_scanner::is_available()
    }
    
    #[pyfunction]
    fn geo_scan(radius: i32, vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::geo_scanner::scan(radius) {
            Some(json) => vm.ctx.new_str(json).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn geo_chunk_analyze(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::geo_scanner::chunk_analyze() {
            Some(json) => vm.ctx.new_str(json).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn geo_scan_cost(radius: i32, vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::geo_scanner::scan_cost(radius) {
            Some(cost) => vm.ctx.new_int(cost).into(),
            None => vm.ctx.none()
        }
    }
    
    // ==================== Block Reader Functions ====================
    
    #[pyfunction]
    fn block_reader_available() -> bool {
        crate::peripherals::block_reader::is_available()
    }
    
    #[pyfunction]
    fn get_block_name(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::block_reader::get_name() {
            Some(s) => vm.ctx.new_str(s).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_block_data(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::block_reader::get_data() {
            Some(json) => vm.ctx.new_str(json).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_block_states(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::block_reader::get_states() {
            Some(json) => vm.ctx.new_str(json).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn is_tile_entity(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::block_reader::is_tile_entity() {
            Some(b) => vm.ctx.new_bool(b).into(),
            None => vm.ctx.none()
        }
    }
    
    // ==================== Energy Detector Functions ====================
    
    #[pyfunction]
    fn energy_detector_available() -> bool {
        crate::peripherals::energy_detector::is_available()
    }
    
    #[pyfunction]
    fn get_energy_transfer_rate(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::energy_detector::get_transfer_rate() {
            Some(rate) => vm.ctx.new_int(rate).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn get_energy_transfer_limit(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::energy_detector::get_transfer_rate_limit() {
            Some(limit) => vm.ctx.new_int(limit).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn set_energy_transfer_limit(rate: i32) -> bool {
        crate::peripherals::energy_detector::set_transfer_rate_limit(rate)
    }
    
    // ==================== NBT Storage Functions ====================
    
    #[pyfunction]
    fn nbt_storage_available() -> bool {
        crate::peripherals::nbt_storage::is_available()
    }
    
    #[pyfunction]
    fn nbt_read(vm: &VirtualMachine) -> rustpython_vm::PyObjectRef {
        match crate::peripherals::nbt_storage::read() {
            Some(json) => vm.ctx.new_str(json).into(),
            None => vm.ctx.none()
        }
    }
    
    #[pyfunction]
    fn nbt_write(json: PyStrRef) -> bool {
        crate::peripherals::nbt_storage::write_json(json.as_str())
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
