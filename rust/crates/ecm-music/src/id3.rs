//! ID3 tags: v2.2, v2.3 and v2.4 at the start of an MP3, and ID3v1 in its
//! last 128 bytes as a fallback for fields v2 leaves empty.
//!
//! `musicd scan` runs this over every file the server owner drops into the
//! library, so it must cope with whatever taggers wrote: unsynchronisation
//! (tag-wide in v2.2/2.3, per frame in v2.4), extended headers, Latin-1,
//! UTF-16 with a byte order mark, UTF-8, v2.4 frame sizes written as plain
//! integers by old iTunes, and plain garbage. Nothing here panics on bad
//! input; a broken frame is skipped and a broken tag gives what was readable.

use std::borrow::Cow;

/// An attached picture (APIC in v2.3/2.4, PIC in v2.2).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picture {
    /// `image/jpeg`, `image/png`, ... (sniffed from the bytes when the tag
    /// leaves it out).
    pub mime: String,
    /// The ID3 picture type: 3 is the front cover, 0 "other".
    pub kind: u8,
    pub description: String,
    pub data: Vec<u8>,
}

/// The ID3 picture type of a front cover.
pub const FRONT_COVER: u8 = 3;

/// What the tags say about a song. Empty text fields are `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub year: Option<u32>,
    /// Genre name: numeric `(17)` / `17` genres are mapped to their names.
    pub genre: Option<String>,
    /// The front cover if there is one, else the first picture.
    pub picture: Option<Picture>,
    /// Major version of the ID3v2 tag that was read (2, 3 or 4), 0 if none.
    pub v2: u8,
    /// Whether an ID3v1 tag was found.
    pub v1: bool,
}

/// Read every tag in `file`: ID3v2 first, then ID3v1 for any field v2 left
/// empty (or for everything when there is no v2 tag).
pub fn read(file: &[u8]) -> Tags {
    let mut t = read_v2(file).unwrap_or_default();
    if let Some(v1) = read_v1(file) {
        t.v1 = true;
        fill(&mut t.title, v1.title);
        fill(&mut t.artist, v1.artist);
        fill(&mut t.album, v1.album);
        fill(&mut t.year, v1.year);
        fill(&mut t.track, v1.track);
        fill(&mut t.genre, v1.genre);
    }
    t
}

fn fill<T>(slot: &mut Option<T>, v: Option<T>) {
    if slot.is_none() {
        *slot = v;
    }
}

fn syncsafe(b: &[u8]) -> u32 {
    ((b[0] as u32 & 0x7f) << 21) | ((b[1] as u32 & 0x7f) << 14) | ((b[2] as u32 & 0x7f) << 7) | (b[3] as u32 & 0x7f)
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Total bytes of the ID3v2 tag at the start of `file` (header, body and a
/// v2.4 footer), 0 when the file does not start with one. MP3 frames start
/// right after it.
pub fn v2_size(file: &[u8]) -> usize {
    if file.len() < 10 || &file[..3] != b"ID3" || !(2..=4).contains(&file[3]) || file[4] == 0xff {
        return 0;
    }
    if file[6..10].iter().any(|b| b & 0x80 != 0) {
        return 0;
    }
    let footer = if file[3] == 4 && file[5] & 0x10 != 0 { 10 } else { 0 };
    10 + syncsafe(&file[6..10]) as usize + footer
}

/// Undo ID3 unsynchronisation: drop every 0x00 that follows an 0xFF.
pub fn unsynchronise(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut prev_ff = false;
    for &b in data {
        if prev_ff && b == 0 {
            prev_ff = false;
            continue;
        }
        out.push(b);
        prev_ff = b == 0xff;
    }
    out
}

fn frame_id_ok(id: &[u8]) -> bool {
    id.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// Read the ID3v2 tag at the start of `file`, if there is one.
pub fn read_v2(file: &[u8]) -> Option<Tags> {
    if v2_size(file) == 0 {
        return None;
    }
    let major = file[3];
    let flags = file[5];
    let body_len = syncsafe(&file[6..10]) as usize;
    let end = 10usize.saturating_add(body_len).min(file.len());
    let mut tags = Tags { v2: major, ..Tags::default() };
    if major == 2 && flags & 0x40 != 0 {
        // v2.2 "compression" was never defined: nothing readable.
        return Some(tags);
    }
    let mut body: Cow<[u8]> = Cow::Borrowed(&file[10..end]);
    if major < 4 && flags & 0x80 != 0 {
        body = Cow::Owned(unsynchronise(&body));
    }
    let body = &body[..];
    let mut pos = 0usize;
    if major >= 3 && flags & 0x40 != 0 && body.len() >= 4 {
        pos = if major == 3 { 4usize.saturating_add(be32(&body[..4]) as usize) } else { syncsafe(&body[..4]) as usize };
    }
    let tag_unsync = major == 4 && flags & 0x80 != 0;
    let (id_len, head_len) = if major == 2 { (3, 6) } else { (4, 10) };
    while pos.saturating_add(head_len) <= body.len() {
        let id = &body[pos..pos + id_len];
        if id[0] == 0 || !frame_id_ok(id) {
            break; // padding, or junk where the next frame should be
        }
        let sz = &body[pos + id_len..pos + head_len];
        let size = match major {
            2 => ((sz[0] as usize) << 16) | ((sz[1] as usize) << 8) | sz[2] as usize,
            3 => be32(sz) as usize,
            _ => v24_frame_size(body, pos, sz),
        };
        let fmt = if major >= 3 { body[pos + 9] } else { 0 };
        let start = pos + head_len;
        let stop = start.saturating_add(size).min(body.len());
        pos = start.saturating_add(size);
        let mut data: Cow<[u8]> = Cow::Borrowed(&body[start..stop]);
        let skip = match major {
            3 => {
                if fmt & 0xc0 != 0 {
                    None // compressed or encrypted
                } else if fmt & 0x20 != 0 {
                    Some(1) // grouping identity
                } else {
                    Some(0)
                }
            }
            4 => {
                if fmt & 0x0c != 0 {
                    None
                } else {
                    Some(if fmt & 0x40 != 0 { 1 } else { 0 } + if fmt & 0x01 != 0 { 4 } else { 0 })
                }
            }
            _ => Some(0),
        };
        let Some(skip) = skip else { continue };
        if skip > data.len() {
            continue;
        }
        if skip > 0 {
            data = Cow::Owned(data[skip..].to_vec());
        }
        if major == 4 && (tag_unsync || fmt & 0x02 != 0) {
            data = Cow::Owned(unsynchronise(&data));
        }
        frame(&mut tags, id, &data);
    }
    Some(tags)
}

/// v2.4 frame sizes are syncsafe, but some writers stored plain integers.
/// Take the plain reading when the syncsafe one can't be right (a byte with
/// the top bit set) or when only the plain reading lands on another frame.
fn v24_frame_size(body: &[u8], pos: usize, sz: &[u8]) -> usize {
    let plain = be32(sz) as usize;
    if sz.iter().any(|b| b & 0x80 != 0) {
        return plain;
    }
    let safe = syncsafe(sz) as usize;
    if safe == plain {
        return safe;
    }
    let lands = |size: usize| {
        let p = pos.saturating_add(10).saturating_add(size);
        p == body.len() || (p < body.len() && body[p] == 0) || (p.saturating_add(4) <= body.len() && frame_id_ok(&body[p..p + 4]))
    };
    if !lands(safe) && lands(plain) {
        plain
    } else {
        safe
    }
}

fn frame(tags: &mut Tags, id: &[u8], data: &[u8]) {
    match id {
        b"TIT2" | b"TT2" => tags.title = text_value(data),
        b"TPE1" | b"TP1" => tags.artist = text_value(data),
        b"TALB" | b"TAL" => tags.album = text_value(data),
        b"TPE2" | b"TP2" => tags.album_artist = text_value(data),
        b"TRCK" | b"TRK" => {
            if let Some(s) = text_value(data) {
                let mut it = s.splitn(2, '/');
                tags.track = it.next().and_then(leading_number).filter(|n| *n > 0);
                tags.track_total = it.next().and_then(leading_number).filter(|n| *n > 0);
            }
        }
        b"TYER" | b"TYE" | b"TDRC" => {
            if let Some(y) = text_value(data).and_then(|s| year(&s)) {
                tags.year = Some(y);
            }
        }
        b"TORY" | b"TOR" | b"TDOR" => {
            if tags.year.is_none() {
                tags.year = text_value(data).and_then(|s| year(&s));
            }
        }
        b"TCON" | b"TCO" => {
            tags.genre = text_values(data).iter().map(|s| genre(s)).find(|g| !g.is_empty());
        }
        b"APIC" => picture(tags, data, false),
        b"PIC" => picture(tags, data, true),
        _ => {}
    }
}

fn leading_number(s: &str) -> Option<u32> {
    let d: String = s.trim().chars().take_while(|c| c.is_ascii_digit()).take(9).collect();
    d.parse().ok()
}

/// The year from `1997`, `1997-06-01`, `1997-06-01T10:00`.
fn year(s: &str) -> Option<u32> {
    let s = s.trim();
    let d: String = s.chars().take(4).collect();
    if d.len() == 4 && d.chars().all(|c| c.is_ascii_digit()) {
        d.parse().ok().filter(|y| *y > 0)
    } else {
        None
    }
}

/// Decode ID3 text in encoding `enc` (0 Latin-1, 1 UTF-16 with BOM,
/// 2 UTF-16BE, 3 UTF-8).
pub fn decode_text(enc: u8, b: &[u8]) -> String {
    match enc {
        1 | 2 => utf16(b, enc == 2),
        3 => String::from_utf8_lossy(b).into_owned(),
        _ => b.iter().map(|&c| c as char).collect(),
    }
}

fn utf16(b: &[u8], mut big: bool) -> String {
    let mut b = b;
    if b.len() >= 2 {
        if b[0] == 0xfe && b[1] == 0xff {
            big = true;
            b = &b[2..];
        } else if b[0] == 0xff && b[1] == 0xfe {
            big = false;
            b = &b[2..];
        }
    }
    let units = b.chunks_exact(2).map(|c| if big { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) });
    char::decode_utf16(units)
        .map(|r| r.unwrap_or('\u{fffd}'))
        // A BOM in front of a second string of a v2.4 multi-value frame.
        .filter(|c| *c != '\u{feff}')
        .collect()
}

/// All the null-separated values of a text frame.
fn text_values(data: &[u8]) -> Vec<String> {
    let Some((&enc, rest)) = data.split_first() else { return Vec::new() };
    decode_text(enc, rest).split('\0').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

fn text_value(data: &[u8]) -> Option<String> {
    text_values(data).into_iter().next()
}

/// Split `b` at the end of a null-terminated string in encoding `enc`: the
/// string bytes and what follows the terminator.
fn split_terminated(enc: u8, b: &[u8]) -> (&[u8], &[u8]) {
    if enc == 1 || enc == 2 {
        let mut i = 0;
        while i + 1 < b.len() {
            if b[i] == 0 && b[i + 1] == 0 {
                return (&b[..i], &b[i + 2..]);
            }
            i += 2;
        }
        (b, &[])
    } else {
        match b.iter().position(|&c| c == 0) {
            Some(i) => (&b[..i], &b[i + 1..]),
            None => (b, &[]),
        }
    }
}

/// The MIME type of image bytes from their magic number.
pub fn sniff_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xff, 0xd8]) {
        Some("image/jpeg")
    } else if data.starts_with(b"\x89PNG") {
        Some("image/png")
    } else if data.starts_with(b"GIF8") {
        Some("image/gif")
    } else {
        None
    }
}

fn picture(tags: &mut Tags, data: &[u8], v22: bool) {
    let Some((&enc, rest)) = data.split_first() else { return };
    let (mime, rest) = if v22 {
        if rest.len() < 3 {
            return;
        }
        let fmt = String::from_utf8_lossy(&rest[..3]).to_ascii_lowercase();
        let mime = match fmt.as_str() {
            "jpg" | "jpe" => "image/jpeg".to_string(),
            "png" => "image/png".to_string(),
            "-->" => return,
            other => format!("image/{other}"),
        };
        (mime, &rest[3..])
    } else {
        let (m, r) = split_terminated(0, rest);
        let m: String = m.iter().map(|&c| c as char).collect::<String>().trim().to_ascii_lowercase();
        if m == "-->" {
            return; // a link to a picture elsewhere
        }
        let m = match m.as_str() {
            "jpg" | "jpeg" | "image/jpg" => "image/jpeg".to_string(),
            "png" => "image/png".to_string(),
            _ => m,
        };
        (m, r)
    };
    let Some((&kind, rest)) = rest.split_first() else { return };
    let (desc, img) = split_terminated(enc, rest);
    if img.is_empty() {
        return;
    }
    let mime = match sniff_mime(img) {
        Some(m) if !mime.contains('/') || mime == "image/" => m.to_string(),
        _ if mime.is_empty() => "application/octet-stream".to_string(),
        _ => mime,
    };
    let pic = Picture { mime, kind, description: decode_text(enc, desc).trim_matches('\0').to_string(), data: img.to_vec() };
    let replace = match &tags.picture {
        None => true,
        Some(old) => old.kind != FRONT_COVER && kind == FRONT_COVER,
    };
    if replace {
        tags.picture = Some(pic);
    }
}

/// Read an ID3v1(.1) tag from the last 128 bytes of `file`.
pub fn read_v1(file: &[u8]) -> Option<Tags> {
    if file.len() < 128 {
        return None;
    }
    let t = &file[file.len() - 128..];
    if &t[..3] != b"TAG" {
        return None;
    }
    let field = |b: &[u8]| -> Option<String> {
        let s: String = b.iter().take_while(|&&c| c != 0).map(|&c| c as char).collect();
        let s = s.trim().to_string();
        (!s.is_empty()).then_some(s)
    };
    let track = (t[125] == 0 && t[126] != 0).then_some(t[126] as u32);
    Some(Tags {
        title: field(&t[3..33]),
        artist: field(&t[33..63]),
        album: field(&t[63..93]),
        year: field(&t[93..97]).and_then(|s| year(&s)),
        track,
        genre: genre_name(t[127]).map(str::to_string),
        v1: true,
        ..Tags::default()
    })
}

/// A genre field as a name: `(17)` and `17` become "Rock", `(17)Indie Rock`
/// keeps the refinement "Indie Rock", `(RX)`/`(CR)` are Remix/Cover, and
/// `((text` is an escaped leading parenthesis.
pub fn genre(s: &str) -> String {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("((") {
        return format!("({rest}");
    }
    if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() {
        return s.parse::<u8>().ok().and_then(genre_name).unwrap_or(s).to_string();
    }
    let mut rest = s;
    let mut last = None;
    while let Some(r) = rest.strip_prefix('(') {
        let Some(close) = r.find(')') else { break };
        let code = &r[..close];
        last = match code {
            "RX" => Some("Remix"),
            "CR" => Some("Cover"),
            n => n.parse::<u8>().ok().and_then(genre_name),
        }
        .or(last);
        rest = &r[close + 1..];
    }
    let rest = rest.trim();
    if !rest.is_empty() {
        rest.to_string()
    } else {
        last.map(str::to_string).unwrap_or_default()
    }
}

/// The ID3v1 genre list with the Winamp extensions (0..=191).
pub fn genre_name(n: u8) -> Option<&'static str> {
    GENRES.get(n as usize).copied()
}

const GENRES: [&str; 192] = [
    "Blues", "Classic Rock", "Country", "Dance", "Disco", "Funk", "Grunge", "Hip-Hop", "Jazz", "Metal", "New Age", "Oldies",
    "Other", "Pop", "R&B", "Rap", "Reggae", "Rock", "Techno", "Industrial", "Alternative", "Ska", "Death Metal", "Pranks",
    "Soundtrack", "Euro-Techno", "Ambient", "Trip-Hop", "Vocal", "Jazz+Funk", "Fusion", "Trance", "Classical", "Instrumental",
    "Acid", "House", "Game", "Sound Clip", "Gospel", "Noise", "Alternative Rock", "Bass", "Soul", "Punk", "Space",
    "Meditative", "Instrumental Pop", "Instrumental Rock", "Ethnic", "Gothic", "Darkwave", "Techno-Industrial", "Electronic",
    "Pop-Folk", "Eurodance", "Dream", "Southern Rock", "Comedy", "Cult", "Gangsta", "Top 40", "Christian Rap", "Pop/Funk",
    "Jungle", "Native American", "Cabaret", "New Wave", "Psychedelic", "Rave", "Showtunes", "Trailer", "Lo-Fi", "Tribal",
    "Acid Punk", "Acid Jazz", "Polka", "Retro", "Musical", "Rock & Roll", "Hard Rock", "Folk", "Folk-Rock", "National Folk",
    "Swing", "Fast Fusion", "Bebop", "Latin", "Revival", "Celtic", "Bluegrass", "Avantgarde", "Gothic Rock",
    "Progressive Rock", "Psychedelic Rock", "Symphonic Rock", "Slow Rock", "Big Band", "Chorus", "Easy Listening", "Acoustic",
    "Humour", "Speech", "Chanson", "Opera", "Chamber Music", "Sonata", "Symphony", "Booty Bass", "Primus", "Porn Groove",
    "Satire", "Slow Jam", "Club", "Tango", "Samba", "Folklore", "Ballad", "Power Ballad", "Rhythmic Soul", "Freestyle", "Duet",
    "Punk Rock", "Drum Solo", "A Cappella", "Euro-House", "Dance Hall", "Goa", "Drum & Bass", "Club-House",
    "Hardcore Techno", "Terror", "Indie", "BritPop", "Afro-Punk", "Polsk Punk", "Beat", "Christian Gangsta Rap",
    "Heavy Metal", "Black Metal", "Crossover", "Contemporary Christian", "Christian Rock", "Merengue", "Salsa",
    "Thrash Metal", "Anime", "JPop", "Synthpop", "Abstract", "Art Rock", "Baroque", "Bhangra", "Big Beat", "Breakbeat",
    "Chillout", "Downtempo", "Dub", "EBM", "Eclectic", "Electro", "Electroclash", "Emo", "Experimental", "Garage", "Global",
    "IDM", "Illbient", "Industro-Goth", "Jam Band", "Krautrock", "Leftfield", "Lounge", "Math Rock", "New Romantic",
    "Nu-Breakz", "Post-Punk", "Post-Rock", "Psytrance", "Shoegaze", "Space Rock", "Trop Rock", "World Music", "Neoclassical",
    "Audiobook", "Audio Theatre", "Neue Deutsche Welle", "Podcast", "Indie Rock", "G-Funk", "Dubstep", "Garage Rock",
    "Psybient",
];

#[cfg(test)]
pub(crate) mod build {
    //! Hand-built tags for the tests (here and in other modules).

    pub fn syncsafe(n: usize) -> [u8; 4] {
        [(n >> 21) as u8 & 0x7f, (n >> 14) as u8 & 0x7f, (n >> 7) as u8 & 0x7f, n as u8 & 0x7f]
    }

    /// A text frame body: encoding byte + encoded text.
    pub fn text(enc: u8, s: &str) -> Vec<u8> {
        let mut v = vec![enc];
        match enc {
            0 => v.extend(s.chars().map(|c| c as u8)),
            1 => {
                v.extend_from_slice(&[0xff, 0xfe]);
                for u in s.encode_utf16() {
                    v.extend_from_slice(&u.to_le_bytes());
                }
            }
            2 => {
                for u in s.encode_utf16() {
                    v.extend_from_slice(&u.to_be_bytes());
                }
            }
            _ => v.extend_from_slice(s.as_bytes()),
        }
        v
    }

    /// A frame for `major` (3 or 4: 10-byte header; 2: 6-byte header).
    pub fn frame(major: u8, id: &str, flags: [u8; 2], body: &[u8]) -> Vec<u8> {
        let mut v = id.as_bytes().to_vec();
        match major {
            2 => v.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]),
            3 => v.extend_from_slice(&(body.len() as u32).to_be_bytes()),
            _ => v.extend_from_slice(&syncsafe(body.len())),
        }
        if major >= 3 {
            v.extend_from_slice(&flags);
        }
        v.extend_from_slice(body);
        v
    }

    /// A whole tag around `frames` (already unsynchronised if `flags` says so).
    pub fn tag(major: u8, flags: u8, frames: &[u8], padding: usize) -> Vec<u8> {
        let mut v = b"ID3".to_vec();
        v.extend_from_slice(&[major, 0, flags]);
        v.extend_from_slice(&syncsafe(frames.len() + padding));
        v.extend_from_slice(frames);
        v.extend(std::iter::repeat(0).take(padding));
        v
    }

    pub fn apic(enc: u8, mime: &str, kind: u8, desc: &str, data: &[u8]) -> Vec<u8> {
        let mut v = vec![enc];
        v.extend_from_slice(mime.as_bytes());
        v.push(0);
        v.push(kind);
        v.extend_from_slice(&text(enc, desc)[1..]);
        v.extend_from_slice(if enc == 1 || enc == 2 { &[0, 0] } else { &[0] });
        v.extend_from_slice(data);
        v
    }

    pub fn unsync(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, &b) in data.iter().enumerate() {
            out.push(b);
            if b == 0xff && (data.get(i + 1).map_or(true, |n| *n == 0 || *n >= 0xe0)) {
                out.push(0);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::build::{apic, frame, syncsafe, tag, text, unsync};
    use super::*;

    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3, 0xff, 0x00, 0xff, 0xd9];

    fn v23() -> Vec<u8> {
        let mut f = Vec::new();
        f.extend(frame(3, "TIT2", [0, 0], &text(1, "Ünïcode Title")));
        f.extend(frame(3, "TPE1", [0, 0], &text(0, "Björk")));
        f.extend(frame(3, "TALB", [0, 0], &text(3, "Album ✓")));
        f.extend(frame(3, "TPE2", [0, 0], &text(2, "Various")));
        f.extend(frame(3, "TRCK", [0, 0], &text(0, "3/12")));
        f.extend(frame(3, "TYER", [0, 0], &text(0, "2001")));
        f.extend(frame(3, "TCON", [0, 0], &text(0, "(17)")));
        f.extend(frame(3, "APIC", [0, 0], &apic(0, "image/png", 0, "other", b"\x89PNGxx")));
        f.extend(frame(3, "APIC", [0, 0], &apic(1, "image/jpeg", 3, "Front", JPEG)));
        f.extend(frame(3, "APIC", [0, 0], &apic(0, "", 4, "back", JPEG)));
        f
    }

    #[test]
    fn reads_v23_fields_encodings_and_prefers_the_front_cover() {
        let t = read(&tag(3, 0, &v23(), 64));
        assert_eq!(t.v2, 3);
        assert_eq!(t.title.as_deref(), Some("Ünïcode Title"));
        assert_eq!(t.artist.as_deref(), Some("Björk"));
        assert_eq!(t.album.as_deref(), Some("Album ✓"));
        assert_eq!(t.album_artist.as_deref(), Some("Various"));
        assert_eq!((t.track, t.track_total), (Some(3), Some(12)));
        assert_eq!(t.year, Some(2001));
        assert_eq!(t.genre.as_deref(), Some("Rock"));
        let p = t.picture.unwrap();
        assert_eq!((p.mime.as_str(), p.kind, p.description.as_str()), ("image/jpeg", 3, "Front"));
        assert_eq!(p.data, JPEG);
    }

    #[test]
    fn tag_wide_unsynchronisation_and_extended_header_v23() {
        let mut body = vec![0, 0, 0, 6, 0, 0, 0, 0, 0, 0]; // extended header, 6 bytes after its size
        body.extend(v23());
        let t = read(&tag(3, 0xc0, &unsync(&body), 0));
        assert_eq!(t.title.as_deref(), Some("Ünïcode Title"));
        assert_eq!(t.picture.unwrap().data, JPEG, "the 0xFF 0x00 pairs inside the picture survive");
    }

    #[test]
    fn v24_syncsafe_sizes_per_frame_unsync_multi_values_and_extended_header() {
        let mut f = vec![0, 0, 0, 6, 1, 0]; // v2.4 extended header: size includes itself
        f.extend(frame(4, "TIT2", [0, 0], &text(3, "Ünïcode")));
        f.extend(frame(4, "TCON", [0, 0], &text(3, "\0Electronic\0Dance")));
        f.extend(frame(4, "TDRC", [0, 0], &text(0, "1997-06-01")));
        // A per-frame unsynchronised picture with a data length indicator.
        let pic = apic(0, "image/jpeg", 3, "", JPEG);
        let mut body = syncsafe(pic.len()).to_vec();
        body.extend(unsync(&pic));
        f.extend(frame(4, "APIC", [0, 0x03], &body));
        // Bigger than 127 bytes, so syncsafe and plain sizes differ.
        let long = "x".repeat(300);
        f.extend(frame(4, "TALB", [0, 0], &text(0, &long)));
        let t = read(&tag(4, 0x40, &f, 10));
        assert_eq!(t.title.as_deref(), Some("Ünïcode"));
        assert_eq!(t.genre.as_deref(), Some("Electronic"));
        assert_eq!(t.year, Some(1997));
        assert_eq!(t.picture.unwrap().data, JPEG);
        assert_eq!(t.album.as_deref(), Some(long.as_str()));
    }

    #[test]
    fn v24_with_plain_integer_frame_sizes() {
        let long = "y".repeat(200);
        let mut a = b"TALB".to_vec();
        let body = text(0, &long);
        a.extend_from_slice(&(body.len() as u32).to_be_bytes());
        a.extend_from_slice(&[0, 0]);
        a.extend(body);
        a.extend(frame(4, "TIT2", [0, 0], &text(0, "After")));
        let t = read(&tag(4, 0, &a, 0));
        assert_eq!(t.album.as_deref(), Some(long.as_str()));
        assert_eq!(t.title.as_deref(), Some("After"));
    }

    #[test]
    fn v22_frames_and_pic() {
        let mut f = Vec::new();
        f.extend(frame(2, "TT2", [0, 0], &text(0, "Old")));
        f.extend(frame(2, "TP1", [0, 0], &text(1, "Ärtist")));
        f.extend(frame(2, "TCO", [0, 0], &text(0, "(13)")));
        let mut pic = vec![0];
        pic.extend_from_slice(b"JPG");
        pic.push(3);
        pic.push(0);
        pic.extend_from_slice(JPEG);
        f.extend(frame(2, "PIC", [0, 0], &pic));
        let t = read(&tag(2, 0, &f, 0));
        assert_eq!(t.v2, 2);
        assert_eq!(t.title.as_deref(), Some("Old"));
        assert_eq!(t.artist.as_deref(), Some("Ärtist"));
        assert_eq!(t.genre.as_deref(), Some("Pop"));
        assert_eq!(t.picture.unwrap().mime, "image/jpeg");
    }

    #[test]
    fn v1_fallback_fills_gaps() {
        let mut v1 = vec![0u8; 128];
        v1[..3].copy_from_slice(b"TAG");
        v1[3..8].copy_from_slice(b"Title");
        v1[33..37].copy_from_slice(b"Band");
        v1[63..68].copy_from_slice(b"Album");
        v1[93..97].copy_from_slice(b"1985");
        v1[126] = 5;
        v1[127] = 17;
        let only_v1 = read(&v1);
        assert!(only_v1.v1 && only_v1.v2 == 0);
        assert_eq!(only_v1.title.as_deref(), Some("Title"));
        assert_eq!(only_v1.track, Some(5));
        assert_eq!(only_v1.year, Some(1985));
        assert_eq!(only_v1.genre.as_deref(), Some("Rock"));
        let mut file = tag(3, 0, &frame(3, "TIT2", [0, 0], &text(0, "V2 title")), 0);
        file.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        file.extend(v1);
        let both = read(&file);
        assert_eq!(both.title.as_deref(), Some("V2 title"));
        assert_eq!(both.artist.as_deref(), Some("Band"));
    }

    #[test]
    fn genres() {
        assert_eq!(genre("(17)"), "Rock");
        assert_eq!(genre("17"), "Rock");
        assert_eq!(genre("(17)Indie Rock"), "Indie Rock");
        assert_eq!(genre("(RX)"), "Remix");
        assert_eq!(genre("((Weird)"), "(Weird)");
        assert_eq!(genre("Synthwave"), "Synthwave");
        assert_eq!(genre("(255)"), "");
        assert_eq!(genre_name(191), Some("Psybient"));
        assert_eq!(genre_name(192), None);
    }

    /// A tiny xorshift so the junk is the same on every run.
    fn rng(seed: &mut u64) -> u64 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        *seed
    }

    #[test]
    fn never_panics_on_truncated_or_random_input() {
        let good = {
            let mut v = tag(3, 0, &v23(), 0);
            v.extend(tag(4, 0x80, &unsync(&frame(4, "TIT2", [0, 2], &text(1, "x"))), 3));
            v
        };
        for cut in 0..good.len() {
            let _ = read(&good[..cut]);
        }
        let mut seed = 0x1234_5678_9abc_def0u64;
        for round in 0..3000 {
            let len = (rng(&mut seed) % 300) as usize;
            let mut junk: Vec<u8> = (0..len).map(|_| rng(&mut seed) as u8).collect();
            if round % 2 == 0 && junk.len() >= 10 {
                junk[..3].copy_from_slice(b"ID3");
                junk[3] = 2 + (round % 3) as u8;
                for b in &mut junk[6..10] {
                    *b &= 0x7f;
                }
            }
            if round % 3 == 0 {
                // Flip bytes of a good tag.
                junk = good.clone();
                for _ in 0..8 {
                    let i = (rng(&mut seed) as usize) % junk.len();
                    junk[i] = rng(&mut seed) as u8;
                }
            }
            let _ = read(&junk);
        }
    }
}
