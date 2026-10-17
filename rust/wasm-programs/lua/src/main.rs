//! Standalone Lua 5.2 REPL / script runner (WASI program).
//!
//! Usage:
//!   lua                    — start interactive REPL
//!   lua <file> [args...]   — run a .lua file (args are in the global `arg`)
//!
//! The interpreter is PUC-Rio Lua 5.2 (the version ComputerCraft programs are
//! written for) through mlua. Lua's own `io` and `os` libraries work on the
//! computer's files. The ECM modules are the same as Python's, with the same
//! functions (globals, also available through `require`):
//!   shell       — terminal I/O, files and redstone (`terminal` is the same module)
//!   net         — TCP/IP networking, DNS, ICMP ping, HTTP
//!   peripheral  — attached blocks and bay modules (lua/peripheral.lua over `_peripheral`)
//!   sensors, controller, audio, cards — lidars, Xbox controllers, the Speaker, card readers and printers
//!   radio       — SDR flowgraphs (lua/radio.lua over `_radio`)
//!   ui          — GUI apps (lua/ui.lua over `_ui` and ecm-ui)
//! `require` also finds the computer's own files (a/b.lua or a/b/init.lua).

mod net;
mod peripheral;

use mlua::{Function, Lua, LuaString, MultiValue, Table, Variadic};
use std::io::{self, Write};

// ============================================================================
// shell module — terminal I/O, filesystem access and redstone
// ============================================================================

/// The name Python and Lua give an upload support state.
fn upload_support_name(s: ecm_host_abi::upload::Support) -> &'static str {
    use ecm_host_abi::upload::Support;
    match s {
        Support::Ready => "ready",
        Support::NotInstalled => "not_installed",
        Support::NoClient => "no_client",
        Support::NotPermitted => "not_permitted",
    }
}

/// Run one upload to the end: the new file's path, or why there is none.
fn upload_blocking(dir: &str) -> Result<String, String> {
    use ecm_host_abi::upload::{self, State, Support};
    let support = upload::supported();
    if support != Support::Ready {
        return Err(support.message().to_string());
    }
    let ticket = upload::begin(dir).map_err(|e| format!("{dir}: {}", e.message()))?;
    loop {
        match upload::poll(ticket) {
            State::Done(path) => return Ok(path),
            State::Failed(e) => {
                upload::cancel(ticket);
                return Err(e.message().to_string());
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
}

fn shell_module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;

    // Write text (no newline).
    m.set("write", lua.create_function(|_, s: LuaString| {
        io::stdout().write_all(&s.as_bytes())?;
        io::stdout().flush()?;
        Ok(())
    })?)?;
    // Print text with a newline.
    m.set("println", lua.create_function(|_, s: LuaString| {
        let mut out = io::stdout();
        out.write_all(&s.as_bytes())?;
        out.write_all(b"\n")?;
        out.flush()?;
        Ok(())
    })?)?;
    // Clear the screen.
    m.set("clear", lua.create_function(|_, ()| {
        print!("\x1b[2J\x1b[H");
        io::stdout().flush()?;
        Ok(())
    })?)?;
    // Move the cursor (0-based column and row).
    m.set("set_cursor", lua.create_function(|_, (x, y): (i32, i32)| {
        print!("\x1b[{};{}H", y + 1, x + 1);
        io::stdout().flush()?;
        Ok(())
    })?)?;
    m.set("get_width", lua.create_function(|_, ()| Ok(80))?)?;
    m.set("get_height", lua.create_function(|_, ()| Ok(24))?)?;
    // Upload: a file from the player's real computer (the evanscomputermod-upload add-on).
    // upload_supported() -> "ready" | "not_installed" | "no_client" | "not_permitted".
    m.set("upload_supported", lua.create_function(|_, ()| Ok(upload_support_name(ecm_host_abi::upload::supported())))?)?;
    // upload([dir]) -> path of the new file, or nil, reason (dir defaults to /home).
    m.set("upload", lua.create_function(|_, dir: Option<String>| {
        Ok(match upload_blocking(dir.as_deref().unwrap_or("/home")) {
            Ok(path) => (Some(path), None),
            Err(e) => (None, Some(format!("upload: {e}"))),
        })
    })?)?;
    m.set("sleep", lua.create_function(|_, seconds: f64| {
        let ms = (seconds.max(0.0) * 1000.0) as u64;
        std::thread::sleep(std::time::Duration::from_millis(ms));
        Ok(())
    })?)?;
    // Read a line of input, echoing it as it's typed. nil at end of input.
    m.set("input", lua.create_function(|_, prompt: Option<String>| {
        Ok(read_line_echo(prompt.as_deref().unwrap_or("")))
    })?)?;

    // Files, by path from the computer's root ("/etc/x" and "etc/x" are the same).
    m.set("read_file", lua.create_function(|lua, path: String| {
        ecm_host_abi::fs::read_file_bytes(rel(&path)).map(|b| lua.create_string(b)).transpose()
    })?)?;
    m.set("write_file", lua.create_function(|_, (path, content): (String, LuaString)| {
        let p = rel(&path);
        if let Some((dir, _)) = p.rsplit_once('/') {
            if !dir.is_empty() {
                ecm_host_abi::fs::mkdir(dir);
            }
        }
        // The host returns the bytes written (or a negative error).
        Ok(ecm_host_abi::fs::write_file_bytes(p, &content.as_bytes()) >= 0)
    })?)?;
    m.set("file_exists", lua.create_function(|_, path: String| Ok(ecm_host_abi::fs::exists(rel(&path))))?)?;
    m.set("delete_file", lua.create_function(|_, path: String| Ok(ecm_host_abi::fs::delete(rel(&path))))?)?;
    m.set("file_size", lua.create_function(|_, path: String| {
        let s = ecm_host_abi::fs::size(rel(&path));
        Ok(if s >= 0 { Some(s) } else { None })
    })?)?;
    // Every file on the computer, as a list of paths.
    m.set("list_files", lua.create_function(|lua, ()| {
        let all = ecm_host_abi::fs::list_all().unwrap_or_default();
        lua.create_sequence_from(all.lines().filter(|l| !l.is_empty()).map(str::to_owned))
    })?)?;

    // Redstone, by side: shell.DOWN, UP, FRONT, BACK, LEFT, RIGHT.
    for (i, side) in ["DOWN", "UP", "FRONT", "BACK", "LEFT", "RIGHT"].iter().enumerate() {
        m.set(*side, i as i32)?;
    }
    m.set("set_redstone", lua.create_function(|_, (side, power): (i32, i32)| {
        Ok(ecm_host_abi::redstone::set_output(side, power) == 0)
    })?)?;
    m.set("get_redstone", lua.create_function(|_, side: i32| Ok(ecm_host_abi::redstone::get_input(side)))?)?;
    // All six input levels, indexed by side + 1 (Lua lists start at 1).
    m.set("get_all_redstone", lua.create_function(|lua, ()| {
        let mut buf = [0i32; 6];
        ecm_host_abi::redstone::get_all_input(&mut buf);
        lua.create_sequence_from(buf)
    })?)?;

    Ok(m)
}

/// Register the host-ABI modules (`shell` also as `terminal`, `net`, `_peripheral`),
/// then the rest (ecm_lua::install: the Lua-source modules, `_radio`, `_ui`, the loader).
fn setup(lua: &Lua) -> mlua::Result<()> {
    let shell = shell_module(lua)?;
    let net = net::module(lua)?;
    for (name, module) in [("shell", &shell), ("terminal", &shell), ("net", &net)] {
        lua.globals().set(name, module.clone())?;
        ecm_lua::register(lua, name, module.clone())?;
    }
    ecm_lua::register(lua, "_peripheral", peripheral::module(lua)?)?;
    ecm_lua::install(lua)
}

// ============================================================================
// REPL
// ============================================================================

/// Outcome of compiling one REPL entry.
enum Chunk {
    Ready(Function),
    /// The statement isn't finished yet: keep reading lines.
    Incomplete,
    Error(String),
}

/// Compile a REPL entry the way lua.c does: as an expression first (so `1 + 1`
/// prints 2), then as a statement.
fn compile(lua: &Lua, code: &str) -> Chunk {
    if let Ok(f) = lua.load(format!("return {code}")).set_name("=stdin").into_function() {
        return Chunk::Ready(f);
    }
    match lua.load(code).set_name("=stdin").into_function() {
        Ok(f) => Chunk::Ready(f),
        Err(mlua::Error::SyntaxError { incomplete_input: true, .. }) => Chunk::Incomplete,
        Err(e) => Chunk::Error(message(&e)),
    }
}

fn repl(lua: &Lua) {
    println!("{} (PUC-Rio, via mlua)", lua.globals().get::<String>("_VERSION").unwrap_or_default());
    println!("Type 'exit' or Ctrl+D to exit.");
    println!("Modules: shell, net, peripheral, sensors, controller, audio, cards, radio, ui (as in Python).");
    println!();

    let mut buffer = String::new();
    load_repl_history();
    loop {
        let line = match read_repl_line(if buffer.is_empty() { "> " } else { ">> " }) {
            // Ctrl+C: forget a half-typed block.
            Err(()) => {
                buffer.clear();
                continue;
            }
            Ok(None) => break,
            Ok(Some(line)) => line,
        };
        if buffer.is_empty() && matches!(line.trim(), "exit" | "exit()" | "quit" | "quit()") {
            break;
        }
        if !buffer.is_empty() {
            buffer.push('\n');
        }
        buffer.push_str(&line);

        match compile(lua, &buffer) {
            Chunk::Incomplete => continue,
            Chunk::Error(e) => eprintln!("{e}"),
            Chunk::Ready(f) => match f.call::<MultiValue>(()) {
                Ok(values) if !values.is_empty() => {
                    // Print results like lua.c: through the global print, tab-separated.
                    if let Ok(print) = lua.globals().get::<Function>("print") {
                        if let Err(e) = print.call::<()>(values) {
                            eprintln!("{}", message(&e));
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("{}", message(&e)),
            },
        }
        buffer.clear();
    }
}

/// Run a script with `arg` set like the standalone interpreter: arg[0] is the
/// script, arg[1..] its arguments, which are also passed as `...`.
fn run_file(lua: &Lua, filename: &str, args: &[String]) -> bool {
    let Some(code) = ecm_host_abi::fs::read_file_bytes(rel(filename)) else {
        eprintln!("lua: cannot open {filename}");
        return false;
    };
    let result = (|| {
        let arg = lua.create_table()?;
        arg.set(0, filename)?;
        for (i, a) in args.iter().enumerate() {
            arg.set(i + 1, a.as_str())?;
        }
        lua.globals().set("arg", arg)?;
        // Skip a #! line, as lua.c does.
        let code = if code.starts_with(b"#") {
            let end = code.iter().position(|&b| b == b'\n').unwrap_or(code.len());
            &code[end..]
        } else {
            &code[..]
        };
        let f = lua.load(code).set_name(format!("@{filename}")).into_function()?;
        f.call::<()>(Variadic::from_iter(args.iter().cloned()))
    })();
    match result {
        Ok(()) => true,
        Err(e) => {
            eprintln!("lua: {}", message(&e));
            false
        }
    }
}

/// An error as Lua itself words it: the message (and traceback), without mlua's
/// "runtime error: " / "syntax error: " prefixes.
fn message(e: &mlua::Error) -> String {
    match e {
        mlua::Error::RuntimeError(m) => m.clone(),
        mlua::Error::SyntaxError { message, .. } => message.clone(),
        mlua::Error::CallbackError { traceback, cause } => format!("{}\n{traceback}", message(cause)),
        other => other.to_string(),
    }
}

/// Paths on the computer are relative to its root: "/etc/x" and "etc/x" are the same file.
fn rel(p: &str) -> &str {
    p.trim_start_matches('/')
}

thread_local! {
    /// The line editor (cursor keys, history, Ctrl+R) for the REPL and input().
    static EDITOR: std::cell::RefCell<ecm_lineedit::Editor> = std::cell::RefCell::new(ecm_lineedit::Editor::new());
}

/// Where the REPL keeps its history.
const HISTORY_FILE: &str = "/.lua_history";

/// Read a line with the line editor, which draws `prompt` and echoes what is
/// typed (the terminal doesn't echo a running program's input). Ctrl+C stops
/// the program as usual. None at end of input.
fn read_line_echo(prompt: &str) -> Option<String> {
    EDITOR.with(|e| match ecm_lineedit::stdio::read_line(&mut e.borrow_mut(), prompt, false) {
        ecm_lineedit::stdio::Input::Line(l) => Some(l),
        ecm_lineedit::stdio::Input::Interrupt => Some(String::new()),
        ecm_lineedit::stdio::Input::Eof => None,
    })
}

/// A REPL line: like [`read_line_echo`], but Ctrl+C gives up the line
/// (Err) instead of stopping the program, and the history is saved.
fn read_repl_line(prompt: &str) -> Result<Option<String>, ()> {
    EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        let r = ecm_lineedit::stdio::read_line(&mut ed, prompt, true);
        ecm_lineedit::stdio::save_history(&mut ed, HISTORY_FILE);
        match r {
            ecm_lineedit::stdio::Input::Line(l) => Ok(Some(l)),
            ecm_lineedit::stdio::Input::Interrupt => Err(()),
            ecm_lineedit::stdio::Input::Eof => Ok(None),
        }
    })
}

/// Load the REPL's saved history.
fn load_repl_history() {
    EDITOR.with(|e| ecm_lineedit::stdio::load_history(&mut e.borrow_mut(), HISTORY_FILE));
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let lua = Lua::new();
    if let Err(e) = setup(&lua) {
        eprintln!("lua: setup failed: {e}");
        std::process::exit(1);
    }

    if args.len() > 1 {
        if !run_file(&lua, &args[1], &args[2..]) {
            std::process::exit(1);
        }
    } else {
        repl(&lua);
    }
}
