//! Shared bits of the text save formats: one record per line, fields split by
//! tabs, with `\\`, `\t` and `\n` escaped so any tag text round-trips.

/// Escape one field.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
    out
}

/// Undo [`esc`].
pub fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// A record line from escaped fields.
pub fn line(fields: &[&str]) -> String {
    let mut s = fields.iter().map(|f| esc(f)).collect::<Vec<_>>().join("\t");
    s.push('\n');
    s
}

/// The unescaped fields of a record line.
pub fn fields(line: &str) -> Vec<String> {
    line.split('\t').map(unesc).collect()
}

/// FNV-1a, 64 bit: stable short ids (song ids from paths, album and artist
/// ids from names) that do not change between runs or machines.
pub fn fnv64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_round_trip() {
        let l = line(&["a\tb", "c\\d\n", "", "é"]);
        assert_eq!(l.matches('\n').count(), 1);
        assert_eq!(fields(l.trim_end_matches('\n')), vec!["a\tb", "c\\d\n", "", "é"]);
        assert_ne!(fnv64("a"), fnv64("b"));
    }
}
