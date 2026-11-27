//! Application catalog, usage tracking, search ranking and pagination for the
//! desktop shell's Start menu and macOS-style All Apps overlay.

use crate::sys::{self, Sys};
use crate::uitoml;
use std::collections::HashMap;

/// System apps that duplicate a core app (the confined Text Editor used by
/// "open with" and the ePhone's built-in Notes copy): never listed twice.
pub const SHADOWED_APPS: &[&str] = &["dev.ecm.editor", "dev.ecm.notes"];

/// Persisted launch counts and recency order for "Most Used".
pub const USAGE_PATH: &str = "/var/desktop/usage.toml";

/// One launchable application in the desktop catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppItem {
    pub label: String,
    /// Program name or installed reverse-DNS app id passed to `Launch`.
    pub program: String,
    pub args: Vec<String>,
    /// Fallback built-in icon name (`ecm_ui::icons`).
    pub icon: String,
    /// Path to an installed app's PNG icon (e.g. `/apps/dev.foo/icon.png`), if present.
    pub custom_icon: Option<String>,
    /// True for the built-in core apps; false for `/apps/<id>` packages.
    pub core: bool,
}

fn core_item(label: &str, program: &str, icon: &str) -> AppItem {
    AppItem {
        label: label.into(),
        program: program.into(),
        args: Vec::new(),
        icon: icon.into(),
        custom_icon: None,
        core: true,
    }
}

/// The built-in core desktop apps, in default order.
pub fn core_apps() -> Vec<AppItem> {
    vec![
        core_item("Terminal", "terminal", "terminal"),
        core_item("Files", "files", "files"),
        core_item("Text Editor", "editor", "editor"),
        core_item("Settings", "settings", "settings"),
        core_item("Notes", "notes", "notes"),
        core_item("Visual Programmer", "visual", "visual"),
    ]
}

/// The built-in icon name for a program name or reverse-DNS app id.
pub fn icon_for(app_id: &str) -> &'static str {
    let last = app_id.rsplit('.').next().unwrap_or(app_id);
    match last {
        "terminal" => "terminal",
        "files" | "folder" => "files",
        "editor" => "editor",
        "settings" => "settings",
        "notes" => "notes",
        "gba" | "game" => "game",
        "visual" => "visual",
        "store" => "store",
        _ => "app",
    }
}

/// An installed app's `name` from `/apps/<id>/app.toml`.
pub fn app_name(sys: &dyn Sys, app_id: &str) -> Option<String> {
    if app_id.is_empty() || app_id.contains('/') || app_id.contains("..") {
        return None;
    }
    let toml = sys::read_file(sys, &format!("/apps/{app_id}/app.toml"))
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())?;
    uitoml::get(&toml, "name").filter(|n| !n.is_empty())
}

/// An installed app's optional PNG icon path from `/apps/<id>/app.toml` (`icon = "..."`).
pub fn app_icon_path(sys: &dyn Sys, app_id: &str) -> Option<String> {
    if app_id.is_empty() || app_id.contains('/') || app_id.contains("..") {
        return None;
    }
    let toml = sys::read_file(sys, &format!("/apps/{app_id}/app.toml"))
        .ok()
        .map(|b| String::from_utf8_lossy(&b).into_owned())?;
    let rel = uitoml::get(&toml, "icon").filter(|v| !v.is_empty() && !v.contains(".."))?;
    let full = format!("/apps/{app_id}/{}", rel.trim_start_matches('/'));
    if sys::exists(sys, &full) {
        Some(full)
    } else {
        None
    }
}

/// Apps installed under `/apps/<id>/` sorted alphabetically by label.
pub fn installed_apps(sys: &dyn Sys) -> Vec<AppItem> {
    let Ok(entries) = sys::list_dir(sys, "/apps") else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.into_iter().filter(|e| e.dir && !SHADOWED_APPS.contains(&e.name.as_str())) {
        let name = app_name(sys, &e.name).unwrap_or_else(|| e.name.clone());
        let custom_icon = app_icon_path(sys, &e.name);
        out.push(AppItem {
            label: name,
            program: e.name.clone(),
            args: Vec::new(),
            icon: icon_for(&e.name).into(),
            custom_icon,
            core: false,
        });
    }
    out.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()).then_with(|| a.program.cmp(&b.program)));
    out
}

/// Every launchable app on the machine: core apps followed by installed apps.
pub fn all_apps(sys: &dyn Sys) -> Vec<AppItem> {
    let mut items = core_apps();
    items.extend(installed_apps(sys));
    items
}

// ------------------------------------------------------------------ usage tracking

/// Per-app launch count and monotonic recency sequence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageEntry {
    pub count: u32,
    pub last_seq: u64,
}

/// Usage history persisted at [`USAGE_PATH`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsageStats {
    pub entries: HashMap<String, UsageEntry>,
    pub next_seq: u64,
}

impl UsageStats {
    /// Parse `program = "count,last_seq"` lines from text.
    pub fn parse(text: &str) -> UsageStats {
        let mut stats = UsageStats::default();
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') || t.starts_with('[') {
                continue;
            }
            let Some((k, v)) = t.split_once('=') else { continue };
            let key = unquote(k.trim());
            let val = unquote(v.trim());
            let Some((c_str, s_str)) = val.split_once(',') else { continue };
            let (Ok(count), Ok(last_seq)) = (c_str.trim().parse::<u32>(), s_str.trim().parse::<u64>()) else {
                continue;
            };
            if !key.is_empty() && count > 0 {
                stats.next_seq = stats.next_seq.max(last_seq);
                stats.entries.insert(key, UsageEntry { count, last_seq });
            }
        }
        stats
    }

    /// Format deterministically for [`USAGE_PATH`].
    pub fn format(&self) -> String {
        let mut keys: Vec<&String> = self.entries.keys().collect();
        keys.sort();
        let mut out = String::from("# Desktop app launch counts (program = \"count,seq\")\n");
        for k in keys {
            let e = &self.entries[k];
            out.push_str(&format!("{} = \"{},{}\"\n", uitoml::quote(k), e.count, e.last_seq));
        }
        out
    }

    /// Record a launch of `program`.
    pub fn record(&mut self, program: &str) {
        if program.is_empty() || program == "console" {
            return;
        }
        self.next_seq = self.next_seq.saturating_add(1);
        let seq = self.next_seq;
        let e = self.entries.entry(program.to_string()).or_default();
        e.count = e.count.saturating_add(1);
        e.last_seq = seq;
    }

    pub fn count(&self, program: &str) -> u32 {
        self.entries.get(program).map_or(0, |e| e.count)
    }

    pub fn last_seq(&self, program: &str) -> u64 {
        self.entries.get(program).map_or(0, |e| e.last_seq)
    }
}

fn unquote(s: &str) -> String {
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        s.to_string()
    }
}

/// Load usage stats from disk (empty if missing).
pub fn load_usage(sys: &dyn Sys) -> UsageStats {
    match sys::read_file(sys, USAGE_PATH) {
        Ok(bytes) => UsageStats::parse(&String::from_utf8_lossy(&bytes)),
        Err(_) => UsageStats::default(),
    }
}

/// Save usage stats to disk (creating `/var/desktop` as needed).
pub fn save_usage(sys: &dyn Sys, stats: &UsageStats) {
    let _ = sys::make_dir_all(sys, "/var/desktop");
    let _ = sys::write_file(sys, USAGE_PATH, stats.format().as_bytes());
}

/// Record one launch and persist it.
pub fn record_launch(sys: &dyn Sys, stats: &mut UsageStats, program: &str) {
    stats.record(program);
    save_usage(sys, stats);
}

/// Top `limit` apps ranked by launch count, recency, core status, then catalog order.
pub fn most_used(items: &[AppItem], usage: &UsageStats, limit: usize) -> Vec<AppItem> {
    let mut indexed: Vec<(usize, &AppItem)> = items.iter().enumerate().collect();
    indexed.sort_by(|(ia, a), (ib, b)| {
        usage
            .count(&b.program)
            .cmp(&usage.count(&a.program))
            .then_with(|| usage.last_seq(&b.program).cmp(&usage.last_seq(&a.program)))
            .then_with(|| b.core.cmp(&a.core))
            .then_with(|| ia.cmp(ib))
    });
    indexed.into_iter().take(limit).map(|(_, it)| it.clone()).collect()
}

// ------------------------------------------------------------------ search & pagination

/// Filter and rank apps against `query` (multi-token case-insensitive match).
/// When `query` is blank, returns `items` in catalog order.
pub fn filter_apps(items: &[AppItem], usage: &UsageStats, query: &str) -> Vec<AppItem> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return items.to_vec();
    }
    let tokens: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<(u8, u32, u64, String, AppItem)> = Vec::new();
    for it in items {
        let label_l = it.label.to_lowercase();
        let prog_l = it.program.to_lowercase();
        if !tokens.iter().all(|tok| label_l.contains(tok) || prog_l.contains(tok)) {
            continue;
        }
        let rank = if label_l.starts_with(&q) {
            0
        } else if label_l.split_whitespace().any(|w| w.starts_with(&q)) {
            1
        } else if label_l.contains(&q) {
            2
        } else {
            3
        };
        scored.push((rank, usage.count(&it.program), usage.last_seq(&it.program), label_l, it.clone()));
    }
    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    scored.into_iter().map(|(_, _, _, _, it)| it).collect()
}

/// Grid dimensions for one page of the macOS-style All Apps overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridMetrics {
    pub cols: usize,
    pub rows: usize,
    pub cell_w: i32,
    pub cell_h: i32,
    pub per_page: usize,
}

/// Compute a responsive page grid that fits `avail_w x avail_h` cleanly for any screen size.
pub fn grid_metrics(avail_w: i32, avail_h: i32, compact: bool) -> GridMetrics {
    let (cell_w, cell_h) = if compact { (72, 54) } else { (92, 64) };
    let cols = (avail_w / cell_w).max(2) as usize;
    let rows = (avail_h / cell_h).max(1) as usize;
    GridMetrics {
        cols,
        rows,
        cell_w,
        cell_h,
        per_page: (cols * rows).max(1),
    }
}

/// Number of pages needed for `total` apps at `per_page` (at least 1).
pub fn page_count(total: usize, per_page: usize) -> usize {
    let p = per_page.max(1);
    total.div_ceil(p).max(1)
}

/// Return `(clamped_page, slice_for_page)` for `items`.
pub fn page_slice<T>(items: &[T], page: usize, per_page: usize) -> (usize, &[T]) {
    let p = per_page.max(1);
    let pages = page_count(items.len(), p);
    let clamped = page.min(pages - 1);
    let start = clamped * p;
    let end = (start + p).min(items.len());
    (clamped, &items[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::MockSys;

    #[test]
    fn usage_roundtrip_and_most_used_ranking() {
        let mut stats = UsageStats::default();
        stats.record("dev.ecm.store");
        stats.record("notes");
        stats.record("dev.ecm.store");
        let text = stats.format();
        let parsed = UsageStats::parse(&text);
        assert_eq!(parsed, stats);
        assert_eq!(parsed.count("dev.ecm.store"), 2);
        assert_eq!(parsed.count("notes"), 1);

        let mut items = core_apps();
        items.push(AppItem {
            label: "App Store".into(),
            program: "dev.ecm.store".into(),
            args: Vec::new(),
            icon: "store".into(),
            custom_icon: None,
            core: false,
        });
        let top = most_used(&items, &parsed, 4);
        assert_eq!(top.len(), 4);
        assert_eq!(top[0].program, "dev.ecm.store", "most launched first");
        assert_eq!(top[1].program, "notes", "second most launched next");
        assert_eq!(top[2].program, "terminal", "remaining slots filled from core order");
    }

    #[test]
    fn search_ranks_prefix_above_substring_and_scales_to_many_apps() {
        let mut items = core_apps();
        for i in 0..150 {
            items.push(AppItem {
                label: format!("Tool {i:03}"),
                program: format!("dev.test.tool{i:03}"),
                args: Vec::new(),
                icon: "app".into(),
                custom_icon: None,
                core: false,
            });
        }
        let mut usage = UsageStats::default();
        usage.record("dev.test.tool042");
        let hits = filter_apps(&items, &usage, "tool 042");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].program, "dev.test.tool042");

        let term_hits = filter_apps(&items, &usage, "term");
        assert_eq!(term_hits[0].program, "terminal");

        let m = grid_metrics(580, 200, false);
        assert!(m.cols >= 5 && m.rows >= 2);
        let pages = page_count(items.len(), m.per_page);
        assert!(pages >= 8, "156 apps span multiple clean pages: {pages}");
        let (p0, s0) = page_slice(&items, 0, m.per_page);
        assert_eq!((p0, s0.len()), (0, m.per_page));
        let (plast, slast) = page_slice(&items, 999, m.per_page);
        assert_eq!(plast, pages - 1);
        assert!(!slast.is_empty());
    }

    #[test]
    fn installed_apps_reads_custom_icon_and_skips_shadowed() {
        let root = std::env::temp_dir().join(format!("ecm-cat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("apps/dev.demo.alpha")).unwrap();
        std::fs::write(root.join("apps/dev.demo.alpha/app.toml"), "name = \"Alpha\"\nicon = \"icon.png\"\n").unwrap();
        std::fs::write(root.join("apps/dev.demo.alpha/icon.png"), b"\x89PNG").unwrap();
        std::fs::create_dir_all(root.join("apps/dev.ecm.editor")).unwrap();
        std::fs::write(root.join("apps/dev.ecm.editor/app.toml"), "name = \"Text Editor\"\n").unwrap();
        let sys = MockSys::new(&root);
        let installed = installed_apps(&sys);
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].label, "Alpha");
        assert_eq!(installed[0].custom_icon.as_deref(), Some("/apps/dev.demo.alpha/icon.png"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
