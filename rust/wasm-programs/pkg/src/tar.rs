//! `.ecmpkg` archives: uncompressed POSIX ustar (docs/gui/APPS.md, "Package format").
//!
//! Only what packages need: regular files (typeflag `0` or NUL) and directories
//! (`5`), names up to 100 bytes plus the 155-byte ustar prefix, sizes in octal.
//! The first member is `app.toml`. GNU/BSD tar can list and extract packages.

use crate::manifest::safe_rel_path;

pub const BLOCK: usize = 512;
/// Upper bounds a reader enforces (a package is small; this stops archive bombs).
pub const MAX_ENTRIES: usize = 512;
pub const MAX_TOTAL: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub path: String,
    pub data: Vec<u8>,
    pub is_dir: bool,
}

fn octal(field: &mut [u8], v: u64) {
    // width-1 octal digits, then NUL.
    let w = field.len() - 1;
    let s = format!("{:0w$o}", v, w = w);
    field[..w].copy_from_slice(&s.as_bytes()[s.len() - w..]);
    field[w] = 0;
}

fn header(path: &str, size: usize, is_dir: bool) -> Result<[u8; BLOCK], String> {
    let mut h = [0u8; BLOCK];
    let name = if is_dir && !path.ends_with('/') { format!("{}/", path) } else { path.to_string() };
    let nb = name.as_bytes();
    if nb.len() <= 100 {
        h[..nb.len()].copy_from_slice(nb);
    } else {
        // Split at a '/' so the name part fits in 100 and the prefix in 155.
        let split = (0..nb.len())
            .rev()
            .find(|&i| nb[i] == b'/' && i <= 155 && nb.len() - i - 1 <= 100 && i > 0)
            .ok_or_else(|| format!("path too long for ustar: {}", path))?;
        h[..nb.len() - split - 1].copy_from_slice(&nb[split + 1..]);
        h[345..345 + split].copy_from_slice(&nb[..split]);
    }
    octal(&mut h[100..108], if is_dir { 0o755 } else { 0o644 });
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size as u64);
    octal(&mut h[136..148], 0);
    h[156] = if is_dir { b'5' } else { b'0' };
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    // Checksum: computed with the field as spaces.
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| b as u32).sum();
    let s = format!("{:06o}\0 ", sum);
    h[148..156].copy_from_slice(s.as_bytes());
    Ok(h)
}

/// Build an archive. Files are written in the given order (put `app.toml` first).
pub fn write(entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for e in entries {
        if !safe_rel_path(e.path.trim_end_matches('/')) {
            return Err(format!("unsafe path in package: {}", e.path));
        }
        let size = if e.is_dir { 0 } else { e.data.len() };
        out.extend_from_slice(&header(&e.path, size, e.is_dir)?);
        if !e.is_dir {
            out.extend_from_slice(&e.data);
            let pad = (BLOCK - e.data.len() % BLOCK) % BLOCK;
            out.extend(std::iter::repeat(0u8).take(pad));
        }
    }
    out.extend(std::iter::repeat(0u8).take(BLOCK * 2));
    Ok(out)
}

fn parse_octal(f: &[u8]) -> Result<u64, String> {
    let s: String = f.iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
    let s = s.trim();
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|_| format!("bad octal field '{}'", s))
}

fn cstr(f: &[u8]) -> String {
    String::from_utf8_lossy(&f[..f.iter().position(|&b| b == 0).unwrap_or(f.len())]).into_owned()
}

/// Read an archive, rejecting unsafe paths, links/devices, bad checksums and
/// archives over the limits.
pub fn read(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut total = 0usize;
    loop {
        if pos + BLOCK > bytes.len() {
            return Err("truncated archive (no end marker)".into());
        }
        let h = &bytes[pos..pos + BLOCK];
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let stored = parse_octal(&h[148..156])? as u32;
        let sum: u32 = h.iter().enumerate().map(|(i, &b)| if (148..156).contains(&i) { 32 } else { b as u32 }).sum();
        if stored != sum {
            return Err(format!("bad header checksum at offset {}", pos));
        }
        let mut name = cstr(&h[0..100]);
        if &h[257..262] == b"ustar" {
            let prefix = cstr(&h[345..500]);
            if !prefix.is_empty() {
                name = format!("{}/{}", prefix, name);
            }
        }
        let size = parse_octal(&h[124..136])? as usize;
        let flag = h[156];
        pos += BLOCK;
        let is_dir = match flag {
            b'0' | 0 => false,
            b'5' => true,
            _ => return Err(format!("{}: only files and directories are allowed (type '{}')", name, flag as char)),
        };
        let clean = name.trim_end_matches('/').to_string();
        if !safe_rel_path(&clean) {
            return Err(format!("unsafe path in package: {}", name));
        }
        if pos + size > bytes.len() {
            return Err(format!("{}: truncated data", clean));
        }
        total += size;
        if total > MAX_TOTAL {
            return Err(format!("package unpacks to more than {} bytes", MAX_TOTAL));
        }
        let data = if is_dir { Vec::new() } else { bytes[pos..pos + size].to_vec() };
        pos += (size + BLOCK - 1) / BLOCK * BLOCK;
        if out.iter().any(|e: &Entry| e.path == clean) {
            return Err(format!("{} appears twice", clean));
        }
        out.push(Entry { path: clean, data, is_dir });
        if out.len() > MAX_ENTRIES {
            return Err(format!("more than {} entries", MAX_ENTRIES));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(p: &str, d: &[u8]) -> Entry {
        Entry { path: p.into(), data: d.to_vec(), is_dir: false }
    }

    #[test]
    fn round_trip_with_dirs_and_long_names() {
        let long = format!("assets/{}/{}.png", "d".repeat(80), "n".repeat(60));
        let entries = vec![
            f("app.toml", b"id = \"a.b\"\n"),
            Entry { path: "assets".into(), data: vec![], is_dir: true },
            f("main.wasm", &[0u8, 97, 115, 109, 1, 0, 0, 0]),
            f(&long, &vec![7u8; 1500]),
            f("empty.txt", b""),
        ];
        let bytes = write(&entries).unwrap();
        assert_eq!(bytes.len() % BLOCK, 0);
        assert_eq!(read(&bytes).unwrap(), entries);
    }

    #[test]
    fn rejects_unsafe_and_corrupt_archives() {
        assert!(write(&[f("../x", b"")]).is_err());
        let mut bytes = write(&[f("app.toml", b"x")]).unwrap();
        // Absolute path written by hand (fix the checksum so only the path is wrong).
        let mut evil = header("etc/passwd", 1, false).unwrap();
        evil[0] = b'/';
        evil[1..11].copy_from_slice(b"etc/passwd");
        evil[148..156].copy_from_slice(b"        ");
        let sum: u32 = evil.iter().map(|&b| b as u32).sum();
        evil[148..156].copy_from_slice(format!("{:06o}\0 ", sum).as_bytes());
        let mut a = evil.to_vec();
        a.extend_from_slice(&[b'x'; 512]);
        a.extend_from_slice(&[0u8; 1024]);
        assert!(read(&a).unwrap_err().contains("unsafe"));
        bytes[0] ^= 1; // checksum now wrong
        assert!(read(&bytes).unwrap_err().contains("checksum"));
        assert!(read(&[0u8; 100]).is_err());
    }

    #[test]
    fn rejects_links() {
        let mut h = header("link", 0, false).unwrap();
        h[156] = b'2';
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|&b| b as u32).sum();
        h[148..156].copy_from_slice(format!("{:06o}\0 ", sum).as_bytes());
        let mut a = h.to_vec();
        a.extend_from_slice(&[0u8; 1024]);
        assert!(read(&a).unwrap_err().contains("only files"));
    }
}
