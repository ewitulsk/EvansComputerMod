//! `desktop` — the desktop shell `dev.ecm.desktop` (see `ecm_apps::desktop`).
//! The compositor starts it as the shell (`/etc/ui.toml`: `shell =
//! "dev.ecm.desktop"`, the default):
//!
//!   desktop [--restart] [--launch PROGRAM[:ARG:ARG...]]...

fn main() {
    let conn = match ecm_ui::conn::default_connection(1) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("desktop: {e} (it is the GUI's shell: start the GUI with `gui`)");
            std::process::exit(1);
        }
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(target_arch = "wasm32")]
    let sys = Box::new(ecm_apps::sys::HostSys);
    #[cfg(not(target_arch = "wasm32"))]
    let sys = Box::new(ecm_apps::sys::MockSys::new("."));
    if let Err(e) = ecm_apps::desktop::run(conn, sys, &args) {
        eprintln!("desktop: {e}");
        std::process::exit(2);
    }
}
