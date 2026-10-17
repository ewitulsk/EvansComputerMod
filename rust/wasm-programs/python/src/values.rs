//! Peripheral values (ecm_host_abi::peripheral::Value) as Python objects and
//! back: peripheral calls, peripheral events and the console's events.

use ecm_host_abi::peripheral::Value;
use rustpython_vm::{
    builtins::{PyBytes, PyDict, PyFloat, PyInt, PyList, PyStr, PyTuple},
    AsObject, PyObjectRef, PyResult, VirtualMachine,
};

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
