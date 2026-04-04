use std::io::{self, BufRead};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: grep <pattern>");
        std::process::exit(1);
    }
    let pattern = &args[1];
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        if let Ok(line) = line {
            if line.contains(pattern.as_str()) {
                println!("{}", line);
            }
        }
    }
}
