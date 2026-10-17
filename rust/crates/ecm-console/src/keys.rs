//! Key codes: the GLFW key codes ComputerCraft programs use (`key` and
//! `key_up` events, the `keys` table), from the USB HID usages the
//! Terminal sends (computer/ui/HidKeys.java maps GLFW to HID; this is the
//! inverse), or from the bytes on stdin when there are no raw keys.

pub const SPACE: u32 = 32;
pub const ENTER: u32 = 257;
pub const TAB: u32 = 258;
pub const BACKSPACE: u32 = 259;
pub const DELETE: u32 = 261;
pub const RIGHT: u32 = 262;
pub const LEFT: u32 = 263;
pub const DOWN: u32 = 264;
pub const UP: u32 = 265;
pub const PAGE_UP: u32 = 266;
pub const PAGE_DOWN: u32 = 267;
pub const HOME: u32 = 268;
pub const END: u32 = 269;
pub const LEFT_CTRL: u32 = 341;

/// The GLFW code of a USB HID usage (keyboard page), or None.
pub fn from_hid(hid: u32) -> Option<u32> {
    Some(match hid {
        0x04..=0x1D => 65 + (hid - 0x04),  // A-Z
        0x1E..=0x26 => 49 + (hid - 0x1E),  // 1-9
        0x27 => 48,                        // 0
        0x3A..=0x45 => 290 + (hid - 0x3A), // F1-F12
        0x68..=0x73 => 302 + (hid - 0x68), // F13-F24
        0x59..=0x61 => 321 + (hid - 0x59), // keypad 1-9
        0x28 => ENTER,
        0x29 => 256, // escape
        0x2A => BACKSPACE,
        0x2B => TAB,
        0x2C => SPACE,
        0x2D => 45,
        0x2E => 61,
        0x2F => 91,
        0x30 => 93,
        0x31 => 92,
        0x33 => 59,
        0x34 => 39,
        0x35 => 96,
        0x36 => 44,
        0x37 => 46,
        0x38 => 47,
        0x39 => 280,
        0x46 => 283,
        0x47 => 281,
        0x48 => 284,
        0x49 => 260,
        0x4A => HOME,
        0x4B => PAGE_UP,
        0x4C => DELETE,
        0x4D => END,
        0x4E => PAGE_DOWN,
        0x4F => RIGHT,
        0x50 => LEFT,
        0x51 => DOWN,
        0x52 => UP,
        0x53 => 282,
        0x54 => 331,
        0x55 => 332,
        0x56 => 333,
        0x57 => 334,
        0x58 => 335,
        0x62 => 320,
        0x63 => 330,
        0x64 => 161,
        0x65 => 348,
        0x67 => 336,
        0xE0 => LEFT_CTRL,
        0xE1 => 340,
        0xE2 => 342,
        0xE3 => 343,
        0xE4 => 345,
        0xE5 => 344,
        0xE6 => 346,
        0xE7 => 347,
        _ => return None,
    })
}

/// The key (on a US layout) that types the printable ASCII byte `b`.
pub fn from_char(b: u8) -> Option<u32> {
    Some(match b {
        b'a'..=b'z' => 65 + (b - b'a') as u32,
        b'A'..=b'Z' => 65 + (b - b'A') as u32,
        b'0'..=b'9' => b as u32,
        b' ' => SPACE,
        b'!' => 49,
        b'@' => 50,
        b'#' => 51,
        b'$' => 52,
        b'%' => 53,
        b'^' => 54,
        b'&' => 55,
        b'*' => 56,
        b'(' => 57,
        b')' => 48,
        b'-' | b'_' => 45,
        b'=' | b'+' => 61,
        b'[' | b'{' => 91,
        b']' | b'}' => 93,
        b'\\' | b'|' => 92,
        b';' | b':' => 59,
        b'\'' | b'"' => 39,
        b'`' | b'~' => 96,
        b',' | b'<' => 44,
        b'.' | b'>' => 46,
        b'/' | b'?' => 47,
        _ => return None,
    })
}

/// Names of the key codes, as the `keys` table has them.
pub fn names() -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for c in b'a'..=b'z' {
        out.push(((c as char).to_string(), 65 + (c - b'a') as u32));
    }
    for (i, n) in ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"].iter().enumerate() {
        out.push((n.to_string(), 48 + i as u32));
    }
    for i in 1..=25u32 {
        out.push((format!("f{i}"), 289 + i));
    }
    for i in 0..=9u32 {
        out.push((format!("numPad{i}"), 320 + i));
    }
    let fixed: &[(&str, u32)] = &[
        ("space", SPACE), ("apostrophe", 39), ("comma", 44), ("minus", 45), ("period", 46), ("slash", 47),
        ("semicolon", 59), ("equals", 61), ("leftBracket", 91), ("backslash", 92), ("rightBracket", 93),
        ("grave", 96), ("enter", ENTER), ("tab", TAB), ("backspace", BACKSPACE), ("insert", 260),
        ("delete", DELETE), ("right", RIGHT), ("left", LEFT), ("down", DOWN), ("up", UP),
        ("pageUp", PAGE_UP), ("pageDown", PAGE_DOWN), ("home", HOME), ("end", END), ("capsLock", 280),
        ("scrollLock", 281), ("numLock", 282), ("printScreen", 283), ("pause", 284),
        ("numPadDecimal", 330), ("numPadDivide", 331), ("numPadMultiply", 332), ("numPadSubtract", 333),
        ("numPadAdd", 334), ("numPadEnter", 335), ("numPadEqual", 336), ("leftShift", 340),
        ("leftCtrl", LEFT_CTRL), ("leftAlt", 342), ("leftSuper", 343), ("rightShift", 344),
        ("rightCtrl", 345), ("rightAlt", 346), ("rightSuper", 347), ("menu", 348),
    ];
    out.extend(fixed.iter().map(|(n, c)| (n.to_string(), *c)));
    out
}

/// The name of a key code, if the `keys` table has one.
pub fn name(code: u32) -> Option<String> {
    names().into_iter().find(|(_, c)| *c == code).map(|(n, _)| n)
}

/// One input from the console: a key press (code, held = auto-repeat), its
/// release, or a typed character.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Key(u32, bool),
    KeyUp(u32),
    Char(u8),
}

/// Turns the bytes the console sends a program (typed text, escape
/// sequences for special keys, control bytes) into key and char inputs.
/// With raw keys available (`raw_keys`), only typed characters are taken
/// from stdin: the presses and releases come from the raw keys.
#[derive(Default)]
pub struct StdinDecoder {
    pending: Vec<u8>,
}

impl StdinDecoder {
    pub fn feed(&mut self, bytes: &[u8], raw_keys: bool, out: &mut Vec<Input>) {
        self.pending.extend_from_slice(bytes);
        let mut i = 0;
        let buf = std::mem::take(&mut self.pending);
        while i < buf.len() {
            let b = buf[i];
            if b == 0x1B {
                // ESC [ <params> <final>: an arrow or editing key.
                if i + 1 >= buf.len() {
                    self.pending.extend_from_slice(&buf[i..]);
                    return;
                }
                if buf[i + 1] != b'[' {
                    i += 1;
                    continue;
                }
                let Some(end) = buf[i + 2..].iter().position(|c| (0x40..=0x7E).contains(c)) else {
                    self.pending.extend_from_slice(&buf[i..]);
                    return;
                };
                let params = &buf[i + 2..i + 2 + end];
                let fin = buf[i + 2 + end];
                i += 3 + end;
                let key = match (fin, params) {
                    (b'A', _) => Some(UP),
                    (b'B', _) => Some(DOWN),
                    (b'C', _) => Some(RIGHT),
                    (b'D', _) => Some(LEFT),
                    (b'H', _) => Some(HOME),
                    (b'F', _) => Some(END),
                    (b'~', b"3") => Some(DELETE),
                    (b'~', b"5") => Some(PAGE_UP),
                    (b'~', b"6") => Some(PAGE_DOWN),
                    (b'~', b"1") | (b'~', b"7") => Some(HOME),
                    (b'~', b"4") | (b'~', b"8") => Some(END),
                    _ => None,
                };
                if let (Some(k), false) = (key, raw_keys) {
                    press(k, out);
                }
                continue;
            }
            i += 1;
            match b {
                b'\n' | b'\r' => {
                    if !raw_keys {
                        press(ENTER, out);
                    }
                }
                b'\t' => {
                    if !raw_keys {
                        press(TAB, out);
                    }
                }
                8 | 127 => {
                    if !raw_keys {
                        press(BACKSPACE, out);
                    }
                }
                1..=26 => {
                    // Ctrl+letter: the letter's key (Ctrl itself is only seen with raw keys).
                    if !raw_keys {
                        press(65 + (b - 1) as u32, out);
                    }
                }
                0 | 27..=31 => {}
                _ => {
                    if !raw_keys {
                        if let Some(k) = from_char(b) {
                            out.push(Input::Key(k, false));
                            out.push(Input::Char(b));
                            out.push(Input::KeyUp(k));
                            continue;
                        }
                    }
                    out.push(Input::Char(b));
                }
            }
        }
    }
}

fn press(k: u32, out: &mut Vec<Input>) {
    out.push(Input::Key(k, false));
    out.push(Input::KeyUp(k));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(chunks: &[&[u8]], raw: bool) -> Vec<Input> {
        let mut d = StdinDecoder::default();
        let mut out = Vec::new();
        for c in chunks {
            d.feed(c, raw, &mut out);
        }
        out
    }

    #[test]
    fn hid_usages_become_glfw_codes() {
        assert_eq!(from_hid(0x04), Some(65), "A");
        assert_eq!(from_hid(0x27), Some(48), "0");
        assert_eq!(from_hid(0x28), Some(ENTER));
        assert_eq!(from_hid(0x52), Some(UP));
        assert_eq!(from_hid(0xE0), Some(LEFT_CTRL));
        assert_eq!(from_hid(0x3A), Some(290), "F1");
        assert_eq!(from_hid(0x02), None);
        assert_eq!(name(ENTER).as_deref(), Some("enter"));
        assert_eq!(name(50).as_deref(), Some("two"));
    }

    #[test]
    fn stdin_bytes_become_keys_and_chars() {
        assert_eq!(decode(&[b"a"], false), [Input::Key(65, false), Input::Char(b'a'), Input::KeyUp(65)]);
        assert_eq!(decode(&[b"\n"], false), [Input::Key(ENTER, false), Input::KeyUp(ENTER)]);
        // An escape sequence split across reads.
        assert_eq!(decode(&[b"\x1b", b"[", b"A"], false), [Input::Key(UP, false), Input::KeyUp(UP)]);
        assert_eq!(decode(&[b"\x1b[3~"], false), [Input::Key(DELETE, false), Input::KeyUp(DELETE)]);
        // With raw keys, stdin only gives the characters.
        assert_eq!(decode(&[b"hi\n\x1b[A"], true), [Input::Char(b'h'), Input::Char(b'i')]);
        assert_eq!(decode(&[&[0xE9]], false), [Input::Char(0xE9)]);
    }
}
