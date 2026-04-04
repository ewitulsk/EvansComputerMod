use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: mv <source> <destination>");
        std::process::exit(1);
    }

    let src = &args[1];
    let dst = &args[2];

    if !std::path::Path::new(src).exists() {
        eprintln!("mv: {}: No such file or directory", src);
        std::process::exit(1);
    }

    // Try rename first, fall back to copy+delete
    if fs::rename(src, dst).is_err() {
        match fs::read(src) {
            Ok(content) => {
                if let Err(e) = fs::write(dst, &content) {
                    eprintln!("mv: cannot move to '{}': {}", dst, e);
                    std::process::exit(1);
                }
                let _ = fs::remove_file(src);
            }
            Err(e) => {
                eprintln!("mv: cannot read '{}': {}", src, e);
                std::process::exit(1);
            }
        }
    }
}
