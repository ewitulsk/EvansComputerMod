//! The Tech Village repository's package, built when the mod is built
//! (`scripts/stage-wasm.sh`, docs/TECH_VILLAGE_NETWORK.md "Apps"):
//!
//!   cargo run -p chatapp --example village_repo -- <chatapp.wasm> <out dir> [<package dir>...]
//!
//! The same steps a player types on a computer, run natively on the same code:
//! `chatapp --source <dir>` (the package folder: `chat_app::source::write`) and
//! `pkg build <dir> <out dir>` (every file of the folder except dot-files, sorted,
//! `app.toml` first: `ecm_pkg::package::build`). It writes
//! `<out dir>/dev.ecm.chat-<version>.ecmpkg` and `packages.txt` (its name). The mod copies it into the package
//! server's `/srv/repo` when a Tech Village world generates and signs it there
//! with the world's repository key (what `pkg build --sign` would add: the
//! `.sig` file and `publisher.pub`); `pkg serve` makes the index from them.
//!
//! Every further `<package dir>` is a ready package folder (an `app.toml` and its
//! files, e.g. the card examples in `examples/cards/*`, docs/cards/CARDS.md) built
//! with the same `pkg build` and listed in `packages.txt` too.

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [wasm, out, dirs @ ..] = args.as_slice() else {
        eprintln!("usage: village_repo <chatapp.wasm> <out dir> [<package dir>...]");
        std::process::exit(2);
    };
    match run(Path::new(wasm), Path::new(out), dirs) {
        Ok(line) => println!("{line}"),
        Err(e) => {
            eprintln!("village_repo: {e}");
            std::process::exit(1);
        }
    }
}

/// `pkg build <dir>`: the folder's files (no dot-files, sorted; `app.toml` first), then the archive.
fn build_dir(dir: &Path) -> Result<(ecm_pkg::package::Package, Vec<u8>), String> {
    let mut files = Vec::new();
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
        .flatten()
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    for n in names {
        files.push((n.clone(), std::fs::read(dir.join(&n)).map_err(|e| e.to_string())?));
    }
    ecm_pkg::package::build(files)
}

fn run(wasm: &Path, out: &Path, dirs: &[String]) -> Result<String, String> {
    let program = std::fs::read(wasm).map_err(|e| format!("cannot read {}: {e}", wasm.display()))?;
    // chatapp --source <dir>
    let src = std::env::temp_dir().join(format!("ecm-village-repo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&src);
    chat_app::source::write(&src.to_string_lossy().replace('\\', "/"), &program)?;
    let mut built = vec![build_dir(&src)?];
    let _ = std::fs::remove_dir_all(&src);
    for d in dirs {
        built.push(build_dir(Path::new(d))?);
    }
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    // Only this build's packages in the folder.
    if let Ok(rd) = std::fs::read_dir(out) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().ends_with(".ecmpkg") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut list = String::new();
    let mut lines = Vec::new();
    for (p, bytes) in &built {
        let name = ecm_pkg::package::file_name(&p.manifest);
        std::fs::write(out.join(&name), bytes).map_err(|e| e.to_string())?;
        list.push_str(&name);
        list.push('\n');
        lines.push(format!("Built {}/{} ({} files, {} bytes, sha256 {})", out.display(), name, p.entries.len(), bytes.len(), p.sha256));
    }
    // The list the mod reads from its resources (a jar can't be listed).
    std::fs::write(out.join("packages.txt"), list).map_err(|e| e.to_string())?;
    Ok(lines.join("\n"))
}

