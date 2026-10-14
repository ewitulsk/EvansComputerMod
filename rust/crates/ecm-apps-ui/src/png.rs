//! Icons and screenshots: a small PNG decoder (8-bit grey, grey+alpha, RGB,
//! RGBA and palette images, not interlaced) so pictures from a repository
//! are checked here and uploaded as raw RGBA. A broken PNG from a stranger's
//! repository then only loses its picture, never the app's UI connection
//! (the compositor closes a connection that uploads an unreadable PNG).
//!
//! [`Images`] keeps the image resources of one UI connection.

use ecm_ui::app::Core;
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::collections::HashMap;

/// Largest picture accepted (pixels per side).
pub const MAX_SIDE: u32 = 512;

/// A decoded picture, RGBA8888 rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Rgba {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Width and height from the header, without decoding.
pub fn size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    Some((be32(&png[16..20]), be32(&png[20..24])))
}

/// Decode a PNG. `Err` names what is wrong.
pub fn decode(png: &[u8]) -> Result<Rgba, String> {
    let (w, h) = size(png).ok_or("not a PNG")?;
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
        return Err(format!("{w}x{h} is too large (at most {MAX_SIDE}x{MAX_SIDE})"));
    }
    let (depth, color, interlace) = (png[24], png[25], png[28]);
    if depth != 8 || interlace != 0 {
        return Err("only 8-bit, non-interlaced PNGs".into());
    }
    let channels = match color {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(format!("unknown PNG colour type {color}")),
    };
    let mut pos = 8;
    let mut idat = Vec::new();
    let mut palette: Vec<[u8; 4]> = Vec::new();
    while pos + 8 <= png.len() {
        let len = be32(&png[pos..]) as usize;
        let kind = &png[pos + 4..pos + 8];
        let data = png.get(pos + 8..pos + 8 + len).ok_or("PNG chunk runs past the end")?;
        match kind {
            b"IDAT" => idat.extend_from_slice(data),
            b"PLTE" => palette = data.chunks(3).filter(|c| c.len() == 3).map(|c| [c[0], c[1], c[2], 255]).collect(),
            b"tRNS" if color == 3 => {
                for (i, a) in data.iter().enumerate() {
                    if let Some(p) = palette.get_mut(i) {
                        p[3] = *a;
                    }
                }
            }
            b"IEND" => break,
            _ => {}
        }
        pos += 12 + len;
    }
    let stride = w as usize * channels;
    let want = (stride + 1) * h as usize;
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, want).map_err(|_| "broken PNG data")?;
    if raw.len() != want {
        return Err("PNG data has the wrong size".into());
    }
    let mut rows = vec![0u8; stride * h as usize];
    let mut prev = vec![0u8; stride];
    for y in 0..h as usize {
        let filter = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        let mut cur = vec![0u8; stride];
        for x in 0..stride {
            let a = if x >= channels { cur[x - channels] as i32 } else { 0 };
            let b = prev[x] as i32;
            let c = if x >= channels { prev[x - channels] as i32 } else { 0 };
            let pred = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => return Err(format!("unknown PNG filter {filter}")),
            };
            cur[x] = (src[x] as i32 + pred) as u8;
        }
        rows[y * stride..(y + 1) * stride].copy_from_slice(&cur);
        prev = cur;
    }
    let mut px = Vec::with_capacity(w as usize * h as usize * 4);
    for p in rows.chunks(channels) {
        match color {
            0 => px.extend_from_slice(&[p[0], p[0], p[0], 255]),
            2 => px.extend_from_slice(&[p[0], p[1], p[2], 255]),
            3 => px.extend_from_slice(palette.get(p[0] as usize).ok_or("palette index out of range")?),
            4 => px.extend_from_slice(&[p[0], p[0], p[0], p[1]]),
            _ => px.extend_from_slice(p),
        }
    }
    Ok(Rgba { w, h, px })
}

/// An image resource on the UI connection.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Img {
    pub resource: u32,
    pub w: i32,
    pub h: i32,
}

/// The image resources of one connection, by a caller's key (`icon:<id>`,
/// `shot:<id>:<n>`). Pictures that don't decode are remembered as missing.
#[derive(Default)]
pub struct Images {
    loaded: HashMap<String, Option<Img>>,
    next: u32,
}

impl Images {
    pub fn new() -> Images {
        Images { loaded: HashMap::new(), next: 100 }
    }

    pub fn get(&self, key: &str) -> Option<Img> {
        self.loaded.get(key).copied().flatten()
    }

    /// Tried already (loaded or failed)?
    pub fn known(&self, key: &str) -> bool {
        self.loaded.contains_key(key)
    }

    pub fn forget_missing(&mut self, key: &str) {
        if self.loaded.get(key) == Some(&None) {
            self.loaded.remove(key);
        }
    }

    /// Decode `png` and upload it as a new resource (once per key).
    pub fn load(&mut self, core: &mut Core, key: &str, png: &[u8]) -> Option<Img> {
        if let Some(i) = self.loaded.get(key) {
            return *i;
        }
        let img = decode(png).ok().and_then(|r| {
            self.next += 1;
            let id = self.next;
            core.send(Message::ResourceCreate(proto::ResourceCreate {
                id,
                kind: proto::ResourceKind::Image,
                w: r.w as i32,
                h: r.h as i32,
                format: proto::PixelFormat::Rgba8888,
            }));
            core.flush().ok()?;
            core.connection().upload(id, 1, &r.px).ok()?;
            Some(Img { resource: id, w: r.w as i32, h: r.h as i32 })
        });
        self.loaded.insert(key.to_string(), img);
        img
    }

    /// Remember a key as having no picture.
    pub fn missing(&mut self, key: &str) {
        self.loaded.entry(key.to_string()).or_insert(None);
    }
}

/// `w x h` scaled to fit inside `max_w x max_h`, keeping the aspect (never enlarged).
pub fn fit(w: i32, h: i32, max_w: i32, max_h: i32) -> (i32, i32) {
    if w <= 0 || h <= 0 {
        return (0, 0);
    }
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    let s = (max_w as f32 / w as f32).min(max_h as f32 / h as f32);
    (((w as f32 * s) as i32).max(1), ((h as f32 * s) as i32).max(1))
}

/// Encode RGBA as a PNG (stored deflate blocks; for generated pictures and tests).
pub fn encode(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut raw = Vec::new();
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_refusals() {
        let mut px = Vec::new();
        for y in 0..5u8 {
            for x in 0..7u8 {
                px.extend_from_slice(&[x * 30, y * 50, x ^ y, 200]);
            }
        }
        let png = encode(7, 5, &px);
        assert_eq!(size(&png), Some((7, 5)));
        assert_eq!(decode(&png).unwrap(), Rgba { w: 7, h: 5, px: px.clone() });
        assert!(decode(b"not a png at all, not at all").is_err());
        let mut broken = png.clone();
        let n = broken.len();
        broken[n - 20] ^= 0xff;
        assert!(decode(&broken).is_err() || decode(&broken).unwrap().px != px);
        let big = encode(600, 1, &vec![0; 600 * 4]);
        assert!(decode(&big).unwrap_err().contains("too large"));
        assert_eq!(fit(320, 200, 160, 160), (160, 100));
        assert_eq!(fit(32, 32, 64, 64), (32, 32));
    }

    #[test]
    fn decodes_the_checked_in_assets() {
        for (name, png) in crate::assets::ALL {
            let r = decode(png).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(r.w >= 16 && r.h >= 16, "{name}");
        }
    }
}
