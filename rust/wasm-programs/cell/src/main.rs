//! `cell` — see the library. Talks to `celld` over its control socket.

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
    match celld::host::query(request(&cmd), 1500) {
        Ok(answer) => {
            print!("{}", render(&cmd, &answer));
            if answer.starts_with("error") {
                std::process::exit(1);
            }
        }
        Err(_) => {
            let framed = peripheral::list().unwrap_or_default().iter().any(|(_, t)| t == "framed_radio");
            if framed {
                eprintln!("cell: celld is not running. Start it with `celld &` (an ePhone starts it by itself");
                eprintln!("when a modem and a SIM are in).");
            } else {
                eprintln!("cell: this computer has no cellular modem.");
                eprintln!("Fit an LTE Modem or mmWave Modem in a module bay (right-click the computer");
                eprintln!("with a Module Expansion Card, then the modem), and give it a SIM card");
                eprintln!("(right-click the modem's bay with the SIM; an ePhone has a SIM tray).");
            }
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    eprintln!("cell runs on an ECM computer (wasm32-wasip1)");
    std::process::exit(1);
}
