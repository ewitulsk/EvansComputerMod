//! Domain names as the rest of the crate keeps them: lowercase, dotted, no trailing dot,
//! the root is `""`. [`fqdn`] adds the trailing dot for display (`www.lab.test.`, `.`).

/// Longest name in presentation form without the trailing dot (255 wire bytes).
pub const MAX_NAME: usize = 253;
pub const MAX_LABEL: usize = 63;

/// Validate and canonicalise a name: lowercase, one optional trailing dot removed,
/// labels 1-63 bytes of printable ASCII without spaces or dots, at most 253 bytes.
/// `"."` and `""` are the root.
pub fn normalize(s: &str) -> Option<String> {
    let s = s.trim();
    if s == "." || s.is_empty() {
        return Some(String::new());
    }
    let s = s.strip_suffix('.').unwrap_or(s);
    if s.len() > MAX_NAME {
        return None;
    }
    for label in s.split('.') {
        if label.is_empty() || label.len() > MAX_LABEL {
            return None;
        }
        if !label.bytes().all(|c| c.is_ascii_graphic() && c != b'.' && c != b'\\') {
            return None;
        }
    }
    Some(s.to_ascii_lowercase())
}

/// `name` with a trailing dot (`"."` for the root).
pub fn fqdn(name: &str) -> String {
    if name.is_empty() {
        ".".into()
    } else {
        format!("{name}.")
    }
}

/// Is `name` equal to or below `zone`?
pub fn is_sub(name: &str, zone: &str) -> bool {
    zone.is_empty()
        || name == zone
        || (name.len() > zone.len() + 1
            && name.ends_with(zone)
            && name.as_bytes()[name.len() - zone.len() - 1] == b'.')
}

/// The parent of a name (`None` for the root).
pub fn parent(name: &str) -> Option<&str> {
    if name.is_empty() {
        None
    } else {
        Some(name.split_once('.').map_or("", |(_, p)| p))
    }
}

/// Number of labels (`0` for the root).
pub fn label_count(name: &str) -> usize {
    if name.is_empty() {
        0
    } else {
        name.split('.').count()
    }
}

/// The top-level label (`"ring"` for `web.v4.ring`, `""` for the root).
pub fn tld(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or("")
}

/// The ancestor of `name` that is one label below `zone` (`name` must be strictly below
/// `zone`): `child_of("www.shop.base", "base") == "shop.base"`.
pub fn child_of<'a>(name: &'a str, zone: &str) -> &'a str {
    let keep = label_count(zone) + 1;
    let labels: Vec<&str> = name.split('.').collect();
    let skip = labels.len().saturating_sub(keep);
    let start: usize = labels[..skip].iter().map(|l| l.len() + 1).sum();
    &name[start..]
}

/// Make `name` (as written in a zone file) absolute: a trailing dot means absolute, `@`
/// is the origin, anything else is relative to `origin`.
pub fn absolute(name: &str, origin: &str) -> Option<String> {
    if name == "@" {
        return Some(origin.to_string());
    }
    if name.ends_with('.') {
        return normalize(name);
    }
    if origin.is_empty() {
        normalize(name)
    } else {
        normalize(&format!("{name}.{origin}"))
    }
}

/// `name` relative to `origin` for writing zone files (`@` for the origin itself).
pub fn relative(name: &str, origin: &str) -> String {
    if name == origin {
        "@".into()
    } else if !origin.is_empty() && is_sub(name, origin) {
        name[..name.len() - origin.len() - 1].to_string()
    } else {
        fqdn(name)
    }
}

/// Is `label` a hostname label (letters, digits, hyphens, not starting or ending with
/// a hyphen)?
pub fn is_ldh(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= MAX_LABEL
        && label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

/// IPv4 addresses as plain bytes.
pub type Ip = [u8; 4];

pub fn parse_ip(s: &str) -> Option<Ip> {
    let mut out = [0u8; 4];
    let mut parts = s.trim().split('.');
    for b in out.iter_mut() {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *b = p.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

pub fn ip_str(ip: Ip) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

/// `a.b.c.d/len` (a bare address is a /32).
pub fn parse_cidr(s: &str) -> Option<(Ip, u8)> {
    match s.split_once('/') {
        Some((a, l)) => {
            let l: u8 = l.parse().ok()?;
            (l <= 32).then_some((parse_ip(a)?, l))
        }
        None => Some((parse_ip(s)?, 32)),
    }
}

pub fn in_cidr(ip: Ip, net: (Ip, u8)) -> bool {
    let mask = if net.1 == 0 { 0 } else { u32::MAX << (32 - net.1 as u32) };
    u32::from_be_bytes(ip) & mask == u32::from_be_bytes(net.0) & mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(normalize("WWW.Lab.Test."), Some("www.lab.test".into()));
        assert_eq!(normalize("."), Some("".into()));
        assert_eq!(normalize("a..b"), None);
        assert_eq!(normalize(&"x".repeat(64)), None);
        assert_eq!(normalize("a b"), None);
        assert!(is_sub("www.lab.test", "lab.test"));
        assert!(is_sub("lab.test", "lab.test"));
        assert!(is_sub("lab.test", ""));
        assert!(!is_sub("xlab.test", "lab.test"));
        assert!(!is_sub("test", "lab.test"));
        assert_eq!(parent("www.lab.test"), Some("lab.test"));
        assert_eq!(parent("test"), Some(""));
        assert_eq!(parent(""), None);
        assert_eq!(child_of("www.shop.base", "base"), "shop.base");
        assert_eq!(child_of("www.shop.base", ""), "base");
        assert_eq!(child_of("shop.base", "base"), "shop.base");
        assert_eq!(absolute("www", "lab.test"), Some("www.lab.test".into()));
        assert_eq!(absolute("@", "lab.test"), Some("lab.test".into()));
        assert_eq!(absolute("ns1.other.", "lab.test"), Some("ns1.other".into()));
        assert_eq!(absolute("a", ""), Some("a".into()));
        assert_eq!(relative("www.lab.test", "lab.test"), "www");
        assert_eq!(relative("lab.test", "lab.test"), "@");
        assert_eq!(relative("x.other", "lab.test"), "x.other.");
        assert_eq!(tld("web.v4.ring"), "ring");
        assert!(is_ldh("my-base1") && !is_ldh("-a") && !is_ldh("a_b"));
    }

    #[test]
    fn ips() {
        assert_eq!(parse_ip("100.65.0.53"), Some([100, 65, 0, 53]));
        assert_eq!(parse_ip("1.2.3"), None);
        assert_eq!(parse_ip("1.2.3.256"), None);
        assert!(in_cidr([100, 70, 1, 2], parse_cidr("100.64.0.0/10").unwrap()));
        assert!(!in_cidr([100, 128, 1, 2], parse_cidr("100.64.0.0/10").unwrap()));
        assert!(in_cidr([9, 9, 9, 9], parse_cidr("0.0.0.0/0").unwrap()));
    }
}
