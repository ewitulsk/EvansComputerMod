//! `chooser` — the "open with" chooser (docs/gui/APPS.md "File picker and
//! chooser"). The compositor starts it when several apps open a file and none
//! is the default:
//!
//!   chooser --req <token> --path <file>
//!
//! and gets the answer as `chosen <token> <app id> [always]|cancel` on /dev/uictl.

use ecm_apps_ui::appctl::HostCtl;
use ecm_apps_ui::chooser::{parse_args, ChooserApp};
use ecm_apps_ui::uictl::UiCtl;
use ecm_ui::conn::default_connection;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = match parse_args(&args) {
        Ok(r) => r,
        Err(e) => {
            println!("chooser: {e}\nusage: chooser --req <token> --path <file>");
            std::process::exit(2);
        }
    };
    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(e) => {
            println!("chooser: {e}");
            std::process::exit(1);
        }
    };
    match ChooserApp::new(conn, req, &mut HostCtl, UiCtl) {
        Ok(mut a) => a.run(),
        Err(e) => {
            println!("chooser: {e}");
            std::process::exit(1);
        }
    }
}
