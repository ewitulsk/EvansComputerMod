//! Phone numbers: a North American style plan inside one world.
//!
//! Numbers are stored as E.164 (`+12145550142`: `+`, the country code, then the
//! national number) and written NANP style, `(214) 555-0142`: a 3-digit area
//! code (NPA, first digit 2-9), a 3-digit exchange (NXX, first digit 2-9) and a
//! 4-digit line. One country code per world (+1 by default, server config).
//!
//! A numbering administrator (`ianad`) hands carriers **blocks** of 10,000
//! numbers, one exchange each (`NPA-NXX`), as NANPA does. ENUM (RFC 6116) names
//! reverse the digits under `e164.arpa`: a block is `5.5.5.4.1.2.1.e164.arpa`, one
//! number `2.4.1.0.5.5.5.4.1.2.1.e164.arpa`.
//!
//! What a person types into the keypad ([`classify`]):
//!
//! | Dialled | Meaning |
//! |---|---|
//! | `2145550142`, `(214) 555-0142`, `1 214 555 0142`, `+12145550142` | a number |
//! | `5550142` (7 digits) | a number in the phone's own area code |
//! | `*67` + number | the call without caller ID |
//! | `*86`, a long press on 1 | voicemail |
//! | `*#06#` | show the IMEI (handled on the phone) |
//! | `*72` + number / `*73` | forward all calls / stop forwarding |
//! | `611` | carrier support (an IVR on the media server) |
//! | 5-6 digits (`22225`) | a short code (texts to a service) |
//! | `911` | not available (left out for now) |

use core::fmt;

/// A number in E.164 form: `+` and 8-15 digits.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct E164(String);

impl E164 {
    /// `+` followed by 8..=15 digits, the first not 0.
    pub fn parse(s: &str) -> Option<E164> {
        let d = s.strip_prefix('+')?;
        if (8..=15).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()) && !d.starts_with('0') {
            Some(E164(s.to_string()))
        } else {
            None
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// The digits without `+`.
    pub fn digits(&self) -> &str {
        &self.0[1..]
    }
    /// The NANP parts `(npa, nxx, line)` when this is a 10-digit number under country code `cc`.
    pub fn nanp_parts(&self, cc: &str) -> Option<(&str, &str, &str)> {
        let rest = self.digits().strip_prefix(cc)?;
        if rest.len() == 10 {
            Some((&rest[0..3], &rest[3..6], &rest[6..10]))
        } else {
            None
        }
    }
    /// `(214) 555-0142` for a number under `cc`; anything else as stored.
    pub fn display(&self, cc: &str) -> String {
        match self.nanp_parts(cc) {
            Some((a, b, c)) => format!("({}) {}-{}", a, b, c),
            None => self.0.clone(),
        }
    }
    /// The ENUM domain, `2.4.1.0.5.5.5.4.1.2.1.e164.arpa`.
    pub fn enum_domain(&self) -> String {
        enum_name(self.digits())
    }
}

impl fmt::Display for E164 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reverse the digits under `e164.arpa` (RFC 6116).
pub fn enum_name(digits: &str) -> String {
    let mut s = String::new();
    for c in digits.chars().rev().filter(|c| c.is_ascii_digit()) {
        s.push(c);
        s.push('.');
    }
    s.push_str("e164.arpa");
    s
}

/// A block of 10,000 numbers: one exchange, `NPA-NXX`, under a country code.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Block {
    pub cc: String,
    pub npa: String,
    pub nxx: String,
}

impl Block {
    /// `214-555` (with `cc`), `+1214555` or `1214555`.
    pub fn parse(s: &str, default_cc: &str) -> Option<Block> {
        let t = s.trim();
        let (cc, rest) = if let Some(d) = t.strip_prefix('+') {
            if !d.starts_with(default_cc) {
                return None;
            }
            (default_cc, &d[default_cc.len()..])
        } else {
            (default_cc, t)
        };
        let digits: String = rest.chars().filter(|c| *c != '-' && *c != ' ').collect();
        let digits = if digits.len() == 6 + cc.len() && digits.starts_with(cc) && !t.contains('-') { digits[cc.len()..].to_string() } else { digits };
        if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let b = Block { cc: cc.to_string(), npa: digits[..3].to_string(), nxx: digits[3..].to_string() };
        if valid_npa(&b.npa) && valid_nxx(&b.nxx) {
            Some(b)
        } else {
            None
        }
    }
    /// The number with line `0000..=9999`.
    pub fn number(&self, line: u16) -> E164 {
        E164(format!("+{}{}{}{:04}", self.cc, self.npa, self.nxx, line.min(9999)))
    }
    pub fn contains(&self, n: &E164) -> bool {
        n.digits().strip_prefix(&self.cc).is_some_and(|r| r.len() == 10 && r.starts_with(&self.npa) && r[3..].starts_with(&self.nxx))
    }
    /// The line of `n` in this block.
    pub fn line(&self, n: &E164) -> Option<u16> {
        if self.contains(n) {
            n.digits()[self.cc.len() + 6..].parse().ok()
        } else {
            None
        }
    }
    /// `+1214555`, the E.164 prefix every number of the block starts with.
    pub fn prefix(&self) -> String {
        format!("+{}{}{}", self.cc, self.npa, self.nxx)
    }
    /// The block's ENUM domain, `5.5.5.4.1.2.1.e164.arpa`.
    pub fn enum_domain(&self) -> String {
        enum_name(&format!("{}{}{}", self.cc, self.npa, self.nxx))
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.npa, self.nxx)
    }
}

/// An area code: 3 digits, first 2-9, not `N11` (service codes) or `555`... (`555` is fine as an exchange).
pub fn valid_npa(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit()) && s.as_bytes()[0] >= b'2' && !(s.as_bytes()[1] == b'1' && s.as_bytes()[2] == b'1')
}

/// An exchange: 3 digits, first 2-9, not `N11`.
pub fn valid_nxx(s: &str) -> bool {
    valid_npa(s)
}

/// What the keypad's digits mean (see the module table).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dialled {
    /// A number; `hide_id` when it was dialled after `*67`.
    Number { number: E164, hide_id: bool },
    /// 5-6 digit short code (texts to services).
    ShortCode(String),
    Voicemail,
    ShowImei,
    /// `*72` + number.
    ForwardAll(E164),
    /// `*73`.
    ForwardOff,
    /// `611`.
    Support,
    /// `911` and other N11 codes the world doesn't run.
    Unavailable(String),
    /// Not a number we can place.
    Invalid,
}

/// The digits (and `+ * #`) of a dial string, dropping spaces, dashes, dots and brackets.
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_digit() || matches!(c, '+' | '*' | '#')).collect()
}

/// A number typed or stored in any common form, as E.164: `cc` is the world's
/// country code, `home_npa` the phone's own area code (for 7-digit dialling).
pub fn normalize(s: &str, cc: &str, home_npa: Option<&str>) -> Option<E164> {
    let d = clean(s);
    if d.starts_with('+') {
        return E164::parse(&d);
    }
    if d.contains('*') || d.contains('#') {
        return None;
    }
    let full = if d.len() == 10 {
        format!("+{}{}", cc, d)
    } else if d.len() == 10 + cc.len() && d.starts_with(cc) {
        format!("+{}", d)
    } else if d.len() == 7 {
        format!("+{}{}{}", cc, home_npa?, d)
    } else {
        return None;
    };
    let n = E164::parse(&full)?;
    if let Some((a, b, _)) = n.nanp_parts(cc) {
        if !valid_npa(a) || !valid_nxx(b) {
            return None;
        }
    }
    Some(n)
}

/// What a keypad entry means.
pub fn classify(s: &str, cc: &str, home_npa: Option<&str>) -> Dialled {
    let d = clean(s);
    match d.as_str() {
        "*86" => return Dialled::Voicemail,
        "*#06#" => return Dialled::ShowImei,
        "*73" => return Dialled::ForwardOff,
        "611" => return Dialled::Support,
        _ => {}
    }
    if d.len() == 3 && d.ends_with("11") {
        return Dialled::Unavailable(d);
    }
    if let Some(rest) = d.strip_prefix("*67") {
        return match normalize(rest, cc, home_npa) {
            Some(number) => Dialled::Number { number, hide_id: true },
            None => Dialled::Invalid,
        };
    }
    if let Some(rest) = d.strip_prefix("*72") {
        return match normalize(rest, cc, home_npa) {
            Some(n) => Dialled::ForwardAll(n),
            None => Dialled::Invalid,
        };
    }
    if (5..=6).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()) {
        return Dialled::ShortCode(d);
    }
    match normalize(&d, cc, home_npa) {
        Some(number) => Dialled::Number { number, hide_id: false },
        None => Dialled::Invalid,
    }
}

/// Formats what is being typed on the keypad as it grows: `214`, `214-55`,
/// `(214) 555-01`, `555-0142`; codes and other strings as typed.
pub fn format_partial(typed: &str) -> String {
    let d = clean(typed);
    if d.is_empty() || !d.bytes().all(|b| b.is_ascii_digit()) {
        return d;
    }
    let (one, d) = match d.strip_prefix('1') {
        Some(r) if d.len() > 7 => ("1 ", r.to_string()),
        _ => ("", d),
    };
    let s = match d.len() {
        0..=3 => d.clone(),
        4..=7 => format!("{}-{}", &d[..3], &d[3..]),
        8..=10 => format!("({}) {}-{}", &d[..3], &d[3..6], &d[6..]),
        _ => return format!("{}{}", one, d),
    };
    format!("{}{}", one, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e164_and_display() {
        let n = E164::parse("+12145550142").unwrap();
        assert_eq!(n.display("1"), "(214) 555-0142");
        assert_eq!(n.enum_domain(), "2.4.1.0.5.5.5.4.1.2.1.e164.arpa");
        assert!(E164::parse("12145550142").is_none());
        assert!(E164::parse("+0123456789").is_none());
        assert!(E164::parse("+1214").is_none());
        assert_eq!(E164::parse("+442071234567").unwrap().display("1"), "+442071234567");
    }

    #[test]
    fn blocks() {
        let b = Block::parse("214-555", "1").unwrap();
        assert_eq!(b.number(142).as_str(), "+12145550142");
        assert_eq!(b.prefix(), "+1214555");
        assert_eq!(b.enum_domain(), "5.5.5.4.1.2.1.e164.arpa");
        assert!(b.contains(&E164::parse("+12145559999").unwrap()));
        assert!(!b.contains(&E164::parse("+12145560000").unwrap()));
        assert_eq!(b.line(&E164::parse("+12145550142").unwrap()), Some(142));
        assert_eq!(Block::parse("+1214555", "1"), Some(b.clone()));
        assert_eq!(Block::parse("214555", "1"), Some(b.clone()));
        assert!(Block::parse("114-555", "1").is_none(), "NPA starts 2-9");
        assert!(Block::parse("211-555", "1").is_none(), "N11");
        assert!(Block::parse("214-55", "1").is_none());
        assert_eq!(b.to_string(), "214-555");
    }

    #[test]
    fn normalizing() {
        let want = Some(E164::parse("+12145550142").unwrap());
        for s in ["2145550142", "(214) 555-0142", "1 214 555 0142", "+1 214-555-0142", "214.555.0142"] {
            assert_eq!(normalize(s, "1", None), want, "{}", s);
        }
        assert_eq!(normalize("555-0142", "1", Some("214")), want);
        assert_eq!(normalize("555-0142", "1", None), None);
        assert_eq!(normalize("0145550142", "1", None), None, "NPA starting 0");
        assert_eq!(normalize("*67", "1", None), None);
    }

    #[test]
    fn classifying() {
        let n = E164::parse("+12145550199").unwrap();
        assert_eq!(classify("2145550199", "1", None), Dialled::Number { number: n.clone(), hide_id: false });
        assert_eq!(classify("*672145550199", "1", None), Dialled::Number { number: n.clone(), hide_id: true });
        assert_eq!(classify("*86", "1", None), Dialled::Voicemail);
        assert_eq!(classify("*#06#", "1", None), Dialled::ShowImei);
        assert_eq!(classify("*72 214 555 0199", "1", None), Dialled::ForwardAll(n));
        assert_eq!(classify("*73", "1", None), Dialled::ForwardOff);
        assert_eq!(classify("611", "1", None), Dialled::Support);
        assert_eq!(classify("911", "1", None), Dialled::Unavailable("911".into()));
        assert_eq!(classify("22225", "1", None), Dialled::ShortCode("22225".into()));
        assert_eq!(classify("12", "1", None), Dialled::Invalid);
    }

    #[test]
    fn partial_formatting() {
        assert_eq!(format_partial("214"), "214");
        assert_eq!(format_partial("21455"), "214-55");
        assert_eq!(format_partial("21455501"), "(214) 555-01");
        assert_eq!(format_partial("2145550142"), "(214) 555-0142");
        assert_eq!(format_partial("12145550142"), "1 (214) 555-0142");
        assert_eq!(format_partial("*86"), "*86");
    }
}
