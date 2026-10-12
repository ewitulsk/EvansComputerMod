//! `picker` — the system file picker (docs/gui/APPS.md "File picker and
//! chooser"). The compositor starts it for an app's `PickFile`:
//!
//!   picker --req <token> --app <app id> --mode open|save [--types .txt,.md] [--suggested <name>]
//!
//! and gets the answer as `picked <token> <path>|cancel` on /dev/uictl.

use ecm_apps_ui::picker::{parse_args, HostFs, PickerApp};
use ecm_apps_ui::uictl::UiCtl;
use ecm_ui::conn::default_connection;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = match parse_args(&args) {
        Ok(r) => r,
        Err(e) => {
            println!("picker: {e}\nusage: picker --req <token> --app <id> --mode open|save [--types .txt,.md] [--suggested <name>]");
            std::process::exit(2);
        }
    };
    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(e) => {
            println!("picker: {e}");
            std::process::exit(1);
        }
    };
    match PickerApp::new(conn, req, Box::new(HostFs { root: String::new() }), UiCtl) {
        Ok(mut a) => a.run(),
        Err(e) => {
            println!("picker: {e}");
            std::process::exit(1);
        }
    }
}
