use std::fs;
use std::io::{self, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut show_long = false;
    let mut target = ".".to_string();

    for arg in args.iter().skip(1) {
        if arg == "-l" {
            show_long = true;
        } else if !arg.starts_with('-') {
            target = arg.clone();
        }
    }

    let entries = match fs::read_dir(&target) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("ls: cannot access '{}': {}", target, e);
            std::process::exit(1);
        }
    };

    let mut items: Vec<(String, bool, u64)> = Vec::new();
    for entry in entries {
        if let Ok(entry) = entry {
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            items.push((name, is_dir, size));
        }
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));

    let stdout = io::stdout();
    let mut out = stdout.lock();

    for (name, is_dir, size) in &items {
        if show_long {
            if *is_dir {
                let _ = write!(out, "  d  ---     ");
            } else {
                let size_str = format_size(*size as usize);
                let _ = write!(out, "  f  {:>7} ", size_str);
            }
        } else {
            let _ = write!(out, "  ");
        }
        let _ = write!(out, "{}", name);
        if *is_dir {
            let _ = write!(out, "/");
        }
        let _ = writeln!(out);
    }

    if items.is_empty() {
        let _ = writeln!(out, "  (empty)");
    }
}

fn format_size(size: usize) -> String {
    if size < 1024 {
        format!("{}B", size)
    } else if size < 1024 * 1024 {
        format!("{:.1}K", size as f64 / 1024.0)
    } else {
        format!("{:.1}M", size as f64 / (1024.0 * 1024.0))
    }
}
