use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: rm [-r] <file> [file2] ...");
        std::process::exit(1);
    }

    let mut recursive = false;
    let mut targets = Vec::new();

    for arg in args.iter().skip(1) {
        if arg == "-r" || arg == "-rf" || arg == "-fr" {
            recursive = true;
        } else {
            targets.push(arg.as_str());
        }
    }

    if targets.is_empty() {
        eprintln!("Usage: rm [-r] <file> [file2] ...");
        std::process::exit(1);
    }

    for target in targets {
        let path = std::path::Path::new(target);
        if !path.exists() {
            eprintln!("rm: {}: No such file or directory", target);
            continue;
        }

        if path.is_dir() {
            if !recursive {
                eprintln!("rm: {}: Is a directory (use -r to remove)", target);
                continue;
            }
            if let Err(e) = fs::remove_dir_all(target) {
                eprintln!("rm: cannot remove '{}': {}", target, e);
            }
        } else {
            if let Err(e) = fs::remove_file(target) {
                eprintln!("rm: cannot remove '{}': {}", target, e);
            }
        }
    }
}
