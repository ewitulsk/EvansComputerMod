use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: cp <source> <destination>");
        std::process::exit(1);
    }

    let src = &args[1];
    let dst = &args[2];

    if !std::path::Path::new(src).exists() {
        eprintln!("cp: {}: No such file or directory", src);
        std::process::exit(1);
    }

    match fs::read(src) {
        Ok(content) => {
            if let Err(e) = fs::write(dst, &content) {
                eprintln!("cp: cannot create '{}': {}", dst, e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("cp: cannot read '{}': {}", src, e);
            std::process::exit(1);
        }
    }
}
