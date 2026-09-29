//! Runs every `scenarios/*.toml` through the simulator binary (headless,
//! virtual clock). Needs the kernel and the WASI programs built; if they are
//! missing this fails and prints the exact build commands.
//!
//! `cargo test -p terminal-simulator --test scenarios` runs all of them;
//! `... --test scenarios -- stp lacp` runs those whose file name contains
//! any of the given words.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const PROGRAMS_NEEDED: &[&str] = &["echo", "sleep", "ifconfig", "ping", "curl", "httpd", "ps"];

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let rust = manifest.parent().unwrap();
    let kernel = rust.join("target/wasm32-unknown-unknown/release/terminal_os.wasm");
    let programs = rust.join("target/wasm32-wasip1/release");
    let mut missing = Vec::new();
    if !kernel.is_file() {
        missing.push(format!("kernel {}", kernel.display()));
    }
    let progs: Vec<&str> = PROGRAMS_NEEDED.iter().copied().filter(|p| *p != "ps").collect();
    for p in &progs {
        if !programs.join(format!("{}.wasm", p)).is_file() {
            missing.push(format!("program {}", programs.join(format!("{}.wasm", p)).display()));
        }
    }
    if !missing.is_empty() {
        eprintln!("scenario tests need the kernel and WASI programs. Missing:");
        for m in &missing {
            eprintln!("  {}", m);
        }
        eprintln!("Build them with (from the repository root):");
        eprintln!("  cd rust && cargo build --release --target wasm32-unknown-unknown -p terminal-os");
        eprintln!(
            "  cd rust && cargo build --release --target wasm32-wasip1 {}",
            progs.iter().map(|p| format!("-p {}", p)).collect::<Vec<_>>().join(" ")
        );
        std::process::exit(1);
    }

    let filters: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let mut files: Vec<PathBuf> = std::fs::read_dir(manifest.join("scenarios"))
        .expect("scenarios dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy().to_string();
            filters.is_empty() || filters.iter().any(|f| n.contains(f.as_str()))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        eprintln!("no scenarios matched {:?}", filters);
        std::process::exit(1);
    }

    let bin = env!("CARGO_BIN_EXE_terminal-simulator");
    let out_dir = rust.join("target/sim-out");
    let queue = Arc::new(Mutex::new(files.clone()));
    let results = Arc::new(Mutex::new(Vec::new()));
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2).min(4);
    let t0 = Instant::now();
    println!("running {} scenarios ({} at a time)", files.len(), workers);
    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let (q, r, out_dir, kernel, programs) =
                (queue.clone(), results.clone(), out_dir.clone(), kernel.clone(), programs.clone());
            std::thread::spawn(move || loop {
                let Some(f) = q.lock().unwrap().pop() else { break };
                let t = Instant::now();
                let out = Command::new(bin)
                    .arg("--scenario")
                    .arg(&f)
                    .arg("--kernel")
                    .arg(&kernel)
                    .arg("--programs")
                    .arg(&programs)
                    .arg("--out-dir")
                    .arg(&out_dir)
                    .arg("--quiet")
                    .output()
                    .expect("run simulator");
                let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                let name = f.file_name().unwrap().to_string_lossy().to_string();
                println!("{} {} ({:.1}s)", if out.status.success() { "ok  " } else { "FAIL" }, name, t.elapsed().as_secs_f64());
                r.lock().unwrap().push((name, out.status.success(), text));
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let results = results.lock().unwrap();
    let failed: Vec<_> = results.iter().filter(|r| !r.1).collect();
    for (name, _, text) in &failed {
        println!("\n==================== {} ====================\n{}", name, text);
    }
    println!(
        "\nscenario result: {} passed; {} failed ({:.1}s)",
        results.len() - failed.len(),
        failed.len(),
        t0.elapsed().as_secs_f64()
    );
    if !failed.is_empty() {
        std::process::exit(1);
    }
}
