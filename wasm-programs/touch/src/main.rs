use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: touch <file>");
        std::process::exit(1);
    }
    for path in &args[1..] {
        if !std::path::Path::new(path).exists() {
            if let Err(e) = fs::File::create(path) {
                eprintln!("touch: cannot create '{}': {}", path, e);
            }
        }
    }
}
