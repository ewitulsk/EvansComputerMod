//! The `.card` design file (CARDS.md "Programs and apps"):
//!
//! ```text
//! {"kind": "bank", "palette": [[r,g,b], …], "front": "<base64>", "back": "<base64>",
//!  "fields": {"name": "…", …}, "stripe": {"t1": "…", "t2": "…", "t3": "…"}}
//! ```
//!
//! A face is a palette-indexed bitmap: 4-bit indices, row-major, two texels
//! per byte with the high nibble first; 32×20 texels for `bank` / `nfc` /
//! `hybrid`, 8×13 (portrait) for `sim`. The palette has at most 16 RGB entries and index
//! 0 is its first entry. [`Design::text`] draws with a 3×5 pixel font
//! (digits, A–Z, space, `-`, `.`; glyphs advance 4 texels).

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::json::Json;

pub const KINDS: [&str; 4] = ["bank", "nfc", "hybrid", "sim"];
pub const MAX_PALETTE: usize = 16;
pub const MAX_FIELDS: usize = 8;
pub const MAX_FIELD_CHARS: usize = 64;
pub const STRIPE_LIMITS: [usize; 3] = [64, 32, 96];

/// Face size in texels for a card kind, or `None` for an unknown kind.
pub fn face_size(kind: &str) -> Option<(usize, usize)> {
    match kind {
        "bank" | "nfc" | "hybrid" => Some((32, 20)),
        "sim" => Some((8, 13)),
        _ => None,
    }
}

/// Packed bytes of a `w × h` face.
pub fn face_bytes(w: usize, h: usize) -> usize {
    (w * h + 1) / 2
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Front,
    Back,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    pub kind: String,
    pub palette: Vec<[u8; 3]>,
    /// Packed 4-bit indices, row-major, high nibble first.
    pub front: Vec<u8>,
    pub back: Vec<u8>,
    pub fields: BTreeMap<String, String>,
    pub stripe: Option<(String, String, String)>,
}

impl Design {
    /// A blank design of `kind` (every texel index 0, a one-entry white palette).
    pub fn new(kind: &str) -> Result<Design, String> {
        let (w, h) = face_size(kind).ok_or_else(|| alloc::format!("unknown card kind '{kind}' (bank, nfc, hybrid, sim)"))?;
        Ok(Design {
            kind: kind.to_string(),
            palette: alloc::vec![[255, 255, 255]],
            front: alloc::vec![0; face_bytes(w, h)],
            back: alloc::vec![0; face_bytes(w, h)],
            fields: BTreeMap::new(),
            stripe: None,
        })
    }

    pub fn width(&self) -> usize {
        face_size(&self.kind).map(|s| s.0).unwrap_or(0)
    }
    pub fn height(&self) -> usize {
        face_size(&self.kind).map(|s| s.1).unwrap_or(0)
    }

    pub fn face(&self, face: Face) -> &[u8] {
        match face {
            Face::Front => &self.front,
            Face::Back => &self.back,
        }
    }
    fn face_mut(&mut self, face: Face) -> &mut Vec<u8> {
        match face {
            Face::Front => &mut self.front,
            Face::Back => &mut self.back,
        }
    }

    /// The palette index at `(x, y)` (0 outside the face).
    pub fn pixel(&self, face: Face, x: usize, y: usize) -> u8 {
        let w = self.width();
        if x >= w || y >= self.height() {
            return 0;
        }
        let i = y * w + x;
        let b = self.face(face).get(i / 2).copied().unwrap_or(0);
        if i % 2 == 0 {
            b >> 4
        } else {
            b & 15
        }
    }

    /// Set the palette index at `(x, y)`; outside the face nothing happens.
    pub fn set_pixel(&mut self, face: Face, x: usize, y: usize, idx: u8) {
        let (w, h) = (self.width(), self.height());
        if x >= w || y >= h {
            return;
        }
        let i = y * w + x;
        let data = self.face_mut(face);
        if data.len() < face_bytes(w, h) {
            data.resize(face_bytes(w, h), 0);
        }
        let b = &mut data[i / 2];
        if i % 2 == 0 {
            *b = (*b & 0x0F) | ((idx & 15) << 4);
        } else {
            *b = (*b & 0xF0) | (idx & 15);
        }
    }

    pub fn fill(&mut self, face: Face, idx: u8) {
        let n = face_bytes(self.width(), self.height());
        let v = ((idx & 15) << 4) | (idx & 15);
        *self.face_mut(face) = alloc::vec![v; n];
    }

    /// Draw `s` with the 3×5 font, its top-left texel at `(x, y)`, in palette
    /// index `idx`; only set pixels are painted. Returns the x after the text.
    pub fn text(&mut self, face: Face, x: i32, y: i32, s: &str, idx: u8) -> i32 {
        let mut cx = x;
        for c in s.chars() {
            if let Some(rows) = glyph(c) {
                for (dy, row) in rows.iter().enumerate() {
                    for dx in 0..3 {
                        if row & (4 >> dx) != 0 {
                            let (px, py) = (cx + dx as i32, y + dy as i32);
                            if px >= 0 && py >= 0 {
                                self.set_pixel(face, px as usize, py as usize, idx);
                            }
                        }
                    }
                }
            }
            cx += GLYPH_ADVANCE;
        }
        cx
    }

    /// Nearest palette entry to an RGB colour (adding it when there is room
    /// and no exact match). Returns the index.
    pub fn color(&mut self, rgb: [u8; 3]) -> u8 {
        if let Some(i) = self.palette.iter().position(|c| *c == rgb) {
            return i as u8;
        }
        if self.palette.len() < MAX_PALETTE {
            self.palette.push(rgb);
            return (self.palette.len() - 1) as u8;
        }
        let dist = |c: &[u8; 3]| c.iter().zip(&rgb).map(|(a, b)| (*a as i32 - *b as i32).pow(2)).sum::<i32>();
        self.palette.iter().enumerate().min_by_key(|(_, c)| dist(c)).map(|(i, _)| i as u8).unwrap_or(0)
    }

    /// Problems a printer would reject the design for.
    pub fn validate(&self) -> Result<(), String> {
        let (w, h) = face_size(&self.kind).ok_or_else(|| alloc::format!("unknown card kind '{}'", self.kind))?;
        let n = face_bytes(w, h);
        if self.front.len() != n || self.back.len() != n {
            return Err(alloc::format!("faces of a {} card are {w}x{h} texels ({n} packed bytes)", self.kind));
        }
        if self.palette.is_empty() || self.palette.len() > MAX_PALETTE {
            return Err(alloc::format!("the palette has 1 to {MAX_PALETTE} colours"));
        }
        if self.fields.len() > MAX_FIELDS {
            return Err(alloc::format!("at most {MAX_FIELDS} fields"));
        }
        for (k, v) in &self.fields {
            if k.is_empty() || k.chars().count() > MAX_FIELD_CHARS || v.chars().count() > MAX_FIELD_CHARS {
                return Err(alloc::format!("field '{k}': keys and values are 1 to {MAX_FIELD_CHARS} characters"));
            }
        }
        if let Some((t1, t2, t3)) = &self.stripe {
            if self.kind != "bank" && self.kind != "hybrid" {
                return Err(alloc::format!("a {} card has no stripe", self.kind));
            }
            for (i, t) in [t1, t2, t3].into_iter().enumerate() {
                if t.len() > STRIPE_LIMITS[i] || !t.bytes().all(|c| (0x20..0x7f).contains(&c)) {
                    return Err(alloc::format!("stripe track {} is up to {} printable ASCII characters", i + 1, STRIPE_LIMITS[i]));
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------ JSON

    pub fn to_json(&self) -> Json {
        let mut kv = alloc::vec![
            ("kind".to_string(), Json::Str(self.kind.clone())),
            (
                "palette".to_string(),
                Json::Arr(self.palette.iter().map(|c| Json::Arr(c.iter().map(|v| Json::Int(*v as i64)).collect())).collect()),
            ),
            ("front".to_string(), Json::Str(base64_encode(&self.front))),
            ("back".to_string(), Json::Str(base64_encode(&self.back))),
            ("fields".to_string(), Json::Obj(self.fields.iter().map(|(k, v)| (k.clone(), Json::Str(v.clone()))).collect())),
        ];
        if let Some((t1, t2, t3)) = &self.stripe {
            kv.push((
                "stripe".to_string(),
                Json::Obj(alloc::vec![
                    ("t1".to_string(), Json::Str(t1.clone())),
                    ("t2".to_string(), Json::Str(t2.clone())),
                    ("t3".to_string(), Json::Str(t3.clone())),
                ]),
            ));
        }
        Json::Obj(kv)
    }

    pub fn to_json_string(&self) -> String {
        self.to_json().to_string()
    }

    pub fn from_json(j: &Json) -> Result<Design, String> {
        let kind = j.get("kind").and_then(Json::as_str).ok_or("missing \"kind\"")?;
        let (w, h) = face_size(kind).ok_or_else(|| alloc::format!("unknown card kind '{kind}'"))?;
        let n = face_bytes(w, h);
        let mut palette = Vec::new();
        for c in j.get("palette").and_then(Json::as_array).ok_or("missing \"palette\"")? {
            let c = c.as_array().ok_or("a palette entry is [r, g, b]")?;
            if c.len() != 3 {
                return Err("a palette entry is [r, g, b]".into());
            }
            let ch = |i: usize| c[i].as_i64().filter(|v| (0..=255).contains(v)).map(|v| v as u8).ok_or("a palette channel is 0..255");
            palette.push([ch(0)?, ch(1)?, ch(2)?]);
        }
        let face = |key: &str| -> Result<Vec<u8>, String> {
            match j.get(key) {
                None | Some(Json::Null) => Ok(alloc::vec![0; n]),
                Some(v) => {
                    let text = v.as_str().ok_or_else(|| alloc::format!("\"{key}\" is base64 text"))?;
                    let mut bytes = base64_decode(text).ok_or_else(|| alloc::format!("\"{key}\" is not base64"))?;
                    if bytes.len() > n {
                        return Err(alloc::format!("\"{key}\" holds more than {w}x{h} texels"));
                    }
                    bytes.resize(n, 0);
                    Ok(bytes)
                }
            }
        };
        let mut fields = BTreeMap::new();
        if let Some(f) = j.get("fields") {
            for (k, v) in f.as_object().ok_or("\"fields\" is an object of strings")? {
                fields.insert(k.clone(), v.as_str().ok_or("\"fields\" is an object of strings")?.to_string());
            }
        }
        let stripe = match j.get("stripe") {
            None | Some(Json::Null) => None,
            Some(s) => {
                let t = |k: &str| s.get(k).and_then(Json::as_str).unwrap_or("").to_string();
                Some((t("t1"), t("t2"), t("t3")))
            }
        };
        Ok(Design { kind: kind.to_string(), palette, front: face("front")?, back: face("back")?, fields, stripe })
    }

    pub fn from_json_string(text: &str) -> Result<Design, String> {
        Design::from_json(&Json::parse(text)?)
    }

    #[cfg(feature = "std")]
    pub fn load(path: &str) -> Result<Design, String> {
        let text = std::fs::read_to_string(path).map_err(|e| alloc::format!("{path}: {e}"))?;
        Design::from_json_string(&text).map_err(|e| alloc::format!("{path}: {e}"))
    }

    #[cfg(feature = "std")]
    pub fn save(&self, path: &str) -> Result<(), String> {
        std::fs::write(path, self.to_json_string()).map_err(|e| alloc::format!("{path}: {e}"))
    }
}

// ---------------------------------------------------------------- font

pub const GLYPH_W: i32 = 3;
pub const GLYPH_H: i32 = 5;
pub const GLYPH_ADVANCE: i32 = 4;

/// The 3×5 font: five rows per glyph, each a 3-bit value with the left
/// texel in bit 2. Shared verbatim with cards.py / cards.lua.
pub const FONT: &[(char, [u8; 5])] = &[
    ('0', [7, 5, 5, 5, 7]),
    ('1', [2, 6, 2, 2, 7]),
    ('2', [7, 1, 7, 4, 7]),
    ('3', [7, 1, 7, 1, 7]),
    ('4', [5, 5, 7, 1, 1]),
    ('5', [7, 4, 7, 1, 7]),
    ('6', [7, 4, 7, 5, 7]),
    ('7', [7, 1, 1, 1, 1]),
    ('8', [7, 5, 7, 5, 7]),
    ('9', [7, 5, 7, 1, 7]),
    ('A', [2, 5, 7, 5, 5]),
    ('B', [6, 5, 6, 5, 6]),
    ('C', [7, 4, 4, 4, 7]),
    ('D', [6, 5, 5, 5, 6]),
    ('E', [7, 4, 7, 4, 7]),
    ('F', [7, 4, 7, 4, 4]),
    ('G', [7, 4, 5, 5, 7]),
    ('H', [5, 5, 7, 5, 5]),
    ('I', [7, 2, 2, 2, 7]),
    ('J', [1, 1, 1, 5, 7]),
    ('K', [5, 5, 6, 5, 5]),
    ('L', [4, 4, 4, 4, 7]),
    ('M', [5, 7, 7, 5, 5]),
    ('N', [6, 5, 5, 5, 5]),
    ('O', [7, 5, 5, 5, 7]),
    ('P', [7, 5, 7, 4, 4]),
    ('Q', [7, 5, 5, 7, 1]),
    ('R', [7, 5, 6, 5, 5]),
    ('S', [7, 4, 7, 1, 7]),
    ('T', [7, 2, 2, 2, 2]),
    ('U', [5, 5, 5, 5, 7]),
    ('V', [5, 5, 5, 5, 2]),
    ('W', [5, 5, 7, 7, 5]),
    ('X', [5, 5, 2, 5, 5]),
    ('Y', [5, 5, 2, 2, 2]),
    ('Z', [7, 1, 2, 4, 7]),
    (' ', [0, 0, 0, 0, 0]),
    ('-', [0, 0, 7, 0, 0]),
    ('.', [0, 0, 0, 0, 2]),
];

/// The rows of a glyph (lower-case letters map to upper-case); `None` for a
/// character the font lacks (it still takes a glyph's advance).
pub fn glyph(c: char) -> Option<&'static [u8; 5]> {
    let c = c.to_ascii_uppercase();
    FONT.iter().find(|(g, _)| *g == c).map(|(_, rows)| rows)
}

// ---------------------------------------------------------------- base64

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.len();
        let v = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(B64[(v >> 18) as usize & 63] as char);
        out.push(B64[(v >> 12) as usize & 63] as char);
        out.push(if n > 1 { B64[(v >> 6) as usize & 63] as char } else { '=' });
        out.push(if n > 2 { B64[v as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for n in 0..10 {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let text = base64_encode(&data);
            assert_eq!(text.len() % 4, 0);
            assert_eq!(base64_decode(&text).unwrap(), data, "{text}");
        }
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert!(base64_decode("TW!u").is_none());
    }

    #[test]
    fn pixels_pack_high_nibble_first() {
        let mut d = Design::new("sim").unwrap();
        assert_eq!((d.width(), d.height()), (8, 13));
        assert_eq!(d.front.len(), 52);
        d.set_pixel(Face::Front, 0, 0, 0xA);
        d.set_pixel(Face::Front, 1, 0, 0xB);
        d.set_pixel(Face::Front, 7, 12, 15);
        assert_eq!(d.front[0], 0xAB);
        assert_eq!(d.front[51], 0x0F);
        assert_eq!(d.pixel(Face::Front, 1, 0), 0xB);
        assert_eq!(d.pixel(Face::Front, 7, 12), 15);
        assert_eq!(d.pixel(Face::Back, 0, 0), 0);
        d.set_pixel(Face::Front, 8, 0, 1); // outside: ignored
        assert_eq!(d.pixel(Face::Front, 8, 0), 0);
        d.fill(Face::Back, 3);
        assert!(d.back.iter().all(|b| *b == 0x33));
        let mut b = Design::new("bank").unwrap();
        assert_eq!(b.front.len(), 320);
        b.set_pixel(Face::Back, 31, 19, 7);
        assert_eq!(b.back[319], 0x07);
    }

    #[test]
    fn design_round_trips_through_json() {
        let mut d = Design::new("hybrid").unwrap();
        d.palette = alloc::vec![[255, 255, 255], [0, 0, 0], [200, 30, 30]];
        d.fill(Face::Front, 2);
        d.text(Face::Front, 1, 1, "ACME BANK", 1);
        d.text(Face::Front, 1, 13, "1234-5678", 0);
        d.set_pixel(Face::Back, 5, 7, 1);
        d.fields.insert("name".into(), "Ann Example".into());
        d.fields.insert("number".into(), "12345678".into());
        d.stripe = Some(("ANN".into(), "12345678".into(), "".into()));
        d.validate().unwrap();
        let text = d.to_json_string();
        assert!(text.starts_with("{\"kind\":\"hybrid\",\"palette\":[[255,255,255],[0,0,0],[200,30,30]],\"front\":\""), "{text}");
        assert!(text.contains("\"stripe\":{\"t1\":\"ANN\",\"t2\":\"12345678\",\"t3\":\"\"}"));
        let back = Design::from_json_string(&text).unwrap();
        assert_eq!(back, d);
        // Missing faces are blank, a missing stripe is None.
        let min = Design::from_json_string(r#"{"kind":"nfc","palette":[[1,2,3]]}"#).unwrap();
        assert_eq!(min.front, alloc::vec![0; 320]);
        assert_eq!(min.stripe, None);
        assert!(Design::from_json_string(r#"{"kind":"ticket","palette":[]}"#).is_err());
        assert!(Design::from_json_string(r#"{"kind":"sim","palette":[[1,2]]}"#).is_err());
        assert!(Design::from_json_string(r#"{"kind":"sim","palette":[[1,2,3]],"front":"*"}"#).is_err());
    }

    #[test]
    fn validation_catches_sizes() {
        let mut d = Design::new("nfc").unwrap();
        d.stripe = Some(("a".into(), "".into(), "".into()));
        assert!(d.validate().unwrap_err().contains("no stripe"));
        let mut b = Design::new("bank").unwrap();
        b.stripe = Some(("x".repeat(65), "".into(), "".into()));
        assert!(b.validate().unwrap_err().contains("track 1"));
        b.stripe = None;
        b.front.pop();
        assert!(b.validate().unwrap_err().contains("32x20"));
        b.front.push(0);
        b.palette.clear();
        assert!(b.validate().unwrap_err().contains("palette"));
        b.palette.push([0, 0, 0]);
        for i in 0..9 {
            b.fields.insert(alloc::format!("k{i}"), "v".into());
        }
        assert!(b.validate().unwrap_err().contains("8 fields"));
    }

    #[test]
    fn font_renders_something_legible() {
        let mut d = Design::new("bank").unwrap();
        let end = d.text(Face::Front, 0, 0, "A1-. ", 1);
        assert_eq!(end, 20);
        // 'A' = 010 / 101 / 111 / 101 / 101
        let rows: Vec<String> = (0..5).map(|y| (0..3).map(|x| if d.pixel(Face::Front, x, y) == 1 { '#' } else { '.' }).collect()).collect();
        assert_eq!(rows, [".#.", "#.#", "###", "#.#", "#.#"]);
        // '1' at x = 4: 010 / 110 / 010 / 010 / 111
        assert_eq!(d.pixel(Face::Front, 5, 0), 1);
        assert_eq!(d.pixel(Face::Front, 4, 1), 1);
        assert_eq!(d.pixel(Face::Front, 6, 4), 1);
        // '-' only on the middle row, '.' only bottom middle, space nothing.
        assert_eq!((d.pixel(Face::Front, 8, 2), d.pixel(Face::Front, 8, 0)), (1, 0));
        assert_eq!((d.pixel(Face::Front, 13, 4), d.pixel(Face::Front, 13, 3)), (1, 0));
        assert!((16..19).all(|x| (0..5).all(|y| d.pixel(Face::Front, x, y) == 0)));
        // Lower case draws as upper case, the font has every expected glyph.
        let mut e = Design::new("bank").unwrap();
        e.text(Face::Back, 0, 0, "a", 2);
        assert_eq!(e.pixel(Face::Back, 1, 0), 2);
        for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ -.".chars() {
            assert!(glyph(c).is_some(), "{c}");
        }
        assert!(glyph('!').is_none());
        // Clipped at the edge, negative start.
        let mut f = Design::new("sim").unwrap();
        f.text(Face::Front, 6, 11, "88", 1);
        f.text(Face::Front, -2, -2, "8", 1);
        assert_eq!((f.pixel(Face::Front, 7, 11), f.pixel(Face::Front, 7, 12)), (1, 0));
        assert_eq!(f.pixel(Face::Front, 0, 0), 1);
    }

    #[test]
    fn color_reuses_and_adds_palette_entries() {
        let mut d = Design::new("bank").unwrap();
        assert_eq!(d.color([255, 255, 255]), 0);
        assert_eq!(d.color([0, 0, 0]), 1);
        assert_eq!(d.color([0, 0, 0]), 1);
        for i in 2..16 {
            assert_eq!(d.color([i as u8 * 10, 0, 0]), i as u8);
        }
        assert_eq!(d.palette.len(), 16);
        assert_eq!(d.color([250, 250, 250]), 0);
    }
}
