//! Traditional desktop icons and folder system backed by `/home/Desktop`.
//!
//! Real files and directories in `/home/Desktop` appear directly on the
//! desktop grid. App shortcuts are small `.desktop` TOML files (`name`,
//! `program`, optional `args`, optional `icon`). Double-clicking a folder
//! launches Files at `/home/Desktop/<folder>`; double-clicking a shortcut
//! launches its app; double-clicking a regular file sends `Open{path}`.
//! Custom icon positions persist in `/var/desktop/layout.toml`.

use crate::app_catalog::{self, AppItem};
use crate::sys::{self, Sys};
use crate::uitoml;
use std::collections::{HashMap, HashSet};

/// The backing directory whose contents appear on the desktop.
pub const DESKTOP_DIR: &str = "/home/Desktop";

/// Persisted `(col, row)` positions of desktop items.
pub const LAYOUT_PATH: &str = "/var/desktop/layout.toml";

/// Parsed contents of a `.desktop` shortcut file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub icon: String,
    pub custom_icon: Option<String>,
}

/// Parse a `.desktop` file (`name`, `program`, optional `args`, `icon`, `custom_icon`).
pub fn parse_shortcut(text: &str) -> Option<Shortcut> {
    let program = uitoml::get(text, "program").filter(|s| !s.is_empty())?;
    let name = uitoml::get(text, "name")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| program.clone());
    let icon = uitoml::get(text, "icon")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| app_catalog::icon_for(&program).to_string());
    let custom_icon = uitoml::get(text, "custom_icon").filter(|s| !s.is_empty());
    let args = uitoml::get(text, "args")
        .filter(|s| !s.is_empty())
        .map(|a| a.split('\x1f').map(String::from).collect())
        .unwrap_or_default();
    Some(Shortcut { name, program, args, icon, custom_icon })
}

/// Serialize a [`Shortcut`] as a `.desktop` file.
pub fn format_shortcut(s: &Shortcut) -> String {
    let mut out = String::new();
    out.push_str(&format!("name = {}\n", uitoml::quote(&s.name)));
    out.push_str(&format!("program = {}\n", uitoml::quote(&s.program)));
    out.push_str(&format!("icon = {}\n", uitoml::quote(&s.icon)));
    if let Some(ci) = &s.custom_icon {
        out.push_str(&format!("custom_icon = {}\n", uitoml::quote(ci)));
    }
    if !s.args.is_empty() {
        out.push_str(&format!("args = {}\n", uitoml::quote(&s.args.join("\x1f"))));
    }
    out
}

/// What a desktop icon represents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    Shortcut(Shortcut),
    File,
}

/// One placed icon on the desktop grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopEntry {
    /// Entry file or directory name inside `/home/Desktop`.
    pub file_name: String,
    /// Display label drawn below the icon.
    pub label: String,
    pub kind: EntryKind,
    /// Built-in fallback icon name.
    pub icon: String,
    /// Optional installed app PNG icon path.
    pub custom_icon: Option<String>,
    pub col: i32,
    pub row: i32,
}

impl DesktopEntry {
    pub fn full_path(&self) -> String {
        sys::join(DESKTOP_DIR, &self.file_name)
    }

    pub fn is_folder(&self) -> bool {
        matches!(self.kind, EntryKind::Folder)
    }
}

/// Saved grid slots: `file_name -> (col, row)`.
pub type LayoutMap = HashMap<String, (i32, i32)>;

/// Parse [`LAYOUT_PATH`] (`"file_name" = "col,row"`).
pub fn parse_layout(text: &str) -> LayoutMap {
    let mut map = HashMap::new();
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with('[') {
            continue;
        }
        let Some((k, v)) = t.split_once('=') else { continue };
        let key = unquote(k.trim());
        let val = unquote(v.trim());
        let Some((c_s, r_s)) = val.split_once(',') else { continue };
        if let (Ok(c), Ok(r)) = (c_s.trim().parse::<i32>(), r_s.trim().parse::<i32>()) {
            if !key.is_empty() && c >= 0 && r >= 0 {
                map.insert(key, (c, r));
            }
        }
    }
    map
}

pub fn format_layout(layout: &LayoutMap) -> String {
    let mut keys: Vec<&String> = layout.keys().collect();
    keys.sort();
    let mut out = String::from("# Desktop icon grid positions (name = \"col,row\")\n");
    for k in keys {
        let (c, r) = layout[k];
        out.push_str(&format!("{} = \"{c},{r}\"\n", uitoml::quote(k)));
    }
    out
}

fn unquote(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

pub fn load_layout(sys: &dyn Sys) -> LayoutMap {
    match sys::read_file(sys, LAYOUT_PATH) {
        Ok(b) => parse_layout(&String::from_utf8_lossy(&b)),
        Err(_) => HashMap::new(),
    }
}

pub fn save_layout(sys: &dyn Sys, layout: &LayoutMap) {
    let _ = sys::make_dir_all(sys, "/var/desktop");
    let _ = sys::write_file(sys, LAYOUT_PATH, format_layout(layout).as_bytes());
}

/// Grid geometry over the desktop work area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesktopGrid {
    pub cols: i32,
    pub rows: i32,
    pub cell_w: i32,
    pub cell_h: i32,
    pub margin_x: i32,
    pub margin_y: i32,
}

impl DesktopGrid {
    pub fn for_work_area(wa: (i32, i32, i32, i32), compact: bool) -> DesktopGrid {
        let (cell_w, cell_h, margin_x, margin_y) = if compact { (58, 54, 6, 6) } else { (68, 62, 10, 10) };
        let cols = ((wa.2 - 2 * margin_x) / cell_w).max(1);
        let rows = ((wa.3 - 2 * margin_y) / cell_h).max(1);
        DesktopGrid { cols, rows, cell_w, cell_h, margin_x, margin_y }
    }

    /// Bounding rectangle `(x, y, w, h)` of grid cell `(col, row)`.
    pub fn cell_rect(&self, col: i32, row: i32) -> (i32, i32, i32, i32) {
        (self.margin_x + col * self.cell_w, self.margin_y + row * self.cell_h, self.cell_w - 4, self.cell_h - 4)
    }

    /// Clamped grid cell `(col, row)` under wallpaper point `(x, y)`.
    pub fn cell_at(&self, x: i32, y: i32) -> (i32, i32) {
        let col = ((x - self.margin_x) / self.cell_w).clamp(0, self.cols - 1);
        let row = ((y - self.margin_y) / self.cell_h).clamp(0, self.rows - 1);
        (col, row)
    }
}

/// Icon for a non-shortcut file on the desktop.
pub fn file_icon(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".gba") {
        "game"
    } else if [".py", ".lua", ".rs", ".vpl", ".json", ".toml", ".sh", ".wasm"].iter().any(|ext| lower.ends_with(ext)) {
        "code"
    } else {
        "file"
    }
}

/// Seed `/home/Desktop` on first boot (only if the directory does not exist yet).
pub fn ensure_desktop_seeded(sys: &dyn Sys) {
    if sys::exists(sys, DESKTOP_DIR) {
        return;
    }
    if sys::make_dir_all(sys, DESKTOP_DIR).is_err() {
        return;
    }
    let mut defaults: Vec<(&str, &str, &str)> = vec![
        ("Terminal", "terminal", "terminal"),
        ("Files", "files", "files"),
        ("Text Editor", "editor", "editor"),
        ("Settings", "settings", "settings"),
    ];
    if sys::exists(sys, "/apps/dev.ecm.store/app.toml") {
        defaults.push(("App Store", "dev.ecm.store", "store"));
    }
    for (name, program, icon) in defaults {
        let s = Shortcut {
            name: name.into(),
            program: program.into(),
            args: Vec::new(),
            icon: icon.into(),
            custom_icon: app_catalog::app_icon_path(sys, program),
        };
        let path = sys::join(DESKTOP_DIR, &format!("{name}.desktop"));
        let _ = sys::write_file(sys, &path, format_shortcut(&s).as_bytes());
    }
}

/// Read `/home/Desktop` and assign every entry a non-overlapping `(col, row)` slot.
pub fn scan_desktop(sys: &dyn Sys, grid: DesktopGrid, layout: &LayoutMap) -> Vec<DesktopEntry> {
    let Ok(raw) = sys::list_dir(sys, DESKTOP_DIR) else { return Vec::new() };
    let mut items: Vec<DesktopEntry> = Vec::new();
    for e in raw {
        if e.name.starts_with('.') {
            continue;
        }
        if e.dir {
            items.push(DesktopEntry {
                label: e.name.clone(),
                file_name: e.name,
                kind: EntryKind::Folder,
                icon: "folder".into(),
                custom_icon: None,
                col: -1,
                row: -1,
            });
        } else if e.name.ends_with(".desktop") {
            let path = sys::join(DESKTOP_DIR, &e.name);
            let parsed = sys::read_file(sys, &path)
                .ok()
                .and_then(|b| parse_shortcut(&String::from_utf8_lossy(&b)));
            if let Some(sc) = parsed {
                let icon = sc.icon.clone();
                let custom_icon = sc
                    .custom_icon
                    .clone()
                    .or_else(|| app_catalog::app_icon_path(sys, &sc.program));
                let label = sc.name.clone();
                items.push(DesktopEntry {
                    file_name: e.name,
                    label,
                    kind: EntryKind::Shortcut(sc),
                    icon,
                    custom_icon,
                    col: -1,
                    row: -1,
                });
            } else {
                items.push(DesktopEntry {
                    icon: file_icon(&e.name).into(),
                    label: e.name.clone(),
                    file_name: e.name,
                    kind: EntryKind::File,
                    custom_icon: None,
                    col: -1,
                    row: -1,
                });
            }
        } else {
            items.push(DesktopEntry {
                icon: file_icon(&e.name).into(),
                label: e.name.clone(),
                file_name: e.name,
                kind: EntryKind::File,
                custom_icon: None,
                col: -1,
                row: -1,
            });
        }
    }

    // Deterministic order before slot assignment: folders, shortcuts, files, A-Z.
    items.sort_by(|a, b| {
        kind_order(&a.kind)
            .cmp(&kind_order(&b.kind))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });

    let mut used: HashSet<(i32, i32)> = HashSet::new();
    // Pass 1: honor saved slots inside the current grid.
    for it in &mut items {
        if let Some(&(c, r)) = layout.get(&it.file_name) {
            if c >= 0 && c < grid.cols && r >= 0 && r < grid.rows && used.insert((c, r)) {
                it.col = c;
                it.row = r;
            }
        }
    }
    // Pass 2: assign free column-major slots to remaining items.
    for it in &mut items {
        if it.col >= 0 {
            continue;
        }
        let slot = first_free_slot(grid, &used);
        used.insert(slot);
        it.col = slot.0;
        it.row = slot.1;
    }
    items
}

fn kind_order(k: &EntryKind) -> u8 {
    match k {
        EntryKind::Folder => 0,
        EntryKind::Shortcut(_) => 1,
        EntryKind::File => 2,
    }
}

fn first_free_slot(grid: DesktopGrid, used: &HashSet<(i32, i32)>) -> (i32, i32) {
    for c in 0..grid.cols {
        for r in 0..grid.rows {
            if !used.contains(&(c, r)) {
                return (c, r);
            }
        }
    }
    // Overflow beyond visible cells: keep extending columns cleanly.
    let mut c = grid.cols;
    loop {
        for r in 0..grid.rows {
            if !used.contains(&(c, r)) {
                return (c, r);
            }
        }
        c += 1;
    }
}

/// Hit-test wallpaper point `(x, y)` against placed desktop icons.
pub fn hit_entry(entries: &[DesktopEntry], grid: DesktopGrid, x: i32, y: i32) -> Option<usize> {
    for (i, e) in entries.iter().enumerate() {
        let (rx, ry, rw, rh) = grid.cell_rect(e.col, e.row);
        if x >= rx && x < rx + rw && y >= ry && y < ry + rh {
            return Some(i);
        }
    }
    None
}

fn valid_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Name can't be empty.".into());
    }
    if n.contains('/') || n.contains('\\') || n == "." || n == ".." || n.starts_with('.') {
        return Err("Name can't contain slashes or start with a dot.".into());
    }
    Ok(n.to_string())
}

/// Create a `.desktop` shortcut for `item` on `/home/Desktop`.
pub fn create_shortcut(sys: &dyn Sys, item: &AppItem) -> Result<String, String> {
    if !sys::exists(sys, DESKTOP_DIR) {
        sys::make_dir_all(sys, DESKTOP_DIR)?;
    }
    let clean: String = item
        .label
        .chars()
        .map(|c| if c == '/' || c == '\\' { '-' } else { c })
        .collect();
    let base = if clean.trim().is_empty() { item.program.clone() } else { clean.trim().to_string() };
    let file_name = sys::free_name(sys, DESKTOP_DIR, &format!("{base}.desktop"));
    let sc = Shortcut {
        name: item.label.clone(),
        program: item.program.clone(),
        args: item.args.clone(),
        icon: item.icon.clone(),
        custom_icon: item.custom_icon.clone(),
    };
    let path = sys::join(DESKTOP_DIR, &file_name);
    sys::write_file(sys, &path, format_shortcut(&sc).as_bytes())?;
    Ok(file_name)
}

/// Create a new folder inside `/home/Desktop`.
pub fn create_folder(sys: &dyn Sys, name: &str) -> Result<String, String> {
    let n = valid_name(name)?;
    if !sys::exists(sys, DESKTOP_DIR) {
        sys::make_dir_all(sys, DESKTOP_DIR)?;
    }
    let path = sys::join(DESKTOP_DIR, &n);
    if sys::exists(sys, &path) {
        return Err(format!("'{n}' already exists."));
    }
    sys::make_dir(sys, &path)?;
    Ok(n)
}

/// Rename a desktop item (preserving its saved grid slot; for `.desktop` files,
/// updates the display `name` and keeps the `.desktop` suffix).
pub fn rename_entry(sys: &dyn Sys, layout: &mut LayoutMap, old_file: &str, new_name: &str) -> Result<String, String> {
    let clean = valid_name(new_name)?;
    let old_path = sys::join(DESKTOP_DIR, old_file);
    let is_shortcut = old_file.ends_with(".desktop") && !sys::is_dir(sys, &old_path);
    let new_file = if is_shortcut && !clean.ends_with(".desktop") {
        format!("{clean}.desktop")
    } else {
        clean.clone()
    };
    let new_path = sys::join(DESKTOP_DIR, &new_file);
    if new_file != old_file && sys::exists(sys, &new_path) {
        return Err(format!("'{new_file}' already exists."));
    }
    if is_shortcut {
        let bytes = sys::read_file(sys, &old_path)?;
        let text = String::from_utf8_lossy(&bytes);
        if let Some(mut sc) = parse_shortcut(&text) {
            sc.name = clean.strip_suffix(".desktop").unwrap_or(&clean).to_string();
            sys::write_file(sys, &old_path, format_shortcut(&sc).as_bytes())?;
        }
    }
    if new_file != old_file {
        sys::rename(sys, &old_path, &new_path)?;
        if let Some(pos) = layout.remove(old_file) {
            layout.insert(new_file.clone(), pos);
            save_layout(sys, layout);
        }
    }
    Ok(new_file)
}

/// Delete a desktop file, shortcut, or folder.
pub fn delete_entry(sys: &dyn Sys, layout: &mut LayoutMap, file_name: &str) -> Result<(), String> {
    let path = sys::join(DESKTOP_DIR, file_name);
    sys::remove(sys, &path)?;
    if layout.remove(file_name).is_some() {
        save_layout(sys, layout);
    }
    Ok(())
}

/// Move a desktop item into a folder on `/home/Desktop/<folder_name>/`.
pub fn move_into_folder(sys: &dyn Sys, layout: &mut LayoutMap, file_name: &str, folder_name: &str) -> Result<String, String> {
    if file_name == folder_name {
        return Err("Cannot move a folder into itself.".into());
    }
    let src = sys::join(DESKTOP_DIR, file_name);
    let folder = sys::join(DESKTOP_DIR, folder_name);
    if !sys::is_dir(sys, &folder) {
        return Err(format!("'{folder_name}' is not a folder."));
    }
    let dst_name = sys::free_name(sys, &folder, file_name);
    let dst = sys::join(&folder, &dst_name);
    sys::rename(sys, &src, &dst)?;
    if layout.remove(file_name).is_some() {
        save_layout(sys, layout);
    }
    Ok(dst)
}

/// Place `file_name` at `(col, row)`, swapping with any existing item at that slot.
pub fn place_at_slot(sys: &dyn Sys, layout: &mut LayoutMap, entries: &[DesktopEntry], file_name: &str, col: i32, row: i32) {
    let old_slot = entries.iter().find(|e| e.file_name == file_name).map(|e| (e.col, e.row));
    let occupant = entries.iter().find(|e| e.col == col && e.row == row && e.file_name != file_name).map(|e| e.file_name.clone());
    // Snapshot every current position so untouched icons stay put.
    for e in entries {
        layout.entry(e.file_name.clone()).or_insert((e.col, e.row));
    }
    layout.insert(file_name.to_string(), (col, row));
    if let (Some(other), Some(prev)) = (occupant, old_slot) {
        layout.insert(other, prev);
    }
    save_layout(sys, layout);
}

/// Re-pack all desktop icons in sorted column-major order and persist.
pub fn sort_by_name(sys: &dyn Sys, grid: DesktopGrid, layout: &mut LayoutMap, entries: &[DesktopEntry]) {
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| {
        kind_order(&a.kind)
            .cmp(&kind_order(&b.kind))
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
            .then_with(|| a.file_name.cmp(&b.file_name))
    });
    layout.clear();
    let mut used = HashSet::new();
    for e in &sorted {
        let slot = first_free_slot(grid, &used);
        used.insert(slot);
        layout.insert(e.file_name.clone(), slot);
    }
    save_layout(sys, layout);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::MockSys;

    #[test]
    fn seed_scan_folder_move_and_layout_persistence() {
        let root = std::env::temp_dir().join(format!("ecm-desk-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let sys = MockSys::new(&root);

        ensure_desktop_seeded(&sys);
        let grid = DesktopGrid::for_work_area((0, 0, 640, 376), false);
        let mut layout = load_layout(&sys);
        let entries = scan_desktop(&sys, grid, &layout);
        assert!(entries.iter().any(|e| e.label == "Terminal"));
        assert!(entries.iter().any(|e| e.label == "Files"));
        assert!(entries.iter().any(|e| e.label == "Settings"));

        // Create a folder, move a shortcut into it.
        create_folder(&sys, "Tools").unwrap();
        let entries = scan_desktop(&sys, grid, &layout);
        assert!(entries.iter().any(|e| e.file_name == "Tools" && e.is_folder()));

        // Reposition Terminal to (2, 1) and verify persistence.
        place_at_slot(&sys, &mut layout, &entries, "Terminal.desktop", 2, 1);
        let reloaded = load_layout(&sys);
        let entries2 = scan_desktop(&sys, grid, &reloaded);
        let term = entries2.iter().find(|e| e.file_name == "Terminal.desktop").unwrap();
        assert_eq!((term.col, term.row), (2, 1));

        // Move Terminal shortcut into Tools folder.
        let moved = move_into_folder(&sys, &mut layout, "Terminal.desktop", "Tools").unwrap();
        assert_eq!(moved, "/home/Desktop/Tools/Terminal.desktop");
        assert!(sys::exists(&sys, &moved));
        let entries3 = scan_desktop(&sys, grid, &layout);
        assert!(!entries3.iter().any(|e| e.file_name == "Terminal.desktop"));

        // Rename folder.
        rename_entry(&sys, &mut layout, "Tools", "MyTools").unwrap();
        assert!(sys::is_dir(&sys, "/home/Desktop/MyTools"));
        assert!(sys::exists(&sys, "/home/Desktop/MyTools/Terminal.desktop"));

        // Deleting a shortcut only removes the .desktop file, never re-seeds an existing Desktop dir.
        delete_entry(&sys, &mut layout, "Files.desktop").unwrap();
        ensure_desktop_seeded(&sys);
        assert!(!sys::exists(&sys, "/home/Desktop/Files.desktop"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn shortcut_roundtrip() {
        let s = Shortcut {
            name: "Net Check".into(),
            program: "dev.ecm.netcheck".into(),
            args: vec!["--fast".into()],
            icon: "app".into(),
            custom_icon: Some("/apps/dev.ecm.netcheck/icon.png".into()),
        };
        let parsed = parse_shortcut(&format_shortcut(&s)).unwrap();
        assert_eq!(parsed, s);
    }
}
