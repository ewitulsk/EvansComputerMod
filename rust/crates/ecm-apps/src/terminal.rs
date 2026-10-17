//! `terminal`: a terminal emulator window over a kernel shell session
//! (`ipc_spawn_shell`, the same sessions `sshd` uses). Output goes through
//! `ecm-vte`, the console's own VT100 parser, and is drawn with the `mono`
//! font as text nodes (one per run of equal colours), with a scrollback,
//! mouse selection (drag, double click a word, triple click a line), and
//! copy / paste through the clipboard service: Ctrl+Shift+C or Ctrl+Insert /
//! Ctrl+Shift+V or Shift+Insert, or the right-click menu. Shift+PgUp/PgDn,
//! Shift+Up/Down, Shift+Home/End and the wheel scroll back (a full-screen
//! program gets the wheel as arrows). The line editing itself is the shell
//! session's (`ecm-lineedit`). The window's size sets the session's size
//! (`session_resize`).

use crate::sys::Sys;
use ecm_ui::diff::SceneNode;
use ecm_ui::element::{column, custom, Element};
use ecm_ui::window::{keys, EventValue};
use ecm_ui::{App, Ctx, PointerEv, Theme};
use ecm_ui_proto as proto;
use ecm_ui_proto::{NodeKind, Prop};
use ecm_vte::{Vte, CELL_SIZE};
use std::cell::RefCell;
use std::rc::Rc;

/// Cell size of the mono font (lu).
pub const CELL_W: i32 = 8;
pub const CELL_H: i32 = 16;
/// Padding around the grid (lu).
pub const PAD: i32 = 2;
/// Narrower windows (a phone: 240 lu) drop the padding, so 240 lu is 30 columns,
/// the phone's console width.
pub const COMPACT_W: i32 = 320;

/// The padding around the grid in a window `w` lu wide.
pub fn pad_for(w: i32) -> i32 {
    if w < COMPACT_W { 0 } else { PAD }
}
/// Rows kept in the scrollback.
pub const SCROLLBACK: usize = 1000;
/// How often the session's output is read (ms) while it is busy / idle.
pub const POLL_BUSY_MS: i32 = 16;

/// The 16 ANSI colours (VGA-like), `0xRRGGBBAA`.
pub const PALETTE: [u32; 16] = [
    0x1e1e1eff, 0xcd3131ff, 0x0dbc79ff, 0xe5e510ff, 0x2472c8ff, 0xbc3fbcff, 0x11a8cdff, 0xe5e5e5ff, 0x666666ff, 0xf14c4cff, 0x23d18bff,
    0xf5f543ff, 0x3b8eeaff, 0xd670d6ff, 0x29b8dbff, 0xffffffff,
];

/// A position in the whole text: an absolute line (scrollback included) and a column.
pub type Pos = (u64, usize);

/// The terminal's state.
pub struct Term {
    pub vte: Vte,
    pub session: i32,
    pub exited: bool,
    pub cols: u16,
    pub rows: u16,
    /// The absolute line shown at the top while scrolled back (None = following the output).
    pub view_top: Option<u64>,
    pub sel: Option<(Pos, Pos)>,
    selecting: bool,
    /// The size last given to the session.
    sent_size: Option<(u16, u16)>,
    /// The grid's padding in the current window (`pad_for`).
    pub pad: i32,
    /// The last left click (time in ms, cell) and how many came in a row.
    last_click: (i64, Pos, u8),
}

impl Term {
    pub fn new(session: i32, cols: u16, rows: u16) -> Term {
        let mut vte = Vte::new_virtual(cols, rows);
        vte.set_scrollback_max(SCROLLBACK);
        Term { vte, session, exited: session < 0, cols, rows, view_top: None, sel: None, selecting: false, sent_size: None, pad: PAD, last_click: (0, (0, 0), 0) }
    }

    /// The absolute number of the first line still in the scrollback.
    pub fn first_line(&self) -> u64 {
        self.vte.scrolled_total() - self.vte.scrollback().len() as u64
    }

    /// The absolute number of the screen's top row.
    pub fn screen_top(&self) -> u64 {
        self.vte.scrolled_total()
    }

    /// The absolute line at the top of the view.
    pub fn top(&self) -> u64 {
        match self.view_top {
            Some(t) => t.clamp(self.first_line(), self.screen_top()),
            None => self.screen_top(),
        }
    }

    /// The cells of absolute line `l` (`(char, attr, flags)` per column).
    pub fn line(&self, l: u64) -> Vec<(u8, u8, u8)> {
        let w = self.cols as usize;
        if l >= self.screen_top() {
            let y = (l - self.screen_top()) as u16;
            if y >= self.rows {
                return vec![(b' ', self.vte.default_attr(), 0); w];
            }
            return (0..self.cols).map(|x| self.vte.cell(x, y)).collect();
        }
        let first = self.first_line();
        if l < first {
            return vec![(b' ', self.vte.default_attr(), 0); w];
        }
        let raw = &self.vte.scrollback()[(l - first) as usize];
        let mut out: Vec<(u8, u8, u8)> = raw.chunks(CELL_SIZE).map(|c| (c[0], c[1], c[2])).collect();
        out.resize(w, (b' ', self.vte.default_attr(), 0));
        out
    }

    /// The text of a line from column `a` to `b` (exclusive), trailing spaces dropped.
    fn line_text(&self, l: u64, a: usize, b: usize) -> String {
        let cells = self.line(l);
        let s: String = cells.iter().skip(a).take(b.saturating_sub(a)).map(|c| cell_char(c.0)).collect();
        s.trim_end().to_string()
    }

    /// The selected text, lines joined with `\n`.
    pub fn selected_text(&self) -> String {
        let Some((a, b)) = self.sel else { return String::new() };
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        let mut out = Vec::new();
        for l in a.0..=b.0 {
            let from = if l == a.0 { a.1 } else { 0 };
            let to = if l == b.0 { b.1 } else { self.cols as usize };
            out.push(self.line_text(l, from, to));
        }
        out.join("\n")
    }

    /// Scroll the view by `rows` (negative = back into the scrollback).
    pub fn scroll(&mut self, rows: i64) {
        let top = self.top() as i64 + rows;
        let top = top.clamp(self.first_line() as i64, self.screen_top() as i64) as u64;
        self.view_top = if top >= self.screen_top() { None } else { Some(top) };
    }

    /// Select the word at `p` (letters, digits and `_-./:~@`), or with
    /// `line` the whole line.
    pub fn select_word(&mut self, p: Pos, line: bool) {
        let cells = self.line(p.0);
        let word = |c: u8| c.is_ascii_alphanumeric() || b"_-./:~@".contains(&c);
        if line {
            self.sel = Some(((p.0, 0), (p.0, self.cols as usize)));
            return;
        }
        let x = p.1.min(cells.len().saturating_sub(1));
        if !cells.get(x).is_some_and(|c| word(c.0)) {
            return;
        }
        let (mut a, mut b) = (x, x + 1);
        while a > 0 && word(cells[a - 1].0) {
            a -= 1;
        }
        while b < cells.len() && word(cells[b].0) {
            b += 1;
        }
        self.sel = Some(((p.0, a), (p.0, b)));
    }

    /// Change the grid; returns true if it changed.
    pub fn resize(&mut self, cols: u16, rows: u16) -> bool {
        if (cols, rows) == (self.cols, self.rows) {
            return false;
        }
        let (c, r) = self.vte.resize(cols, rows);
        self.cols = c;
        self.rows = r;
        self.sel = None;
        true
    }
}

fn cell_char(b: u8) -> char {
    match b {
        0..=0x1f | 0x7f => ' ',
        b => b as char,
    }
}

/// The grid size for a window of `w x h` lu.
pub fn grid_for(w: i32, h: i32) -> (u16, u16) {
    let pad = pad_for(w);
    (((w - 2 * pad) / CELL_W).clamp(10, 400) as u16, ((h - 2 * pad) / CELL_H).clamp(4, 200) as u16)
}

/// Colours of a cell: (foreground, background or None for the default).
pub fn colors(attr: u8, flags: u8, default_attr: u8, theme: &Theme) -> (u32, Option<u32>) {
    let (f, b) = (attr & 0x0f, attr >> 4);
    let mut fg = if f == default_attr & 0x0f { theme.text } else { PALETTE[f as usize] };
    if flags & 0x01 != 0 && f < 8 && f != default_attr & 0x0f {
        fg = PALETTE[f as usize + 8];
    }
    let mut bg = if b == default_attr >> 4 { None } else { Some(PALETTE[b as usize]) };
    if flags & 0x08 != 0 {
        let swapped_fg = bg.unwrap_or(theme.bg);
        bg = Some(fg);
        fg = swapped_fg;
    }
    (fg, bg)
}

/// The display-list nodes of the visible grid.
pub fn nodes(t: &Term, theme: &Theme, w: i32, h: i32, active: bool) -> Vec<SceneNode> {
    let pad = pad_for(w);
    let mut out = vec![SceneNode::new("bg", NodeKind::Rect, vec![Prop::W(w), Prop::H(h), Prop::Fill(theme.bg)])];
    let def = t.vte.default_attr();
    let top = t.top();
    let sel = t.sel.map(|(a, b)| if a <= b { (a, b) } else { (b, a) });
    let mut texts = Vec::new();
    for y in 0..t.rows {
        let l = top + y as u64;
        let cells = t.line(l);
        let ry = pad + y as i32 * CELL_H;
        // Runs of equal colours.
        let mut x = 0;
        let mut run = 0;
        while x < cells.len() {
            let (fg, bg) = colors(cells[x].1, cells[x].2, def, theme);
            let underline = cells[x].2 & 0x02 != 0;
            let mut e = x + 1;
            while e < cells.len() && colors(cells[e].1, cells[e].2, def, theme) == (fg, bg) && (cells[e].2 & 0x02 != 0) == underline {
                e += 1;
            }
            let rx = pad + x as i32 * CELL_W;
            let rw = (e - x) as i32 * CELL_W;
            if let Some(bg) = bg {
                out.push(SceneNode::new(format!("r{y}b{run}"), NodeKind::Rect, vec![Prop::X(rx), Prop::Y(ry), Prop::W(rw), Prop::H(CELL_H), Prop::Fill(bg)]));
            }
            let s: String = cells[x..e].iter().map(|c| cell_char(c.0)).collect();
            let trimmed = s.trim_end();
            if !trimmed.is_empty() {
                texts.push(SceneNode::new(
                    format!("r{y}t{run}"),
                    NodeKind::Text,
                    vec![Prop::X(rx), Prop::Y(ry), Prop::Text(trimmed.to_string()), Prop::Font(proto::Font::Mono), Prop::Size(16), Prop::Color(fg)],
                ));
            }
            if underline {
                out.push(SceneNode::new(format!("r{y}u{run}"), NodeKind::Rect, vec![Prop::X(rx), Prop::Y(ry + CELL_H - 2), Prop::W(rw), Prop::H(1), Prop::Fill(fg)]));
            }
            run += 1;
            x = e;
        }
        if let Some((a, b)) = sel {
            if l >= a.0 && l <= b.0 {
                let from = if l == a.0 { a.1 } else { 0 };
                let to = if l == b.0 { b.1 } else { t.cols as usize };
                if to > from {
                    out.push(SceneNode::new(
                        format!("s{y}"),
                        NodeKind::Rect,
                        vec![Prop::X(pad + from as i32 * CELL_W), Prop::Y(ry), Prop::W((to - from) as i32 * CELL_W), Prop::H(CELL_H), Prop::Fill(theme.selection)],
                    ));
                }
            }
        }
    }
    out.extend(texts);
    // The cursor: a block over the character, the character in the background colour.
    if t.view_top.is_none() && t.vte.cursor_visible() && !t.exited {
        let (cx, cy) = t.vte.cursor();
        let (px, py) = (pad + cx as i32 * CELL_W, pad + cy as i32 * CELL_H);
        let fill = if active { theme.text } else { theme.text_muted & 0xffff_ff00 | 0x80 };
        out.push(SceneNode::new("cur", NodeKind::Rect, vec![Prop::X(px), Prop::Y(py), Prop::W(CELL_W), Prop::H(CELL_H), Prop::Fill(fill)]));
        let ch = cell_char(t.vte.cell(cx.min(t.cols - 1), cy).0);
        if active && ch != ' ' {
            out.push(SceneNode::new(
                "curc",
                NodeKind::Text,
                vec![Prop::X(px), Prop::Y(py), Prop::Text(ch.to_string()), Prop::Font(proto::Font::Mono), Prop::Size(16), Prop::Color(theme.bg)],
            ));
        }
    }
    if t.view_top.is_some() {
        let tag = format!(" -{} ", t.screen_top() - top);
        let tw = tag.len() as i32 * CELL_W;
        let x = w - pad - tw;
        out.push(SceneNode::new("scrolltagb", NodeKind::Rect, vec![Prop::X(x), Prop::Y(pad), Prop::W(tw), Prop::H(CELL_H), Prop::Fill(theme.text)]));
        out.push(SceneNode::new(
            "scrolltag",
            NodeKind::Text,
            vec![Prop::X(x), Prop::Y(pad), Prop::Text(tag), Prop::Font(proto::Font::Mono), Prop::Size(16), Prop::Color(theme.bg)],
        ));
    }
    if t.exited {
        let msg = "[session ended - close the window]";
        out.push(SceneNode::new(
            "ended",
            NodeKind::Text,
            vec![Prop::X(pad), Prop::Y(h - CELL_H - pad), Prop::Text(msg.into()), Prop::Font(proto::Font::Mono), Prop::Size(16), Prop::Color(theme.danger)],
        ));
    }
    out
}

/// Bytes a key sends to the session (None: not a terminal key), as xterm
/// does: Ctrl+letters as control characters, the cursor and editing keys
/// with the modifier parameter (`ESC[1;5D` is Ctrl+Left), F1-F12.
pub fn key_bytes(key: u16, mods: u16) -> Option<Vec<u8>> {
    let ctrl = mods & proto::mods::CTRL != 0;
    let alt = mods & proto::mods::ALT != 0;
    let shift = mods & proto::mods::SHIFT != 0;
    if ctrl && (0x04..=0x1d).contains(&key) {
        // Ctrl+A .. Ctrl+Z
        return Some(vec![(key - 0x04 + 1) as u8]);
    }
    let m = 1 + shift as u8 + 2 * alt as u8 + 4 * ctrl as u8;
    let csi = |fin: &str| if m == 1 { format!("\x1b[{fin}") } else { format!("\x1b[1;{m}{fin}") };
    let tilde = |n: &str| if m == 1 { format!("\x1b[{n}~") } else { format!("\x1b[{n};{m}~") };
    let seq: String = match key {
        keys::ENTER | keys::KP_ENTER => "\r".into(),
        keys::BACKSPACE if alt => "\x1b\x7f".into(),
        keys::BACKSPACE => "\x7f".into(),
        keys::TAB if shift => "\x1b[Z".into(),
        keys::TAB => "\t".into(),
        keys::ESCAPE => "\x1b".into(),
        keys::UP => csi("A"),
        keys::DOWN => csi("B"),
        keys::RIGHT => csi("C"),
        keys::LEFT => csi("D"),
        keys::HOME => csi("H"),
        keys::END => csi("F"),
        keys::INSERT => tilde("2"),
        keys::DELETE => tilde("3"),
        keys::PAGE_UP => tilde("5"),
        keys::PAGE_DOWN => tilde("6"),
        // F1-F4 (HID 0x3a-0x3d), F5-F12.
        0x3a..=0x3d => {
            let c = (b'P' + (key - 0x3a) as u8) as char;
            if m == 1 { format!("\x1bO{c}") } else { format!("\x1b[1;{m}{c}") }
        }
        0x3e..=0x45 => tilde(["15", "17", "18", "19", "20", "21", "23", "24"][(key - 0x3e) as usize]),
        _ => return None,
    };
    Some(seq.into_bytes())
}

/// Clipboard text as session input: newlines become Enter, and the whole
/// is marked as a paste (bracketed paste) so a Tab or key in it is only text.
pub fn paste_bytes(s: &str) -> Vec<u8> {
    let body = s.replace("\r\n", "\r").replace('\n', "\r").replace('\x1b', "");
    format!("\x1b[200~{body}\x1b[201~").into_bytes()
}

type TermRef = Rc<RefCell<Term>>;
type SysRef = Rc<RefCell<Box<dyn Sys>>>;

fn send(term: &TermRef, sys: &SysRef, data: &[u8]) {
    let mut t = term.borrow_mut();
    if t.exited || data.is_empty() {
        return;
    }
    t.view_top = None;
    let id = t.session;
    if sys.borrow_mut().session_write(id, data) < 0 {
        t.exited = true;
    }
}

/// Read what the session wrote; true if anything changed.
pub fn pump(term: &TermRef, sys: &SysRef) -> bool {
    let mut t = term.borrow_mut();
    if t.exited {
        return false;
    }
    let mut buf = [0u8; 4096];
    let mut changed = false;
    for _ in 0..16 {
        let id = t.session;
        let n = sys.borrow_mut().session_read(id, &mut buf);
        if n < 0 {
            t.exited = true;
            return true;
        }
        if n == 0 {
            break;
        }
        let top_before = t.view_top;
        t.vte.write(&buf[..n as usize]);
        // A scrolled-back view stays where it was.
        t.view_top = top_before;
        changed = true;
        // Answers to the program's queries (cursor position, size).
        let replies = t.vte.take_responses();
        if !replies.is_empty() {
            sys.borrow_mut().session_write(id, &replies);
        }
    }
    if !changed {
        let id = t.session;
        if sys.borrow_mut().session_status(id) == 1 {
            t.exited = true;
            return true;
        }
    }
    changed
}

/// Wall-clock milliseconds (for double clicks).
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn cell_at(t: &Term, p: &PointerEv) -> Pos {
    let x = ((p.x - t.pad).max(0) / CELL_W) as usize;
    let y = ((p.y - t.pad).max(0) / CELL_H) as u64;
    (t.top() + y.min(t.rows as u64 - 1), x.min(t.cols as usize))
}

/// The Terminal app: a window with the session's grid.
pub fn app(conn: Box<dyn ecm_ui::Connection>, sys: Box<dyn Sys>) -> Result<App, ecm_ui::ConnError> {
    let mut app = App::with_connection("dev.ecm.terminal", conn)?;
    let sys: SysRef = Rc::new(RefCell::new(sys));
    let session = sys.borrow_mut().session_open();
    let term: TermRef = Rc::new(RefCell::new(Term::new(session, 80, 24)));
    if session < 0 {
        term.borrow_mut().vte.write(b"terminal: the computer has no free shell session (8 at most).\r\n");
    }
    let (t1, s1) = (term.clone(), sys.clone());
    let win = app.window("Terminal", move |vcx| view(vcx, &t1, &s1));
    // Keys go to the grid from the start.
    if let Some(w) = app.core.window_mut(win) {
        w.focus(Some("term"));
    }
    let (t2, s2) = (term.clone(), sys.clone());
    app.every(POLL_BUSY_MS, move |cx| {
        let changed = pump(&t2, &s2);
        if changed {
            cx.invalidate();
        }
        changed
    });
    let (t3, s3) = (term.clone(), sys.clone());
    app.on_close(move |cx| {
        let id = t3.borrow().session;
        if id >= 0 {
            s3.borrow_mut().session_close(id);
        }
        cx.close_window();
        cx.quit();
    });
    Ok(app)
}

fn view(vcx: &ecm_ui::ViewCtx, term: &TermRef, sys: &SysRef) -> Element {
    let (w, h) = (vcx.width, vcx.height);
    let (cols, rows) = grid_for(w, h);
    {
        let mut t = term.borrow_mut();
        if w > 0 {
            t.pad = pad_for(w);
            t.resize(cols, rows);
            if t.session >= 0 && t.sent_size != Some((t.cols, t.rows)) {
                let id = t.session;
                t.sent_size = Some((t.cols, t.rows));
                sys.borrow_mut().session_resize(id, t.cols, t.rows);
            }
            // A program that asked for resize reports (?2048, vim) gets the new size.
            let replies = t.vte.take_responses();
            if !replies.is_empty() && t.session >= 0 {
                let id = t.session;
                sys.borrow_mut().session_write(id, &replies);
            }
        }
    }
    let t = term.borrow();
    let active = true;
    let nodes = nodes(&t, vcx.theme, w, h, active);
    let (t1, s1) = (term.clone(), sys.clone());
    let (t2, s2) = (term.clone(), sys.clone());
    let (t3, s3) = (term.clone(), sys.clone());
    let (t4, s4) = (term.clone(), sys.clone());
    let area = custom()
        .id("term")
        .grow(1)
        .nodes(nodes)
        .on_key(move |cx, e| on_key(cx, &t1, &s1, e))
        .on_text(move |_, e| {
            if let EventValue::Text(s) = &e.value {
                send(&t2, &s2, s.as_bytes());
            }
        })
        .on_paste(move |_, e| {
            if let EventValue::Text(s) = &e.value {
                send(&t3, &s3, &paste_bytes(s));
            }
        })
        .on_pointer(move |cx, e| {
            if let EventValue::Pointer(p) = &e.value {
                on_pointer(cx, &t4, &s4, p);
            }
        });
    column().gap(0).child(area)
}

fn on_key(cx: &mut Ctx, term: &TermRef, sys: &SysRef, e: &ecm_ui::UiEvent) {
    let EventValue::Key { key, mods, down: true, .. } = e.value else { return };
    let ctrl = mods & proto::mods::CTRL != 0;
    let shift = mods & proto::mods::SHIFT != 0;
    if (ctrl && shift && key == keys::C) || (ctrl && key == keys::INSERT) {
        let s = term.borrow().selected_text();
        if !s.is_empty() {
            cx.copy(&s);
        }
        return;
    }
    if (ctrl && shift && key == keys::V) || (shift && key == keys::INSERT) {
        cx.paste_into("term");
        return;
    }
    if shift && !ctrl && matches!(key, keys::PAGE_UP | keys::PAGE_DOWN | keys::UP | keys::DOWN | keys::HOME | keys::END) {
        let mut t = term.borrow_mut();
        if !t.vte.alternate_screen() {
            let page = t.rows as i64 - 1;
            let by = match key {
                keys::PAGE_UP => -page,
                keys::PAGE_DOWN => page,
                keys::UP => -1,
                keys::DOWN => 1,
                keys::HOME => i64::MIN / 2,
                _ => i64::MAX / 2,
            };
            t.scroll(by);
            cx.invalidate();
            return;
        }
    }
    if let Some(b) = key_bytes(key, mods) {
        send(term, sys, &b);
        cx.invalidate();
    }
}

fn on_pointer(cx: &mut Ctx, term: &TermRef, sys: &SysRef, p: &PointerEv) {
    match p.kind {
        proto::PointerKind::Down if p.button == proto::Button::Left => {
            let mut t = term.borrow_mut();
            let at = cell_at(&t, p);
            let now = now_ms();
            let (last, pos, n) = t.last_click;
            let n = if now - last < 400 && pos == at { n % 3 + 1 } else { 1 };
            t.last_click = (now, at, n);
            if n >= 2 {
                t.select_word(at, n == 3);
                t.selecting = false;
            } else {
                t.sel = Some((at, at));
                t.selecting = true;
            }
            cx.invalidate();
        }
        proto::PointerKind::Down if p.button == proto::Button::Right => {
            let (t1, s1) = (term.clone(), sys.clone());
            cx.popup_menu(p.x, p.y, ["Copy", "Paste", "Clear selection"], move |cx, i| match i {
                0 => {
                    let s = t1.borrow().selected_text();
                    if !s.is_empty() {
                        cx.copy(&s);
                    }
                }
                1 => cx.paste_into("term"),
                _ => {
                    t1.borrow_mut().sel = None;
                    let _ = &s1;
                    cx.invalidate();
                }
            });
        }
        proto::PointerKind::Move => {
            let mut t = term.borrow_mut();
            if t.selecting {
                let at = cell_at(&t, p);
                if let Some(s) = &mut t.sel {
                    if s.1 != at {
                        s.1 = at;
                        cx.invalidate();
                    }
                }
            }
        }
        proto::PointerKind::Up => {
            let mut t = term.borrow_mut();
            if t.selecting {
                t.selecting = false;
                if t.sel.is_some_and(|(a, b)| a == b) {
                    t.sel = None;
                }
                cx.invalidate();
            }
        }
        proto::PointerKind::Scroll => {
            if term.borrow().vte.alternate_screen() {
                // A full-screen program scrolls itself: three arrows.
                let arrow: &[u8] = if p.scroll_y < 0 { b"\x1b[A" } else { b"\x1b[B" };
                send(term, sys, &arrow.repeat(3));
                return;
            }
            term.borrow_mut().scroll(p.scroll_y as i64 * 3);
            cx.invalidate();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_paste_become_terminal_input() {
        assert_eq!(key_bytes(keys::ENTER, 0).unwrap(), b"\r");
        assert_eq!(key_bytes(0x06, proto::mods::CTRL).unwrap(), vec![3], "Ctrl+C");
        assert_eq!(key_bytes(keys::UP, 0).unwrap(), b"\x1b[A");
        assert_eq!(key_bytes(keys::BACKSPACE, 0).unwrap(), b"\x7f");
        assert!(key_bytes(0x04, 0).is_none(), "letters come as text");
        assert_eq!(paste_bytes("ls\necho hi\r\n"), b"\x1b[200~ls\recho hi\r\x1b[201~");
        assert_eq!(key_bytes(keys::LEFT, proto::mods::CTRL).unwrap(), b"\x1b[1;5D", "Ctrl+Left: word left");
        assert_eq!(key_bytes(keys::PAGE_UP, proto::mods::SHIFT).unwrap(), b"\x1b[5;2~");
        assert_eq!(key_bytes(keys::HOME, 0).unwrap(), b"\x1b[H");
        assert_eq!(key_bytes(0x3a, 0).unwrap(), b"\x1bOP", "F1");
        assert_eq!(key_bytes(0x45, 0).unwrap(), b"\x1b[24~", "F12");
    }

    #[test]
    fn double_click_selects_a_word_and_triple_a_line() {
        let mut t = Term::new(1, 30, 3);
        t.vte.write(b"ping 10.0.0.2 now");
        t.select_word((0, 7), false);
        assert_eq!(t.selected_text(), "10.0.0.2");
        t.select_word((0, 7), true);
        assert_eq!(t.selected_text(), "ping 10.0.0.2 now");
        t.sel = None;
        t.select_word((0, 4), false);
        assert_eq!(t.sel, None, "a space selects nothing");
    }

    #[test]
    fn scrollback_selection_and_colours() {
        let mut t = Term::new(1, 20, 3);
        t.vte.write(b"one\r\ntwo\r\nthree\r\n\x1b[31mfour\x1b[0m");
        assert_eq!(t.first_line(), 0);
        assert_eq!(t.screen_top(), 1);
        t.scroll(-2);
        assert_eq!(t.top(), 0);
        t.sel = Some(((0, 0), (1, 3)));
        assert_eq!(t.selected_text(), "one\ntwo");
        t.scroll(10);
        assert_eq!(t.view_top, None);
        let theme = Theme::default();
        let (fg, bg) = colors(0x01 | (t.vte.default_attr() & 0xf0), 0, t.vte.default_attr(), &theme);
        assert_eq!((fg, bg), (PALETTE[1], None));
        let n = nodes(&t, &theme, 200, 60, true);
        assert!(n.iter().any(|x| x.props.contains(&Prop::Text("four".into())) && x.props.contains(&Prop::Color(PALETTE[1]))));
        assert!(n.iter().any(|x| x.key == "cur"));
        assert_eq!(grid_for(644, 388), (80, 24));
        // A phone (240 lu between its bars, 364 lu tall): the phone console's 30 columns.
        assert_eq!(grid_for(240, 364), (30, 22));
    }
}
