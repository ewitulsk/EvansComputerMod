//! `_peripheral` native module: thin bindings over `ecm_host_abi::peripheral`.
//! The user-facing `peripheral` module (peripheral.py) is built on top of it.

use ecm_host_abi::peripheral::{self as abi, Error, Value};
use rustpython_vm::{
    builtins::{PyBytes, PyDict, PyFloat, PyInt, PyList, PyStr, PyTuple},
    pymodule, AsObject, PyObjectRef, PyResult, VirtualMachine,
};

/// Source of the Python-level `peripheral` module, executed by the bootstrap.
pub const PERIPHERAL_PY: &str = include_str!("peripheral.py");

/// Source of the Python-level `sensors` module (Wired Sensor Module helpers).
pub const SENSORS_PY: &str = include_str!("sensors.py");

/// Source of the Python-level `controller` module (Wireless Xbox Controller).
pub const CONTROLLER_PY: &str = include_str!("controller.py");

const MAX_DEPTH: usize = 32;

pub fn to_value(obj: &PyObjectRef, vm: &VirtualMachine, depth: usize) -> PyResult<Value> {
    if depth > MAX_DEPTH {
        return Err(vm.new_value_error("value nested too deeply to send to a peripheral".to_owned()));
    }
    if vm.is_none(obj) {
        return Ok(Value::Nil);
    }
    // bool is a subclass of int: check it first.
    if obj.class().is(vm.ctx.types.bool_type) {
        return Ok(Value::Bool(obj.is(&vm.ctx.true_value)));
    }
    if let Some(i) = obj.payload::<PyInt>() {
        return Ok(Value::Int(i.try_to_primitive::<i64>(vm)?));
    }
    if let Some(f) = obj.payload::<PyFloat>() {
        return Ok(Value::Float(f.to_f64()));
    }
    if let Some(s) = obj.payload::<PyStr>() {
        return Ok(Value::Str(s.as_str().to_owned()));
    }
    if let Some(b) = obj.payload::<PyBytes>() {
        return Ok(Value::Bytes(b.as_bytes().to_vec()));
    }
    if let Some(l) = obj.payload::<PyList>() {
        let items: Vec<PyObjectRef> = l.borrow_vec().to_vec();
        return items.iter().map(|o| to_value(o, vm, depth + 1)).collect::<PyResult<_>>().map(Value::List);
    }
    if let Some(t) = obj.payload::<PyTuple>() {
        return t.as_slice().iter().map(|o| to_value(o, vm, depth + 1)).collect::<PyResult<_>>().map(Value::List);
    }
    if let Some(d) = obj.payload::<PyDict>() {
        let mut entries = Vec::new();
        for (k, v) in d {
            entries.push((to_value(&k, vm, depth + 1)?, to_value(&v, vm, depth + 1)?));
        }
        return Ok(Value::Map(entries));
    }
    Err(vm.new_type_error(format!(
        "can't send a '{}' to a peripheral (use None, bool, int, float, str, bytes, list, tuple or dict)",
        obj.class().name()
    )))
}

pub fn to_py(value: Value, vm: &VirtualMachine) -> PyResult<PyObjectRef> {
    Ok(match value {
        Value::Nil => vm.ctx.none(),
        Value::Bool(b) => vm.ctx.new_bool(b).into(),
        Value::Int(i) => vm.ctx.new_int(i).into(),
        Value::Float(f) => vm.ctx.new_float(f).into(),
        Value::Str(s) => vm.ctx.new_str(s).into(),
        Value::Bytes(b) => vm.ctx.new_bytes(b).into(),
        Value::List(items) => {
            let objs = items.into_iter().map(|v| to_py(v, vm)).collect::<PyResult<Vec<_>>>()?;
            vm.ctx.new_list(objs).into()
        }
        Value::Map(entries) => {
            let dict = vm.ctx.new_dict();
            for (k, v) in entries {
                dict.set_item(&*to_py(k, vm)?, to_py(v, vm)?, vm)?;
            }
            dict.into()
        }
    })
}

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
