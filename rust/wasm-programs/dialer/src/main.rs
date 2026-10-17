//! `dialer` — Phone (dev.ecm.dialer): calls through telephonyd (docs/TELEPHONY.md).
//!
//!   dialer [--incoming <call id>] [--number <n>] [--call <n>]
//!
//! telephonyd starts it with `--incoming` for a ringing call; Contacts and the
//! `phone` module use `--number` (the keypad with that number) and `--call`.
//! It needs the GUI and the `phone` permission (the control socket, port 7720).

#[cfg(target_os = "wasi")]
fn main() {
    use ecm_tel_ui::dialer::{Args, DialerApp};
    use ecm_tel_ui::net::{Fs, Net};
    use ecm_ui::conn::{default_connection, ConnError};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(ConnError::Console) => {
            println!("dialer: Phone needs the GUI (start it with `gui`); in the console use tel");
            std::process::exit(1);
        }
        Err(e) => {
            println!("dialer: {e}");
            std::process::exit(1);
        }
    };
    let mut app = match DialerApp::new(conn, Net::new(ecm_tel::ctl::PORT), Fs) {
        Ok(a) => a,
        Err(e) => {
            println!("dialer: {e}");
            std::process::exit(1);
        }
    };
    app.start(&Args::parse(&args));
    app.run();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    let _ = ecm_tel_ui::POLL_MS;
    let _ = ecm_ui::conn::default_connection;
    eprintln!("dialer runs on a computer (wasm32-wasip1)");
}
