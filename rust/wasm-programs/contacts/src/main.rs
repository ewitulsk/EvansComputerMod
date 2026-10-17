//! `contacts` — Contacts (dev.ecm.contacts): the phone's address book
//! (`/home/.contacts/contacts.vcf`) in a window. Run from a terminal it needs the GUI.
//!
//! ```text
//! contacts                     the list
//! contacts --add <number>      Add, with the number filled in
//! contacts --show <number>     that contact's page (Add when unknown)
//! contacts <file>.vcf          import the cards in the file
//! ```

#[cfg(target_os = "wasi")]
fn main() {
    use ecm_tel_ui::contacts::{ContactsApp, Start};
    use ecm_tel_ui::net::{Fs, Net};
    use ecm_ui::conn::{default_connection, ConnError};

    let args: Vec<String> = std::env::args().skip(1).collect();
    let conn = match default_connection(0) {
        Ok(c) => c,
        Err(ConnError::Console) => {
            println!("contacts: Contacts needs the GUI (start it with `gui`); the book is /home/.contacts/contacts.vcf");
            std::process::exit(1);
        }
        Err(e) => {
            println!("contacts: {e}");
            std::process::exit(1);
        }
    };
    // My card's number comes from telephonyd's `status` on the texts port.
    let mut app = match ContactsApp::new(conn, Net::new(ecm_tel::ctl::SMS_PORT), Fs) {
        Ok(a) => a,
        Err(e) => {
            println!("contacts: {e}");
            std::process::exit(1);
        }
    };
    app.start(Start::parse(&args));
    app.run();
}

#[cfg(not(target_os = "wasi"))]
fn main() {
    let _ = ecm_tel_ui::POLL_MS;
    let _ = ecm_ui::conn::default_connection;
    eprintln!("contacts runs on a computer (wasm32-wasip1)");
}
