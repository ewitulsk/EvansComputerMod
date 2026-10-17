//! `_auth` native module under the Python `auth` module (auth.py): accounts on an Auth
//! server (`authd`) or a backend's own realm. Every call goes to
//! `ecm_auth::host::script`, the same functions Lua's `auth` calls, so both
//! interpreters pin, save and refuse the same way. The network calls need a computer;
//! off one (host tests) they raise "auth needs a computer's network".

use ecm_auth::host::script;
use ecm_auth::store::Login;
use rustpython_vm::{builtins::PyStrRef, pymodule, PyObjectRef, PyResult, VirtualMachine};

pub const AUTH_PY: &str = include_str!("auth.py");

/// The `_auth` native module for the interpreter's module table.
pub fn make_native_module(vm: &VirtualMachine) -> rustpython_vm::PyRef<rustpython_vm::builtins::PyModule> {
    auth_native::make_module(vm)
}

fn err(vm: &VirtualMachine, e: ecm_auth::AuthError) -> rustpython_vm::builtins::PyBaseExceptionRef {
    vm.new_runtime_error(e.to_string())
}

/// [realm, fingerprint, host, user, display, id] (auth.py makes the dict).
fn login_list(l: Login, vm: &VirtualMachine) -> PyObjectRef {
    vm.ctx
        .new_list(vec![
            vm.ctx.new_str(l.realm_name).into(),
            vm.ctx.new_str(l.realm_fp).into(),
            vm.ctx.new_str(l.host).into(),
            vm.ctx.new_str(l.username).into(),
            vm.ctx.new_str(l.display_name).into(),
            vm.ctx.new_int(l.user_id).into(),
        ])
        .into()
}

fn opt(s: &Option<PyStrRef>) -> Option<&str> {
    s.as_ref().map(|s| s.as_str()).filter(|s| !s.is_empty())
}

#[pymodule]
pub mod auth_native {
    use super::*;

    #[pyattr]
    fn _source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(AUTH_PY).into()
    }

    #[pyattr]
    const DEFAULT_PORT: u16 = ecm_auth::DEFAULT_PORT;

    #[pyfunction]
    fn signin(dir: PyStrRef, host: PyStrRef, user: PyStrRef, password: PyStrRef, vm: &VirtualMachine) -> PyResult {
        script::signin(dir.as_str(), host.as_str(), user.as_str(), password.as_str()).map(|l| login_list(l, vm)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn signup(dir: PyStrRef, host: PyStrRef, user: PyStrRef, password: PyStrRef, display: Option<PyStrRef>, vm: &VirtualMachine) -> PyResult {
        script::signup(dir.as_str(), host.as_str(), user.as_str(), password.as_str(), opt(&display)).map(|l| login_list(l, vm)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn token(dir: PyStrRef, audience: PyStrRef, host: Option<PyStrRef>, vm: &VirtualMachine) -> PyResult<String> {
        script::token(dir.as_str(), audience.as_str(), opt(&host)).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn prove(dir: PyStrRef, nonce: PyStrRef, audience: PyStrRef, host: Option<PyStrRef>, vm: &VirtualMachine) -> PyResult<String> {
        script::prove(dir.as_str(), nonce.as_str(), audience.as_str(), opt(&host)).map_err(|e| err(vm, e))
    }

    /// [fingerprint, realm name, pinned].
    #[pyfunction]
    fn fingerprint(dir: PyStrRef, host: PyStrRef, vm: &VirtualMachine) -> PyResult {
        let (fp, name, pinned) = script::fingerprint(dir.as_str(), host.as_str()).map_err(|e| err(vm, e))?;
        Ok(vm.ctx.new_list(vec![vm.ctx.new_str(fp).into(), vm.ctx.new_str(name).into(), vm.ctx.new_bool(pinned).into()]).into())
    }

    #[pyfunction]
    fn logout(dir: PyStrRef, host: Option<PyStrRef>, vm: &VirtualMachine) -> PyResult<bool> {
        script::logout(dir.as_str(), opt(&host)).map_err(|e| err(vm, e))
    }

    /// [[fingerprint, host, realm], ...].
    #[pyfunction]
    fn pins(dir: PyStrRef, vm: &VirtualMachine) -> PyObjectRef {
        let rows = script::pins(dir.as_str())
            .into_iter()
            .map(|p| vm.ctx.new_list(vec![vm.ctx.new_str(p.fp).into(), vm.ctx.new_str(p.host).into(), vm.ctx.new_str(p.name).into()]).into())
            .collect();
        vm.ctx.new_list(rows).into()
    }

    #[pyfunction]
    fn unpin(dir: PyStrRef, which: PyStrRef, vm: &VirtualMachine) -> PyResult<bool> {
        script::unpin(dir.as_str(), which.as_str()).map_err(|e| err(vm, e))
    }

    #[pyfunction]
    fn display_fingerprint(fp: PyStrRef, vm: &VirtualMachine) -> PyResult<String> {
        match ecm_auth::parse_fingerprint(fp.as_str()) {
            Some(fp) => Ok(ecm_auth::display_fingerprint(&fp)),
            None => Err(vm.new_value_error(format!("'{}' is not a fingerprint (32 hex digits)", fp.as_str()))),
        }
    }
}

#[cfg(test)]
mod tests;
