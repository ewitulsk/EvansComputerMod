//! Cover art: the ID3 picture (usually a JPEG, sometimes a PNG) to the
//! 64x64 and 160x160 PNGs `musicd` serves at `/art/<id>/<size>.png`. The
//! apps only decode PNG, so the JPEG decoding happens once, here, at scan
//! time.
//!
//! The picture is cropped to the centred square and scaled with an area
//! average (each output pixel is the mean of the source pixels it covers),
//! which keeps detail without the shimmer of nearest-neighbour; alpha is
//! premultiplied while averaging so transparent edges don't go dark.
//!
//! The JPEG decoder is `jpeg-decoder` (pure Rust, no default features).

/// The thumbnail sizes `musicd` makes.
pub const THUMB_SIZES: [u32; 2] = [64, 160];
/// Largest picture decoded (pixels); bigger covers are refused.
pub const MAX_PIXELS: u64 = 16_000_000;

/// RGBA8888 rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Decode a JPEG or PNG (by its magic number).
pub fn decode(bytes: &[u8]) -> Result<Image, String> {
    if bytes.starts_with(&[0xff, 0xd8]) {
        decode_jpeg(bytes)
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        decode_png(bytes)
    } else {
        Err("not a JPEG or PNG picture".into())
    }
}

pub fn decode_jpeg(bytes: &[u8]) -> Result<Image, String> {
    use jpeg_decoder::{Decoder, PixelFormat};
    let mut d = Decoder::new(bytes);
    d.read_info().map_err(|e| format!("JPEG: {e}"))?;
    let info = d.info().ok_or("JPEG: no header")?;
    let (w, h) = (info.width as u32, info.height as u32);
    if w == 0 || h == 0 || w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!("JPEG: {w}x{h} is too large"));
    }
    let px = d.decode().map_err(|e| format!("JPEG: {e}"))?;
    let n = (w * h) as usize;
    let mut rgba = Vec::with_capacity(n * 4);
    match info.pixel_format {
        PixelFormat::L8 => px.iter().take(n).for_each(|&l| rgba.extend_from_slice(&[l, l, l, 255])),
        PixelFormat::L16 => px.chunks_exact(2).take(n).for_each(|c| rgba.extend_from_slice(&[c[0], c[0], c[0], 255])),
        PixelFormat::RGB24 => px.chunks_exact(3).take(n).for_each(|c| rgba.extend_from_slice(&[c[0], c[1], c[2], 255])),
        PixelFormat::CMYK32 => px.chunks_exact(4).take(n).for_each(|c| {
            let k = 255 - c[3] as u32;
            let ch = |v: u8| ((255 - v as u32) * k / 255) as u8;
            rgba.extend_from_slice(&[ch(c[0]), ch(c[1]), ch(c[2]), 255]);
        }),
    }
    if rgba.len() != n * 4 {
        return Err("JPEG: short pixel data".into());
    }
    Ok(Image { w, h, rgba })
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Decode a non-interlaced PNG of any colour type, 1 to 16 bits.
pub fn decode_png(png: &[u8]) -> Result<Image, String> {
    if png.len() < 33 || &png[12..16] != b"IHDR" {
        return Err("PNG: no header".into());
    }
    let (w, h) = (be32(&png[16..20]), be32(&png[20..24]));
    let (depth, color, interlace) = (png[24] as usize, png[25], png[28]);
    if w == 0 || h == 0 || w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!("PNG: {w}x{h} is too large"));
    }
    if interlace != 0 {
        return Err("PNG: interlaced pictures are not supported".into());
    }
    let channels = match (color, depth) {
        (0, 1 | 2 | 4 | 8 | 16) | (3, 1 | 2 | 4 | 8) => 1,
        (2, 8 | 16) => 3,
        (4, 8 | 16) => 2,
        (6, 8 | 16) => 4,
        _ => return Err(format!("PNG: unsupported colour type {color} / depth {depth}")),
    };
    let mut pos = 8;
    let mut idat = Vec::new();
    let mut palette: Vec<[u8; 4]> = Vec::new();
    while pos + 8 <= png.len() {
        let len = be32(&png[pos..]) as usize;
        let kind = &png[pos + 4..pos + 8];
        let data = png.get(pos + 8..pos + 8 + len).ok_or("PNG: chunk runs past the end")?;
        match kind {
            b"IDAT" => idat.extend_from_slice(data),
            b"PLTE" => palette = data.chunks_exact(3).map(|c| [c[0], c[1], c[2], 255]).collect(),
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
    let bits = channels * depth;
    let stride = (w as usize * bits).div_ceil(8);
    let bpp = bits.div_ceil(8).max(1);
    let want = (stride + 1) * h as usize;
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&idat, want).map_err(|_| "PNG: broken data")?;
    if raw.len() != want {
        return Err("PNG: data has the wrong size".into());
    }
    let mut prev = vec![0u8; stride];
    let mut cur = vec![0u8; stride];
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let filter = raw[y * (stride + 1)];
        let src = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= bpp { cur[x - bpp] as i32 } else { 0 };
            let b = prev[x] as i32;
            let c = if x >= bpp { prev[x - bpp] as i32 } else { 0 };
            let pred = match filter {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => paeth(a, b, c),
                _ => return Err(format!("PNG: unknown filter {filter}")),
            };
            cur[x] = (src[x] as i32 + pred) as u8;
        }
        for x in 0..w as usize {
            let sample = |ch: usize| -> u8 {
                match depth {
                    8 => cur[x * channels + ch],
                    16 => cur[(x * channels + ch) * 2],
                    d => {
                        let bit = (x * channels + ch) * d;
                        let v = (cur[bit / 8] >> (8 - d - bit % 8)) & ((1 << d) - 1) as u8;
                        if color == 3 {
                            v
                        } else {
                            (v as u32 * 255 / ((1u32 << d) - 1)) as u8
                        }
                    }
                }
            };
            match color {
                0 => {
                    let l = sample(0);
                    rgba.extend_from_slice(&[l, l, l, 255]);
                }
                2 => rgba.extend_from_slice(&[sample(0), sample(1), sample(2), 255]),
                3 => rgba.extend_from_slice(palette.get(sample(0) as usize).ok_or("PNG: palette index out of range")?),
                4 => {
                    let l = sample(0);
                    rgba.extend_from_slice(&[l, l, l, sample(1)]);
                }
                _ => rgba.extend_from_slice(&[sample(0), sample(1), sample(2), sample(3)]),
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(Image { w, h, rgba })
}

fn paeth(a: i32, b: i32, c: i32) -> i32 {
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

/// For each of `dst` outputs: the source indices and weights (summing to 1).
/// An area average when shrinking, linear interpolation when enlarging.
fn weights(src: usize, dst: usize) -> Vec<Vec<(usize, f32)>> {
    let scale = src as f64 / dst as f64;
    (0..dst)
        .map(|o| {
            if src >= dst {
                let (a, b) = (o as f64 * scale, (o + 1) as f64 * scale);
                let mut v = Vec::new();
                let mut i = a.floor() as usize;
                while (i as f64) < b && i < src {
                    let cover = (b.min(i as f64 + 1.0) - a.max(i as f64)) as f32;
                    if cover > 0.0 {
                        v.push((i, cover / scale as f32));
                    }
                    i += 1;
                }
                v
            } else {
                let x = ((o as f64 + 0.5) * scale - 0.5).clamp(0.0, (src - 1) as f64);
                let i = x.floor() as usize;
                let f = (x - i as f64) as f32;
                if i + 1 < src {
                    vec![(i, 1.0 - f), (i + 1, f)]
                } else {
                    vec![(i, 1.0)]
                }
            }
        })
        .collect()
}

/// Crop the centred square and scale it to `side` x `side`.
pub fn square(img: &Image, side: u32) -> Image {
    let s = img.w.min(img.h) as usize;
    let (x0, y0) = (((img.w as usize) - s) / 2, ((img.h as usize) - s) / 2);
    let n = side.max(1) as usize;
    let wx = weights(s, n);
    // Premultiplied f32 RGBA of the crop, scaled across first.
    let mut rows = vec![0f32; s * n * 4];
    for y in 0..s {
        let src = &img.rgba[((y0 + y) * img.w as usize + x0) * 4..];
        for (o, ws) in wx.iter().enumerate() {
            let mut acc = [0f32; 4];
            for &(i, w) in ws {
                let p = &src[i * 4..i * 4 + 4];
                let a = p[3] as f32 / 255.0;
                acc[0] += w * p[0] as f32 * a;
                acc[1] += w * p[1] as f32 * a;
                acc[2] += w * p[2] as f32 * a;
                acc[3] += w * a;
            }
            rows[(y * n + o) * 4..(y * n + o) * 4 + 4].copy_from_slice(&acc);
        }
    }
    let mut out = vec![0u8; n * n * 4];
    for (o, ws) in wx.iter().enumerate() {
        for x in 0..n {
            let mut acc = [0f32; 4];
            for &(i, w) in ws {
                let p = &rows[(i * n + x) * 4..(i * n + x) * 4 + 4];
                for c in 0..4 {
                    acc[c] += w * p[c];
                }
            }
            let a = acc[3];
            let px = &mut out[(o * n + x) * 4..(o * n + x) * 4 + 4];
            for c in 0..3 {
                px[c] = if a > 0.0 { (acc[c] / a).round().clamp(0.0, 255.0) as u8 } else { 0 };
            }
            px[3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Image { w: side.max(1), h: side.max(1), rgba: out }
}

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

/// Encode as PNG: RGB when every pixel is opaque, else RGBA; each row gets
/// the filter that makes its bytes smallest, then zlib.
pub fn encode_png(img: &Image) -> Vec<u8> {
    let opaque = img.rgba.chunks_exact(4).all(|p| p[3] == 255);
    let ch = if opaque { 3 } else { 4 };
    let stride = img.w as usize * ch;
    let mut prev = vec![0u8; stride];
    let mut raw = Vec::with_capacity((stride + 1) * img.h as usize);
    let mut cand = vec![0u8; stride];
    let mut best = vec![0u8; stride];
    for row in img.rgba.chunks_exact(img.w as usize * 4) {
        let cur: Vec<u8> = if opaque { row.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect() } else { row.to_vec() };
        let mut best_f = 0u8;
        let mut best_cost = u64::MAX;
        for f in 0..5u8 {
            for x in 0..stride {
                let a = if x >= ch { cur[x - ch] as i32 } else { 0 };
                let b = prev[x] as i32;
                let c = if x >= ch { prev[x - ch] as i32 } else { 0 };
                let pred = match f {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => (a + b) / 2,
                    _ => paeth(a, b, c),
                };
                cand[x] = (cur[x] as i32 - pred) as u8;
            }
            let cost: u64 = cand.iter().map(|&v| (v as i8).unsigned_abs() as u64).sum();
            if cost < best_cost {
                best_cost = cost;
                best_f = f;
                best.copy_from_slice(&cand);
            }
        }
        raw.push(best_f);
        raw.extend_from_slice(&best);
        prev = cur;
    }
    let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 8);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&img.w.to_be_bytes());
    ihdr.extend_from_slice(&img.h.to_be_bytes());
    ihdr.extend_from_slice(&[8, if opaque { 2 } else { 6 }, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

/// A `side` x `side` PNG of a cover picture.
pub fn thumbnail(picture: &[u8], side: u32) -> Result<Vec<u8>, String> {
    Ok(encode_png(&square(&decode(picture)?, side)))
}

/// The 64x64 and 160x160 PNGs of a cover picture (decoded once).
pub fn thumbnails(picture: &[u8]) -> Result<[Vec<u8>; 2], String> {
    let img = decode(picture)?;
    Ok(THUMB_SIZES.map(|s| encode_png(&square(&img, s))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Image {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let on = (x / 4 + y / 4) % 2 == 0;
                rgba.extend_from_slice(&if on { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
            }
        }
        Image { w, h, rgba }
    }

    #[test]
    fn png_round_trip_rgb_and_rgba() {
        let img = checker(13, 7);
        let png = encode_png(&img);
        assert_eq!(png[25], 2, "opaque pictures are stored as RGB");
        assert_eq!(decode(&png).unwrap(), img);
        let mut t = img.clone();
        t.rgba[3] = 10;
        let png = encode_png(&t);
        assert_eq!(png[25], 6);
        assert_eq!(decode(&png).unwrap(), t);
        assert!(decode(b"nope").is_err());
        assert!(decode(&png[..40]).is_err());
    }

    #[test]
    fn square_crops_the_centre_and_averages() {
        // 40x20: the centre 20x20 is x 10..30. Left part red, right part blue.
        let mut img = Image { w: 40, h: 20, rgba: Vec::new() };
        for _y in 0..20 {
            for x in 0..40 {
                img.rgba.extend_from_slice(if x < 20 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
            }
        }
        let s = square(&img, 2);
        assert_eq!((s.w, s.h), (2, 2));
        assert_eq!(&s.rgba[..4], &[255, 0, 0, 255], "left half of the crop is red");
        assert_eq!(&s.rgba[4..8], &[0, 0, 255, 255]);
        // A fine checkerboard averages to purple.
        let c = square(&checker(64, 64), 1);
        assert!((c.rgba[0] as i32 - 128).abs() <= 1 && (c.rgba[2] as i32 - 128).abs() <= 1, "{:?}", c.rgba);
        // Enlarging works too.
        let big = square(&checker(8, 8), 64);
        assert_eq!(big.rgba.len(), 64 * 64 * 4);
        // Transparent pixels don't darken their neighbours.
        let t = Image { w: 2, h: 1, rgba: vec![255, 255, 255, 255, 0, 0, 0, 0] };
        let m = square(&Image { w: 2, h: 2, rgba: [t.rgba.clone(), t.rgba].concat() }, 1);
        assert_eq!(&m.rgba, &[255, 255, 255, 128]);
    }

    #[test]
    fn cover_from_the_fixture_jpeg() {
        let file = std::fs::read(format!("{}/tests/data/tone_cbr.mp3", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let pic = crate::id3::read(&file).picture.unwrap();
        let img = decode(&pic.data).unwrap();
        assert_eq!((img.w, img.h), (48, 32));
        let [small, large] = thumbnails(&pic.data).unwrap();
        let s = decode(&small).unwrap();
        let l = decode(&large).unwrap();
        assert_eq!((s.w, s.h, l.w, l.h), (64, 64, 160, 160));
        // The ffmpeg test pattern is colourful, not one flat colour.
        let distinct: std::collections::HashSet<&[u8]> = l.rgba.chunks(4).collect();
        assert!(distinct.len() > 20);
        for cut in (0..pic.data.len()).step_by(37) {
            let _ = decode(&pic.data[..cut]);
        }
    }
}
