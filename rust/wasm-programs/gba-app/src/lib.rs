//! The GBA app's screens and key bindings, without the emulator or the
//! connection (`main.rs` drives them), so they can be tested on the host.
//!
//! * **Games**: the ROM list, with a **Settings** button.
//! * **Settings**: which keyboard key presses each GBA button. Tap a button's
//!   row, then press the key; a key another button already uses is refused
//!   (the message names that button). **Reset to defaults** puts the
//!   defaults back. Every change is saved at once to
//!   `/var/apps/dev.ecm.gba/keys.json` (the app's own data folder).
//! * **Playing**: the game in a canvas node, the touch buttons, and a
//!   **GAMES** button that goes back to the list (the cartridge save is
//!   written first).

use ecm_ui::diff::SceneNode;
use ecm_ui::Json;
use ecm_ui_proto as proto;
use ecm_ui_proto::{Font, NodeKind, Prop};
use gba::{keys, HEIGHT, WIDTH};
use std::path::{Path, PathBuf};

pub const APP_ID: &str = "dev.ecm.gba";
/// Where the key bindings are kept.
pub const KEYS_FILE: &str = "/var/apps/dev.ecm.gba/keys.json";

/// Hit ids: a touch button is its GBA key bit + 1 (1..=10).
pub const GAMES_HIT: u32 = 40;
pub const SETTINGS_HIT: u32 = 41;
pub const DONE_HIT: u32 = 42;
pub const RESET_HIT: u32 = 43;
/// Settings rows: BIND_HIT + button index.
pub const BIND_HIT: u32 = 60;
/// ROM list rows: ROM_HIT + index.
pub const ROM_HIT: u32 = 100;

/// HID usages the app knows by name.
pub mod hid {
    pub const A: u16 = 0x04;
    pub const S: u16 = 0x16;
    pub const X: u16 = 0x1B;
    pub const Z: u16 = 0x1D;
    pub const ENTER: u16 = 0x28;
    pub const ESCAPE: u16 = 0x29;
    pub const BACKSPACE: u16 = 0x2A;
    pub const RIGHT: u16 = 0x4F;
    pub const LEFT: u16 = 0x50;
    pub const DOWN: u16 = 0x51;
    pub const UP: u16 = 0x52;
}

/// The GBA buttons in the order Settings lists them: (name, key bit).
pub const BUTTONS: [(&str, u16); 10] = [
    ("A", keys::A),
    ("B", keys::B),
    ("L", keys::L),
    ("R", keys::R),
    ("Start", keys::START),
    ("Select", keys::SELECT),
    ("Up", keys::UP),
    ("Down", keys::DOWN),
    ("Left", keys::LEFT),
    ("Right", keys::RIGHT),
];

/// The default keys, in [`BUTTONS`] order: Z = A, X = B, A = L, S = R,
/// Enter = Start, Backspace = Select, arrows = D-pad.
pub const DEFAULT_KEYS: [u16; 10] = [hid::Z, hid::X, hid::A, hid::S, hid::ENTER, hid::BACKSPACE, hid::UP, hid::DOWN, hid::LEFT, hid::RIGHT];

/// A key's name for the Settings page.
pub fn key_name(usage: u16) -> String {
    let s = match usage {
        0x04..=0x1D => return ((b'A' + (usage - 0x04) as u8) as char).to_string(),
        0x1E..=0x26 => return ((b'1' + (usage - 0x1E) as u8) as char).to_string(),
        0x27 => "0",
        0x28 => "Enter",
        0x29 => "Esc",
        0x2A => "Backspace",
        0x2B => "Tab",
        0x2C => "Space",
        0x2D => "-",
        0x2E => "=",
        0x2F => "[",
        0x30 => "]",
        0x31 => "\\",
        0x33 => ";",
        0x34 => "'",
        0x35 => "`",
        0x36 => ",",
        0x37 => ".",
        0x38 => "/",
        0x39 => "Caps Lock",
        0x3A..=0x45 => return format!("F{}", usage - 0x39),
        0x49 => "Insert",
        0x4A => "Home",
        0x4B => "Page Up",
        0x4C => "Delete",
        0x4D => "End",
        0x4E => "Page Down",
        0x4F => "Right",
        0x50 => "Left",
        0x51 => "Down",
        0x52 => "Up",
        0x59..=0x61 => return format!("Num {}", usage - 0x58),
        0x62 => "Num 0",
        0xE0 => "Left Ctrl",
        0xE1 => "Left Shift",
        0xE2 => "Left Alt",
        0xE4 => "Right Ctrl",
        0xE5 => "Right Shift",
        0xE6 => "Right Alt",
        _ => return format!("Key {usage:#04x}"),
    };
    s.to_string()
}

/// Which keyboard key presses each GBA button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bindings {
    /// HID usage per button, in [`BUTTONS`] order.
    pub keys: [u16; 10],
}

impl Default for Bindings {
    fn default() -> Self {
        Bindings { keys: DEFAULT_KEYS }
    }
}

impl Bindings {
    /// The GBA key bit a keyboard key presses (0 = none).
    pub fn bit_for(&self, usage: u16) -> u16 {
        self.keys.iter().position(|k| *k == usage).map_or(0, |i| BUTTONS[i].1)
    }

    /// Give button `i` the key `usage`. A key another button uses is refused
    /// with that button's index. Esc is never bound (it cancels).
    pub fn assign(&mut self, i: usize, usage: u16) -> Result<(), usize> {
        if let Some(j) = self.keys.iter().position(|k| *k == usage) {
            if j != i {
                return Err(j);
            }
        }
        self.keys[i] = usage;
        Ok(())
    }

    pub fn to_json(&self) -> String {
        let keys = BUTTONS.iter().zip(self.keys).map(|((n, _), k)| (n.to_string(), Json::Int(k as i64))).collect();
        Json::Obj(vec![("version".into(), Json::Int(1)), ("keys".into(), Json::Obj(keys))]).to_string()
    }

    /// Parse a saved file. Buttons it doesn't name keep their default; a file
    /// that binds one key twice, or isn't ours, gives None.
    pub fn from_json(text: &str) -> Option<Bindings> {
        let j = Json::parse(text).ok()?;
        let keys = j.get("keys")?;
        let mut b = Bindings::default();
        for (i, (name, _)) in BUTTONS.iter().enumerate() {
            if let Some(v) = keys.get(name) {
                let Json::Int(k) = v else { return None };
                if !(1..=0xFFFF).contains(k) || *k as u16 == hid::ESCAPE {
                    return None;
                }
                b.keys[i] = *k as u16;
            }
        }
        for (i, k) in b.keys.iter().enumerate() {
            if b.keys[i + 1..].contains(k) {
                return None;
            }
        }
        Some(b)
    }

    /// The bindings saved at `path`, or the defaults.
    pub fn load(path: &Path) -> Bindings {
        std::fs::read_to_string(path).ok().and_then(|t| Bindings::from_json(&t)).unwrap_or_default()
    }

    /// Save to `path` (making its folder).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, self.to_json())
    }
}

/// A touch button: GBA key, label, rect (lu), round.
pub struct Pad {
    pub key: u16,
    pub label: &'static str,
    pub r: (i32, i32, i32, i32),
    pub round: bool,
}

/// Where the game and the buttons go in a `w` x `h` window: the game view's
/// rect, whether the canvas is half size, the touch buttons and the GAMES
/// button (between L and R).
pub fn layout(w: i32, h: i32) -> ((i32, i32, i32, i32), bool, Vec<Pad>, (i32, i32, i32, i32)) {
    let half = w < WIDTH as i32;
    let (gw, gh) = if half { (WIDTH as i32 / 2, HEIGHT as i32 / 2) } else { (WIDTH as i32, HEIGHT as i32) };
    let game = ((w - gw) / 2, 4, gw, gh);
    let top = game.1 + gh + 6;
    let room = (h - top).max(40);
    let u = (room / 6).clamp(10, 30); // button unit
    let mut pads = Vec::new();
    // L and R under the game view, GAMES between them.
    pads.push(Pad { key: keys::L, label: "L", r: (4, top, u * 2, u), round: false });
    pads.push(Pad { key: keys::R, label: "R", r: (w - 4 - u * 2, top, u * 2, u), round: false });
    let games = (w / 2 - u * 3 / 2, top, u * 3, u);
    // D-pad on the left.
    let cx = 6 + u * 3 / 2;
    let cy = top + u + 6 + u * 3 / 2;
    pads.push(Pad { key: keys::UP, label: "^", r: (cx - u / 2, cy - u * 3 / 2, u, u), round: false });
    pads.push(Pad { key: keys::DOWN, label: "v", r: (cx - u / 2, cy + u / 2, u, u), round: false });
    pads.push(Pad { key: keys::LEFT, label: "<", r: (cx - u * 3 / 2, cy - u / 2, u, u), round: false });
    pads.push(Pad { key: keys::RIGHT, label: ">", r: (cx + u / 2, cy - u / 2, u, u), round: false });
    // A and B on the right.
    let ax = w - 6 - u - u / 2;
    pads.push(Pad { key: keys::A, label: "A", r: (ax, cy - u, u + u / 4, u + u / 4), round: true });
    pads.push(Pad { key: keys::B, label: "B", r: (ax - u - u / 2, cy - u / 4, u + u / 4, u + u / 4), round: true });
    // Select and Start at the bottom middle.
    let sy = (cy + u * 3 / 2 + 6).min(h - u / 2 - 4);
    pads.push(Pad { key: keys::SELECT, label: "SELECT", r: (w / 2 - u * 2 - 2, sy, u * 2, u / 2 + 2), round: true });
    pads.push(Pad { key: keys::START, label: "START", r: (w / 2 + 2, sy, u * 2, u / 2 + 2), round: true });
    (game, half, pads, games)
}

pub fn text_w(s: &str, size: u8) -> i32 {
    proto::fonts::face(Font::Ui, size).map(|f| f.text_width(s)).unwrap_or(0)
}

fn rect(key: &str, r: (i32, i32, i32, i32), fill: u32, radius: u16, hit: u32) -> SceneNode {
    let mut p = vec![Prop::X(r.0), Prop::Y(r.1), Prop::W(r.2), Prop::H(r.3), Prop::Fill(fill), Prop::Radius(radius)];
    if hit != 0 {
        p.push(Prop::HitId(hit));
    }
    SceneNode::new(key, NodeKind::Rect, p)
}

fn text(key: &str, x: i32, y: i32, s: &str, size: u8, color: u32, hit: u32) -> SceneNode {
    let mut p = vec![Prop::X(x), Prop::Y(y), Prop::Text(s.into()), Prop::Font(Font::Ui), Prop::Size(size), Prop::Color(color)];
    if hit != 0 {
        p.push(Prop::HitId(hit));
    }
    SceneNode::new(key, NodeKind::Text, p)
}

/// A pill button with centred text.
fn pill(key: &str, r: (i32, i32, i32, i32), label: &str, size: u8, fill: u32, hit: u32, out: &mut Vec<SceneNode>) {
    out.push(rect(key, r, fill, (r.3 / 2) as u16, hit));
    let lh = proto::fonts::face(Font::Ui, size).map_or(size as i32, |f| f.line_height());
    out.push(text(&format!("{key}:t"), r.0 + (r.2 - text_w(label, size)) / 2, r.1 + (r.3 - lh) / 2, label, size, FG, hit));
}

const BG: u32 = 0x2b2d42ff;
const BUTTON: u32 = 0x4a4e69ff;
const LIGHT: u32 = 0x8d99aeff;
const HOT: u32 = 0xef8354ff;
const FG: u32 = 0xedf2f4ff;
const MUTED: u32 = 0x8d99aeff;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Games,
    Settings,
    Playing,
}

/// What a press asks `main` to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Start this ROM.
    Play(String),
    /// Leave the game for the list (write the cartridge save first).
    BackToGames,
}

/// The app's screens and bindings.
pub struct Ui {
    pub screen: Screen,
    pub roms: Vec<String>,
    pub error: String,
    pub keys: Bindings,
    pub keys_path: PathBuf,
    /// Settings: the button waiting for its new key.
    pub waiting: Option<usize>,
    /// Settings: the last message (saved, refused, reset).
    pub note: String,
}

impl Ui {
    pub fn new(keys_path: impl Into<PathBuf>) -> Ui {
        let keys_path = keys_path.into();
        let keys = Bindings::load(&keys_path);
        Ui { screen: Screen::Games, roms: Vec::new(), error: String::new(), keys, keys_path, waiting: None, note: String::new() }
    }

    /// A press with this hit id (the touch buttons are `main`'s).
    pub fn pointer_down(&mut self, hit: u32) -> Action {
        match self.screen {
            Screen::Games => {
                if hit == SETTINGS_HIT {
                    self.screen = Screen::Settings;
                    self.waiting = None;
                    self.note = String::new();
                } else if hit >= ROM_HIT {
                    if let Some(r) = self.roms.get((hit - ROM_HIT) as usize) {
                        return Action::Play(r.clone());
                    }
                }
            }
            Screen::Settings => match hit {
                DONE_HIT => {
                    self.screen = Screen::Games;
                    self.waiting = None;
                }
                RESET_HIT => {
                    self.keys = Bindings::default();
                    self.waiting = None;
                    self.note = match self.keys.save(&self.keys_path) {
                        Ok(()) => "Back to the default keys.".into(),
                        Err(e) => format!("Couldn't save: {e}"),
                    };
                }
                h if (BIND_HIT..BIND_HIT + BUTTONS.len() as u32).contains(&h) => {
                    let i = (h - BIND_HIT) as usize;
                    if self.waiting == Some(i) {
                        self.waiting = None;
                        self.note = String::new();
                    } else {
                        self.waiting = Some(i);
                        self.note = format!("Press the key for {} (Esc cancels).", BUTTONS[i].0);
                    }
                }
                _ => {}
            },
            Screen::Playing => {
                if hit == GAMES_HIT {
                    return Action::BackToGames;
                }
            }
        }
        Action::None
    }

    /// A key on the Settings page while a button waits for one: binds it
    /// (true = the key was used here and is not GBA input).
    pub fn settings_key(&mut self, usage: u16, down: bool) -> bool {
        if self.screen != Screen::Settings {
            return false;
        }
        let Some(i) = self.waiting else { return true };
        if !down {
            return true;
        }
        if usage == hid::ESCAPE {
            self.waiting = None;
            self.note = "Cancelled.".into();
            return true;
        }
        match self.keys.assign(i, usage) {
            Ok(()) => {
                self.waiting = None;
                self.note = match self.keys.save(&self.keys_path) {
                    Ok(()) => format!("{} = {}. Saved.", BUTTONS[i].0, key_name(usage)),
                    Err(e) => format!("{} = {}, but it couldn't be saved: {e}", BUTTONS[i].0, key_name(usage)),
                };
            }
            Err(j) => {
                self.note = format!("{} is already {}. Press another key, or change {} first.", key_name(usage), BUTTONS[j].0, BUTTONS[j].0);
            }
        }
        true
    }

    /// The window's nodes. `held`: GBA keys down (Playing).
    pub fn nodes(&self, w: i32, h: i32, held: u16) -> Vec<SceneNode> {
        let mut n = vec![rect("bg", (0, 0, w, h), BG, 0, 0)];
        match self.screen {
            Screen::Playing => {
                let (g, _, pads, games) = layout(w, h);
                n.push(rect("bezel", (g.0 - 2, g.1 - 2, g.2 + 4, g.3 + 4), 0x111111ff, 2, 0));
                n.push(SceneNode::new("game", NodeKind::Canvas, vec![Prop::X(g.0), Prop::Y(g.1), Prop::W(g.2), Prop::H(g.3), Prop::Resource(1), Prop::Scale(1)]));
                for p in &pads {
                    let bit = p.key.trailing_zeros();
                    let down = held & p.key != 0;
                    let fill = if down { HOT } else if p.round && (p.key == keys::A || p.key == keys::B) { LIGHT } else { BUTTON };
                    let k = format!("pad{bit}");
                    n.push(rect(&k, p.r, fill, if p.round { (p.r.3 / 2) as u16 } else { 3 }, bit + 1));
                    let size = if p.label.len() > 1 { 8 } else if p.r.3 >= 16 { 12 } else { 8 };
                    let tw = text_w(p.label, size);
                    n.push(text(&format!("{k}:t"), p.r.0 + (p.r.2 - tw) / 2, p.r.1 + (p.r.3 - size as i32) / 2, p.label, size, FG, bit + 1));
                }
                pill("games", games, "GAMES", 8, 0x3a3d56ff, GAMES_HIT, &mut n);
            }
            Screen::Games => {
                n.push(text("title", 8, 8, "Choose a game", 12, FG, 0));
                let hint = if self.roms.is_empty() { "No .gba files in /home" } else { "Tap a game to play" };
                n.push(text("hint", 8, 24, hint, 8, MUTED, 0));
                let sw = text_w("Settings", 12) + 16;
                pill("settings", (w - 6 - sw, 6, sw, 18), "Settings", 12, BUTTON, SETTINGS_HIT, &mut n);
                for (i, r) in self.roms.iter().enumerate().take(((h - 52) / 22).clamp(1, 12) as usize) {
                    let y = 40 + i as i32 * 22;
                    let name = r.rsplit('/').next().unwrap_or(r).to_string();
                    n.push(rect(&format!("rom{i}"), (6, y, w - 12, 18), BUTTON, 3, ROM_HIT + i as u32));
                    n.push(text(&format!("rom{i}:t"), 12, y + 3, &name, 12, FG, ROM_HIT + i as u32));
                }
                if !self.error.is_empty() {
                    n.push(text("err", 8, h - 14, &self.error.chars().take(60).collect::<String>(), 8, HOT, 0));
                }
            }
            Screen::Settings => self.settings_nodes(w, h, &mut n),
        }
        n
    }

    fn settings_nodes(&self, w: i32, h: i32, n: &mut Vec<SceneNode>) {
        n.push(text("title", 8, 8, "Controls", 12, FG, 0));
        let bw = text_w("Back", 12) + 16;
        pill("done", (w - 6 - bw, 6, bw, 18), "Back", 12, BUTTON, DONE_HIT, n);
        n.push(text("hint", 8, 28, "Tap a button, then press its key.", 8, MUTED, 0));
        // Rows of 20 lu in as many columns as the height needs.
        let top = 42;
        let bottom = 46;
        let per_col = ((h - top - bottom) / 20).clamp(1, BUTTONS.len() as i32) as usize;
        let cols = BUTTONS.len().div_ceil(per_col);
        let cw = (w - 12 - 6 * (cols as i32 - 1)) / cols as i32;
        for (i, (name, _)) in BUTTONS.iter().enumerate() {
            let (col, row) = (i / per_col, i % per_col);
            let x = 6 + col as i32 * (cw + 6);
            let y = top + row as i32 * 20;
            let waiting = self.waiting == Some(i);
            let fill = if waiting { HOT } else { BUTTON };
            let hit = BIND_HIT + i as u32;
            n.push(rect(&format!("bind{i}"), (x, y, cw, 18), fill, 3, hit));
            n.push(text(&format!("bind{i}:n"), x + 6, y + 3, name, 12, FG, hit));
            let key = if waiting { "press a key...".to_string() } else { key_name(self.keys.keys[i]) };
            n.push(text(&format!("bind{i}:k"), x + cw - 6 - text_w(&key, 12), y + 3, &key, 12, FG, hit));
        }
        let rows_end = top + per_col as i32 * 20;
        let rw = text_w("Reset to defaults", 12) + 16;
        pill("reset", (6, rows_end + 4, rw, 18), "Reset to defaults", 12, BUTTON, RESET_HIT, n);
        if !self.note.is_empty() {
            let face = proto::fonts::face(Font::Ui, 8).unwrap();
            for (i, line) in ecm_ui::text::wrap(&face, &self.note, Some(w - 16)).iter().take(2).enumerate() {
                n.push(text(&format!("note{i}"), 8, rows_end + 26 + i as i32 * 10, line, 8, if self.note.contains("already") { HOT } else { MUTED }, 0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("gba-app-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("var/apps/dev.ecm.gba/keys.json")
    }

    fn texts(n: &[SceneNode]) -> Vec<String> {
        n.iter().filter_map(|s| s.props.iter().find_map(|p| if let Prop::Text(t) = p { Some(t.clone()) } else { None })).collect()
    }

    #[test]
    fn default_keys_are_the_documented_ones() {
        let b = Bindings::default();
        assert_eq!(b.bit_for(hid::Z), keys::A);
        assert_eq!(b.bit_for(hid::X), keys::B);
        assert_eq!(b.bit_for(hid::A), keys::L);
        assert_eq!(b.bit_for(hid::S), keys::R);
        assert_eq!(b.bit_for(hid::ENTER), keys::START);
        assert_eq!(b.bit_for(hid::BACKSPACE), keys::SELECT);
        assert_eq!((b.bit_for(hid::UP), b.bit_for(hid::DOWN), b.bit_for(hid::LEFT), b.bit_for(hid::RIGHT)), (keys::UP, keys::DOWN, keys::LEFT, keys::RIGHT));
        assert_eq!(b.bit_for(0x2C), 0, "Space does nothing by default");
        assert_eq!(key_name(hid::Z), "Z");
        assert_eq!(key_name(hid::BACKSPACE), "Backspace");
        assert_eq!(key_name(0x3B), "F2");
        assert_eq!(key_name(0x1E), "1");
    }

    #[test]
    fn rebinding_refuses_conflicts_and_persists() {
        let path = tmp("persist");
        let mut ui = Ui::new(&path);
        assert_eq!(ui.keys, Bindings::default(), "no file yet: the defaults");
        // Games -> Settings, pick A, press Space.
        assert_eq!(ui.pointer_down(SETTINGS_HIT), Action::None);
        assert_eq!(ui.screen, Screen::Settings);
        ui.pointer_down(BIND_HIT);
        assert_eq!(ui.waiting, Some(0));
        assert!(texts(&ui.nodes(240, 360, 0)).contains(&"press a key...".to_string()));
        assert!(ui.settings_key(0x2C, true));
        assert_eq!(ui.keys.bit_for(0x2C), keys::A);
        assert_eq!(ui.keys.bit_for(hid::Z), 0, "Z no longer presses A");
        assert!(ui.note.contains("A = Space"), "{}", ui.note);
        // A conflict: B can't take Space (A has it); B keeps X and still waits.
        ui.pointer_down(BIND_HIT + 1);
        ui.settings_key(0x2C, true);
        assert_eq!(ui.keys.keys[1], hid::X);
        assert_eq!(ui.waiting, Some(1));
        assert_eq!(ui.note, "Space is already A. Press another key, or change A first.");
        // Esc cancels; key releases are ignored.
        ui.settings_key(0x2C, false);
        ui.settings_key(hid::ESCAPE, true);
        assert_eq!(ui.waiting, None);
        // Saved: a fresh app reads the same bindings.
        let again = Ui::new(&path);
        assert_eq!(again.keys.keys[0], 0x2C);
        assert_eq!(again.keys.keys[1], hid::X);
        // Reset to defaults saves too.
        ui.pointer_down(RESET_HIT);
        assert_eq!(ui.keys, Bindings::default());
        assert_eq!(Ui::new(&path).keys, Bindings::default());
        // Back to the list.
        ui.pointer_down(DONE_HIT);
        assert_eq!(ui.screen, Screen::Games);
    }

    #[test]
    fn a_bad_keys_file_gives_the_defaults() {
        let path = tmp("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // One key bound twice: refused as a whole.
        std::fs::write(&path, r#"{"version":1,"keys":{"A":29,"B":29}}"#).unwrap();
        assert_eq!(Bindings::load(&path), Bindings::default());
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(Bindings::load(&path), Bindings::default());
        // A partial file keeps the other defaults.
        std::fs::write(&path, r#"{"version":1,"keys":{"Start":44}}"#).unwrap();
        let b = Bindings::load(&path);
        assert_eq!(b.bit_for(0x2C), keys::START);
        assert_eq!(b.bit_for(hid::Z), keys::A);
        assert_eq!(b.bit_for(hid::ENTER), 0);
        assert_eq!(Bindings::from_json(&b.to_json()), Some(b));
    }

    #[test]
    fn the_game_has_a_games_button_and_settings_fit_terminal_and_phone_sizes() {
        let mut ui = Ui::new(tmp("layout"));
        ui.roms = vec!["/home/arm.gba".into()];
        assert_eq!(ui.pointer_down(ROM_HIT), Action::Play("/home/arm.gba".into()));
        ui.screen = Screen::Playing;
        for (w, h) in [(240, 360), (640, 400), (320, 200)] {
            let n = ui.nodes(w, h, 0);
            let games = n.iter().find(|s| s.key == "games").unwrap();
            let get = |p: fn(&Prop) -> Option<i32>| games.props.iter().find_map(p).unwrap();
            let (x, y, gw, gh) = (get(|p| if let Prop::X(v) = p { Some(*v) } else { None }), get(|p| if let Prop::Y(v) = p { Some(*v) } else { None }), get(|p| if let Prop::W(v) = p { Some(*v) } else { None }), get(|p| if let Prop::H(v) = p { Some(*v) } else { None }));
            assert!(x >= 0 && y >= 0 && x + gw <= w && y + gh <= h, "{w}x{h}: GAMES at {x},{y} {gw}x{gh}");
            // It doesn't cover L or R.
            let (_, _, pads, _) = layout(w, h);
            for p in pads.iter().filter(|p| p.key == keys::L || p.key == keys::R) {
                assert!(p.r.0 + p.r.2 <= x || x + gw <= p.r.0, "{w}x{h}: GAMES overlaps {}", p.label);
            }
        }
        assert_eq!(ui.pointer_down(GAMES_HIT), Action::BackToGames);
        ui.screen = Screen::Settings;
        for (w, h) in [(240, 360), (640, 400), (320, 200), (640, 180)] {
            let n = ui.nodes(w, h, 0);
            for s in &n {
                let v = |f: fn(&Prop) -> Option<i32>| s.props.iter().find_map(f);
                if let (Some(x), Some(y)) = (v(|p| if let Prop::X(v) = p { Some(*v) } else { None }), v(|p| if let Prop::Y(v) = p { Some(*v) } else { None })) {
                    let ww = v(|p| if let Prop::W(v) = p { Some(*v) } else { None }).unwrap_or(0);
                    assert!(x >= 0 && x + ww <= w, "{w}x{h}: {} runs out sideways", s.key);
                    if s.key.starts_with("bind") || s.key.starts_with("reset") {
                        assert!(y + 18 <= h, "{w}x{h}: {} is below the window", s.key);
                    }
                }
            }
            assert_eq!(n.iter().filter(|s| s.key.starts_with("bind") && !s.key.contains(':')).count(), 10, "{w}x{h}: all ten buttons");
        }
    }
}
