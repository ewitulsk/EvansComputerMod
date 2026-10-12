//! `visual` — the Visual Programmer (docs/gui/VISUAL_PROGRAMMER.md).
//!
//!   visual [FILE.vpl | FILE.ui.json]   open the editor (a GUI app)
//!   visual --install-samples           write sample programs into /visual
//!   visual --python FILE.vpl           print the Python a program generates
//!   visual --run FILE.vpl              run a program here, showing its output
//!   visual --package FILE.vpl          write a GUI app's folder under /visual/apps
//!
//! Without a GUI the editor says how to start one; the other forms work in
//! console mode.

#[cfg(target_arch = "wasm32")]
fn editor() -> ecm_visual::editor::Editor {
    use ecm_visual::blocks::Registry;
    let mut reg = Registry::builtin();
    if let Ok(types) = ecm_host_abi::peripheral::catalog() {
        reg.add_peripherals(&types.into_iter().map(convert).collect::<Vec<_>>());
    }
    ecm_visual::editor::Editor::new(reg, Box::new(ecm_visual::files::HostFiles), Box::new(ecm_visual::runner::SessionRunner::default()))
}

#[cfg(target_arch = "wasm32")]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    match arg(0).as_str() {
        "--help" | "-h" | "help" => println!("{}", ecm_visual::cli::USAGE),
        "--install-samples" => match ecm_visual::cli::install_samples(&mut editor()) {
            Ok(paths) => {
                for p in &paths {
                    println!("  /{p}");
                }
                println!("visual: installed {} sample programs", paths.len());
            }
            Err(e) => {
                eprintln!("visual: {e}");
                std::process::exit(1);
            }
        },
        "--python" => match ecm_visual::cli::python(&mut editor(), &arg(1)) {
            Ok(code) => print!("{code}"),
            Err(e) => {
                eprintln!("visual: {e}");
                std::process::exit(1);
            }
        },
        "--package" => match ecm_visual::cli::package(&mut editor(), &arg(1)) {
            Ok(lines) => {
                for l in lines {
                    println!("{l}");
                }
            }
            Err(e) => {
                eprintln!("visual: {e}");
                std::process::exit(1);
            }
        },
        "--run" => {
            let mut out = std::io::stdout();
            let code = ecm_visual::cli::run(&mut editor(), &arg(1), &mut out, &|| std::thread::sleep(std::time::Duration::from_millis(50)));
            std::process::exit(code);
        }
        a if a.starts_with("--") => {
            eprintln!("{}", ecm_visual::cli::USAGE);
            std::process::exit(2);
        }
        _ => gui(args.first().cloned()),
    }
}

#[cfg(target_arch = "wasm32")]
fn gui(open: Option<String>) {
    use ecm_ui::conn::{default_connection, ConnError};
    use ecm_ui::App;
    use ecm_ui_proto::Message;
    use std::cell::RefCell;
    use std::rc::Rc;

    let conn = match default_connection(ecm_ui_proto::host::ROLE_APP) {
        Ok(c) => c,
        Err(ConnError::Console) => {
            println!("{}", ecm_visual::CONSOLE_MESSAGE);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("visual: {e}");
            std::process::exit(1);
        }
    };
    let mut app = match App::with_connection(ecm_visual::APP_ID, conn) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("visual: {e}");
            std::process::exit(1);
        }
    };
    let ed = Rc::new(RefCell::new(editor()));
    if let Some(path) = open {
        let r = ed.borrow_mut().open(&path);
        if let Err(e) = r {
            ed.borrow_mut().status = e;
        }
    }
    ecm_visual::view::install(&mut app, ed.clone());
    // "Open with" from Files.
    let opener = ed.clone();
    app.on_message(move |_, m| {
        if let Message::OpenFile(f) = m {
            let r = opener.borrow_mut().open(&f.path);
            if let Err(e) = r {
                opener.borrow_mut().status = e;
            }
        }
    });
    loop {
        // Poll the running program's output while one runs.
        let timeout = if ed.borrow().runner.running() { 100 } else { -1 };
        if !app.step(timeout) {
            break;
        }
        ed.borrow_mut().poll_run();
    }
    ed.borrow_mut().runner.stop();
}

#[cfg(target_arch = "wasm32")]
fn convert(t: ecm_host_abi::peripheral::CatalogType) -> ecm_visual::blocks::PeripheralType {
    ecm_visual::blocks::PeripheralType {
        name: t.name,
        description: t.description,
        methods: t
            .methods
            .into_iter()
            .map(|m| ecm_visual::blocks::PeripheralMethod { name: m.name, description: m.description, params: m.params, returns: m.returns })
            .collect(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    // Native builds exist for the library's tests; there is no computer here.
    println!("{}", ecm_visual::CONSOLE_MESSAGE);
    std::process::exit(1);
}
