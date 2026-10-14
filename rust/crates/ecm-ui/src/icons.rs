//! Pixel-art icons, drawn as text so they stay readable and diffable: one
//! character per pixel, looked up in a palette. [`Icon::rgba`] gives the
//! RGBA8888 rows to upload with `Core::image_rgba`.
//!
//! The built-in set ([`builtin`]) is the core apps' icons (16x16) used by the
//! desktop's launcher and taskbar and by Files.

/// A small RGBA image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Icon {
    pub w: i32,
    pub h: i32,
    pub rgba: Vec<u8>,
}

/// The shared palette: `.` is transparent.
pub const PALETTE: &[(char, u32)] = &[
    ('.', 0x0000_0000),
    ('k', 0x1a1d_22ff), // outline
    ('w', 0xf4f6_f8ff), // white
    ('s', 0xaab3_c0ff), // silver
    ('d', 0x8c95_a3ff), // grey (text lines)
    ('D', 0x2a2f_37ff), // dark screen
    ('g', 0x5be0_7aff), // terminal green
    ('y', 0xf6c4_53ff), // folder / note yellow
    ('o', 0xe0a0_30ff), // folder front
    ('b', 0x3d8b_fdff), // blue
    ('l', 0x9cc8_ffff), // light blue
    ('r', 0xe5_534bff), // red
    ('p', 0x6a5a_cdff), // purple
];

/// Build an icon from rows of palette characters (all rows the same length).
pub fn from_ascii(rows: &[&str], palette: &[(char, u32)]) -> Icon {
    let h = rows.len() as i32;
    let w = rows.first().map_or(0, |r| r.chars().count()) as i32;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for r in rows {
        let mut n = 0;
        for c in r.chars() {
            let v = palette.iter().find(|(k, _)| *k == c).map_or(0xff00_ffff, |(_, v)| *v);
            rgba.extend_from_slice(&v.to_be_bytes());
            n += 1;
        }
        // Short rows are padded transparent (the tests reject them anyway).
        for _ in n..w {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        }
    }
    Icon { w, h, rgba }
}

const TERMINAL: [&str; 16] = [
    "................",
    ".kkkkkkkkkkkkkk.",
    ".kssssssssssrsk.",
    ".kkkkkkkkkkkkkk.",
    ".kDDDDDDDDDDDDk.",
    ".kDgDDDDDDDDDDk.",
    ".kDDgDDDDDDDDDk.",
    ".kDDDgDDDDDDDDk.",
    ".kDDgDDDDDDDDDk.",
    ".kDgDDggggDDDDk.",
    ".kDDDDDDDDDDDDk.",
    ".kDDDDDDDDDDDDk.",
    ".kDDDDDDDDDDDDk.",
    ".kkkkkkkkkkkkkk.",
    "................",
    "................",
];

const FOLDER: [&str; 16] = [
    "................",
    "................",
    ".kkkkk..........",
    "kyyyyyk.........",
    "kyyyyyykkkkkkkk.",
    "kyyyyyyyyyyyyyyk",
    "kyyyyyyyyyyyyyyk",
    "kooooooooooooook",
    "kooooooooooooook",
    "kooooooooooooook",
    "kooooooooooooook",
    "kooooooooooooook",
    "kooooooooooooook",
    "kkkkkkkkkkkkkkkk",
    "................",
    "................",
];

const SETTINGS: [&str; 16] = [
    "................",
    "................",
    "...bbb..........",
    "ssbbbbbsssssss..",
    "...bbb..........",
    "................",
    "..........bbb...",
    "ssssssssbbbbbss.",
    "..........bbb...",
    "................",
    "......bbb.......",
    "ssssbbbbbssssss.",
    "......bbb.......",
    "................",
    "................",
    "................",
];

const EDITOR: [&str; 16] = [
    "..kkkkkkkkk.....",
    "..kwwwwwwwkk....",
    "..kwwwwwwwkwk...",
    "..kwdddddwkkkk..",
    "..kwwwwwwwwwwk..",
    "..kwddddddddwk..",
    "..kwwwwwwwwwwk..",
    "..kwddddddddwk..",
    "..kwwwwwwwwwwk..",
    "..kwdddddwwwwk..",
    "..kwwwwwwwwwwk..",
    "..kwddddddddwk..",
    "..kwwwwwwwwwwk..",
    "..kkkkkkkkkkkk..",
    "................",
    "................",
];

const NOTES: [&str; 16] = [
    "................",
    ".kkkkkkkkkkkkkk.",
    ".kyyyyyyyyyyyyk.",
    ".kyddddddddddyk.",
    ".kyyyyyyyyyyyyk.",
    ".kyddddddddyyyk.",
    ".kyyyyyyyyyyyyk.",
    ".kydddddddddyyk.",
    ".kyyyyyyyyyyyyk.",
    ".kyddddddyyyyyk.",
    ".kyyyyyyyyyyyyk.",
    ".kyyyyyyyyyyyyk.",
    ".kkkkkkkkkkkkkk.",
    "................",
    "................",
    "................",
];

const GAME: [&str; 16] = [
    "................",
    "................",
    "................",
    "...kkkkkkkkkk...",
    "..kppppppppppk..",
    ".kppwppppppprpk.",
    ".kpwwwpppprprpk.",
    ".kppwppppppprpk.",
    ".kppppppppppppk.",
    ".kpppkkkkkkpppk.",
    ".kppk......kppk.",
    "..kk........kk..",
    "................",
    "................",
    "................",
    "................",
];

const PAGE: [&str; 16] = [
    "...kkkkkkk......",
    "...kwwwwwkk.....",
    "...kwwwwwkwk....",
    "...kwwwwwkkkk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kkkkkkkkkk...",
    "................",
    "................",
];

const CODE: [&str; 16] = [
    "...kkkkkkk......",
    "...kwwwwwkk.....",
    "...kwwwwwkwk....",
    "...kwwwwwkkkk...",
    "...kwwwwwwwwk...",
    "...kwbbwwwwwk...",
    "...kwwbbwwwwk...",
    "...kwwwbbwwwk...",
    "...kwwbbwwwwk...",
    "...kwbbwwyywk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kwwwwwwwwk...",
    "...kkkkkkkkkk...",
    "................",
    "................",
];

const APP: [&str; 16] = [
    "................",
    ".kkkkkkkkkkkkkk.",
    ".kbbbbbbbbbbbbk.",
    ".kkkkkkkkkkkkkk.",
    ".kwwwwwwwwwwwwk.",
    ".kwllllwwwwwwwk.",
    ".kwllllwddddwwk.",
    ".kwllllwwwwwwwk.",
    ".kwwwwwwddddwwk.",
    ".kwwwwwwwwwwwwk.",
    ".kwddddddddddwk.",
    ".kwwwwwwwwwwwwk.",
    ".kkkkkkkkkkkkkk.",
    "................",
    "................",
    "................",
];

/// The Visual Programmer: three nodes wired together.
const VISUAL: [&str; 16] = [
    "................",
    "kkkkkkk.........",
    "kyyyyyk.........",
    "kyyyyykssssss...",
    "kyyyyyk.....s...",
    "kkkkkkk.....s...",
    "..s.......kkkkkk",
    "..s.......kbbbbk",
    "..s.......kbbbbk",
    "..s.......kbbbbk",
    "..s.......kkkkkk",
    "kkkkkkk.....s...",
    "kgggggk.....s...",
    "kgggggkssssss...",
    "kgggggk.........",
    "kkkkkkk.........",
];

/// The App Store: a bag with a download arrow.
const STORE: [&str; 16] = [
    "................",
    "......ssss......",
    ".....s....s.....",
    ".....s....s.....",
    "..kkkkkkkkkkkk..",
    "..kbbbbbbbbbbk..",
    "..kbbbbwwbbbbk..",
    "..kbbbbwwbbbbk..",
    "..kbbbbwwbbbbk..",
    "..kbbwwwwwwbbk..",
    "..kbbbwwwwbbbk..",
    "..kbbbbwwbbbbk..",
    "..kbbbbbbbbbbk..",
    "..kkkkkkkkkkkk..",
    "................",
    "................",
];

/// The built-in icon names.
pub const NAMES: &[&str] = &["terminal", "files", "folder", "settings", "editor", "notes", "game", "file", "code", "app", "visual", "store"];

/// A built-in icon by name (see [`NAMES`]).
pub fn builtin(name: &str) -> Option<Icon> {
    let rows: &[&str] = match name {
        "terminal" => &TERMINAL,
        "files" | "folder" => &FOLDER,
        "settings" => &SETTINGS,
        "editor" => &EDITOR,
        "notes" => &NOTES,
        "game" | "gba" => &GAME,
        "file" => &PAGE,
        "code" => &CODE,
        "app" => &APP,
        "visual" => &VISUAL,
        "store" => &STORE,
        _ => return None,
    };
    Some(from_ascii(rows, PALETTE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_is_16_by_16_with_known_colours() {
        for n in NAMES {
            let rows: Vec<&str> = match *n {
                "terminal" => TERMINAL.to_vec(),
                "files" | "folder" => FOLDER.to_vec(),
                "settings" => SETTINGS.to_vec(),
                "editor" => EDITOR.to_vec(),
                "notes" => NOTES.to_vec(),
                "game" => GAME.to_vec(),
                "file" => PAGE.to_vec(),
                "code" => CODE.to_vec(),
                "visual" => VISUAL.to_vec(),
                "store" => STORE.to_vec(),
                _ => APP.to_vec(),
            };
            assert_eq!(rows.len(), 16, "{n} rows");
            for (i, r) in rows.iter().enumerate() {
                assert_eq!(r.chars().count(), 16, "{n} row {i}: {r:?}");
                for c in r.chars() {
                    assert!(PALETTE.iter().any(|(k, _)| *k == c), "{n} row {i}: unknown colour {c:?}");
                }
            }
            let i = builtin(n).unwrap();
            assert_eq!((i.w, i.h, i.rgba.len()), (16, 16, 16 * 16 * 4));
        }
    }
}
