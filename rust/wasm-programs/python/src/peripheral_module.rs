//! `_peripheral` native module: thin bindings over `ecm_host_abi::peripheral`.
//! The user-facing `peripheral` module (peripheral.py) is built on top of it.

use ecm_host_abi::peripheral::{self as abi, Error, Value};
use ecm_python::values::{to_py, to_value};
use rustpython_vm::{pymodule, PyObjectRef, PyResult, VirtualMachine};

/// Source of the Python-level `peripheral` module, executed by the bootstrap.
pub const PERIPHERAL_PY: &str = include_str!("peripheral.py");

/// Source of the Python-level `sensors` module (Wired Sensor Module helpers).
pub const SENSORS_PY: &str = include_str!("sensors.py");

/// Source of the Python-level `controller` module (Wireless Xbox Controller).
pub const CONTROLLER_PY: &str = include_str!("controller.py");

/// Source of the Python-level `audio` module (Speaker, /dev/audio).
pub const AUDIO_PY: &str = include_str!("audio.py");

/// Map a bridge failure to a Python exception. Peripheral errors come back as
/// `(False, message)` from `call` instead, so peripheral.py can raise its own
/// `PeripheralError`.
fn host_error(e: Error, vm: &VirtualMachine) -> rustpython_vm::builtins::PyBaseExceptionRef {
    match e {
        Error::Unavailable => vm.new_runtime_error("peripherals are not available here".to_owned()),
        Error::Malformed(what) => vm.new_runtime_error(format!("bad reply from the computer: {what}")),
        Error::Peripheral(msg) => vm.new_runtime_error(msg),
    }
}

#[pymodule]
pub mod peripheral_native {
    use super::*;

    /// Source of the user-facing module (see peripheral.py).
    #[pyattr]
    fn _source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(PERIPHERAL_PY).into()
    }

    /// Source of the `sensors` module (see sensors.py).
    #[pyattr]
    fn _sensors_source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(SENSORS_PY).into()
    }

    /// Source of the `controller` module (see controller.py).
    #[pyattr]
    fn _controller_source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(CONTROLLER_PY).into()
    }

    /// Source of the `audio` module (see audio.py).
    #[pyattr]
    fn _audio_source(vm: &VirtualMachine) -> PyObjectRef {
        vm.ctx.new_str(AUDIO_PY).into()
    }

    /// Little-endian float32 values packed in `data` (lidar ranges, points) as a list of floats.
    #[pyfunction]
    fn unpack_f32(data: rustpython_vm::builtins::PyBytesRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let bytes = data.as_bytes();
        if bytes.len() % 4 != 0 {
            return Err(vm.new_value_error("float32 data length must be a multiple of 4".to_owned()));
        }
        let objs = bytes
            .chunks_exact(4)
            .map(|c| vm.ctx.new_float(f32::from_le_bytes([c[0], c[1], c[2], c[3]]) as f64).into())
            .collect();
        Ok(vm.ctx.new_list(objs).into())
    }

    /// [(name, type), ...] for every attached peripheral.
    #[pyfunction]
    fn list(vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let items = abi::list().map_err(|e| host_error(e, vm))?;
        let objs = items
            .into_iter()
            .map(|(n, t)| vm.ctx.new_tuple(vec![vm.ctx.new_str(n).into(), vm.ctx.new_str(t).into()]).into())
            .collect();
        Ok(vm.ctx.new_list(objs).into())
    }

    /// (ok, (type, [methods])) or (False, message).
    #[pyfunction]
    fn methods(name: rustpython_vm::builtins::PyStrRef, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        Ok(match abi::methods(name.as_str()) {
            Ok((ty, names)) => {
                let names = names.into_iter().map(|n| vm.ctx.new_str(n).into()).collect();
                let inner = vm.ctx.new_tuple(vec![vm.ctx.new_str(ty).into(), vm.ctx.new_list(names).into()]);
                vm.ctx.new_tuple(vec![vm.ctx.new_bool(true).into(), inner.into()]).into()
            }
            Err(Error::Peripheral(msg)) => {
                vm.ctx.new_tuple(vec![vm.ctx.new_bool(false).into(), vm.ctx.new_str(msg).into()]).into()
            }
            Err(e) => return Err(host_error(e, vm)),
        })
    }

    /// (True, result) or (False, message).
    #[pyfunction]
    fn call(
        name: rustpython_vm::builtins::PyStrRef,
        method: rustpython_vm::builtins::PyStrRef,
        args: PyObjectRef,
        vm: &VirtualMachine,
    ) -> PyResult<PyObjectRef> {
        let Value::List(args) = to_value(&args, vm, 0)? else {
            return Err(vm.new_type_error("call() arguments must be a list or tuple".to_owned()));
        };
        Ok(match abi::call(name.as_str(), method.as_str(), &args) {
            Ok(v) => vm.ctx.new_tuple(vec![vm.ctx.new_bool(true).into(), to_py(v, vm)?]).into(),
            Err(Error::Peripheral(msg)) => {
                vm.ctx.new_tuple(vec![vm.ctx.new_bool(false).into(), vm.ctx.new_str(msg).into()]).into()
            }
            Err(e) => return Err(host_error(e, vm)),
        })
    }

    /// Next event as a tuple (event, attachment, *args), or None on timeout.
    /// `filter` None = any event; `timeout_ms` < 0 = wait forever.
    #[pyfunction]
    fn wait_event(filter: PyObjectRef, timeout_ms: i32, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
        let filter = if vm.is_none(&filter) {
            None
        } else {
            Some(filter.str(vm)?.as_str().to_owned())
        };
        match abi::wait_event(filter.as_deref(), timeout_ms).map_err(|e| host_error(e, vm))? {
            None => Ok(vm.ctx.none()),
            Some(items) => {
                let objs = items.into_iter().map(|v| to_py(v, vm)).collect::<PyResult<Vec<_>>>()?;
                Ok(vm.ctx.new_tuple(objs).into())
            }
        }
    }
}
