//! Switch OS — a second WASM kernel targeted at network-switch workloads.
//!
//! Unlike `terminal-os`, the networking utilities (`ifconfig`, `ip`, `ping`,
//! `nslookup`, `resolvectl`) are **baked into the kernel**: they are pulled in
//! as library crates and dispatched in-process by `BakedDispatcher`, without
//! spawning WASI sub-processes. All other code is shared via
//! `ecm-kernel-core`, `ecm-net`, and `ecm-net-tools`.

extern crate alloc;

use ecm_kernel_core::framebuffer;
use ecm_kernel_core::interrupt;
use ecm_kernel_core::shell_parse;
use ecm_kernel_core::vte::Vte;
use ecm_net as net;

mod net_tools;
mod term;

use net_tools::KernelNetTools;

// ---- Custom getrandom (same pattern as terminal-os) ----
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

// ---- Kernel globals ----

/// Physical VTE writing into the memory-mapped framebuffer at FB_BASE.
pub(crate) static mut PHYSICAL_VTE: Option<Vte> = None;

/// Line buffer for interactive input (Enter-terminated).
static mut INPUT_BUF: [u8; 256] = [0u8; 256];
static mut INPUT_LEN: usize = 0;

// ---- WASM exports ----

#[cfg(all(target_arch = "wasm32", not(test)))]
#[unsafe(no_mangle)]
pub fn main() {
    framebuffer::init();
    unsafe {
        PHYSICAL_VTE = Some(Vte::new_physical(
            framebuffer::DEFAULT_WIDTH,
            framebuffer::DEFAULT_HEIGHT,
        ));
    }
    term::clear();

    net::NetStack::init();
    interrupt::register(interrupt::IRQ_NETWORK, |_irq, _data| {
        if let Some(stack) = net::NetStack::get() {
            stack.poll_rx();
        }
    });

    // Restore any network.cfg persisted by a previous session. The netlink
    // handler writes this file back whenever interfaces/routes/DNS mutate.
    let config_restored = if let Some(stack) = net::NetStack::get() {
        ecm_kernel_core::net_config::load(stack)
    } else {
        false
    };

    term::println("================================================================================");
    term::println("                         SWITCH OS v0.1                                        ");
    term::println("================================================================================");
    term::println("");
    if config_restored {
        term::println("Network config restored.");
    }
    term::println("Baked-in commands: ifconfig, ip, ping, nslookup, resolvectl, help, clear");
    term::println("");
    print_prompt();
}

#[cfg(all(target_arch = "wasm32", not(test)))]
#[unsafe(no_mangle)]
pub fn on_input(ptr: *const u8, len: usize) {
    let input = unsafe {
        let slice = core::slice::from_raw_parts(ptr, len);
        core::str::from_utf8_unchecked(slice)
    };
    handle_input(input);
}

#[cfg(all(target_arch = "wasm32", not(test)))]
#[unsafe(no_mangle)]
pub fn on_interrupt(irq: i32, data_ptr: *const u8, data_len: usize) {
    let data = unsafe {
        let slice = core::slice::from_raw_parts(data_ptr, data_len);
        core::str::from_utf8_unchecked(slice)
    };
    if irq == interrupt::IRQ_TERMINATE {
        reset_to_shell();
        return;
    }
    interrupt::dispatch_rust(irq, data);
}

// ---- Shell loop ----

fn print_prompt() {
    term::print("/ > ");
}

fn reset_to_shell() {
    unsafe {
        INPUT_LEN = 0;
    }
    term::clear();
    term::println("^T - reset");
    print_prompt();
}

/// Character-at-a-time input handler, mirroring terminal-os's editing behavior.
fn handle_input(input: &str) {
    for &byte in input.as_bytes() {
        if byte == 0x14 {
            // Ctrl+T
            reset_to_shell();
            return;
        }
        match byte {
            b'\n' | b'\r' => {
                term::println("");
                let line = unsafe {
                    core::str::from_utf8_unchecked(&INPUT_BUF[..INPUT_LEN]).to_string()
                };
                unsafe {
                    INPUT_LEN = 0;
                }
                if !line.trim().is_empty() {
                    process_command(&line);
                }
                print_prompt();
            }
            8 | 127 => unsafe {
                if INPUT_LEN > 0 {
                    INPUT_LEN -= 1;
                    term::print("\x08 \x08");
                }
            },
            b if (32..127).contains(&b) => unsafe {
                if INPUT_LEN < INPUT_BUF.len() {
                    INPUT_BUF[INPUT_LEN] = b;
                    INPUT_LEN += 1;
                    let ch = [b];
                    if let Ok(s) = core::str::from_utf8(&ch) {
                        term::print(s);
                    }
                }
            },
            _ => {}
        }
    }
}

fn process_command(line: &str) {
    let pipeline = shell_parse::parse_pipeline(line);
    // Switch-OS MVP ignores pipes/redirects/backgrounding and just runs the
    // first (usually only) stage synchronously. Pipes can be added later once
    // the kernel grows proper fd plumbing.
    let stage = match pipeline.stages.first() {
        Some(s) => s,
        None => return,
    };
    if stage.command.is_empty() {
        return;
    }

    match stage.command.as_str() {
        "help" => {
            term::println("Commands:");
            term::println("  ifconfig [iface [<ip>/<prefix>|up|down]]");
            term::println("  ip addr | ip route | ip link");
            term::println("  ping <host> [count]");
            term::println("  nslookup <host>");
            term::println("  resolvectl status | query <host>");
            term::println("  clear      clear the screen");
            term::println("  help       show this message");
        }
        "clear" => term::clear(),
        "ifconfig" => {
            let mut nt = KernelNetTools::new();
            ifconfig::run(&stage.args, &mut nt);
        }
        "ip" => {
            let mut nt = KernelNetTools::new();
            ip::run(&stage.args, &mut nt);
        }
        "ping" => {
            let mut nt = KernelNetTools::new();
            ping::run(&stage.args, &mut nt);
        }
        "nslookup" => {
            let mut nt = KernelNetTools::new();
            nslookup::run(&stage.args, &mut nt);
        }
        "resolvectl" => {
            let mut nt = KernelNetTools::new();
            resolvectl::run(&stage.args, &mut nt);
        }
        other => {
            term::print(other);
            term::println(": command not found");
        }
    }
}
