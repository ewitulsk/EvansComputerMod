//! Dynamic wasm-bindgen stub registration.
//!
//! The WASM binary imports ~50 functions with hashed names from wasm-bindgen.
//! We match by prefix pattern and create appropriate stubs for each.

use wasmtime::*;
use crate::wasm_host::HostState;

/// Register wasm-bindgen stubs for all unresolved imports in the module.
/// Call this AFTER registering all known host functions in the linker.
pub fn register_stubs(linker: &mut Linker<HostState>, module: &Module) -> Result<()> {
    for import in module.imports() {
        let module_name = import.module();
        let name = import.name();

        // Skip if already defined
        if linker.get(&mut Store::new(linker.engine(), HostState::dummy()), module_name, name).is_some() {
            // Can't easily check without a store, so we'll use a try-define approach instead
            continue;
        }

        // We'll handle this differently — register all stubs first, then the linker
        // will use them. See register_all_stubs below.
    }
    Ok(())
}

/// Register all wasm-bindgen stubs by iterating module imports.
/// This must be called AFTER known host functions are registered.
/// Uses Linker::func_wrap which won't error if the name already exists (it overwrites).
/// So we register stubs first, then overwrite with real implementations.
///
/// Actually, the correct approach: register stubs for names we DON'T have.
/// We'll collect import names first, then register stubs for unknowns.
pub fn register_all_stubs(
    linker: &mut Linker<HostState>,
    module: &Module,
    known_names: &[&str],
) -> Result<()> {
    for import in module.imports() {
        let module_name = import.module().to_string();
        let name = import.name().to_string();

        // Skip known host functions
        if known_names.contains(&name.as_str()) {
            continue;
        }

        let extern_type = import.ty();
        let func_type = match extern_type {
            ExternType::Func(ft) => ft,
            _ => continue, // Skip non-function imports (memory, table, etc.)
        };

        register_stub_for_import(linker, &module_name, &name, &func_type)?;
    }
    Ok(())
}

fn register_stub_for_import(
    linker: &mut Linker<HostState>,
    module_name: &str,
    name: &str,
    func_type: &FuncType,
) -> Result<()> {
    // Match by prefix to determine behavior
    if name.starts_with("__wbg_now_") {
        // Date.now() -> f64 (current time in millis)
        linker.func_wrap(module_name, name, || -> f64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64
        })?;
    } else if name.starts_with("__wbg_new_0_") || name.starts_with("__wbg_new0_") {
        // new Date() -> i32 (handle)
        linker.func_wrap(module_name, name, |mut caller: Caller<'_, HostState>| -> i32 {
            let state = caller.data_mut();
            state.next_object_handle += 1;
            state.next_object_handle
        })?;
    } else if name.starts_with("__wbg_getTime_") {
        // Date.getTime() -> f64
        linker.func_wrap(module_name, name, |_handle: i32| -> f64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as f64
        })?;
    } else if name.starts_with("__wbg_getTimezoneOffset_") {
        // Date.getTimezoneOffset() -> f64 (minutes)
        linker.func_wrap(module_name, name, |_handle: i32| -> f64 {
            let offset = chrono::Local::now().offset().local_minus_utc();
            -(offset as f64) / 60.0
        })?;
    } else if name.starts_with("__wbg_crypto_") {
        // Get crypto object -> i32 (non-zero handle)
        linker.func_wrap(module_name, name, |mut caller: Caller<'_, HostState>, _: i32| -> i32 {
            let state = caller.data_mut();
            state.next_object_handle += 1;
            state.next_object_handle
        })?;
    } else if name.starts_with("__wbg_msCrypto_") {
        linker.func_wrap(module_name, name, |_: i32| -> i32 { 0 })?;
    } else if name.starts_with("__wbg_getRandomValues_") || name.starts_with("__wbg_randomFillSync_") {
        linker.func_wrap(module_name, name, |_: i32, _: i32| {})?;
    } else if name.contains("__wbindgen_is_object_") {
        linker.func_wrap(module_name, name, |handle: i32| -> i32 {
            if handle != 0 { 1 } else { 0 }
        })?;
    } else if name.contains("__wbindgen_is_string_") || name.contains("__wbindgen_is_function_") {
        linker.func_wrap(module_name, name, |_: i32| -> i32 { 0 })?;
    } else if name.contains("__wbindgen_is_undefined_") {
        linker.func_wrap(module_name, name, |_: i32| -> i32 { 1 })?;
    } else if name == "__wbindgen_object_drop_ref" {
        linker.func_wrap(module_name, name, |_: i32| {})?;
    } else if name == "__wbindgen_object_clone_ref" {
        linker.func_wrap(module_name, name, |mut caller: Caller<'_, HostState>, _: i32| -> i32 {
            let state = caller.data_mut();
            state.next_object_handle += 1;
            state.next_object_handle
        })?;
    } else if name == "__wbindgen_externref_table_grow" {
        linker.func_wrap(module_name, name, |mut caller: Caller<'_, HostState>, delta: i32| -> i32 {
            let state = caller.data_mut();
            let old = state.next_object_handle;
            state.next_object_handle += delta;
            old
        })?;
    } else if name == "__wbindgen_externref_table_set_null" {
        linker.func_wrap(module_name, name, |_: i32| {})?;
    } else if name == "__wbindgen_describe" {
        linker.func_wrap(module_name, name, |_: i32| {})?;
    } else if name == "__wbindgen_describe_cast" || name.starts_with("__wbindgen_describe_cast") {
        linker.func_wrap(module_name, name, |_: i32, _: i32| -> i32 { 0 })?;
    } else if name.starts_with("__wbg_error_") {
        // Error handler: read string from WASM memory and print to stderr
        linker.func_wrap(module_name, name, |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
            if let Some(msg) = read_string_from_caller(&mut caller, ptr, len) {
                eprintln!("[WASM Error] {}", msg);
            }
        })?;
    } else if name.contains("__wbindgen_throw_") || name.starts_with("__wbindgen_throw") {
        let name_owned = name.to_string();
        linker.func_wrap(module_name, name, move |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> Result<()> {
            let msg = read_string_from_caller(&mut caller, ptr, len)
                .unwrap_or_else(|| "<unknown>".to_string());
            eprintln!("[WASM Throw] {}: {}", name_owned, msg);
            Err(anyhow::anyhow!("WASM throw: {}", msg))
        })?;
    } else if name.starts_with("__wbg_new_with_length_") {
        linker.func_wrap(module_name, name, |mut caller: Caller<'_, HostState>, _: i32| -> i32 {
            let state = caller.data_mut();
            state.next_object_handle += 1;
            state.next_object_handle
        })?;
    } else if name.starts_with("__wbg_subarray_") {
        linker.func_wrap(module_name, name, |mut caller: Caller<'_, HostState>, _: i32, _: i32, _: i32| -> i32 {
            let state = caller.data_mut();
            state.next_object_handle += 1;
            state.next_object_handle
        })?;
    } else if name.starts_with("__wbg_length_") {
        linker.func_wrap(module_name, name, |_: i32| -> i32 { 0 })?;
    } else if name.starts_with("__wbg_static_accessor_") || name.starts_with("__wbg_require_") {
        // () -> i32, return 0
        if func_type.params().len() == 0 {
            linker.func_wrap(module_name, name, || -> i32 { 0 })?;
        } else {
            register_generic_stub(linker, module_name, name, func_type)?;
        }
    } else if name.starts_with("__wbg_process_") || name.starts_with("__wbg_versions_") || name.starts_with("__wbg_node_") {
        linker.func_wrap(module_name, name, |_: i32| -> i32 { 0 })?;
    } else if name.starts_with("__wbg_call_") || name.starts_with("__wbg_new_no_args_") {
        // Match by param count
        register_generic_stub(linker, module_name, name, func_type)?;
    } else if name.starts_with("__wbg_prototypesetcall_") || name.starts_with("__wbg_set_") {
        register_generic_stub(linker, module_name, name, func_type)?;
    } else if name.starts_with("__wbg_new_b") {
        // Date constructor with timestamp: (i32) -> i32
        linker.func_wrap(module_name, name, |mut caller: Caller<'_, HostState>, _: i32| -> i32 {
            let state = caller.data_mut();
            state.next_object_handle += 1;
            state.next_object_handle
        })?;
    } else {
        // Generic fallback: create type-matched stub
        register_generic_stub(linker, module_name, name, func_type)?;
    }

    Ok(())
}

/// Create a generic stub that matches the function's type signature.
/// Returns 0 for i32, 0 for i64, 0.0 for f32/f64.
fn register_generic_stub(
    linker: &mut Linker<HostState>,
    module_name: &str,
    name: &str,
    func_type: &FuncType,
) -> Result<()> {
    let name_for_log = name.to_string();
    let return_types: Vec<ValType> = func_type.results().collect();

    linker.func_new(module_name, name, func_type.clone(), move |_caller, _params, results| {
        eprintln!("[WASM Stub] {} called", name_for_log);
        for (i, result) in results.iter_mut().enumerate() {
            *result = match return_types.get(i) {
                Some(ValType::I32) => Val::I32(0),
                Some(ValType::I64) => Val::I64(0),
                Some(ValType::F32) => Val::F32(0),
                Some(ValType::F64) => Val::F64(0.0f64.to_bits()),
                _ => Val::I32(0),
            };
        }
        Ok(())
    })?;

    Ok(())
}

fn read_string_from_caller(caller: &mut Caller<'_, HostState>, ptr: i32, len: i32) -> Option<String> {
    let memory = caller.get_export("memory")?.into_memory()?;
    let data = memory.data(&caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > data.len() {
        return None;
    }
    String::from_utf8(data[start..end].to_vec()).ok()
}
