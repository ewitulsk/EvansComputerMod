//! Music's package folder, for `pkg build`: `music --source <dir>` writes
//! `app.toml`, `icon.png` and `main.wasm` (a copy of this program from
//! `server-bin/`), then `pkg build <dir> <repo dir>` packs it.

/// The manifest (docs/gui/APPS.md "app.toml").
pub const APP_TOML: &str = r#"id          = "dev.ecm.music"
name        = "Music"
version     = "1.0.0"
entry       = "main.wasm"
summary     = "Stream songs from the music servers around you, with search, playlists and a queue"
publisher   = "ECM"
sizes       = ["compact", "medium", "wide"]
permissions = ["network", "peripherals:speaker", "notifications"]
background  = { refresh = false, modes = ["audio"] }
hotkeys     = [{ slot = 1, action = "playpause", label = "Play/Pause" }, { slot = 2, action = "previous", label = "Previous" }, { slot = 3, action = "next", label = "Next" }]
icon        = "icon.png"
"#;

/// The files of the package folder: (name, bytes). `program` is this program's `.wasm`.
pub fn files(program: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    vec![("app.toml", APP_TOML.as_bytes().to_vec()), ("icon.png", crate::ICON_PNG.to_vec()), ("main.wasm", program.to_vec())]
}

/// Write the folder; returns one line per file for the console.
pub fn write(dir: &str, program: &[u8]) -> Result<Vec<String>, String> {
    if program.len() < 8 || &program[..4] != b"\0asm" {
        return Err("the program to package is not a WebAssembly module".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {dir}: {e}"))?;
    let mut out = Vec::new();
    for (name, bytes) in files(program) {
        let path = format!("{}/{}", dir.trim_end_matches('/'), name);
        std::fs::write(&path, &bytes).map_err(|e| format!("cannot write {path}: {e}"))?;
        out.push(format!("{path} ({} bytes)", bytes.len()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_folder_builds_into_a_valid_package() {
        let d = std::env::temp_dir().join(format!("ecm-music-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dir = d.to_string_lossy().replace('\\', "/");
        let lines = write(&dir, b"\0asm\x01\0\0\0").unwrap();
        assert_eq!(lines.len(), 3);
        assert!(write(&dir, b"not wasm").is_err());
        let files: Vec<(String, Vec<u8>)> = ["app.toml", "icon.png", "main.wasm"].iter().map(|n| (n.to_string(), std::fs::read(d.join(n)).unwrap())).collect();
        let (p, _) = ecm_pkg::package::build(files).unwrap();
        let m = p.manifest;
        assert_eq!((m.id.as_str(), m.entry.as_str()), (crate::APP_ID, "main.wasm"));
        for perm in ["network", "notifications", "peripherals:speaker", "background:audio"] {
            assert!(m.permissions.contains(&perm.to_string()), "{perm}: {:?}", m.permissions);
        }
        assert!(!m.permissions.contains(&"background:refresh".to_string()));
        let _ = std::fs::remove_dir_all(&d);
    }
}
