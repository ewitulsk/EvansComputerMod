//! `notes` — the §13 Notes sample of `ecm-ui` as a program: a text field, an
//! Add button and the list of notes, drawn as a display-list tree.
//! Needs the GUI (`gui`); in console mode it says so and exits.

fn main() {
    let conn = match ecm_ui::conn::default_connection(0) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("notes: {e} (start the GUI with `gui`)");
            std::process::exit(1);
        }
    };
    let mut app = ecm_ui::samples::notes(conn);
    println!("notes: connected");
    app.run();
}
