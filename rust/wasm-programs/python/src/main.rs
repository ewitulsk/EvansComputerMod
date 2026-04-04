//! Standalone Python REPL / script runner (WASI program).
//!
//! Usage:
//!   python          — start interactive REPL
//!   python <file>   — execute a .py file
//!
//! Custom modules exposed to Python:
//!   shell       — terminal I/O and filesystem access
//!   peripheral  — CC:Tweaked peripheral interaction
//!   net         — TCP/IP networking, DNS, ICMP ping, HTTP
//!   redstone    — redstone I/O

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

use std::io::{self, BufRead, Write};

// Custom random implementation for WASM (required by RustPython)
use getrandom::register_custom_getrandom;

fn custom_getrandom(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    static mut STATE: u64 = 0x853c_49e6_748f_ea9b;
    unsafe {
        for byte in buf.iter_mut() {
            STATE ^= STATE >> 12;
            STATE ^= STATE << 25;
            STATE ^= STATE >> 27;
            *byte = (STATE.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8;
        }
    }
    Ok(())
}

register_custom_getrandom!(custom_getrandom);

/// Bootstrap code for setting up virtual filesystem imports
const PYTHON_BOOTSTRAP: &str = include_str!("python_bootstrap.py");

// ============================================================================
// Shell module — terminal I/O and filesystem access
// ============================================================================

#[pymodule]
mod shell_module {
    use super::*;

    /// Write text (no newline).
    #[pyfunction]
    fn write(s: PyStrRef) {
        print!("{}", s.as_str());
        let _ = io::stdout().flush();
    }

    /// Print text with a newline.
    #[pyfunction]
    fn println(s: PyStrRef) {
        println!("{}", s.as_str());
    }

    /// Clear the screen.
    #[pyfunction]
    fn clear() {
        print!("\x1b[2J\x1b[H");
        let _ = io::stdout().flush();
    }

    /// Set cursor position.
    #[pyfunction]
    fn set_cursor(x: i32, y: i32) {
        // ANSI escape: ESC[row;colH (1-based)
        print!("\x1b[{};{}H", y + 1, x + 1);
        let _ = io::stdout().flush();
    }

    /// Get terminal width.
    #[pyfunction]
    fn get_width() -> i32 {
        // Default terminal width in ECM
        80
    }

    /// Get terminal height.
    #[pyfunction]
    fn get_height() -> i32 {
        // Default terminal height in ECM
        24
    }

    /// Read a file's contents.
    #[pyfunction]
    fn read_file(path: PyStrRef) -> Option<String> {
        ecm_host_abi::fs::read_file(path.as_str())
    }

    /// Write content to a file.
    #[pyfunction]
    fn write_file(path: PyStrRef, content: PyStrRef) -> bool {
        ecm_host_abi::fs::write_file(path.as_str(), content.as_str()) == 0
    }

    /// Check if a file exists.
    #[pyfunction]
    fn file_exists(path: PyStrRef) -> bool {
        ecm_host_abi::fs::exists(path.as_str())
    }

    /// Delete a file.
    #[pyfunction]
    fn delete_file(path: PyStrRef) -> bool {
        ecm_host_abi::fs::delete(path.as_str())
    }

    /// List all files.
    #[pyfunction]
    fn list_files() -> String {
        ecm_host_abi::fs::list_all().unwrap_or_default()
    }

    /// Get file size in bytes.
    #[pyfunction]
    fn file_size(path: PyStrRef) -> Option<usize> {
        let s = ecm_host_abi::fs::size(path.as_str());
        if s >= 0 { Some(s as usize) } else { None }
    }

    /// Sleep for the specified number of seconds.
    #[pyfunction]
    fn sleep(seconds: f64) {
        let ms = (seconds * 1000.0) as u64;
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }

    /// Read a line of text input from the user.
    #[pyfunction]
    fn input(prompt: OptionalArg<PyStrRef>) -> String {
        let prompt_str = match &prompt {
            OptionalArg::Present(s) => s.as_str(),
            OptionalArg::Missing => "",
        };
        if !prompt_str.is_empty() {
            print!("{}", prompt_str);
            let _ = io::stdout().flush();
        }
        let mut line = String::new();
        let _ = io::stdin().lock().read_line(&mut line);
        // Strip trailing newline
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        line
    }

    // ==================== Redstone Functions ====================

    /// Side constants
    #[pyattr]
    const DOWN: i32 = 0;
    #[pyattr]
    const UP: i32 = 1;
    #[pyattr]
    const FRONT: i32 = 2;
    #[pyattr]
    const BACK: i32 = 3;
    #[pyattr]
    const LEFT: i32 = 4;
    #[pyattr]
    const RIGHT: i32 = 5;

    /// Set redstone output power.
    #[pyfunction]
    fn set_redstone(side: i32, power: i32) -> bool {
        ecm_host_abi::redstone::set_output(side, power) == 0
    }

    /// Read redstone input power.
    #[pyfunction]
    fn get_redstone(side: i32) -> i32 {
        ecm_host_abi::redstone::get_input(side)
    }

    /// Read all 6 redstone input power levels.
    #[pyfunction]
    fn get_all_redstone(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let mut buf = [0i32; 6];
        ecm_host_abi::redstone::get_all_input(&mut buf);
        let list = vm.ctx.new_list(
            buf.iter().map(|&v| vm.new_pyobj(v)).collect()
        );
        Ok(list.into())
    }
}

// ============================================================================
// Peripheral module — CC:Tweaked peripheral interaction
// ============================================================================

#[pymodule]
mod peripheral_module {
    use super::*;

    /// List all connected peripherals.
    /// Returns JSON parsed into a list of dicts with 'name', 'type', 'side'.
    #[pyfunction]
    fn list(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let json = ecm_host_abi::peripheral::list_raw().unwrap_or_else(|| "[]".to_string());
        // Parse the JSON array of peripherals
        // Format: [{"name":"...","type":"...","side":"..."},...]
        let result = vm.ctx.new_list(Vec::new());

        // Simple JSON array parser
        let trimmed = json.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return Ok(result.into());
        }
        let inner = &trimmed[1..trimmed.len()-1].trim();
        if inner.is_empty() {
            return Ok(result.into());
        }

        // Split by objects (handling nested braces)
        let objects = split_json_array(inner);
        for obj_str in objects {
            let obj_str = obj_str.trim();
            if !obj_str.starts_with('{') || !obj_str.ends_with('}') {
                continue;
            }
            let dict = vm.ctx.new_dict();
            let pairs = parse_json_object(obj_str);
            for (key, value) in pairs {
                let _ = dict.set_item(&*key, vm.new_pyobj(value), vm);
            }
            result.borrow_vec_mut().push(dict.into());
        }

        Ok(result.into())
    }

    /// Get peripheral names.
    #[pyfunction]
    fn get_names(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let json = ecm_host_abi::peripheral::list_raw().unwrap_or_else(|| "[]".to_string());
        let result = vm.ctx.new_list(Vec::new());
        let trimmed = json.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return Ok(result.into());
        }
        let inner = &trimmed[1..trimmed.len()-1].trim();
        if inner.is_empty() {
            return Ok(result.into());
        }
        let objects = split_json_array(inner);
        for obj_str in objects {
            let pairs = parse_json_object(obj_str.trim());
            for (key, value) in &pairs {
                if key == "name" {
                    result.borrow_vec_mut().push(vm.new_pyobj(value.clone()));
                }
            }
        }
        Ok(result.into())
    }

    /// Get methods available on a peripheral.
    #[pyfunction]
    fn get_methods(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        match ecm_host_abi::peripheral::get_methods_raw(name.as_str()) {
            Some(json) => {
                // JSON array of strings: ["method1","method2",...]
                let list = vm.ctx.new_list(Vec::new());
                let trimmed = json.trim();
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    let inner = &trimmed[1..trimmed.len()-1];
                    for item in inner.split(',') {
                        let item = item.trim().trim_matches('"');
                        if !item.is_empty() {
                            list.borrow_vec_mut().push(vm.new_pyobj(item.to_string()));
                        }
                    }
                }
                Ok(list.into())
            }
            None => {
                Err(vm.new_runtime_error(format!("Peripheral not found: {}", name.as_str())))
            }
        }
    }

    /// Call a method on a peripheral.
    #[pyfunction]
    fn call(name: PyStrRef, method: PyStrRef, args: Option<PyStrRef>) -> String {
        let args_str = args.map(|s| s.as_str().to_string()).unwrap_or_else(|| "[]".to_string());
        match ecm_host_abi::peripheral::call_method(name.as_str(), method.as_str(), &args_str) {
            Some(result) => result,
            None => format!("{{\"ok\":false,\"error\":\"Call failed\"}}"),
        }
    }

    /// Check if a peripheral exists.
    #[pyfunction]
    fn is_present(name: PyStrRef) -> bool {
        let json = ecm_host_abi::peripheral::list_raw().unwrap_or_else(|| "[]".to_string());
        json.contains(&format!("\"name\":\"{}\"", name.as_str()))
    }

    /// Find a peripheral by type.
    #[pyfunction]
    fn find(peripheral_type: PyStrRef) -> Option<String> {
        let json = ecm_host_abi::peripheral::list_raw()?;
        let trimmed = json.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return None;
        }
        let inner = &trimmed[1..trimmed.len()-1].trim();
        let objects = split_json_array(inner);
        for obj_str in objects {
            let pairs = parse_json_object(obj_str.trim());
            let mut found_name = None;
            let mut found_type = false;
            for (key, value) in &pairs {
                if key == "name" {
                    found_name = Some(value.clone());
                }
                if key == "type" && value == peripheral_type.as_str() {
                    found_type = true;
                }
            }
            if found_type {
                return found_name;
            }
        }
        None
    }

    /// Wrap a peripheral.
    #[pyfunction]
    fn wrap(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let json = ecm_host_abi::peripheral::list_raw().unwrap_or_else(|| "[]".to_string());
        let trimmed = json.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return Ok(vm.ctx.none());
        }
        let inner = &trimmed[1..trimmed.len()-1].trim();
        let objects = split_json_array(inner);
        for obj_str in objects {
            let pairs = parse_json_object(obj_str.trim());
            let mut p_name = String::new();
            let mut p_type = String::new();
            for (key, value) in &pairs {
                if key == "name" { p_name = value.clone(); }
                if key == "type" { p_type = value.clone(); }
            }
            if p_name == name.as_str() {
                let dict = vm.ctx.new_dict();
                dict.set_item("name", vm.new_pyobj(p_name), vm)?;
                dict.set_item("type", vm.new_pyobj(p_type), vm)?;
                return Ok(dict.into());
            }
        }
        Ok(vm.ctx.none())
    }
}

// ============================================================================
// Network module — TCP/IP, DNS, ICMP, HTTP
// ============================================================================

#[pymodule]
mod net_module {
    use super::*;

    /// List all interfaces.
    #[pyfunction]
    fn interfaces(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let count = ecm_host_abi::net_config::iface_count();
        let list = vm.ctx.new_list(Vec::new());
        for i in 0..count {
            if let Some(json) = ecm_host_abi::net_config::iface_info(i) {
                let dict = vm.ctx.new_dict();
                let pairs = parse_json_object(json.trim());
                for (key, value) in pairs {
                    match key.as_str() {
                        "prefix_len" | "prefix" => {
                            if let Ok(n) = value.parse::<i32>() {
                                let _ = dict.set_item(&*key, vm.new_pyobj(n), vm);
                            }
                        }
                        "link_up" => {
                            let _ = dict.set_item("link_up", vm.ctx.new_bool(value == "true").into(), vm);
                        }
                        "vlan" => {
                            if value == "null" {
                                let _ = dict.set_item("vlan", vm.ctx.none(), vm);
                            } else if let Ok(n) = value.parse::<i32>() {
                                let _ = dict.set_item("vlan", vm.new_pyobj(n), vm);
                            }
                        }
                        _ => {
                            let _ = dict.set_item(&*key, vm.new_pyobj(value), vm);
                        }
                    }
                }
                list.borrow_vec_mut().push(dict.into());
            }
        }
        Ok(list.into())
    }

    /// Configure interface IP: net.iface_set("eth0", "10.0.0.1/24")
    #[pyfunction]
    fn iface_set(name: PyStrRef, cidr: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let idx = find_iface_index(name.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", name.as_str())))?;
        let (ip, prefix) = parse_cidr(cidr.as_str())
            .ok_or_else(|| vm.new_value_error("Invalid CIDR (e.g. 10.0.0.1/24)".to_string()))?;
        let rc = ecm_host_abi::net_config::iface_configure(idx, &ip, prefix);
        if rc < 0 {
            Err(vm.new_runtime_error("Failed to configure interface".to_string()))
        } else {
            Ok(())
        }
    }

    /// Bring interface up.
    #[pyfunction]
    fn iface_up(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let idx = find_iface_index(name.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", name.as_str())))?;
        ecm_host_abi::net_config::iface_set_link(idx, true);
        Ok(())
    }

    /// Bring interface down.
    #[pyfunction]
    fn iface_down(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let idx = find_iface_index(name.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", name.as_str())))?;
        ecm_host_abi::net_config::iface_set_link(idx, false);
        Ok(())
    }

    /// Set/clear VLAN: net.iface_vlan("eth0", 100) or net.iface_vlan("eth0", None)
    #[pyfunction]
    fn iface_vlan(name: PyStrRef, vid: rustpython_vm::PyObjectRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let idx = find_iface_index(name.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", name.as_str())))?;
        if vm.is_none(&vid) {
            ecm_host_abi::net_config::iface_set_vlan(idx, -1);
        } else {
            let v: i32 = vid.try_into_value(vm)?;
            if v < 0 || v > 4094 {
                return Err(vm.new_value_error("VLAN ID must be 0-4094".to_string()));
            }
            ecm_host_abi::net_config::iface_set_vlan(idx, v);
        }
        Ok(())
    }

    /// List routes.
    #[pyfunction]
    fn routes(vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let json = ecm_host_abi::net_config::route_list().unwrap_or_else(|| "[]".to_string());
        let list = vm.ctx.new_list(Vec::new());
        let trimmed = json.trim();
        if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
            return Ok(list.into());
        }
        let inner = &trimmed[1..trimmed.len()-1].trim();
        if inner.is_empty() {
            return Ok(list.into());
        }
        let objects = split_json_array(inner);

        // We also need iface names for the "dev" field
        let iface_count = ecm_host_abi::net_config::iface_count();

        for obj_str in objects {
            let pairs = parse_json_object(obj_str.trim());
            let dict = vm.ctx.new_dict();
            let mut iface_idx_val: Option<i32> = None;
            for (key, value) in &pairs {
                match key.as_str() {
                    "prefix" | "prefix_len" => {
                        if let Ok(n) = value.parse::<i32>() {
                            let _ = dict.set_item("prefix_len", vm.new_pyobj(n), vm);
                        }
                    }
                    "dest" | "destination" => {
                        let _ = dict.set_item("destination", vm.new_pyobj(value.clone()), vm);
                    }
                    "gateway" => {
                        let _ = dict.set_item("gateway", vm.new_pyobj(value.clone()), vm);
                    }
                    "iface_idx" => {
                        if let Ok(n) = value.parse::<i32>() {
                            iface_idx_val = Some(n);
                        }
                    }
                    _ => {
                        let _ = dict.set_item(&**key, vm.new_pyobj(value.clone()), vm);
                    }
                }
            }
            // Resolve iface index to name
            if let Some(idx) = iface_idx_val {
                if idx >= 0 && idx < iface_count {
                    if let Some(info) = ecm_host_abi::net_config::iface_info(idx) {
                        let iface_pairs = parse_json_object(info.trim());
                        for (k, v) in &iface_pairs {
                            if k == "name" {
                                let _ = dict.set_item("dev", vm.new_pyobj(v.clone()), vm);
                                break;
                            }
                        }
                    }
                }
            }
            list.borrow_vec_mut().push(dict.into());
        }
        Ok(list.into())
    }

    /// Add route: net.route_add("default", "10.0.0.1", "eth0")
    #[pyfunction]
    fn route_add(dest: PyStrRef, gw: PyStrRef, dev: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let iface_idx = find_iface_index(dev.as_str())
            .ok_or_else(|| vm.new_value_error(format!("Unknown interface: {}", dev.as_str())))?;
        if dest.as_str() == "default" {
            let rc = ecm_host_abi::net_config::route_add("0.0.0.0", 0, gw.as_str(), iface_idx);
            if rc < 0 {
                return Err(vm.new_runtime_error("Failed to add route".to_string()));
            }
        } else {
            let (d, p) = parse_cidr(dest.as_str())
                .ok_or_else(|| vm.new_value_error("Invalid CIDR destination".to_string()))?;
            let rc = ecm_host_abi::net_config::route_add(&d, p, gw.as_str(), iface_idx);
            if rc < 0 {
                return Err(vm.new_runtime_error("Failed to add route".to_string()));
            }
        }
        Ok(())
    }

    /// Delete route.
    #[pyfunction]
    fn route_del(dest: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        if dest.as_str() == "default" {
            let rc = ecm_host_abi::net_config::route_del("0.0.0.0", 0);
            if rc < 0 {
                return Err(vm.new_runtime_error("No default route".to_string()));
            }
        } else {
            let (d, p) = parse_cidr(dest.as_str())
                .ok_or_else(|| vm.new_value_error("Invalid CIDR destination".to_string()))?;
            let rc = ecm_host_abi::net_config::route_del(&d, p);
            if rc < 0 {
                return Err(vm.new_runtime_error("Route not found".to_string()));
            }
        }
        Ok(())
    }

    /// Set DNS server.
    #[pyfunction]
    fn dns_set(ip: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<()> {
        let rc = ecm_host_abi::net_config::dns_set(ip.as_str());
        if rc < 0 {
            Err(vm.new_runtime_error("Failed to set DNS".to_string()))
        } else {
            Ok(())
        }
    }

    /// Get DNS server.
    #[pyfunction]
    fn dns_get() -> String {
        ecm_host_abi::net_config::dns_get().unwrap_or_else(|| "0.0.0.0".to_string())
    }

    /// Send ICMP ping. Returns RTT in ms.
    #[pyfunction]
    fn ping(target: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        let rtt = ecm_host_abi::net_config::ping(target.as_str(), 2000);
        if rtt < 0 {
            Err(vm.new_runtime_error(format!("Ping failed for {}", target.as_str())))
        } else {
            Ok(rtt)
        }
    }

    /// Resolve hostname via DNS.
    #[pyfunction]
    fn resolve(name: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<String> {
        let mut ip_out = [0u8; 4];
        let rc = ecm_host_abi::net_ipc::dns_resolve(name.as_str(), &mut ip_out);
        if rc < 0 {
            Err(vm.new_runtime_error(format!("DNS resolution failed for {}", name.as_str())))
        } else {
            Ok(format!("{}.{}.{}.{}", ip_out[0], ip_out[1], ip_out[2], ip_out[3]))
        }
    }

    /// Open a UDP socket (not supported via IPC yet — placeholder).
    #[pyfunction]
    fn udp_open(_port: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        Err(vm.new_runtime_error("UDP not available in WASI python (use kernel python)".to_string()))
    }

    /// Connect to a TCP server.
    #[pyfunction]
    fn tcp_connect(host: PyStrRef, port: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        let idx = ecm_host_abi::net_ipc::tcp_connect(host.as_str(), port as u16, 10000);
        if idx < 0 {
            Err(vm.new_runtime_error(format!("TCP connect failed to {}:{}", host.as_str(), port)))
        } else {
            Ok(idx)
        }
    }

    /// Listen on a TCP port.
    #[pyfunction]
    fn tcp_listen(port: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        let idx = ecm_host_abi::net_ipc::tcp_listen(port as u16);
        if idx < 0 {
            Err(vm.new_runtime_error(format!("TCP listen failed on port {}", port)))
        } else {
            Ok(idx)
        }
    }

    /// Accept a TCP connection.
    #[pyfunction]
    fn tcp_accept(listener: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<i32> {
        let idx = ecm_host_abi::net_ipc::tcp_accept(listener, 30000);
        if idx < 0 {
            Err(vm.new_runtime_error("TCP accept failed".to_string()))
        } else {
            Ok(idx)
        }
    }

    /// Send data on a TCP connection.
    #[pyfunction]
    fn tcp_send(
        conn: i32,
        data: rustpython_vm::builtins::PyBytesRef,
        vm: &VirtualMachine,
    ) -> rustpython_vm::PyResult<i32> {
        let n = ecm_host_abi::net_ipc::tcp_send(conn, data.as_bytes());
        if n < 0 {
            Err(vm.new_runtime_error("TCP send failed".to_string()))
        } else {
            Ok(n)
        }
    }

    /// Receive data from a TCP connection.
    #[pyfunction]
    fn tcp_recv(conn: i32, max_len: i32, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        let mut buf = vec![0u8; max_len as usize];
        let n = ecm_host_abi::net_ipc::tcp_recv(conn, &mut buf, 10000);
        if n < 0 {
            Err(vm.new_runtime_error("TCP recv failed".to_string()))
        } else {
            buf.truncate(n as usize);
            Ok(vm.ctx.new_bytes(buf).into())
        }
    }

    /// Close a TCP connection.
    #[pyfunction]
    fn tcp_close(conn: i32) {
        ecm_host_abi::net_ipc::tcp_close(conn);
    }

    /// HTTP GET request. Returns dict with 'status', 'body', 'headers'.
    /// (Simple implementation using TCP and manual HTTP/1.1)
    #[pyfunction]
    fn http_get(url: PyStrRef, vm: &VirtualMachine) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        do_http_request("GET", url.as_str(), None, vm)
    }

    /// HTTP POST request.
    #[pyfunction(name = "http_post")]
    fn http_post_fn(
        url: PyStrRef,
        body: rustpython_vm::builtins::PyBytesRef,
        vm: &VirtualMachine,
    ) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
        do_http_request("POST", url.as_str(), Some(body.as_bytes()), vm)
    }
}

// ============================================================================
// Helper functions
// ============================================================================

/// Parse a CIDR string like "10.0.0.1/24" into (ip_string, prefix_len).
fn parse_cidr(cidr: &str) -> Option<(String, u8)> {
    let parts: Vec<&str> = cidr.splitn(2, '/').collect();
    if parts.len() != 2 {
        return None;
    }
    let prefix: u8 = parts[1].parse().ok()?;
    // Basic IP validation
    let octets: Vec<&str> = parts[0].split('.').collect();
    if octets.len() != 4 {
        return None;
    }
    for octet in &octets {
        let _: u8 = octet.parse().ok()?;
    }
    Some((parts[0].to_string(), prefix))
}

/// Find an interface index by name.
fn find_iface_index(name: &str) -> Option<i32> {
    let count = ecm_host_abi::net_config::iface_count();
    for i in 0..count {
        if let Some(info) = ecm_host_abi::net_config::iface_info(i) {
            let pairs = parse_json_object(info.trim());
            for (key, value) in &pairs {
                if key == "name" && value == name {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Simple JSON object parser: extracts key-value string pairs from {"key":"value",...}
/// Handles string, number, boolean, and null values (all returned as strings).
fn parse_json_object(json: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let trimmed = json.trim();
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return pairs;
    }
    let inner = &trimmed[1..trimmed.len()-1];

    let mut pos = 0;
    let bytes = inner.as_bytes();
    let len = bytes.len();

    while pos < len {
        // Skip whitespace and commas
        while pos < len && (bytes[pos] == b' ' || bytes[pos] == b',' || bytes[pos] == b'\n' || bytes[pos] == b'\r' || bytes[pos] == b'\t') {
            pos += 1;
        }
        if pos >= len { break; }

        // Expect quoted key
        if bytes[pos] != b'"' { break; }
        pos += 1;
        let key_start = pos;
        while pos < len && bytes[pos] != b'"' { pos += 1; }
        if pos >= len { break; }
        let key = inner[key_start..pos].to_string();
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
                if bytes[pos] == b'\\' { pos += 1; }
                pos += 1;
            }
            let val = inner[val_start..pos].to_string();
            pos += 1;
            pairs.push((key, val));
        } else if bytes[pos] == b't' || bytes[pos] == b'f' {
            // Boolean
            if inner[pos..].starts_with("true") {
                pairs.push((key, "true".to_string()));
                pos += 4;
            } else if inner[pos..].starts_with("false") {
                pairs.push((key, "false".to_string()));
                pos += 5;
            }
        } else if bytes[pos] == b'n' {
            // null
            if inner[pos..].starts_with("null") {
                pairs.push((key, "null".to_string()));
                pos += 4;
            }
        } else if bytes[pos] == b'-' || (bytes[pos] >= b'0' && bytes[pos] <= b'9') {
            // Number
            let num_start = pos;
            if bytes[pos] == b'-' { pos += 1; }
            while pos < len && (bytes[pos] >= b'0' && bytes[pos] <= b'9' || bytes[pos] == b'.') {
                pos += 1;
            }
            pairs.push((key, inner[num_start..pos].to_string()));
        } else if bytes[pos] == b'[' {
            // Array — skip it (return as string)
            let arr_start = pos;
            let mut depth = 1;
            pos += 1;
            while pos < len && depth > 0 {
                if bytes[pos] == b'[' { depth += 1; }
                if bytes[pos] == b']' { depth -= 1; }
                pos += 1;
            }
            pairs.push((key, inner[arr_start..pos].to_string()));
        } else if bytes[pos] == b'{' {
            // Nested object — skip
            let obj_start = pos;
            let mut depth = 1;
            pos += 1;
            while pos < len && depth > 0 {
                if bytes[pos] == b'{' { depth += 1; }
                if bytes[pos] == b'}' { depth -= 1; }
                pos += 1;
            }
            pairs.push((key, inner[obj_start..pos].to_string()));
        } else {
            break;
        }
    }
    pairs
}

/// Split a JSON array's inner content into elements.
fn split_json_array(s: &str) -> Vec<&str> {
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
                ',' if depth == 0 => {
                    result.push(&s[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
    }
    if start < s.len() {
        result.push(&s[start..]);
    }
    result
}

/// Perform a simple HTTP request over TCP using the net_ipc layer.
fn do_http_request(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    vm: &VirtualMachine,
) -> rustpython_vm::PyResult<rustpython_vm::PyObjectRef> {
    // Parse URL: http://host[:port]/path
    let url = if url.starts_with("http://") { &url[7..] } else { url };
    let (host_port, path) = match url.find('/') {
        Some(i) => (&url[..i], &url[i..]),
        None => (url, "/"),
    };
    let (host, port): (&str, u16) = match host_port.find(':') {
        Some(i) => (&host_port[..i], host_port[i+1..].parse().unwrap_or(80)),
        None => (host_port, 80),
    };

    // Resolve hostname if needed
    let ip_str = if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        host.to_string()
    } else {
        let mut ip_out = [0u8; 4];
        let rc = ecm_host_abi::net_ipc::dns_resolve(host, &mut ip_out);
        if rc < 0 {
            return Err(vm.new_runtime_error(format!("DNS resolution failed for {}", host)));
        }
        format!("{}.{}.{}.{}", ip_out[0], ip_out[1], ip_out[2], ip_out[3])
    };

    // Connect
    let conn = ecm_host_abi::net_ipc::tcp_connect(&ip_str, port, 10000);
    if conn < 0 {
        return Err(vm.new_runtime_error(format!("TCP connect failed to {}:{}", ip_str, port)));
    }

    // Build request
    let mut req = format!("{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n", method, path, host);
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");

    // Send request header
    let sent = ecm_host_abi::net_ipc::tcp_send(conn, req.as_bytes());
    if sent < 0 {
        ecm_host_abi::net_ipc::tcp_close(conn);
        return Err(vm.new_runtime_error("Failed to send HTTP request".to_string()));
    }
    // Send body
    if let Some(b) = body {
        ecm_host_abi::net_ipc::tcp_send(conn, b);
    }

    // Read response
    let mut response_data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = ecm_host_abi::net_ipc::tcp_recv(conn, &mut buf, 5000);
        if n <= 0 { break; }
        response_data.extend_from_slice(&buf[..n as usize]);
    }
    ecm_host_abi::net_ipc::tcp_close(conn);

    // Parse response
    let response_str = String::from_utf8_lossy(&response_data);
    let (header_section, body_section) = match response_str.find("\r\n\r\n") {
        Some(i) => (&response_str[..i], &response_data[i+4..]),
        None => return Err(vm.new_runtime_error("Invalid HTTP response".to_string())),
    };

    // Parse status code
    let status_code: i32 = header_section.lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Parse headers
    let headers_dict = vm.ctx.new_dict();
    for line in header_section.lines().skip(1) {
        if let Some(colon) = line.find(':') {
            let key = line[..colon].trim().to_lowercase();
            let value = line[colon+1..].trim().to_string();
            let _ = headers_dict.set_item(&*key, vm.new_pyobj(value), vm);
        }
    }

    let dict = vm.ctx.new_dict();
    dict.set_item("status", vm.new_pyobj(status_code), vm)?;
    let body_str = String::from_utf8_lossy(body_section).to_string();
    dict.set_item("body", vm.new_pyobj(body_str), vm)?;
    dict.set_item("headers", headers_dict.into(), vm)?;

    Ok(dict.into())
}

// ============================================================================
// Python REPL
// ============================================================================

struct PythonRepl {
    input_buffer: String,
    continuation: bool,
    interpreter: Interpreter,
    scope: Option<Scope>,
}

impl PythonRepl {
    fn new() -> Self {
        let settings = Settings::default();

        let interpreter = Interpreter::with_init(settings, |vm| {
            vm.add_native_module("shell".to_owned(), Box::new(shell_module::make_module));
            // Keep "terminal" as alias for backward compatibility
            vm.add_native_module("terminal".to_owned(), Box::new(shell_module::make_module));
            vm.add_native_module("peripheral".to_owned(), Box::new(peripheral_module::make_module));
            vm.add_native_module("net".to_owned(), Box::new(net_module::make_module));
        });

        let scope = interpreter.enter(|vm| {
            vm.new_scope_with_builtins()
        });

        // Execute bootstrap code
        interpreter.enter(|vm| {
            match vm.compile(PYTHON_BOOTSTRAP, Mode::Exec, "<bootstrap>".to_owned()) {
                Ok(code_obj) => {
                    if let Err(e) = vm.run_code_obj(code_obj, scope.clone()) {
                        eprintln!("[Bootstrap Error]");
                        let type_name = e.class().name().to_string();
                        eprint!("{}: ", type_name);
                        if let Ok(msg) = e.as_object().str(vm) {
                            eprintln!("{}", msg.as_str());
                        } else {
                            eprintln!("(unknown error)");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("[Bootstrap Compile Error] {}", e);
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

    fn show_banner(&self) {
        println!("Python 3.11 (RustPython)");
        println!("Type 'exit()' or Ctrl+D to exit.");
        println!("Use 'import shell' for shell I/O functions.");
        println!("Use 'import peripheral' for CC peripherals.");
        println!();
    }

    fn print_prompt(&self) {
        if self.continuation {
            print!("... ");
        } else {
            print!(">>> ");
        }
        let _ = io::stdout().flush();
    }

    fn handle_input(&mut self, input: &str) -> bool {
        let trimmed = input.trim();
        if trimmed == "exit()" || trimmed == "quit()" {
            return true;
        }
        if input.contains('\x04') {
            println!();
            return true;
        }

        if !self.input_buffer.is_empty() {
            self.input_buffer.push('\n');
        }
        self.input_buffer.push_str(input);

        let code = self.input_buffer.clone();

        if self.is_complete(&code) {
            self.continuation = false;
            self.execute(&code);
            self.input_buffer.clear();
        } else {
            self.continuation = true;
        }

        false
    }

    fn is_complete(&self, code: &str) -> bool {
        let trimmed = code.trim();
        if trimmed.is_empty() { return true; }
        if trimmed.ends_with('\\') { return false; }
        if trimmed.ends_with(':') { return false; }

        let mut parens = 0i32;
        let mut brackets = 0i32;
        let mut braces = 0i32;
        let mut in_string = false;
        let mut string_char = ' ';

        for c in code.chars() {
            if in_string {
                if c == string_char { in_string = false; }
            } else {
                match c {
                    '"' | '\'' => { in_string = true; string_char = c; }
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

    fn execute(&mut self, code: &str) {
        let scope = self.scope.take().expect("scope should always exist");

        let scope = self.interpreter.enter(|vm| {
            match vm.compile(code, Mode::Single, "<stdin>".to_owned()) {
                Ok(code_obj) => {
                    if let Err(e) = vm.run_code_obj(code_obj, scope.clone()) {
                        self.print_exception(vm, &e);
                    }
                }
                Err(e) => {
                    eprintln!("SyntaxError: {}", e);
                }
            }
            scope
        });

        self.scope = Some(scope);
    }

    fn run_file(&mut self, code: &str, filename: &str) {
        let scope = self.scope.take().expect("scope should always exist");

        let scope = self.interpreter.enter(|vm| {
            match vm.compile(code, Mode::Exec, filename.to_owned()) {
                Ok(code_obj) => {
                    if let Err(e) = vm.run_code_obj(code_obj, scope.clone()) {
                        self.print_exception(vm, &e);
                    }
                }
                Err(e) => {
                    eprintln!("SyntaxError: {}", e);
                }
            }
            scope
        });

        self.scope = Some(scope);
    }

    fn print_exception(&self, vm: &VirtualMachine, exc: &rustpython_vm::PyRef<rustpython_vm::builtins::PyBaseException>) {
        let type_name = exc.class().name().to_string();
        eprint!("{}: ", type_name);
        if let Ok(msg) = exc.as_object().str(vm) {
            eprintln!("{}", msg.as_str());
        } else {
            eprintln!("(unknown error)");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut repl = PythonRepl::new();

    if args.len() > 1 {
        // Script mode: python <file>
        let filename = &args[1];
        match ecm_host_abi::fs::read_file(filename) {
            Some(code) => {
                repl.run_file(&code, filename);
            }
            None => {
                eprintln!("python: {}: No such file", filename);
                std::process::exit(1);
            }
        }
    } else {
        // Interactive REPL mode
        repl.show_banner();
        let stdin = io::stdin();
        loop {
            repl.print_prompt();
            let mut line = String::new();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => {
                    // EOF
                    println!();
                    break;
                }
                Ok(_) => {
                    // Strip trailing newline
                    if line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }
                    if repl.handle_input(&line) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    }
}
