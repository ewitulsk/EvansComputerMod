//! The `web` module (web.py): small web servers (`web.serve`) and CGI scripts
//! (`web.cgi`) for the Browser, shipped as source inside the native `_web`
//! module like `ui`. Lua's `web` has the same names (lua/src/lua/web.lua).

use rustpython_vm::{pymodule, PyObjectRef, VirtualMachine};

pub const WEB_PY: &str = include_str!("web.py");

/// The `_web` native module for the interpreter's module table.
pub fn make_native_module(vm: &VirtualMachine) -> rustpython_vm::PyRef<rustpython_vm::builtins::PyModule> {
    web_native::make_module(vm)
}

#[pymodule]
pub mod web_native {
    use super::*;

    /// Source of the user-facing module (see web.py).
    #[pyattr]
    fn _source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(WEB_PY).into()
    }
}

#[cfg(test)]
mod tests;
