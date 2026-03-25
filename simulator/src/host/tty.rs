//! TTY host functions for the kernel.

use wasmtime::*;
use crate::wasm_host::HostState;
use crate::tty::TtyRegistry;
use super::memory;
use std::sync::{Arc, Mutex};

pub const FUNCTIONS: &[&str] = &[
    "tty_create",
    "tty_attach_fd",
    "tty_set_foreground",
    "tty_get_size",
    "tty_write_input",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // tty_create(width, height) -> tty_id or -1
    linker.func_wrap("env", "tty_create",
        |caller: Caller<'_, HostState>, width: i32, height: i32| -> i32 {
            let registry = match caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>() {
                Some(r) => r.clone(),
                None => return -1,
            };
            let mut reg = registry.lock().unwrap();
            reg.create(width as u16, height as u16) as i32
        },
    )?;

    // tty_attach_fd(tty_id, mode) -> fd or -1
    // mode: 0=read, 1=write
    linker.func_wrap("env", "tty_attach_fd",
        |mut caller: Caller<'_, HostState>, tty_id: i32, mode: i32| -> i32 {
            let fd_box: Box<dyn crate::fd::FileDescriptor> = {
                let registry = match caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>() {
                    Some(r) => r.clone(),
                    None => return -1,
                };
                let reg = registry.lock().unwrap();
                let tty = match reg.get(tty_id as u32) {
                    Some(t) => t,
                    None => return -1,
                };

                if mode == 0 {
                    Box::new(tty.create_read_fd())
                } else {
                    Box::new(tty.create_write_fd())
                }
            };

            let fd_table = match caller.data_mut().get_custom_mut::<crate::fd::FdTable>() {
                Some(t) => t,
                None => return -1,
            };
            fd_table.allocate(fd_box)
        },
    )?;

    // tty_set_foreground(tty_id) -> 0 ok, -1 error
    linker.func_wrap("env", "tty_set_foreground",
        |caller: Caller<'_, HostState>, tty_id: i32| -> i32 {
            let registry = match caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>() {
                Some(r) => r.clone(),
                None => return -1,
            };
            let mut reg = registry.lock().unwrap();
            if reg.set_foreground(tty_id as u32) { 0 } else { -1 }
        },
    )?;

    // tty_get_size(tty_id, width_ptr, height_ptr) -> 0 ok, -1 error
    linker.func_wrap("env", "tty_get_size",
        |mut caller: Caller<'_, HostState>, tty_id: i32, width_ptr: i32, height_ptr: i32| -> i32 {
            let (w, h) = {
                let registry = match caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>() {
                    Some(r) => r.clone(),
                    None => return -1,
                };
                let reg = registry.lock().unwrap();
                let tty = match reg.get(tty_id as u32) {
                    Some(t) => t,
                    None => return -1,
                };
                (tty.width as i32, tty.height as i32)
            };

            memory::write_bytes(&mut caller, width_ptr, &w.to_le_bytes());
            memory::write_bytes(&mut caller, height_ptr, &h.to_le_bytes());
            0
        },
    )?;

    // tty_write_input(tty_id, buf_ptr, buf_len) -> bytes_written or -1
    linker.func_wrap("env", "tty_write_input",
        |mut caller: Caller<'_, HostState>, tty_id: i32, buf_ptr: i32, buf_len: i32| -> i32 {
            let data = match memory::read_bytes(&mut caller, buf_ptr, buf_len) {
                Some(d) => d,
                None => return -1,
            };
            let registry = match caller.data().get_custom::<Arc<Mutex<TtyRegistry>>>() {
                Some(r) => r.clone(),
                None => return -1,
            };
            let reg = registry.lock().unwrap();
            match reg.get(tty_id as u32) {
                Some(tty) => {
                    tty.push_input(&data);
                    data.len() as i32
                }
                None => -1,
            }
        },
    )?;

    Ok(())
}
