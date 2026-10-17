//! `less [file]`: read a text file, or the output of a pipeline (`ls | less`),
//! a screen at a time (the `ecm-man` pager). From a pipe the keys come from
//! /dev/tty, the keyboard.

use std::io::Read;

fn main() {
    let path = std::env::args().nth(1);
    let piped = path.is_none() && !ecm_man::tty::is_terminal(0);
    let (name, bytes) = match &path {
        Some(p) => match std::fs::read(p) {
            Ok(b) => (p.rsplit('/').next().unwrap_or(p).to_string(), b),
            Err(e) => {
                eprintln!("less: {p}: {e}");
                std::process::exit(1);
            }
        },
        None if piped => {
            let mut b = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut b);
            ("(stdin)".to_string(), b)
        }
        None => {
            eprintln!("Usage: less <file>   or   command | less");
            std::process::exit(1);
        }
    };
    let text = String::from_utf8_lossy(&bytes);
    let (w, _) = ecm_man::tty::size();
    let mut lines = Vec::new();
    for l in text.lines() {
        // Tabs to spaces, control characters out, long lines wrapped.
        let mut s = String::new();
        for c in l.chars() {
            match c {
                '\t' => s.push_str(&" ".repeat(8 - s.chars().count() % 8)),
                c if c.is_control() => {}
                c => s.push(c),
            }
        }
        let chars: Vec<char> = s.chars().collect();
        if chars.is_empty() {
            lines.push(String::new());
        }
        for chunk in chars.chunks(w.max(1)) {
            lines.push(chunk.iter().collect());
        }
    }
    if piped {
        match std::fs::File::open("/dev/tty") {
            Ok(mut tty) => ecm_man::tty::show_with_keys(&name, lines, false, &mut tty),
            Err(_) => ecm_man::tty::show_with_keys(&name, lines, true, &mut std::io::empty()),
        }
    } else {
        ecm_man::tty::show(&name, lines, false);
    }
}
