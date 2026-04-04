use std::io::{self, Read, Write};
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        for path in &args[1..] {
            match fs::read_to_string(path) {
                Ok(content) => print!("{}", content),
                Err(e) => eprintln!("cat: {}: {}", path, e),
            }
        }
    } else {
        let mut buf = Vec::new();
        io::stdin().read_to_end(&mut buf).unwrap_or(0);
        io::stdout().write_all(&buf).unwrap_or(());
    }
}
