//! `visual` on the command line, which works without a GUI:
//!
//!   visual --install-samples   write sample programs into /visual
//!   visual --python FILE.vpl   print the Python a program generates
//!   visual --run FILE.vpl      run it (as `python`, in a process of its own)
//!                              and show its output here

use crate::editor::Editor;
use std::io::Write;

pub const USAGE: &str = "usage: visual [FILE.vpl | FILE.ui.json]   open the Visual Programmer (GUI)\n       visual --install-samples          write sample programs into /visual\n       visual --python FILE.vpl          print the Python a program generates\n       visual --run FILE.vpl             run a program here and show its output";

/// Write the samples; the paths written.
pub fn install_samples(ed: &mut Editor) -> Result<Vec<String>, String> {
    let mut done = Vec::new();
    for (path, text) in crate::samples::sample_files(&ed.reg) {
        ed.files.write(&path, text.as_bytes())?;
        done.push(path);
    }
    Ok(done)
}

/// The Python of a program file.
pub fn python(ed: &mut Editor, path: &str) -> Result<String, String> {
    ed.open(path)?;
    let code = ed.python();
    if code.starts_with("ERROR:") {
        return Err(code);
    }
    Ok(code)
}

/// Run a program headless: start it like Run does and copy its output to
/// `out` until it ends. `wait` sleeps a little between polls. Returns the
/// exit status for `visual`.
pub fn run(ed: &mut Editor, path: &str, out: &mut dyn Write, wait: &dyn Fn()) -> i32 {
    if let Err(e) = ed.open(path) {
        let _ = writeln!(out, "visual: {e}");
        return 1;
    }
    if crate::codegen::is_app(&ed.graph) {
        let _ = writeln!(out, "visual: {path} is a GUI app; run it from the Visual Programmer (start the GUI with `gui`).");
        return 1;
    }
    ed.run();
    if ed.run.is_none() {
        let _ = writeln!(out, "visual: {}", ed.status);
        return 1;
    }
    let mut shown = 0;
    loop {
        ed.poll_run();
        let Some(r) = &ed.run else { break };
        if r.text.len() > shown {
            let _ = out.write_all(&r.text.as_bytes()[shown..]);
            let _ = out.flush();
            shown = r.text.len();
        } else if r.text.len() < shown {
            shown = r.text.len();
        }
        if !r.running {
            break;
        }
        wait();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::Registry;
    use crate::files::{Files, MemFiles};
    use crate::runner::MockRunner;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn samples_install_and_run_headless() {
        let files = Rc::new(RefCell::new(MemFiles::default()));
        let runner = Rc::new(RefCell::new(MockRunner { script: vec!["Hello, world!\r\n".into()], ..Default::default() }));
        let mut ed = Editor::new(Registry::builtin(), Box::new(files.clone()), Box::new(runner.clone()));
        let paths = install_samples(&mut ed).unwrap();
        assert_eq!(paths.len(), 8);
        assert!(files.borrow().exists("visual/counter.ui.json"));
        assert_eq!(python(&mut ed, "visual/hello.vpl").unwrap(), include_str!("../tests/vpl/hello.py"));
        let mut out = Vec::new();
        assert_eq!(run(&mut ed, "visual/hello.vpl", &mut out, &|| {}), 0);
        assert_eq!(String::from_utf8(out).unwrap(), "Hello, world!\n[finished]\n");
        assert_eq!(runner.borrow().commands, vec!["python /visual/hello.py".to_string()]);
        let mut out = Vec::new();
        assert_eq!(run(&mut ed, "visual/counter.vpl", &mut out, &|| {}), 1, "GUI apps don't run headless");
        assert!(python(&mut ed, "visual/nope.vpl").is_err());
    }
}
