//! vCard 3.0 (RFC 2426), the contact format of Contacts (`/home/.contacts/contacts.vcf`),
//! shared contacts in Messages (`text/x-vcard` in an MMS) and `.vcf` imports.
//!
//! ```text
//! BEGIN:VCARD
//! VERSION:3.0
//! UID:6f1c...
//! FN:Alex Smith
//! N:Smith;Alex;;;
//! TEL;TYPE=CELL:+12145550142
//! TEL;TYPE=WORK:+12145550100
//! EMAIL:alex@village.ecm
//! NOTE:met at the Phone Store
//! PHOTO;VALUE=uri:file:///home/Pictures/alex.png
//! X-ECM-FAVOURITE:1
//! END:VCARD
//! ```
//!
//! Lines longer than 75 octets are folded on write (CRLF + space) and unfolded on
//! read; values escape `\\`, `,`, `;` and newlines (`\n`). Reading is lenient
//! about what it doesn't know (other properties are kept as they are and written
//! back), strict about structure (a card must end).

use core::fmt::Write;

/// One phone number of a contact, with its label (`CELL`, `HOME`, `WORK`, `MAIN`, `OTHER`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tel {
    pub label: String,
    pub number: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    pub uid: String,
    /// The formatted name (`FN`).
    pub name: String,
    pub tels: Vec<Tel>,
    pub emails: Vec<String>,
    pub note: String,
    /// A picture: a path in the computer's files (`PHOTO;VALUE=uri:file://...`).
    pub photo: String,
    pub favourite: bool,
    /// Properties this module doesn't model, kept as whole lines.
    pub other: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VcardError {
    pub line: usize,
    pub msg: String,
}

impl core::fmt::Display for VcardError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

fn escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            ',' => o.push_str("\\,"),
            ';' => o.push_str("\\;"),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            c => o.push(c),
        }
    }
    o
}

fn unescape(s: &str) -> String {
    let mut o = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') | Some('N') => o.push('\n'),
                Some(x) => o.push(x),
                None => {}
            }
        } else {
            o.push(c);
        }
    }
    o
}

/// Split at `;` not preceded by a backslash.
fn split_unescaped(s: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut esc = false;
    for c in s.chars() {
        if esc {
            cur.push('\\');
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == sep {
            out.push(core::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    if esc {
        cur.push('\\');
    }
    out.push(cur);
    out
}

fn fold(line: &str, out: &mut String) {
    let mut n = 0;
    for c in line.chars() {
        let l = c.len_utf8();
        if n + l > 75 {
            out.push_str("\r\n ");
            n = 1;
        }
        out.push(c);
        n += l;
    }
    out.push_str("\r\n");
}

impl Contact {
    pub fn new(name: &str, number: &str) -> Contact {
        Contact { name: name.into(), tels: vec![Tel { label: "CELL".into(), number: number.into() }], ..Default::default() }
    }

    /// The first number (the one Call and Text use).
    pub fn primary(&self) -> Option<&str> {
        self.tels.first().map(|t| t.number.as_str())
    }

    /// Initials for the avatar circle ("AS" for "Alex Smith").
    pub fn initials(&self) -> String {
        let mut s: String = self.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
        if s.is_empty() {
            s.push('#');
        }
        s.to_uppercase()
    }

    pub fn to_vcard(&self) -> String {
        let mut o = String::new();
        fold("BEGIN:VCARD", &mut o);
        fold("VERSION:3.0", &mut o);
        if !self.uid.is_empty() {
            fold(&format!("UID:{}", escape(&self.uid)), &mut o);
        }
        fold(&format!("FN:{}", escape(&self.name)), &mut o);
        // N: family;given — the last word is the family name.
        let mut words: Vec<&str> = self.name.split_whitespace().collect();
        let family = if words.len() > 1 { words.pop().unwrap_or("") } else { "" };
        fold(&format!("N:{};{};;;", escape(family), escape(&words.join(" "))), &mut o);
        for t in &self.tels {
            let mut l = String::from("TEL");
            if !t.label.is_empty() {
                let _ = write!(l, ";TYPE={}", t.label.to_uppercase());
            }
            let _ = write!(l, ":{}", escape(&t.number));
            fold(&l, &mut o);
        }
        for e in &self.emails {
            fold(&format!("EMAIL:{}", escape(e)), &mut o);
        }
        if !self.note.is_empty() {
            fold(&format!("NOTE:{}", escape(&self.note)), &mut o);
        }
        if !self.photo.is_empty() {
            fold(&format!("PHOTO;VALUE=uri:file://{}", self.photo), &mut o);
        }
        if self.favourite {
            fold("X-ECM-FAVOURITE:1", &mut o);
        }
        for x in &self.other {
            fold(x, &mut o);
        }
        fold("END:VCARD", &mut o);
        o
    }
}

/// Every card in `text` (a `.vcf` file holds any number).
pub fn parse(text: &str) -> Result<Vec<Contact>, VcardError> {
    // unfold: a line starting with space or tab continues the previous one
    let mut lines: Vec<(usize, String)> = Vec::new();
    for (i, raw) in text.split('\n').enumerate() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().expect("checked").1.push_str(&raw[1..]);
        } else if !raw.trim().is_empty() {
            lines.push((i + 1, raw.to_string()));
        }
    }
    let mut out = Vec::new();
    let mut cur: Option<Contact> = None;
    let mut last_line = 0;
    for (line, l) in lines {
        last_line = line;
        let err = |msg: &str| VcardError { line, msg: msg.into() };
        let Some((head, value)) = l.split_once(':') else {
            return Err(err("expected NAME:value"));
        };
        let mut params = head.split(';');
        let name = params.next().unwrap_or("").to_ascii_uppercase();
        let params: Vec<String> = params.map(|p| p.to_ascii_uppercase()).collect();
        match name.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VCARD") => {
                if cur.is_some() {
                    return Err(err("BEGIN:VCARD inside a card"));
                }
                cur = Some(Contact::default());
                continue;
            }
            "END" if value.eq_ignore_ascii_case("VCARD") => {
                let c = cur.take().ok_or_else(|| err("END:VCARD without BEGIN"))?;
                out.push(c);
                continue;
            }
            _ => {}
        }
        let Some(c) = cur.as_mut() else {
            return Err(err("property outside a card"));
        };
        match name.as_str() {
            "VERSION" | "PRODID" => {}
            "UID" => c.uid = unescape(value),
            "FN" => c.name = unescape(value),
            "N" => {
                if c.name.is_empty() {
                    let p = split_unescaped(value, ';');
                    let family = p.first().map(|s| unescape(s)).unwrap_or_default();
                    let given = p.get(1).map(|s| unescape(s)).unwrap_or_default();
                    c.name = [given, family].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
                }
            }
            "TEL" => {
                let label = params
                    .iter()
                    .filter_map(|p| p.strip_prefix("TYPE=").map(|t| t.to_string()).or_else(|| (!p.contains('=')).then(|| p.clone())))
                    .flat_map(|t| t.split(',').map(|x| x.to_string()).collect::<Vec<_>>())
                    .find(|t| matches!(t.as_str(), "CELL" | "HOME" | "WORK" | "MAIN" | "OTHER" | "IPHONE" | "MOBILE"))
                    .map(|t| if t == "MOBILE" || t == "IPHONE" { "CELL".to_string() } else { t })
                    .unwrap_or_else(|| "CELL".into());
                let number = unescape(value.strip_prefix("tel:").unwrap_or(value));
                c.tels.push(Tel { label, number });
            }
            "EMAIL" => c.emails.push(unescape(value)),
            "NOTE" => c.note = unescape(value),
            "PHOTO" => {
                if let Some(p) = value.strip_prefix("file://") {
                    c.photo = p.to_string();
                } else {
                    c.other.push(l.clone());
                }
            }
            "X-ECM-FAVOURITE" => c.favourite = value.trim() == "1",
            _ => c.other.push(l.clone()),
        }
    }
    if cur.is_some() {
        return Err(VcardError { line: last_line, msg: "card not ended (END:VCARD missing)".into() });
    }
    Ok(out)
}

/// Every contact as one `.vcf` text.
pub fn write_all(cards: &[Contact]) -> String {
    cards.iter().map(|c| c.to_vcard()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut c = Contact::new("Alex Smith", "+12145550142");
        c.uid = "a1".into();
        c.tels.push(Tel { label: "WORK".into(), number: "+12145550100".into() });
        c.emails.push("alex@village.ecm".into());
        c.note = "met at the store; likes trains, mostly\nsecond line".into();
        c.photo = "/home/Pictures/alex.png".into();
        c.favourite = true;
        let text = c.to_vcard();
        assert!(text.contains("FN:Alex Smith\r\n"));
        assert!(text.contains("N:Smith;Alex;;;\r\n"));
        assert!(text.contains("TEL;TYPE=CELL:+12145550142\r\n"));
        assert!(text.contains("NOTE:met at the store\\; likes trains\\, mostly\\nsecond line"));
        let back = parse(&text).unwrap();
        assert_eq!(back, vec![c.clone()]);
        assert_eq!(c.initials(), "AS");
        let two = write_all(&[c.clone(), Contact::new("Bo", "+12145550143")]);
        assert_eq!(parse(&two).unwrap().len(), 2);
    }

    #[test]
    fn folding_and_other_writers() {
        let mut c = Contact::new("Cy", "+12145550144");
        c.note = "x".repeat(200);
        let t = c.to_vcard();
        assert!(t.lines().all(|l| l.trim_end_matches('\r').len() <= 75));
        assert_eq!(parse(&t).unwrap()[0].note, c.note);
        // iOS-style export
        let ios = "BEGIN:VCARD\nVERSION:3.0\nN:Doe;Jane;;;\nTEL;type=CELL;type=VOICE;type=pref:(214) 555-0150\nitem1.ADR;type=HOME:;;1 Road;;;;\nEND:VCARD\n";
        let j = &parse(ios).unwrap()[0];
        assert_eq!(j.name, "Jane Doe");
        assert_eq!(j.tels, vec![Tel { label: "CELL".into(), number: "(214) 555-0150".into() }]);
        assert_eq!(j.other.len(), 1, "unknown properties kept");
        assert!(j.to_vcard().contains("item1.ADR"));
    }

    #[test]
    fn errors() {
        assert!(parse("BEGIN:VCARD\nFN:x\n").is_err());
        assert!(parse("FN:x\n").is_err());
        assert!(parse("BEGIN:VCARD\nBEGIN:VCARD\n").is_err());
        assert_eq!(parse("").unwrap(), vec![]);
        let mut x: u32 = 7;
        for _ in 0..2000 {
            let s: String = (0..(x % 60))
                .map(|_| {
                    x = x.wrapping_mul(1103515245).wrapping_add(12345);
                    b"BEGIN:VCARDEND;=\\\n TEL"[(x >> 16) as usize % 22] as char
                })
                .collect();
            let _ = parse(&s);
        }
    }
}
