//! The parts of the `lua` program that don't touch the host ABI, so they can be
//! tested on the host with a real Lua (`cargo test -p lua`):
//!
//! - the `_radio` and `_ui` native modules (the same `ecm-radio` / `ecm-ui`
//!   engines the Python modules and Rust programs use),
//! - `_ecm` (a wall clock, sleep, float32 unpacking for the Lua modules),
//! - the Lua-source modules `peripheral`, `sensors`, `controller`, `audio`,
//!   `cards`, `radio`, `ui`, `web`, `term`, `events`, `computer`, `colors` and
//!   `keys` (the counterparts of the Python modules of the same names), and
//!   the loader that finds them and the computer's own files,
//! - `_console` (ecm-console: the term, the event queue, the clocks) and
//!   ComputerCraft's globals on top of it (lua/cc.lua, with `colours`).
//!
//! The binary adds the host-ABI modules (`shell`, `terminal`, `net`,
//! `_peripheral`) before calling [`install`].

pub mod console_native;
pub mod periph_values;
pub mod radio_native;
pub mod ui_native;
pub mod values;

use mlua::{Lua, LuaString, Table};

/// The Lua-source modules, by name: `require(name)` loads them, and the first
/// use of the global of the same name does too.
pub const MODULES: &[(&str, &str)] = &[
    ("peripheral", include_str!("lua/peripheral.lua")),
    ("sensors", include_str!("lua/sensors.lua")),
    ("controller", include_str!("lua/controller.lua")),
    ("audio", include_str!("lua/audio.lua")),
    ("cards", include_str!("lua/cards.lua")),
    ("radio", include_str!("lua/radio.lua")),
    ("ui", include_str!("lua/ui.lua")),
    ("web", include_str!("lua/web.lua")),
    ("term", include_str!("lua/term.lua")),
    ("events", include_str!("lua/events.lua")),
    ("computer", include_str!("lua/computer.lua")),
    ("colors", include_str!("lua/colors.lua")),
    ("colours", include_str!("lua/colours.lua")),
    ("keys", include_str!("lua/keys.lua")),
    ("fs", include_str!("lua/fs.lua")),
    ("textutils", include_str!("lua/textutils.lua")),
    ("redstone", include_str!("lua/redstone.lua")),
    ("parallel", include_str!("lua/parallel.lua")),
    ("disk", include_str!("lua/disk.lua")),
];

/// ComputerCraft's globals (`os.pullEvent`, `sleep`, `print` through the
/// term, ...), run at start-up: always on, the modules behind them load lazily.
const CC: &str = include_str!("lua/cc.lua");

/// Module loader, `require` of the computer's files, compat names, lazy globals.
const BOOTSTRAP: &str = include_str!("lua/bootstrap.lua");

/// `_ecm`: small helpers the Lua modules need that Lua 5.2 doesn't have.
fn ecm_module(lua: &Lua) -> mlua::Result<Table> {
    let m = lua.create_table()?;
    // Seconds since the Unix epoch, as a float (Lua's os.time() only has whole seconds).
    m.set("now", lua.create_function(|_, ()| {
        Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0))
    })?)?;
    m.set("sleep", lua.create_function(|_, seconds: f64| {
        if seconds > 0.0 {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds.min(1.0e6)));
        }
        Ok(())
    })?)?;
    // Little-endian float32 values packed in a string (lidar ranges, points) as a list.
    m.set("unpack_f32", lua.create_function(|lua, data: LuaString| {
        let bytes = data.as_bytes();
        if bytes.len() % 4 != 0 {
            return Err(mlua::Error::runtime("float32 data length must be a multiple of 4"));
        }
        let t = lua.create_table_with_capacity(bytes.len() / 4, 0)?;
        for (i, c) in bytes.chunks_exact(4).enumerate() {
            t.raw_set(i + 1, f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64)?;
        }
        Ok(t)
    })?)?;
    Ok(m)
}

/// Register a native module under `name` in `package.loaded`.
pub fn register(lua: &Lua, name: &str, module: Table) -> mlua::Result<()> {
    let loaded: Table = lua.globals().get::<Table>("package")?.get("loaded")?;
    loaded.set(name, module)
}

/// Register the program's console (`_console`): its term, events and clocks.
pub fn register_console(lua: &Lua, session: ecm_console::session::Session) -> mlua::Result<()> {
    let m = console_native::module(lua, session)?;
    lua.load(PLAIN_ERRORS).set_name("=_console").call::<()>(m.clone())?;
    register(lua, "_console", m)
}

/// The natives' errors as plain strings, the way ComputerCraft's functions
/// raise them ("/x: No such file"), so programs can match and print them.
const PLAIN_ERRORS: &str = r#"
local m = ...
for name, f in pairs(m) do
  if type(f) == "function" then
    m[name] = function(...)
      local r = table.pack(pcall(f, ...))
      if r[1] then return table.unpack(r, 2, r.n) end
      local e = r[2]
      if type(e) ~= "string" then
        e = tostring(e):gsub("^runtime error: ", ""):gsub("\nstack traceback:.*$", "")
      end
      error(e, 0)
    end
  end
end
"#;

/// Restore the console's colours and cursor after the program, if it drew.
pub fn finish_console(lua: &Lua) {
    if let Ok(c) = lua.load("return package.loaded._console").eval::<Table>() {
        if let Ok(f) = c.get::<mlua::Function>("finish") {
            let _ = f.call::<()>(());
        }
    }
}

/// Install the natives (`_radio`, `_ui`, `_ecm`), the Lua-source modules and
/// the loader. Modules that need the host (`shell`, `_peripheral`, `net`) are
/// looked up when first used, so a test can register stand-ins for them.
pub fn install(lua: &Lua) -> mlua::Result<()> {
    register(lua, "_radio", radio_native::module(lua)?)?;
    register(lua, "_ui", ui_native::module(lua)?)?;
    register(lua, "_ecm", ecm_module(lua)?)?;
    let loaded: Table = lua.globals().get::<Table>("package")?.get("loaded")?;
    if loaded.get::<mlua::Value>("_console")?.is_nil() {
        // No computer console (host tests): a headless one on virtual time.
        let host = ecm_console::session::fake::Fake::default();
        let term = ecm_console::term::Term::new(51, 19, 1, Some(1));
        register_console(lua, ecm_console::session::Session::new(term, Box::new(host)))?;
    }
    let sources = lua.create_table()?;
    for (name, src) in MODULES {
        sources.set(*name, *src)?;
    }
    let boot: mlua::Function = lua.load(BOOTSTRAP).set_name("=bootstrap").into_function()?;
    boot.call::<()>(sources)?;
    lua.load(CC).set_name("=cc").exec()
}

#[cfg(test)]
mod tests;
