use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: mkdir [-p] <dir>");
        std::process::exit(1);
    }

    let mut parents = false;
    let mut dirs = Vec::new();

    for arg in args.iter().skip(1) {
        if arg == "-p" {
            parents = true;
        } else {
            dirs.push(arg.as_str());
        }
    }

    if dirs.is_empty() {
        eprintln!("Usage: mkdir [-p] <dir>");
        std::process::exit(1);
    }

    for dir in dirs {
        let result = if parents {
            fs::create_dir_all(dir)
        } else {
            fs::create_dir(dir)
        };
        if let Err(e) = result {
            eprintln!("mkdir: cannot create directory '{}': {}", dir, e);
        }
    }
}
