//! `less <file>`: read a text file a screen at a time (the `ecm-man` pager).

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("Usage: less <file>");
        std::process::exit(1);
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("less: {path}: {e}");
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
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    ecm_man::tty::show(&name, lines, false);
}
