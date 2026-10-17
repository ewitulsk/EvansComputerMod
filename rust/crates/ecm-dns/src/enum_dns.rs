//! ENUM (RFC 6116): telephone numbers in DNS. An E.164 number becomes a name under
//! `e164.arpa` (digits reversed, one per label), whose NAPTR records (RFC 3403) say how
//! to reach that number: `100 10 "u" "E2U+sip" "!^.*$!sip:\1@carrier.ecm!" .` means
//! "rewrite the number into a SIP URI at carrier.ecm".
//!
//! * [`enum_domain`], [`enum_block_domain`]: number (or number block) to domain name.
//! * [`aus`]: the Application Unique String (`+` and the digits) the regexps run on.
//! * [`apply_regexp`]: one NAPTR substitution expression, `!pattern!replacement!flags`.
//! * [`enum_sip_uri`]: pick the SIP record by order/preference and rewrite the number.
//!
//! The regexp subset ([`apply_regexp`]) is a small POSIX ERE, enough for what carriers
//! write in ENUM zones (`^.*$`, `^\+1(.*)$`, `^(.*)$`, `^\+1([0-9]+)$`):
//!
//! * anchors `^` (first) and `$` (last); without them the pattern may match anywhere;
//! * literal characters, `\x` for a literal `x` (`\+`, `\.`, `\\`, `\!`);
//! * `.` (any character) and bracket sets `[0-9]`, `[a-z+]`, `[^5]` (ranges, negation;
//!   no `[:digit:]` classes);
//! * the quantifiers `*`, `+` and `?` after one atom (greedy, backtracking);
//! * capture groups `( ... )` without nesting, referenced as `\1` .. `\9` in the
//!   replacement (no alternation `|`, no `{m,n}`); in a pattern without any group,
//!   `\1` is the whole match, so the common `!^.*$!sip:\1@carrier.ecm!` gives
//!   `sip:+12145550142@carrier.ecm`;
//! * the flag `i` after the last delimiter: case-insensitive matching.
//!
//! As with `sed`, the matched part of the AUS is replaced and the rest kept, so an
//! anchored `^...$` pattern rewrites the whole number.

use crate::wire::{Naptr, RData, Record};

/// The ENUM tree (RFC 6116).
pub const E164_ARPA: &str = "e164.arpa";

/// The digits of a number, `+` and separators (spaces, `-`, `.`, `(`, `)`) dropped.
fn digits(e164: &str) -> impl DoubleEndedIterator<Item = char> + '_ {
    e164.chars().filter(|c| c.is_ascii_digit())
}

/// The ENUM domain of an E.164 number: `enum_domain("+12145550142", "e164.arpa")` is
/// `2.4.1.0.5.5.5.4.1.2.1.e164.arpa`. Anything but digits is ignored; `suffix` may
/// have a trailing dot and may be empty (then the name is the digits alone).
pub fn enum_domain(e164: &str, suffix: &str) -> String {
    let mut labels: Vec<String> = digits(e164).rev().map(String::from).collect();
    let suffix = suffix.trim().trim_end_matches('.');
    if !suffix.is_empty() {
        labels.push(suffix.to_ascii_lowercase());
    }
    labels.join(".")
}

/// The ENUM domain of a number block: the country code and leading digits shared by
/// every number in it. A North American NPA-NXX (10,000 numbers) with its country
/// code: `enum_block_domain("+1214555", "e164.arpa")` is `5.5.5.4.1.2.1.e164.arpa`, the
/// zone a carrier holding 214-555-xxxx delegates to its own name servers.
pub fn enum_block_domain(prefix: &str, suffix: &str) -> String {
    enum_domain(prefix, suffix)
}

/// The Application Unique String for a number: `+` and its digits (`+12145550142`).
pub fn aus(e164: &str) -> String {
    let mut s = String::from("+");
    s.extend(digits(e164));
    s
}

/// Is this NAPTR services field ENUM SIP? `E2U+sip` (RFC 6116/3764) and the older
/// `SIP+E2U`, in any case.
pub fn is_sip_service(services: &str) -> bool {
    services.eq_ignore_ascii_case("E2U+sip") || services.eq_ignore_ascii_case("SIP+E2U")
}

/// The NAPTR data among `records`.
pub fn naptrs(records: &[Record]) -> Vec<&Naptr> {
    records.iter().filter_map(|r| if let RData::Naptr(n) = &r.data { Some(n) } else { None }).collect()
}

/// The SIP URI for a number from the NAPTR records of its ENUM domain: the records
/// with flag `u` and a SIP service, lowest order then lowest preference first; the
/// first whose regexp applies to the AUS gives the URI. `None` if no record does.
pub fn enum_sip_uri<'a>(records: impl IntoIterator<Item = &'a Naptr>, e164: &str) -> Option<String> {
    let mut sip: Vec<&Naptr> = records
        .into_iter()
        .filter(|n| n.flags.eq_ignore_ascii_case("u") && is_sip_service(&n.services) && !n.regexp.is_empty())
        .collect();
    sip.sort_by_key(|n| (n.order, n.preference));
    let aus = aus(e164);
    sip.into_iter().find_map(|n| apply_regexp(&n.regexp, &aus))
}

/// Apply one substitution expression (`!^.*$!sip:\1@carrier.ecm!`, RFC 3402 section
/// 3.2) to `input`. The first character is the delimiter. `None` when the expression is
/// malformed or outside the subset (see the module docs) or the pattern does not match.
pub fn apply_regexp(expr: &str, input: &str) -> Option<String> {
    let mut chars = expr.chars();
    let delim = chars.next()?;
    if delim.is_ascii_digit() || delim == '\\' || delim == 'i' {
        return None;
    }
    // Split into pattern, replacement, flags; `\delim` stays escaped in its part.
    let mut parts: Vec<String> = vec![String::new()];
    let mut it = chars.peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            let n = it.next()?;
            let last = parts.last_mut()?;
            last.push('\\');
            last.push(n);
        } else if c == delim {
            parts.push(String::new());
        } else {
            parts.last_mut()?.push(c);
        }
    }
    if parts.len() != 3 {
        return None;
    }
    let case_insensitive = match parts[2].as_str() {
        "" => false,
        "i" => true,
        _ => return None,
    };
    let re = Regex::compile(&parts[0], case_insensitive)?;
    let text: Vec<char> = input.chars().collect();
    let (start, end, caps) = re.find(&text)?;
    let mut out: String = text[..start].iter().collect();
    let mut r = parts[1].chars();
    while let Some(c) = r.next() {
        if c == '\\' {
            match r.next()? {
                d @ '1'..='9' => {
                    let i = d as usize - '1' as usize;
                    // Without any group, the first backreference is the whole match.
                    let (s, e) = if caps.is_empty() && i == 0 { (start, end) } else { caps.get(i).copied().flatten()? };
                    out.extend(&text[s..e]);
                }
                other => out.push(other),
            }
        } else {
            out.push(c);
        }
    }
    out.extend(&text[end..]);
    Some(out)
}

#[derive(Clone, Debug, PartialEq)]
enum Atom {
    Char(char),
    Any,
    Set { negated: bool, items: Vec<(char, char)> },
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Atom { atom: Atom, min: usize, max: usize },
    Open(usize),
    Close(usize),
}

#[derive(Debug)]
struct Regex {
    nodes: Vec<Node>,
    groups: usize,
    start_anchor: bool,
    end_anchor: bool,
    ci: bool,
}

type Caps = Vec<Option<(usize, usize)>>;

impl Regex {
    fn compile(pat: &str, ci: bool) -> Option<Regex> {
        let mut p: Vec<char> = pat.chars().collect();
        let start_anchor = p.first() == Some(&'^');
        if start_anchor {
            p.remove(0);
        }
        // A trailing `$` is an anchor unless escaped (an odd run of backslashes).
        let mut end_anchor = false;
        if p.last() == Some(&'$') {
            let bs = p[..p.len() - 1].iter().rev().take_while(|&&c| c == '\\').count();
            if bs % 2 == 0 {
                end_anchor = true;
                p.pop();
            }
        }
        let mut nodes = Vec::new();
        let mut groups = 0;
        let mut open: Option<usize> = None;
        let mut i = 0;
        while i < p.len() {
            let c = p[i];
            i += 1;
            let atom = match c {
                '(' => {
                    if open.is_some() || groups >= 9 {
                        return None; // nesting is outside the subset
                    }
                    open = Some(groups);
                    nodes.push(Node::Open(groups));
                    groups += 1;
                    continue;
                }
                ')' => {
                    nodes.push(Node::Close(open.take()?));
                    continue;
                }
                '\\' => {
                    let n = *p.get(i)?;
                    i += 1;
                    Atom::Char(n)
                }
                '.' => Atom::Any,
                '[' => {
                    let mut negated = false;
                    if p.get(i) == Some(&'^') {
                        negated = true;
                        i += 1;
                    }
                    let mut items = Vec::new();
                    let mut first = true;
                    loop {
                        let c = *p.get(i)?;
                        if c == ']' && !first {
                            i += 1;
                            break;
                        }
                        first = false;
                        i += 1;
                        if p.get(i) == Some(&'-') && p.get(i + 1).is_some_and(|&e| e != ']') {
                            let e = p[i + 1];
                            if e < c {
                                return None;
                            }
                            items.push((c, e));
                            i += 2;
                        } else {
                            items.push((c, c));
                        }
                    }
                    Atom::Set { negated, items }
                }
                '*' | '+' | '?' | '|' | '{' | '}' | '^' | '$' => return None,
                c => Atom::Char(c),
            };
            let (min, max) = match p.get(i) {
                Some('*') => (0, usize::MAX),
                Some('+') => (1, usize::MAX),
                Some('?') => (0, 1),
                _ => (1, 1),
            };
            if max != 1 || min != 1 {
                i += 1;
            }
            nodes.push(Node::Atom { atom, min, max });
        }
        if open.is_some() {
            return None;
        }
        Some(Regex { nodes, groups, start_anchor, end_anchor, ci })
    }

    fn atom_matches(&self, a: &Atom, c: char) -> bool {
        let fold = |x: char| if self.ci { x.to_ascii_lowercase() } else { x };
        match a {
            Atom::Any => true,
            Atom::Char(x) => fold(*x) == fold(c),
            Atom::Set { negated, items } => {
                let hit = items.iter().any(|&(lo, hi)| {
                    (lo..=hi).contains(&c) || (self.ci && ((lo..=hi).contains(&c.to_ascii_lowercase()) || (lo..=hi).contains(&c.to_ascii_uppercase())))
                });
                hit != *negated
            }
        }
    }

    /// Leftmost match: (start, end, captures).
    fn find(&self, text: &[char]) -> Option<(usize, usize, Caps)> {
        let last = if self.start_anchor { 0 } else { text.len() };
        for start in 0..=last {
            let mut caps: Caps = vec![None; self.groups];
            let mut open = vec![0usize; self.groups];
            if let Some(end) = self.step(text, 0, start, &mut caps, &mut open) {
                return Some((start, end, caps));
            }
        }
        None
    }

    fn step(&self, text: &[char], ni: usize, pos: usize, caps: &mut Caps, open: &mut Vec<usize>) -> Option<usize> {
        let Some(node) = self.nodes.get(ni) else {
            return (!self.end_anchor || pos == text.len()).then_some(pos);
        };
        match node {
            Node::Open(g) => {
                let saved = open[*g];
                open[*g] = pos;
                let r = self.step(text, ni + 1, pos, caps, open);
                if r.is_none() {
                    open[*g] = saved;
                }
                r
            }
            Node::Close(g) => {
                let saved = caps[*g];
                caps[*g] = Some((open[*g], pos));
                let r = self.step(text, ni + 1, pos, caps, open);
                if r.is_none() {
                    caps[*g] = saved;
                }
                r
            }
            Node::Atom { atom, min, max } => {
                let mut n = 0;
                while n < *max && pos + n < text.len() && self.atom_matches(atom, text[pos + n]) {
                    n += 1;
                }
                if n < *min {
                    return None;
                }
                // Greedy: the longest run first, then give back one at a time.
                (*min..=n).rev().find_map(|k| self.step(text, ni + 1, pos + k, caps, open))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naptr(order: u16, preference: u16, flags: &str, services: &str, regexp: &str) -> Naptr {
        Naptr {
            order,
            preference,
            flags: flags.into(),
            services: services.into(),
            regexp: regexp.into(),
            replacement: String::new(),
        }
    }

    #[test]
    fn enum_names() {
        assert_eq!(enum_domain("+12145550142", E164_ARPA), "2.4.1.0.5.5.5.4.1.2.1.e164.arpa");
        assert_eq!(enum_domain("+1 (214) 555-0142", "e164.arpa."), "2.4.1.0.5.5.5.4.1.2.1.e164.arpa");
        assert_eq!(enum_domain("+4420", "e164.ecm"), "0.2.4.4.e164.ecm");
        assert_eq!(enum_domain("+12", ""), "2.1");
        assert_eq!(enum_block_domain("+1214555", E164_ARPA), "5.5.5.4.1.2.1.e164.arpa");
        assert!(enum_domain("+12145550142", E164_ARPA).ends_with(&enum_block_domain("+1214555", E164_ARPA)));
        assert_eq!(aus("1 214 555 0142"), "+12145550142");
    }

    #[test]
    fn regexp_subset() {
        let n = "+12145550142";
        assert_eq!(apply_regexp("!^.*$!sip:\\1@carrier.ecm!", n).as_deref(), Some("sip:+12145550142@carrier.ecm"), "\\1 without a group");
        assert_eq!(apply_regexp("!^(.*)$!sip:\\2@carrier.ecm!", n), None, "no second group");
        assert_eq!(apply_regexp("!^.*$!sip:info@carrier.ecm!", n).as_deref(), Some("sip:info@carrier.ecm"));
        assert_eq!(apply_regexp("!^(.*)$!sip:\\1@carrier.ecm!", n).as_deref(), Some("sip:+12145550142@carrier.ecm"));
        assert_eq!(apply_regexp("!^\\+1(.*)$!sip:\\1@carrier.ecm!", n).as_deref(), Some("sip:2145550142@carrier.ecm"));
        assert_eq!(apply_regexp("!^\\+44(.*)$!sip:\\1@uk.ecm!", n), None, "no match");
        assert_eq!(apply_regexp("/^\\+1([0-9]+)$/sip:\\1@c.ecm;user=phone/", n).as_deref(), Some("sip:2145550142@c.ecm;user=phone"));
        assert_eq!(apply_regexp("!^\\+1214(555)(.*)$!sip:\\2@x\\1.ecm!", n).as_deref(), Some("sip:0142@x555.ecm"));
        assert_eq!(apply_regexp("!^\\+1[^2].*$!x!", n), None);
        assert_eq!(apply_regexp("!0142$!9999!", n).as_deref(), Some("+12145559999"), "unanchored start, sed-like");
        assert_eq!(apply_regexp("!555!-!", n).as_deref(), Some("+1214-0142"));
        assert_eq!(apply_regexp("!^\\+1(.?)!<\\1>!", n).as_deref(), Some("<2>145550142"));
        assert_eq!(apply_regexp("!^SIP:(.*)$!\\1!i", "sip:abc").as_deref(), Some("abc"));
        assert_eq!(apply_regexp("!^SIP:(.*)$!\\1!", "sip:abc"), None, "case matters without i");
        assert_eq!(apply_regexp("#^(.*)$#sip:\\1\\#x#", "+1").as_deref(), Some("sip:+1#x"), "escaped delimiter");
        // malformed or outside the subset
        for bad in ["", "!^.*$!x", "!^.*$!x!y!", "!^((.*))$!x!", "!a|b!x!", "!(.*!x!", "1^.*$1x1", "!^.*$!x!g", "![z-a]!x!"] {
            assert_eq!(apply_regexp(bad, n), None, "{bad}");
        }
    }

    #[test]
    fn picks_sip_by_order_and_preference() {
        let n = "+12145550142";
        let recs = vec![
            naptr(100, 20, "u", "E2U+sip", "!^.*$!sip:backup@b.ecm!"),
            naptr(100, 10, "u", "E2U+sip", "!^\\+1(.*)$!sip:\\1@carrier.ecm!"),
            naptr(50, 10, "u", "E2U+mailto", "!^.*$!mailto:x@y.ecm!"),
            naptr(10, 10, "", "E2U+sip", "!^.*$!sip:nonterminal@x.ecm!"),
        ];
        assert_eq!(enum_sip_uri(&recs, n).as_deref(), Some("sip:2145550142@carrier.ecm"));
        // a better record that does not apply to this number is skipped
        let mut more = recs.clone();
        more.push(naptr(90, 10, "U", "SIP+E2U", "!^\\+44(.*)$!sip:\\1@uk.ecm!"));
        assert_eq!(enum_sip_uri(&more, n).as_deref(), Some("sip:2145550142@carrier.ecm"));
        more.push(naptr(90, 5, "u", "e2u+SIP", "!^(.*)$!sip:\\1@first.ecm!"));
        assert_eq!(enum_sip_uri(&more, n).as_deref(), Some("sip:+12145550142@first.ecm"));
        assert_eq!(enum_sip_uri(&recs[2..3], n), None);
        let rec = Record::new("2.4.1.0.5.5.5.4.1.2.1.e164.arpa", 60, RData::Naptr(recs[1].clone()));
        assert_eq!(enum_sip_uri(naptrs(&[rec]), n).as_deref(), Some("sip:2145550142@carrier.ecm"));
    }
}
