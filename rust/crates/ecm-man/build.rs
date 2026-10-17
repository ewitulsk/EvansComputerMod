//! Compile every `pages/*.txt` into the crate (`PAGES`, sorted by name).

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("pages");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names = Vec::new();
    if std::env::var_os("CARGO_FEATURE_PAGES").is_some() {
        for e in std::fs::read_dir(&dir).expect("pages/") {
            let p = e.expect("entry").path();
            if p.extension().is_some_and(|x| x == "txt") {
                println!("cargo:rerun-if-changed={}", p.display());
                names.push(p);
            }
        }
    }
    names.sort_by_key(|p| p.file_stem().unwrap().to_os_string());
    let mut out = String::from("/// Built-in pages: (name, text), sorted by name.\npub static PAGES: &[(&str, &str)] = &[\n");
    for p in &names {
        let name = p.file_stem().unwrap().to_string_lossy();
        let _ = writeln!(out, "    ({:?}, include_str!({:?})),", name, p.display().to_string());
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("pages.rs");
    std::fs::write(dest, out).expect("write pages.rs");
}
