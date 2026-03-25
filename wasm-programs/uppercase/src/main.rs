use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for line in stdin.lock().lines() {
        if let Ok(line) = line {
            writeln!(out, "{}", line.to_uppercase()).unwrap_or(());
        }
    }
}
