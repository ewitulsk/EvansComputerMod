//! `settings` — Settings (see `ecm_apps::settings`).   settings [wifi|cellular|display|appearance|about]
//! Needs the GUI; in console mode it says so and exits.

fn main() {
    let conn = match ecm_ui::conn::default_connection(0) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("settings: {e} (start the GUI with `gui`)");
            std::process::exit(1);
        }
    };
    #[allow(unused_variables)]
    let arg: Option<String> = std::env::args().nth(1);
    #[cfg(target_arch = "wasm32")]
    let sys = Box::new(ecm_apps::sys::HostSys);
    #[cfg(not(target_arch = "wasm32"))]
    let sys = Box::new(ecm_apps::sys::MockSys::new("."));
    match ecm_apps::settings::app(conn, sys, arg.as_deref()) {
        Ok(mut app) => app.run(),
        Err(e) => {
            eprintln!("settings: {e}");
            std::process::exit(2);
        }
    }
}
