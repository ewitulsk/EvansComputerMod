//! Reading configuration from the environment.

/// A variable's value, or `None` when it is unset or empty.
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn var_or(name: &str, default: &str) -> String {
    var(name).unwrap_or_else(|| default.to_string())
}

pub fn required(name: &str) -> Result<String, String> {
    var(name).ok_or_else(|| format!("{name} is not set"))
}

/// `true`, `1`, `yes`, `on` (any case) are true; unset is `default`.
pub fn flag(name: &str, default: bool) -> bool {
    match var(name) {
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on"),
        None => default,
    }
}

pub fn number(name: &str, default: u64) -> Result<u64, String> {
    match var(name) {
        Some(v) => v.parse().map_err(|_| format!("{name} must be a number, not '{v}'")),
        None => Ok(default),
    }
}

/// The public origin without a trailing slash; must be http(s).
pub fn origin(name: &str, default: &str) -> Result<String, String> {
    let o = var_or(name, default).trim_end_matches('/').to_string();
    if o.starts_with("https://") || o.starts_with("http://") {
        Ok(o)
    } else {
        Err(format!("{name} must start with http:// or https://"))
    }
}
