//! `store` — the App Store (docs/gui/APPS.md "App Store"), a system app
//! (`/apps/dev.ecm.store`, permissions `packages` and `network`).
//!
//!   store                         the App Store window (needs the GUI)
//!   store --app <id>              open on an app's page
//!   store --sample-repo <dir> [--sign <key file>]
//!                                 write the sample repository's packages into
//!                                 <dir> for `pkg serve <dir>`

use ecm_apps_ui::appctl::HostCtl;
use ecm_apps_ui::store::StoreApp;
use ecm_apps_ui::uictl::arg;
use ecm_pkg::client::{Client, SocketNet};
use ecm_ui::conn::{default_connection, ConnError};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--help") {
        println!("usage: store [--app <id>] | store --sample-repo <dir> [--sign <key file>]");
        return;
    }
    if let Some(dir) = arg(&args, "--sample-repo") {
        let secret = match arg(&args, "--sign") {
            Some(f) => match std::fs::read_to_string(&f) {
                Ok(s) => Some(s.trim().to_string()),
                Err(e) => {
                    println!("store: cannot read {f}: {e}");
                    std::process::exit(1);
                }
            },
            None => None,
        };
        match ecm_apps_ui::samples::write_repo(&dir, secret.as_deref()) {
            Ok(lines) => {
                for l in &lines {
                    println!("  {l}");
                }
                println!("store: wrote the sample repository ({} apps) to {dir}; serve it with: pkg serve {dir}", ecm_apps_ui::samples::APPS.len());
            }
            Err(e) => {
                println!("store: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(ConnError::Console) => {
            println!("store: the App Store needs the GUI (start it with `gui`); in the console use pkg");
            std::process::exit(1);
        }
        Err(e) => {
            println!("store: {e}");
            std::process::exit(1);
        }
    };
    let mut app = match StoreApp::new(conn, Client::new("", SocketNet), HostCtl, true) {
        Ok(a) => a,
        Err(e) => {
            println!("store: {e}");
            std::process::exit(1);
        }
    };
    if let Some(id) = arg(&args, "--app") {
        app.show(&id);
    }
    println!("store: open");
    app.run();
}
