//! `/etc/ui.toml` (UI_PROTOCOL §2) for the apps: read a top-level key and
//! change one in place, keeping every other line and comment, the way the
//! compositor's `UiConfig` reads it (top-level `key = value` lines, a
//! `[screen]` table).

/// The config file.
pub const PATH: &str = "/etc/ui.toml";

/// The value of top-level `key` (strings unquoted), if set.
pub fn get(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            break;
        }
        let Some((k, v)) = t.split_once('=') else { continue };
        if k.trim() != key {
            continue;
        }
        let v = strip_comment(v).trim().to_string();
        return Some(if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') { v[1..v.len() - 1].to_string() } else { v });
    }
    None
}

fn strip_comment(v: &str) -> &str {
    let mut in_str = false;
    for (i, c) in v.char_indices() {
        if c == '"' {
            in_str = !in_str;
        } else if c == '#' && !in_str {
            return &v[..i];
        }
    }
    v
}

/// A TOML string literal.
pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `text` with top-level `key` set to `value` (a TOML literal such as
/// `"ecm-light"` or `true`): the existing line is replaced, or a new one is
/// added before the first table.
pub fn set(text: &str, key: &str, value: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut done = false;
    let mut in_table = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            if !done {
                out.push(format!("{key} = {value}"));
                done = true;
            }
            in_table = true;
        }
        if !in_table && !done {
            if let Some((k, _)) = t.split_once('=') {
                if k.trim() == key && !t.starts_with('#') {
                    out.push(format!("{key} = {value}"));
                    done = true;
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if !done {
        out.push(format!("{key} = {value}"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_and_set_keep_the_rest() {
        let t = "# GUI\nautostart = true\ntheme = \"ecm-default\" # dark\n\n[screen]\nmode = \"mirror\"\n";
        assert_eq!(get(t, "theme").as_deref(), Some("ecm-default"));
        assert_eq!(get(t, "autostart").as_deref(), Some("true"));
        assert_eq!(get(t, "mode"), None, "table keys aren't top level");
        let t2 = set(t, "theme", &quote("ecm-light"));
        assert_eq!(get(&t2, "theme").as_deref(), Some("ecm-light"));
        assert!(t2.contains("# GUI") && t2.contains("[screen]\nmode = \"mirror\""));
        let t3 = set(&t2, "shell", &quote("dev.ecm.desktop"));
        assert!(t3.find("shell =").unwrap() < t3.find("[screen]").unwrap(), "new keys go before the first table");
        assert_eq!(get(&set("", "autostart", "true"), "autostart").as_deref(), Some("true"));
    }
}
