//! Discord's package folder, for `pkg build`: `discord --source <dir>`
//! writes `app.toml`, `icon.png` and `main.wasm` (a copy of this program from
//! `server-bin/`, the server's programs), then `pkg build <dir> <repo dir>`
//! packs it and `pkg serve <repo dir>` publishes it.

/// The manifest (docs/gui/APPS.md "app.toml").
pub const APP_TOML: &str = r#"id          = "dev.ecm.discord"
name        = "Discord"
version     = "1.0.0"
entry       = "main.wasm"
summary     = "Servers with text and voice channels on every Discord backend around you"
publisher   = "ECM"
sizes       = ["compact", "medium", "wide"]
permissions = ["network", "notifications", "microphone", "peripherals:speaker"]
background  = { refresh = true, modes = ["audio"] }
hotkeys     = [{ slot = 1, action = "send", label = "Send" }, { slot = 2, action = "mute", label = "Mute" }, { slot = 3, action = "hangup", label = "Hang up" }]
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
        let d = std::env::temp_dir().join(format!("ecm-discord-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let dir = d.to_string_lossy().replace('\\', "/");
        let fake = b"\0asm\x01\0\0\0".to_vec();
        let lines = write(&dir, &fake).unwrap();
        assert_eq!(lines.len(), 3);
        assert!(write(&dir, b"not wasm").is_err());
        // What pkg build makes of it: a valid manifest, the icon and the entry in the package.
        let files: Vec<(String, Vec<u8>)> = ["app.toml", "icon.png", "main.wasm"]
            .iter()
            .map(|n| (n.to_string(), std::fs::read(d.join(n)).unwrap()))
            .collect();
        let (p, _) = ecm_pkg::package::build(files).unwrap();
        let m = p.manifest;
        assert_eq!(m.id, crate::APP_ID);
        assert_eq!(m.entry, "main.wasm");
        assert!(m.permissions.contains(&"network".to_string()));
        assert!(m.permissions.contains(&"notifications".to_string()));
        assert!(m.permissions.contains(&"microphone".to_string()));
        assert!(m.permissions.contains(&"background:refresh".to_string()), "{:?}", m.permissions);
        assert!(m.permissions.contains(&"background:audio".to_string()), "{:?}", m.permissions);
        let _ = std::fs::remove_dir_all(&d);
    }
}
