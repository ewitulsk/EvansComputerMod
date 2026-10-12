//! `cellsite` — the Cell Site app (dev.ecm.cellsite): this computer's cell site
//! (`celld-site`) in a window. Installed as a system app on a computer that has
//! `/etc/celld-site.toml` (docs/gui/CELLULAR_V2.md); run from a terminal it needs the GUI.

#[cfg(target_os = "wasi")]
fn main() {
    use ecm_cell_net::AdminClient;
    use ecm_ui::conn::{default_connection, ConnError};
    use std::time::Duration;

    /// The admin socket, reconnected when the daemon comes and goes.
    struct Net {
        port: u16,
        c: Option<AdminClient>,
    }
    impl ecm_cell_ui::Admin for Net {
        fn call(&mut self, cmd: &str) -> Result<Result<Vec<String>, String>, String> {
            for _ in 0..2 {
                if self.c.is_none() {
                    self.c = AdminClient::connect(self.port);
                }
                let Some(c) = self.c.as_mut() else { return Err("not running".into()) };
                match c.call(cmd, Duration::from_secs(5)) {
                    Ok(r) => return Ok(r),
                    Err(_) => {
                        if let Some(c) = self.c.take() {
                            c.close();
                        }
                    }
                }
            }
            Err("the connection to celld-site failed".into())
        }
    }

    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(ConnError::Console) => {
            println!("cellsite: the Cell Site app needs the GUI (start it with `gui`); in the console use celld-site status");
            std::process::exit(1);
        }
        Err(e) => {
            println!("cellsite: {e}");
            std::process::exit(1);
        }
    };
    // The admin port from the site's settings when this program may read them (typed in a terminal).
    let port = std::fs::read_to_string(celld_site::config::PATH)
        .ok()
        .and_then(|t| celld_site::config::Conf::parse(&t).ok())
        .and_then(|c| c.get("admin_port").parse().ok())
        .unwrap_or(celld_site::config::ADMIN_PORT);
    let mut app = match ecm_cell_ui::site::SiteApp::new(conn, Net { port, c: None }) {
        Ok(a) => a,
        Err(e) => {
            println!("cellsite: {e}");
            std::process::exit(1);
        }
    };
    app.run();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    let _ = ecm_cell_ui::POLL_MS;
    let _ = ecm_ui::conn::default_connection;
    eprintln!("cellsite runs on a computer (wasm32-wasip1)");
}
