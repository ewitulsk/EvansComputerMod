//! SigMF metadata (`.sigmf-meta`): what inspectrum, GQRX and Universal Radio
//! Hacker read next to a `.sigmf-data` / `.cf32` / `.cs16` recording.

use ecm_dsp::iq::SampleFormat;
use std::collections::BTreeMap;

/// The fields `iqrec` writes and `iqplay` needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Meta {
    pub format: SampleFormat,
    pub sample_rate: f64,
    pub frequency: f64,
    pub description: String,
    /// Device timestamp (sample counter) of the first sample, if known.
    pub start_timestamp: Option<i64>,
    pub hw: String,
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

impl Meta {
    /// SigMF 1.0 JSON (global, one capture, no annotations).
    pub fn to_json(&self) -> String {
        let mut g = format!(
            "    \"core:datatype\": \"{}\",\n    \"core:sample_rate\": {},\n    \"core:version\": \"1.0.0\",\n    \"core:description\": \"{}\",\n    \"core:recorder\": \"ecm iqrec\",\n    \"core:hw\": \"{}\"",
            self.format.sigmf_datatype(),
            num(self.sample_rate),
            esc(&self.description),
            esc(&self.hw)
        );
        if let Some(t) = self.start_timestamp {
            g.push_str(&format!(",\n    \"ecm:start_timestamp\": {}", t));
        }
        format!(
            "{{\n  \"global\": {{\n{}\n  }},\n  \"captures\": [\n    {{\n      \"core:sample_start\": 0,\n      \"core:frequency\": {}\n    }}\n  ],\n  \"annotations\": []\n}}\n",
            g,
            num(self.frequency)
        )
    }

    /// Parse the fields above from SigMF JSON.
    pub fn parse(text: &str) -> Result<Meta, String> {
        let v = Json::parse(text)?;
        let g = v.get("global").ok_or("no global object")?;
        let dt = g.get("core:datatype").and_then(Json::as_str).ok_or("no core:datatype")?;
        let format = format_from_datatype(dt).ok_or_else(|| format!("unsupported datatype {dt}"))?;
        let sample_rate = g.get("core:sample_rate").and_then(Json::as_f64).ok_or("no core:sample_rate")?;
        let frequency = v
            .get("captures")
            .and_then(|c| match c {
                Json::Arr(a) => a.first(),
                _ => None,
            })
            .and_then(|c| c.get("core:frequency"))
            .and_then(Json::as_f64)
            .unwrap_or(0.0);
        Ok(Meta {
            format,
            sample_rate,
            frequency,
            description: g.get("core:description").and_then(Json::as_str).unwrap_or("").to_string(),
            start_timestamp: g.get("ecm:start_timestamp").and_then(Json::as_f64).map(|t| t as i64),
            hw: g.get("core:hw").and_then(Json::as_str).unwrap_or("").to_string(),
        })
    }
}

/// `cf32_le` -> Cf32, `ci16_le` -> Cs16 (also the bare names `cf32` / `cs16`).
pub fn format_from_datatype(dt: &str) -> Option<SampleFormat> {
    match dt {
        "cf32_le" | "cf32" => Some(SampleFormat::Cf32),
        "ci16_le" | "cs16" | "ci16" => Some(SampleFormat::Cs16),
        _ => dt.parse().ok(),
    }
}

/// Sample format from a file name: `.cf32`/`.cfile`/`.fc32` or `.cs16`/`.ci16`/`.sc16`.
pub fn format_from_path(path: &str) -> Option<SampleFormat> {
    let lower = path.to_ascii_lowercase();
    let ext = lower.rsplit('.').next()?;
    match ext {
        "cf32" | "cfile" | "fc32" => Some(SampleFormat::Cf32),
        "cs16" | "ci16" | "sc16" => Some(SampleFormat::Cs16),
        _ => None,
    }
}

/// `rec.cf32` -> `rec.sigmf-meta`; `rec.sigmf-data` -> `rec.sigmf-meta`.
pub fn meta_path(data_path: &str) -> String {
    let slash = data_path.rfind('/').map(|i| i + 1).unwrap_or(0);
    match data_path[slash..].rfind('.') {
        Some(i) => format!("{}.sigmf-meta", &data_path[..slash + i]),
        None => format!("{}.sigmf-meta", data_path),
    }
}

/// A minimal JSON value (enough for SigMF).
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn parse(s: &str) -> Result<Json, String> {
        let b = s.as_bytes();
        let mut p = 0;
        let v = value(b, &mut p, 0)?;
        ws(b, &mut p);
        if p != b.len() {
            return Err(format!("trailing data at {p}"));
        }
        Ok(v)
    }
}

fn ws(b: &[u8], p: &mut usize) {
    while *p < b.len() && b[*p].is_ascii_whitespace() {
        *p += 1;
    }
}

fn value(b: &[u8], p: &mut usize, depth: usize) -> Result<Json, String> {
    if depth > 64 {
        return Err("nested too deeply".into());
    }
    ws(b, p);
    let c = *b.get(*p).ok_or("unexpected end")?;
    match c {
        b'{' => {
            *p += 1;
            let mut m = BTreeMap::new();
            ws(b, p);
            if b.get(*p) == Some(&b'}') {
                *p += 1;
                return Ok(Json::Obj(m));
            }
            loop {
                ws(b, p);
                let k = string(b, p)?;
                ws(b, p);
                if b.get(*p) != Some(&b':') {
                    return Err(format!("expected ':' at {p}"));
                }
                *p += 1;
                let v = value(b, p, depth + 1)?;
                m.insert(k, v);
                ws(b, p);
                match b.get(*p) {
                    Some(b',') => *p += 1,
                    Some(b'}') => {
                        *p += 1;
                        return Ok(Json::Obj(m));
                    }
                    _ => return Err(format!("expected ',' or '}}' at {p}")),
                }
            }
        }
        b'[' => {
            *p += 1;
            let mut a = Vec::new();
            ws(b, p);
            if b.get(*p) == Some(&b']') {
                *p += 1;
                return Ok(Json::Arr(a));
            }
            loop {
                a.push(value(b, p, depth + 1)?);
                ws(b, p);
                match b.get(*p) {
                    Some(b',') => *p += 1,
                    Some(b']') => {
                        *p += 1;
                        return Ok(Json::Arr(a));
                    }
                    _ => return Err(format!("expected ',' or ']' at {p}")),
                }
            }
        }
        b'"' => string(b, p).map(Json::Str),
        b't' if b[*p..].starts_with(b"true") => {
            *p += 4;
            Ok(Json::Bool(true))
        }
        b'f' if b[*p..].starts_with(b"false") => {
            *p += 5;
            Ok(Json::Bool(false))
        }
        b'n' if b[*p..].starts_with(b"null") => {
            *p += 4;
            Ok(Json::Null)
        }
        _ => {
            let start = *p;
            while *p < b.len() && matches!(b[*p], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                *p += 1;
            }
            std::str::from_utf8(&b[start..*p])
                .ok()
                .and_then(|t| t.parse().ok())
                .map(Json::Num)
                .ok_or_else(|| format!("bad value at {start}"))
        }
    }
}

fn string(b: &[u8], p: &mut usize) -> Result<String, String> {
    if b.get(*p) != Some(&b'"') {
        return Err(format!("expected string at {p}"));
    }
    *p += 1;
    let mut out = Vec::new();
    while let Some(&c) = b.get(*p) {
        *p += 1;
        match c {
            b'"' => return Ok(String::from_utf8_lossy(&out).into_owned()),
            b'\\' => {
                let e = *b.get(*p).ok_or("bad escape")?;
                *p += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'u' => {
                        let h = b.get(*p..*p + 4).and_then(|h| std::str::from_utf8(h).ok()).ok_or("bad \\u")?;
                        let cp = u32::from_str_radix(h, 16).map_err(|_| "bad \\u")?;
                        *p += 4;
                        let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                    }
                    other => out.push(other),
                }
            }
            c => out.push(c),
        }
    }
    Err("unterminated string".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigmf_meta_round_trip_and_fields() {
        let m = Meta {
            format: SampleFormat::Cf32,
            sample_rate: 48_000.0,
            frequency: 146.52e6,
            description: "test \"quote\"".into(),
            start_timestamp: Some(123456),
            hw: "sdr_standard".into(),
        };
        let j = m.to_json();
        assert!(j.contains("\"core:datatype\": \"cf32_le\""), "{j}");
        assert!(j.contains("\"core:sample_rate\": 48000"), "{j}");
        assert!(j.contains("\"core:version\": \"1.0.0\""), "{j}");
        assert!(j.contains("\"core:frequency\": 146520000"), "{j}");
        assert!(j.contains("\"core:sample_start\": 0"), "{j}");
        let v = Json::parse(&j).unwrap();
        assert!(matches!(v.get("annotations"), Some(Json::Arr(a)) if a.is_empty()));
        assert_eq!(Meta::parse(&j).unwrap(), m);
        let cs = Meta { format: SampleFormat::Cs16, start_timestamp: None, ..m };
        assert!(cs.to_json().contains("ci16_le"));
        assert_eq!(Meta::parse(&cs.to_json()).unwrap(), cs);
    }

    #[test]
    fn paths_and_formats() {
        assert_eq!(meta_path("rec.cf32"), "rec.sigmf-meta");
        assert_eq!(meta_path("dir.x/rec"), "dir.x/rec.sigmf-meta");
        assert_eq!(meta_path("a/rec.sigmf-data"), "a/rec.sigmf-meta");
        assert_eq!(format_from_path("x.CS16"), Some(SampleFormat::Cs16));
        assert_eq!(format_from_path("x.cfile"), Some(SampleFormat::Cf32));
        assert_eq!(format_from_path("x.wav"), None);
        assert!(Json::parse("{\"a\": [1, 2.5e3, \"\\u0041\", true, null]}").is_ok());
        assert!(Json::parse("{\"a\": }").is_err());
    }
}
