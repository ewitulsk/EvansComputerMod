//! Minimal JSON reader for the shared test-vector files (no external dependency).

#![allow(dead_code)]

use std::collections::BTreeMap;

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
    pub fn get(&self, k: &str) -> &Json {
        match self {
            Json::Obj(m) => m.get(k).unwrap_or_else(|| panic!("missing key {k}")),
            _ => panic!("not an object"),
        }
    }
    pub fn str(&self) -> &str {
        match self {
            Json::Str(s) => s,
            _ => panic!("not a string: {self:?}"),
        }
    }
    pub fn num(&self) -> f64 {
        match self {
            Json::Num(n) => *n,
            _ => panic!("not a number"),
        }
    }
    pub fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(a) => a,
            _ => panic!("not an array"),
        }
    }
    pub fn hex(&self) -> Vec<u8> {
        hex(self.str())
    }
}

pub fn hex(s: &str) -> Vec<u8> {
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()).collect()
}

pub fn parse(s: &str) -> Json {
    let b = s.as_bytes();
    let mut p = 0;
    let v = value(b, &mut p);
    v
}

pub fn load(name: &str) -> Json {
    let path = format!("{}/tests/vectors/{}", env!("CARGO_MANIFEST_DIR"), name);
    parse(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}")))
}

fn ws(b: &[u8], p: &mut usize) {
    while *p < b.len() && (b[*p] as char).is_whitespace() {
        *p += 1;
    }
}

fn value(b: &[u8], p: &mut usize) -> Json {
    ws(b, p);
    match b[*p] {
        b'{' => {
            *p += 1;
            let mut m = BTreeMap::new();
            loop {
                ws(b, p);
                if b[*p] == b'}' {
                    *p += 1;
                    break;
                }
                let Json::Str(k) = value(b, p) else { panic!("key") };
                ws(b, p);
                assert_eq!(b[*p], b':');
                *p += 1;
                let v = value(b, p);
                m.insert(k, v);
                ws(b, p);
                if b[*p] == b',' {
                    *p += 1;
                }
            }
            Json::Obj(m)
        }
        b'[' => {
            *p += 1;
            let mut a = Vec::new();
            loop {
                ws(b, p);
                if b[*p] == b']' {
                    *p += 1;
                    break;
                }
                a.push(value(b, p));
                ws(b, p);
                if b[*p] == b',' {
                    *p += 1;
                }
            }
            Json::Arr(a)
        }
        b'"' => {
            *p += 1;
            let mut s = String::new();
            while b[*p] != b'"' {
                if b[*p] == b'\\' {
                    *p += 1;
                    match b[*p] {
                        b'n' => s.push('\n'),
                        b't' => s.push('\t'),
                        b'u' => {
                            let h = std::str::from_utf8(&b[*p + 1..*p + 5]).unwrap();
                            s.push(char::from_u32(u32::from_str_radix(h, 16).unwrap()).unwrap());
                            *p += 4;
                        }
                        c => s.push(c as char),
                    }
                } else {
                    s.push(b[*p] as char);
                }
                *p += 1;
            }
            *p += 1;
            Json::Str(s)
        }
        b't' => {
            *p += 4;
            Json::Bool(true)
        }
        b'f' => {
            *p += 5;
            Json::Bool(false)
        }
        b'n' => {
            *p += 4;
            Json::Null
        }
        _ => {
            let st = *p;
            while *p < b.len() && matches!(b[*p], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                *p += 1;
            }
            Json::Num(std::str::from_utf8(&b[st..*p]).unwrap().parse().unwrap())
        }
    }
}
