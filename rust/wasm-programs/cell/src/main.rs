//! `cell` — see the library. Cellular v2 through `celld`; the v1 modem engine's
//! pages while a computer's modem still runs it.

#[cfg(target_os = "wasi")]
mod v1;

#[cfg(target_os = "wasi")]
fn main() {
    use cell::{parse, render, request};
    use ecm_host_abi::peripheral;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = match parse(&args) {
        Ok(c) => c,
        Err(u) => {
            eprintln!("{u}");
            std::process::exit(2);
        }
    };
    let list = peripheral::list().unwrap_or_default();
    let framed = list.iter().any(|(_, t)| t == "framed_radio");
    let v1: Vec<(String, String)> = list.iter().filter(|(_, t)| v1::TYPES.contains(&t.as_str())).cloned().collect();
    if !framed && !v1.is_empty() {
        v1::run(&cmd, &v1);
        return;
    }
    match celld::host::query(request(&cmd), 1500) {
        Ok(answer) => {
            print!("{}", render(&cmd, &answer));
            if answer.starts_with("error") {
                std::process::exit(1);
            }
        }
        Err(_) if framed => {
            eprintln!("cell: celld is not running. Start it with `celld &` (an ePhone starts it by itself");
            eprintln!("when a modem and a SIM are in).");
            std::process::exit(1);
        }
        Err(_) => {
            v1::no_modem();
        }
    }
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("cell runs on an ECM computer (wasm32-wasip1)");
    std::process::exit(1);
}
