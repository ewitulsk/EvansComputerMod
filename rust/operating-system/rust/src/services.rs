//! Persistent kernel services and background commands, replayed once at boot.
use crate::fs;
pub const PATH: &str = "services.cfg";
pub fn commands() -> Vec<String> {
    fs::read_to_string(PATH)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
        .map(str::to_string)
        .collect()
}
pub fn enable(name: &str, on: bool) {
    let command = format!("{} on", name);
    let mut lines = commands();
    lines.retain(|l| l != &command);
    if on {
        lines.push(command);
    }
    fs::write(PATH, format!("{}\n", lines.join("\n")).as_bytes());
}
