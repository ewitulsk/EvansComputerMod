//! Immediate-mode core: persistent [`Ui`] state and the per-frame [`Frame`].

use std::collections::HashMap;

use crate::canvas::{fit_text, text_width, Align, Canvas, CAP_HEIGHT, CHAR_ADVANCE, GLYPH_HEIGHT};
use crate::geom::{Color, Rect};
use crate::input::{InputEvent, Key, KeyOrChar, Modifiers, MouseButton};
use crate::theme::Theme;

/// Stable widget identity (FNV-1a hash).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WidgetId(pub u64);

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

impl WidgetId {
    pub fn new(name: &str) -> WidgetId {
        WidgetId(fnv(FNV_OFFSET, name.as_bytes()))
    }

    /// Derive a child id (e.g. per cell / per tab).
    pub fn with(self, index: u64) -> WidgetId {
        WidgetId(fnv(self.0 ^ 0x9e37_79b9_7f4a_7c15, &index.to_le_bytes()))
    }
}

/// `id("grid", i)`: hash of a name plus an index.
pub fn id(name: &str, index: u64) -> WidgetId {
    WidgetId::new(name).with(index)
}

/// Placement flag: the cell is selected.
pub const ITEM_FLAG_SELECTED: u8 = 0x01;
/// Placement flag: the cell is hovered.
pub const ITEM_FLAG_HOVERED: u8 = 0x02;
/// Placement flag: a modal is open above this item (render it dimmed).
pub const ITEM_FLAG_DIMMED: u8 = 0x80;

/// An item icon for the client to draw over the framebuffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemPlacement {
    pub x: i32,
    pub y: i32,
    pub size: i32,
    /// Clip in effect when emitted; the client must not draw outside it.
    pub clip: Rect,
    pub item: String,
    pub label: String,
    pub flags: u8,
}

impl ItemPlacement {
    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.size, self.size)
    }
}

/// Result of an interactive widget.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// Left press and release both inside the widget.
    pub clicked: bool,
    pub right_clicked: bool,
    pub middle_clicked: bool,
    pub hovered: bool,
    /// Left button currently held on this widget.
    pub pressed: bool,
    /// Value changed (text edited, toggle flipped, tab switched).
    pub changed: bool,
    /// Text field: Enter pressed.
    pub submitted: bool,
    /// Text field: has keyboard focus after this call.
    pub focused: bool,
    /// Modifier state for this frame (shift-click = request a full stack).
    pub shift: bool,
    pub ctrl: bool,
}

/// Modal behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModalOptions {
    pub close_on_outside_click: bool,
    pub close_on_escape: bool,
    /// Draw an "x" close button in the title bar.
    pub close_button: bool,
}

impl Default for ModalOptions {
    fn default() -> Self {
        ModalOptions { close_on_outside_click: true, close_on_escape: true, close_button: true }
    }
}

#[derive(Clone, Copy, Debug)]
struct ModalState {
    id: WidgetId,
    opts: ModalOptions,
    opened_frame: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct TextState {
    cursor: usize,
    scroll: i32,
    activity_ms: u64,
}

/// Scroll/virtualised-grid result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GridResponse {
    pub cols: usize,
    pub rows: usize,
    pub offset: i32,
    pub max_offset: i32,
    /// First index visited (== count when nothing visible).
    pub first_visible: usize,
    /// Number of indices visited.
    pub visited: usize,
    pub hovered: bool,
}

/// What a frame produced besides pixels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameOutput {
    pub items: Vec<ItemPlacement>,
    /// Items differ from the previous frame's list.
    pub items_changed: bool,
    /// Something animates (caret blink): run another frame in ~250-500ms.
    pub wants_redraw_soon: bool,
    /// Run another frame immediately (on-screen keyboard injected input).
    pub redraw_now: bool,
    /// The frame had any input events.
    pub had_input: bool,
}

/// Persistent UI state. Keep one per program and call
/// [`begin_frame`](Ui::begin_frame) for every redraw.
#[derive(Debug, Default)]
pub struct Ui {
    /// Current modifier state (also updated by [`InputEvent::Modifiers`]).
    pub modifiers: Modifiers,
    /// Optional clock for caret blinking; `None` = solid caret.
    pub time_ms: Option<u64>,
    mouse: (i32, i32),
    mouse_known: bool,
    buttons: [bool; 3],
    hot: Option<WidgetId>,
    active: Option<WidgetId>,
    focused: Option<WidgetId>,
    text: TextState,
    scroll: HashMap<WidgetId, i32>,
    drag_grab: i32,
    modal: Option<ModalState>,
    pending: Vec<InputEvent>,
    osk_shift: bool,
    last_items: Vec<ItemPlacement>,
    frame: u64,
}

impl Ui {
    pub fn new() -> Ui {
        Ui::default()
    }

    pub fn mouse_pos(&self) -> (i32, i32) {
        self.mouse
    }
    pub fn hot(&self) -> Option<WidgetId> {
        self.hot
    }
    pub fn active(&self) -> Option<WidgetId> {
        self.active
    }
    pub fn focused(&self) -> Option<WidgetId> {
        self.focused
    }
    pub fn focus(&mut self, id: WidgetId) {
        if self.focused != Some(id) {
            self.focused = Some(id);
            self.text = TextState { cursor: usize::MAX, scroll: 0, activity_ms: self.time_ms.unwrap_or(0) };
        }
    }
    pub fn unfocus(&mut self) {
        self.focused = None;
    }

    pub fn scroll_offset(&self, id: WidgetId) -> i32 {
        self.scroll.get(&id).copied().unwrap_or(0)
    }
    /// Set a scroll offset (clamped when the area is next drawn).
    pub fn set_scroll(&mut self, id: WidgetId, offset: i32) {
        self.scroll.insert(id, offset.max(0));
    }
    pub fn scroll_to_top(&mut self, id: WidgetId) {
        self.scroll.insert(id, 0);
    }

    pub fn open_modal(&mut self, id: WidgetId) {
        self.open_modal_with(id, ModalOptions::default());
    }
    pub fn open_modal_with(&mut self, id: WidgetId, opts: ModalOptions) {
        self.modal = Some(ModalState { id, opts, opened_frame: self.frame });
        self.focused = None;
        self.active = None;
    }
    pub fn close_modal(&mut self) {
        self.modal = None;
    }
    pub fn is_modal_open(&self, id: WidgetId) -> bool {
        self.modal.is_some_and(|m| m.id == id)
    }
    pub fn any_modal_open(&self) -> bool {
        self.modal.is_some()
    }

    /// Queue a synthetic key/char for the next frame (delivered first).
    pub fn inject(&mut self, k: KeyOrChar) {
        self.pending.push(k.to_event());
    }

    /// Start a frame. `events` are processed in order by the widgets called
    /// on the returned [`Frame`]; finish with [`Frame::end`].
    pub fn begin_frame<'a>(&'a mut self, canvas: &'a mut Canvas, events: &[InputEvent], theme: &'a Theme) -> Frame<'a> {
        self.frame = self.frame.wrapping_add(1);
        canvas.reset_clip();
        let mut evs: Vec<InputEvent> = std::mem::take(&mut self.pending);
        evs.extend_from_slice(events);
        let mut any_left_down = false;
        for e in &evs {
            match *e {
                InputEvent::MouseMove { x, y } => {
                    self.mouse = (x, y);
                    self.mouse_known = true;
                }
                InputEvent::MouseDown { x, y, button } => {
                    self.mouse = (x, y);
                    self.mouse_known = true;
                    self.buttons[button.index()] = true;
                    if button == MouseButton::Left {
                        any_left_down = true;
                    }
                }
                InputEvent::MouseUp { x, y, button } => {
                    self.mouse = (x, y);
                    self.mouse_known = true;
                    self.buttons[button.index()] = false;
                }
                InputEvent::Scroll { x, y, .. } => {
                    self.mouse = (x, y);
                    self.mouse_known = true;
                }
                InputEvent::Modifiers(m) => self.modifiers = m,
                _ => {}
            }
        }
        let had_input = !evs.is_empty();
        self.hot = None;
        Frame {
            ui: self,
            canvas,
            theme,
            events: evs,
            items: Vec::new(),
            in_modal: false,
            any_left_down,
            down_keeps_focus: false,
            focus_seen: false,
            keys_consumed: false,
            wants_redraw_soon: false,
            redraw_now: false,
            tooltip: None,
            had_input,
        }
    }
}

/// One frame in progress. Draw calls go to the canvas immediately; call
/// widgets back-to-front (a [`Frame::modal`] must come after what it covers).
pub struct Frame<'a> {
    ui: &'a mut Ui,
    canvas: &'a mut Canvas,
    theme: &'a Theme,
    events: Vec<InputEvent>,
    items: Vec<ItemPlacement>,
    in_modal: bool,
    any_left_down: bool,
    down_keeps_focus: bool,
    focus_seen: bool,
    keys_consumed: bool,
    wants_redraw_soon: bool,
    redraw_now: bool,
    tooltip: Option<String>,
    had_input: bool,
}

const ENTER_KEYS: [Key; 1] = [Key::Enter];

impl<'a> Frame<'a> {
    // ---- access ---------------------------------------------------------

    pub fn ui(&mut self) -> &mut Ui {
        self.ui
    }
    pub fn canvas(&mut self) -> &mut Canvas {
        self.canvas
    }
    pub fn theme(&self) -> &'a Theme {
        self.theme
    }
    /// Whole-canvas rect.
    pub fn rect(&self) -> Rect {
        self.canvas.bounds()
    }
    pub fn scale(&self) -> i32 {
        self.theme.text_scale.max(1)
    }
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }
    pub fn modifiers(&self) -> Modifiers {
        self.ui.modifiers
    }
    pub fn open_modal(&mut self, id: WidgetId) {
        self.ui.open_modal(id);
    }
    pub fn close_modal(&mut self) {
        self.ui.close_modal();
    }
    pub fn scroll_to_top(&mut self, id: WidgetId) {
        self.ui.scroll_to_top(id);
    }

    /// Whether input to widgets in the current layer is blocked by a modal.
    pub fn blocked(&self) -> bool {
        self.ui.modal.is_some() && !self.in_modal
    }

    /// `k` was pressed this frame and not consumed by a focused text field
    /// (always false beneath an open modal).
    pub fn key_pressed(&self, k: Key) -> bool {
        !self.blocked()
            && !self.keys_consumed
            && self.ui.focused.is_none()
            && self.events.contains(&InputEvent::Key(k))
    }

    /// Mark the frame as needing another run soon (custom animation).
    pub fn request_redraw_soon(&mut self) {
        self.wants_redraw_soon = true;
    }

    // ---- drawing shortcuts ---------------------------------------------

    pub fn fill(&mut self, r: Rect, c: Color) {
        self.canvas.fill_rect(r, c);
    }
    pub fn stroke(&mut self, r: Rect, c: Color) {
        self.canvas.stroke_rect(r, c);
    }
    pub fn text(&mut self, x: i32, y: i32, s: &str, c: Color) -> i32 {
        let sc = self.scale();
        self.canvas.draw_text(x, y, s, c, sc)
    }
    pub fn push_clip(&mut self, r: Rect) {
        self.canvas.push_clip(r);
    }
    pub fn pop_clip(&mut self) {
        self.canvas.pop_clip();
    }
    /// Fill the whole canvas with the theme background.
    pub fn clear(&mut self) {
        let c = self.theme.background;
        self.canvas.clear(c);
    }
    /// Filled panel with a border.
    pub fn panel(&mut self, r: Rect) {
        let t = self.theme;
        self.canvas.fill_rect(r, t.panel);
        self.canvas.stroke_rect(r, t.border);
    }

    // ---- interaction core ----------------------------------------------

    /// The part of `r` that can receive pointer input right now.
    pub fn hit_rect(&self, r: Rect) -> Rect {
        if self.blocked() {
            Rect::new(r.x, r.y, 0, 0)
        } else {
            self.canvas.clip().intersect(r)
        }
    }

    /// Generic press/release/hover logic for a rect. `keep_focus` stops a
    /// press here from unfocusing the focused text field.
    pub fn interact(&mut self, id: WidgetId, r: Rect, keep_focus: bool) -> Response {
        let hr = self.hit_rect(r);
        let mut pressing = self.ui.active == Some(id) && !self.blocked();
        let mut right = false;
        let mut middle = false;
        let mut resp = Response::default();
        for i in 0..self.events.len() {
            match self.events[i] {
                InputEvent::MouseDown { x, y, button } if hr.contains(x, y) => {
                    if keep_focus {
                        self.down_keeps_focus = true;
                    }
                    match button {
                        MouseButton::Left => pressing = true,
                        MouseButton::Right => right = true,
                        MouseButton::Middle => middle = true,
                    }
                }
                InputEvent::MouseUp { x, y, button } => {
                    let inside = hr.contains(x, y);
                    match button {
                        MouseButton::Left => {
                            if pressing && inside {
                                resp.clicked = true;
                            }
                            pressing = false;
                        }
                        MouseButton::Right => {
                            resp.right_clicked |= right && inside;
                            right = false;
                        }
                        MouseButton::Middle => {
                            resp.middle_clicked |= middle && inside;
                            middle = false;
                        }
                    }
                }
                _ => {}
            }
        }
        if pressing && self.ui.buttons[0] {
            self.ui.active = Some(id);
        } else if self.ui.active == Some(id) {
            self.ui.active = None;
            pressing = false;
        }
        let (mx, my) = self.ui.mouse;
        resp.hovered = self.ui.mouse_known && hr.contains(mx, my);
        if resp.hovered {
            self.ui.hot = Some(id);
        }
        resp.pressed = pressing;
        resp.shift = self.ui.modifiers.shift;
        resp.ctrl = self.ui.modifiers.ctrl;
        resp
    }

    // ---- basic widgets --------------------------------------------------

    /// Text in `r`, vertically centred, truncated with "..".
    pub fn label(&mut self, r: Rect, text: &str, color: Color, align: Align) {
        let sc = self.scale();
        self.canvas.draw_text_in(r, text, color, sc, align);
    }

    /// Left-aligned dim label.
    pub fn label_dim(&mut self, r: Rect, text: &str) {
        let c = self.theme.text_dim;
        self.label(r, text, c, Align::Left);
    }

    pub fn button(&mut self, id: WidgetId, r: Rect, text: &str) -> Response {
        self.button_styled(id, r, text, false)
    }

    /// Button drawn with the accent colour (primary action).
    pub fn button_primary(&mut self, id: WidgetId, r: Rect, text: &str) -> Response {
        self.button_styled(id, r, text, true)
    }

    fn button_styled(&mut self, id: WidgetId, r: Rect, text: &str, primary: bool) -> Response {
        let resp = self.interact(id, r, false);
        self.draw_button(r, text, resp, primary);
        resp
    }

    fn draw_button(&mut self, r: Rect, text: &str, resp: Response, lit: bool) {
        let t = self.theme;
        let (bg, fg) = if resp.pressed && resp.hovered {
            (t.button_active, t.accent_text)
        } else if lit {
            (if resp.hovered { t.accent.mix(Color::WHITE, 40) } else { t.accent }, t.accent_text)
        } else if resp.hovered {
            (t.button_hover, t.text)
        } else {
            (t.button, t.text)
        };
        self.canvas.fill_rect(r, bg);
        self.canvas.stroke_rect(r, t.border);
        let sc = self.scale();
        self.canvas.draw_text_in(r.inset_xy(2, 0), text, fg, sc, Align::Center);
    }

    /// Checkbox with a label; toggles `value` on click.
    pub fn checkbox(&mut self, id: WidgetId, r: Rect, value: &mut bool, label: &str) -> Response {
        let mut resp = self.interact(id, r, false);
        if resp.clicked {
            *value = !*value;
            resp.changed = true;
        }
        let t = self.theme;
        let bs = (CAP_HEIGHT * self.scale() + 2).min(r.h).max(0);
        let (bx, rest) = r.split_left(bs + 4);
        let b = bx.center_fit(bs, bs);
        self.canvas.fill_rect(b, t.field_bg);
        self.canvas.stroke_rect(b, if resp.hovered { t.accent } else { t.border });
        if *value {
            self.canvas.fill_rect(b.inset(2), t.accent);
        }
        let c = t.text;
        self.label(rest, label, c, Align::Left);
        resp
    }

    /// Toggle button (lit when on).
    pub fn toggle(&mut self, id: WidgetId, r: Rect, value: &mut bool, text: &str) -> Response {
        let mut resp = self.interact(id, r, false);
        if resp.clicked {
            *value = !*value;
            resp.changed = true;
        }
        self.draw_button(r, text, resp, *value);
        resp
    }

    /// Equal-width tabs; returns true when the selection changed.
    pub fn tabs(&mut self, id: WidgetId, r: Rect, labels: &[&str], selected: &mut usize) -> bool {
        if labels.is_empty() {
            return false;
        }
        let mut changed = false;
        let t = self.theme;
        let cells = r.cols(labels.len());
        for (i, (cell, text)) in cells.iter().zip(labels).enumerate() {
            let resp = self.interact(id.with(i as u64), *cell, false);
            if resp.clicked && *selected != i {
                *selected = i;
                changed = true;
            }
            let sel = *selected == i;
            let bg = if sel { t.panel } else if resp.hovered { t.button_hover } else { t.button };
            self.canvas.fill_rect(*cell, bg);
            self.canvas.stroke_rect(*cell, t.border);
            if sel {
                let (bar, _) = cell.split_bottom(2);
                self.canvas.fill_rect(bar, t.accent);
            }
            let fg = if sel { t.text } else { t.text_dim };
            let sc = self.scale();
            self.canvas.draw_text_in(cell.inset_xy(2, 0), text, fg, sc, Align::Center);
        }
        changed
    }

    /// Horizontal bar; `fraction` is clamped to 0..=1. `label` is centred.
    pub fn progress_bar(&mut self, r: Rect, fraction: f32, label: &str) {
        let t = self.theme;
        let f = if fraction.is_nan() { 0.0 } else { fraction.clamp(0.0, 1.0) };
        self.canvas.fill_rect(r, t.bar_bg);
        let inner = r.inset(1);
        let fw = (inner.w.max(0) as f32 * f).round() as i32;
        let fill = if f > 0.9 { t.danger.mix(t.bar_fill, 60) } else { t.bar_fill };
        self.canvas.fill_rect(Rect::new(inner.x, inner.y, fw, inner.h), fill);
        self.canvas.stroke_rect(r, t.border);
        if !label.is_empty() {
            let sc = self.scale();
            self.canvas.draw_text_in(inner, label, t.text, sc, Align::Center);
        }
    }

    /// Show `text` near the mouse at the end of the frame (call while hovered).
    pub fn tooltip(&mut self, text: &str) {
        self.tooltip = Some(text.to_string());
    }

    // ---- text field -----------------------------------------------------

    /// Single-line editable text. Click to focus; typing inserts at the
    /// caret; Backspace/Delete/Left/Right/Home/End edit; Enter sets
    /// `submitted`; Escape unfocuses.
    pub fn text_field(&mut self, id: WidgetId, r: Rect, value: &mut String, placeholder: &str) -> Response {
        let t = self.theme;
        let sc = self.scale();
        let adv = CHAR_ADVANCE * sc;
        let inner = r.inset_xy(3, 0);
        let hr = self.hit_rect(r);
        let mut resp = Response::default();
        let mut focused = self.ui.focused == Some(id) && !self.blocked();
        let now = self.ui.time_ms.unwrap_or(0);
        let mut len = value.chars().count();
        if focused {
            self.ui.text.cursor = self.ui.text.cursor.min(len);
        }
        for i in 0..self.events.len() {
            match self.events[i] {
                InputEvent::MouseDown { x, y, button: MouseButton::Left } if hr.contains(x, y) => {
                    self.down_keeps_focus = true;
                    if !focused {
                        self.ui.focus(id);
                        focused = true;
                    }
                    let rel = x - (inner.x - self.ui.text.scroll);
                    self.ui.text.cursor = (((rel + adv / 2) / adv).max(0) as usize).min(len);
                    self.ui.text.activity_ms = now;
                }
                InputEvent::Char(c) if focused => {
                    self.keys_consumed = true;
                    if !c.is_control() {
                        let cur = self.ui.text.cursor.min(len);
                        let b = byte_at(value, cur);
                        value.insert(b, c);
                        self.ui.text.cursor = cur + 1;
                        len += 1;
                        resp.changed = true;
                        self.ui.text.activity_ms = now;
                    }
                }
                InputEvent::Key(k) if focused => {
                    self.keys_consumed = true;
                    self.ui.text.activity_ms = now;
                    let cur = self.ui.text.cursor.min(len);
                    match k {
                        Key::Backspace if cur > 0 => {
                            let b = byte_at(value, cur - 1);
                            value.remove(b);
                            self.ui.text.cursor = cur - 1;
                            len -= 1;
                            resp.changed = true;
                        }
                        Key::Delete if cur < len => {
                            let b = byte_at(value, cur);
                            value.remove(b);
                            len -= 1;
                            resp.changed = true;
                        }
                        Key::Left => self.ui.text.cursor = cur.saturating_sub(1),
                        Key::Right => self.ui.text.cursor = (cur + 1).min(len),
                        Key::Home | Key::Up | Key::PageUp => self.ui.text.cursor = 0,
                        Key::End | Key::Down | Key::PageDown => self.ui.text.cursor = len,
                        k if ENTER_KEYS.contains(&k) => resp.submitted = true,
                        Key::Escape => {
                            self.ui.focused = None;
                            focused = false;
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        let (mx, my) = self.ui.mouse;
        resp.hovered = self.ui.mouse_known && hr.contains(mx, my);
        if resp.hovered {
            self.ui.hot = Some(id);
        }
        resp.focused = focused;
        resp.shift = self.ui.modifiers.shift;
        resp.ctrl = self.ui.modifiers.ctrl;

        // Keep the caret visible.
        let mut scroll = 0;
        if focused {
            self.focus_seen = true;
            let cur = self.ui.text.cursor.min(len) as i64;
            let caret_px = (cur * adv as i64).min(i32::MAX as i64 / 2) as i32;
            scroll = self.ui.text.scroll;
            let vis = (inner.w - sc).max(0);
            if caret_px - scroll > vis {
                scroll = caret_px - vis;
            }
            if caret_px < scroll {
                scroll = caret_px;
            }
            let total = text_width(value, sc) + sc;
            if total - scroll < vis {
                scroll = (total - vis).max(0);
            }
            scroll = scroll.max(0).min(caret_px);
            self.ui.text.scroll = scroll;
        }

        // Draw.
        self.canvas.fill_rect(r, t.field_bg);
        self.canvas.stroke_rect(r, if focused { t.accent } else if resp.hovered { t.text_dim } else { t.border });
        let ty = inner.y + (inner.h - CAP_HEIGHT * sc) / 2;
        self.canvas.push_clip(inner);
        if value.is_empty() {
            let p = fit_text(placeholder, inner.w, sc);
            self.canvas.draw_text(inner.x, ty, &p, t.text_dim, sc);
        } else if focused {
            self.canvas.draw_text(inner.x - scroll, ty, value, t.text, sc);
        } else {
            let s = fit_text(value, inner.w, sc);
            self.canvas.draw_text(inner.x, ty, &s, t.text, sc);
        }
        if focused {
            let blink_on = match self.ui.time_ms {
                Some(t_ms) => {
                    self.wants_redraw_soon = true;
                    (t_ms.saturating_sub(self.ui.text.activity_ms) / 530) % 2 == 0
                }
                None => true,
            };
            if blink_on {
                let cx = inner.x - scroll + self.ui.text.cursor.min(len) as i32 * adv - (sc + 1) / 2;
                self.canvas.fill_rect(Rect::new(cx, ty - sc, sc.max(1), (GLYPH_HEIGHT - 1) * sc), t.text);
            }
        }
        self.canvas.pop_clip();
        resp
    }

    /// Caret position (in chars) of the focused text field.
    pub fn text_cursor(&self) -> usize {
        self.ui.text.cursor
    }

    // ---- items ----------------------------------------------------------

    /// Emit an item icon placement (skipped when fully outside the clip).
    pub fn emit_item(&mut self, x: i32, y: i32, size: i32, item: &str, label: &str, flags: u8) {
        let clip = self.canvas.clip();
        let r = Rect::new(x, y, size, size);
        if item.is_empty() || !clip.intersects(r) {
            return;
        }
        self.items.push(ItemPlacement { x, y, size, clip, item: item.to_string(), label: label.to_string(), flags });
    }

    /// Icon size for a cell: largest multiple of 16 fitting inside a 1px
    /// border, or 8 when not even 16 fits.
    pub fn item_icon_size(cell: Rect) -> i32 {
        let fit = cell.w.min(cell.h) - 2;
        if fit >= 16 {
            fit / 16 * 16
        } else {
            8
        }
    }

    /// An inventory-style slot showing `item` (drawn by the client overlay)
    /// with `label` (count text, also drawn by the client).
    pub fn item_cell(&mut self, id: WidgetId, r: Rect, item: &str, label: &str, selected: bool) -> Response {
        let resp = self.interact(id, r, false);
        let t = self.theme;
        let bg = if resp.pressed && resp.hovered {
            t.button_active
        } else if resp.hovered {
            t.slot_hover
        } else {
            t.slot
        };
        let slot = r.inset(1);
        self.canvas.fill_rect(slot, bg);
        if selected {
            self.canvas.stroke_rect(slot, t.selection);
        } else {
            self.canvas.stroke_rect(slot, t.border);
        }
        let size = Self::item_icon_size(r);
        let mut flags = 0;
        if selected {
            flags |= ITEM_FLAG_SELECTED;
        }
        if resp.hovered {
            flags |= ITEM_FLAG_HOVERED;
        }
        let x = r.x + (r.w - size) / 2;
        let y = r.y + (r.h - size) / 2;
        self.emit_item(x, y, size, item, label, flags);
        resp
    }

    // ---- scrolling ------------------------------------------------------

    /// Scroll bookkeeping shared by grid/list/scroll_area. Returns
    /// `(view rect, offset, max_offset, bar rect if any)`.
    fn scroll_begin(&mut self, id: WidgetId, r: Rect, content_h: i64, step: i32) -> (Rect, i32, i32, Option<Rect>) {
        let t = self.theme;
        let sbw = t.scrollbar_width.max(2);
        let r = Rect::new(r.x, r.y, r.w.max(0), r.h.max(0));
        let content_h = content_h.clamp(0, i32::MAX as i64) as i32;
        let has_bar = content_h > r.h && r.w > sbw * 2;
        let (bar, view) = if has_bar { r.split_right(sbw) } else { (Rect::EMPTY, r) };
        let max_off = (content_h - r.h).max(0);
        let mut off = self.ui.scroll_offset(id).clamp(0, max_off);
        let hr = self.hit_rect(r);
        let bar_id = id.with(0xB0B);
        let hbar = self.hit_rect(bar);
        let thumb_h = if has_bar {
            ((bar.h as i64 * r.h as i64 / content_h.max(1) as i64) as i32).clamp(sbw.min(bar.h), bar.h)
        } else {
            0
        };
        let travel = (bar.h - thumb_h).max(0);
        let thumb_y = |off: i32| -> i32 {
            if max_off == 0 {
                bar.y
            } else {
                bar.y + (travel as i64 * off as i64 / max_off as i64) as i32
            }
        };
        let from_y = |y: i32, grab: i32| -> i32 {
            if travel == 0 {
                0
            } else {
                (((y - grab - bar.y) as i64 * max_off as i64 / travel as i64) as i32).clamp(0, max_off)
            }
        };
        let mut dragging = has_bar && self.ui.active == Some(bar_id) && !self.blocked();
        for i in 0..self.events.len() {
            match self.events[i] {
                InputEvent::Scroll { x, y, delta } if hr.contains(x, y) => {
                    off = (off as i64 - delta as i64 * step.max(1) as i64).clamp(0, max_off as i64) as i32;
                }
                InputEvent::MouseDown { x, y, button: MouseButton::Left } if has_bar && hbar.contains(x, y) => {
                    let ty = thumb_y(off);
                    self.ui.drag_grab = if y >= ty && y < ty + thumb_h { y - ty } else { thumb_h / 2 };
                    off = from_y(y, self.ui.drag_grab);
                    dragging = true;
                }
                InputEvent::MouseMove { y, .. } if dragging => off = from_y(y, self.ui.drag_grab),
                InputEvent::MouseUp { y, button: MouseButton::Left, .. } if dragging => {
                    off = from_y(y, self.ui.drag_grab);
                    dragging = false;
                }
                _ => {}
            }
        }
        if dragging && self.ui.buttons[0] {
            self.ui.active = Some(bar_id);
        } else if self.ui.active == Some(bar_id) {
            self.ui.active = None;
        }
        self.ui.scroll.insert(id, off);
        if has_bar {
            self.canvas.fill_rect(bar, t.scrollbar_track);
            let thumb = Rect::new(bar.x, thumb_y(off), bar.w, thumb_h).inset_xy(1, 0);
            let c = if dragging { t.accent } else { t.scrollbar_thumb };
            self.canvas.fill_rect(thumb, c);
        }
        (view, off, max_off, if has_bar { Some(bar) } else { None })
    }

    /// Free-form vertical scroll area. `body` gets the content rect (full
    /// content height, already shifted by the scroll offset) and is clipped
    /// to the view.
    pub fn scroll_area<R>(&mut self, id: WidgetId, r: Rect, content_h: i32, body: impl FnOnce(&mut Frame<'a>, Rect) -> R) -> R {
        let step = (LINE_STEP * self.scale()).max(1);
        let (view, off, _, _) = self.scroll_begin(id, r, content_h as i64, step);
        self.canvas.push_clip(view);
        let out = body(self, Rect::new(view.x, view.y - off, view.w, content_h.max(0)));
        self.canvas.pop_clip();
        out
    }

    /// Virtualised grid of `count` cells of `cell_w`x`cell_h`. Columns fill
    /// the width (min 1, centred); the closure runs only for (partly)
    /// visible cells, clipped to the grid. Wheel scrolls one row per notch.
    pub fn grid(&mut self, id: WidgetId, r: Rect, cell_w: i32, cell_h: i32, count: usize, cell: impl FnMut(&mut Frame<'a>, usize, Rect)) -> GridResponse {
        self.grid_impl(id, r, Some(cell_w), cell_h, count, cell_h, cell)
    }

    /// Virtualised list of full-width rows. Wheel scrolls 3 rows per notch.
    pub fn list(&mut self, id: WidgetId, r: Rect, row_h: i32, count: usize, row: impl FnMut(&mut Frame<'a>, usize, Rect)) -> GridResponse {
        let step = row_h.max(1).saturating_mul(3).min(r.h.max(1));
        self.grid_impl(id, r, None, row_h, count, step, row)
    }

    #[allow(clippy::too_many_arguments)]
    fn grid_impl(
        &mut self,
        id: WidgetId,
        r: Rect,
        cell_w: Option<i32>,
        cell_h: i32,
        count: usize,
        step: i32,
        mut cell: impl FnMut(&mut Frame<'a>, usize, Rect),
    ) -> GridResponse {
        let cell_h = cell_h.max(1);
        let sbw = self.theme.scrollbar_width.max(2);
        let r = Rect::new(r.x, r.y, r.w.max(0), r.h.max(0));
        let layout = |w: i32| -> (usize, usize, i64) {
            let cols = match cell_w {
                Some(cw) => (w / cw.max(1)).max(1) as usize,
                None => 1,
            };
            let rows = count.div_ceil(cols);
            (cols, rows, rows as i64 * cell_h as i64)
        };
        let (mut cols, mut rows, mut content_h) = layout(r.w);
        if content_h > r.h as i64 && r.w > sbw * 2 {
            (cols, rows, content_h) = layout(r.w - sbw);
        }
        let (view, off, max_off, _) = self.scroll_begin(id, r, content_h, step);
        let cw = cell_w.map(|c| c.max(1)).unwrap_or(view.w.max(1));
        let pad_x = if cell_w.is_some() { ((view.w - cw * cols as i32) / 2).max(0) } else { 0 };
        let mut resp = GridResponse { cols, rows, offset: off, max_offset: max_off, first_visible: count, visited: 0, hovered: false };
        let (mx, my) = self.ui.mouse;
        resp.hovered = self.ui.mouse_known && self.hit_rect(r).contains(mx, my);
        if view.is_empty() || count == 0 {
            return resp;
        }
        let first_row = (off / cell_h) as usize;
        let last_row = (((off as i64 + view.h as i64 - 1) / cell_h as i64) as usize).min(rows.saturating_sub(1));
        self.canvas.push_clip(view);
        'outer: for row in first_row..=last_row {
            for col in 0..cols {
                let i = row * cols + col;
                if i >= count {
                    break 'outer;
                }
                let cr = Rect::new(
                    view.x + pad_x + col as i32 * cw,
                    (view.y as i64 + row as i64 * cell_h as i64 - off as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                    cw,
                    cell_h,
                );
                if resp.visited == 0 {
                    resp.first_visible = i;
                }
                resp.visited += 1;
                cell(self, i, cr);
            }
        }
        self.canvas.pop_clip();
        resp
    }

    // ---- modal ----------------------------------------------------------

    /// Draw modal `id` if open: dims everything drawn so far, drops item
    /// placements it covers (others get [`ITEM_FLAG_DIMMED`]), draws a titled
    /// panel and runs `body` with the content rect. Widgets outside the
    /// modal receive no input while it is open. Returns `None` when closed.
    pub fn modal<R>(&mut self, id: WidgetId, r: Rect, title: &str, body: impl FnOnce(&mut Frame<'a>, Rect) -> R) -> Option<R> {
        let st = match self.ui.modal {
            Some(m) if m.id == id => m,
            _ => return None,
        };
        let t = self.theme;
        let sc = self.scale();
        self.items.retain(|p| !p.rect().intersect(p.clip).intersects(r));
        for p in &mut self.items {
            p.flags |= ITEM_FLAG_DIMMED;
        }
        let full = self.canvas.bounds();
        self.canvas.reset_clip();
        self.canvas.fill_rect(full, t.backdrop);
        self.canvas.fill_rect(r, t.panel);
        self.canvas.stroke_rect(r, t.accent);
        let title_h = (GLYPH_HEIGHT * sc + 4).min(r.h.max(0));
        let (bar, content) = r.inset(1).split_top(title_h);
        self.canvas.fill_rect(bar, t.button);
        let prev = self.in_modal;
        self.in_modal = true;
        let (close_r, title_r) = if st.opts.close_button { bar.split_right(bar.h) } else { (Rect::EMPTY, bar) };
        self.canvas.draw_text_in(title_r.inset_xy(3, 0), title, t.text, sc, Align::Left);
        if st.opts.close_button {
            let cid = id.with(0xC105E);
            let resp = self.interact(cid, close_r, false);
            let bg = if resp.hovered { t.danger } else { t.button };
            self.canvas.fill_rect(close_r, bg);
            self.canvas.draw_text_in(close_r, "x", t.text, sc, Align::Center);
            if resp.clicked {
                self.ui.close_modal();
            }
        }
        self.canvas.push_clip(content);
        let out = body(self, content.inset(2));
        self.canvas.pop_clip();
        self.in_modal = prev;
        let fresh = st.opened_frame == self.ui.frame;
        if self.ui.is_modal_open(id) && !fresh {
            let outside = st.opts.close_on_outside_click
                && self.events.iter().any(|e| matches!(*e, InputEvent::MouseDown { x, y, .. } if !r.contains(x, y)));
            let esc = st.opts.close_on_escape
                && !self.keys_consumed
                && self.ui.focused.is_none()
                && self.events.contains(&InputEvent::Key(Key::Escape));
            if outside || esc {
                self.ui.close_modal();
            }
        }
        Some(out)
    }

    // ---- on-screen keyboard ---------------------------------------------

    /// Compact QWERTY keyboard filling `r` (5 rows). Taps are returned (first
    /// one) and, when a text field is focused, injected into it on the next
    /// frame (`FrameOutput::redraw_now` is set so the caller re-runs at once).
    /// Pressing it does not steal focus from the field.
    pub fn on_screen_keyboard(&mut self, id: WidgetId, r: Rect) -> Option<KeyOrChar> {
        let t = self.theme;
        let sc = self.scale();
        let hr = self.hit_rect(r);
        if self.events.iter().any(|e| matches!(*e, InputEvent::MouseDown { x, y, .. } if hr.contains(x, y))) {
            self.down_keeps_focus = true;
        }
        let shift = self.ui.osk_shift;
        let mut outputs: Vec<KeyOrChar> = Vec::new();
        let mut toggle_shift = false;
        let rows = r.rows(OSK_ROWS.len());
        let mut kidx = 0u64;
        for (row_r, row) in rows.iter().zip(OSK_ROWS.iter()) {
            let mut u = 0i64;
            for key in row.iter() {
                let x0 = row_r.x as i64 + row_r.w as i64 * u / OSK_UNITS;
                u += key.units as i64;
                let x1 = row_r.x as i64 + row_r.w as i64 * u / OSK_UNITS;
                let kr = Rect::new(x0 as i32, row_r.y, (x1 - x0) as i32 - 1, row_r.h - 1);
                let resp = self.interact(id.with(kidx), kr, true);
                kidx += 1;
                let (label, short, out): (String, &str, Option<KeyOrChar>) = match key.kind {
                    OskKind::Char(lo, hi) => {
                        let c = if shift { hi } else { lo };
                        (c.to_string(), "", Some(KeyOrChar::Char(c)))
                    }
                    OskKind::Key(k, name, short) => (name.to_string(), short, Some(KeyOrChar::Key(k))),
                    OskKind::Space => ("Space".to_string(), "", Some(KeyOrChar::Char(' '))),
                    OskKind::Shift => ("Shift".to_string(), "^", None),
                };
                if resp.clicked {
                    match out {
                        Some(o) => outputs.push(o),
                        None => toggle_shift = true,
                    }
                }
                let lit = matches!(key.kind, OskKind::Shift) && shift;
                let bg = if resp.pressed && resp.hovered {
                    t.button_active
                } else if lit {
                    t.accent
                } else if resp.hovered {
                    t.button_hover
                } else {
                    t.button
                };
                self.canvas.fill_rect(kr, bg);
                let text = if text_width(&label, sc) <= kr.w - 2 { label } else { short.to_string() };
                self.canvas.draw_text_in(kr, &text, t.text, sc, Align::Center);
            }
        }
        if toggle_shift {
            self.ui.osk_shift = !self.ui.osk_shift;
        }
        if outputs.iter().any(|o| matches!(o, KeyOrChar::Char(c) if c.is_alphabetic())) && shift && !toggle_shift {
            self.ui.osk_shift = false;
        }
        if self.ui.focused.is_some() && !outputs.is_empty() {
            for o in &outputs {
                self.ui.inject(*o);
            }
            self.redraw_now = true;
        }
        if toggle_shift {
            self.redraw_now = true;
        }
        outputs.first().copied()
    }

    // ---- end ------------------------------------------------------------

    /// Finish the frame: resolve focus/active state, draw the tooltip, and
    /// return the item placements.
    pub fn end(self) -> FrameOutput {
        let t = self.theme;
        // A press that landed on neither a text field nor the on-screen
        // keyboard unfocuses; so does not drawing the focused field.
        if (self.any_left_down && !self.down_keeps_focus) || !self.focus_seen {
            self.ui.focused = None;
        }
        if !self.ui.buttons[0] {
            self.ui.active = None;
        }
        if let Some(tip) = &self.tooltip {
            let sc = t.text_scale.max(1);
            self.canvas.reset_clip();
            let b = self.canvas.bounds();
            let w = (text_width(tip, sc) + 6).min(b.w);
            let h = GLYPH_HEIGHT * sc + 4;
            let (mx, my) = self.ui.mouse;
            let x = (mx + 8).min(b.w - w).max(0);
            let mut y = my + 12;
            if y + h > b.h {
                y = (my - h - 2).max(0);
            }
            let tr = Rect::new(x, y, w, h);
            self.canvas.fill_rect(tr, t.tooltip_bg);
            self.canvas.stroke_rect(tr, t.border);
            self.canvas.draw_text_in(tr.inset_xy(3, 0), tip, t.text, sc, Align::Left);
        }
        self.canvas.reset_clip();
        let items_changed = self.items != self.ui.last_items;
        if items_changed {
            self.ui.last_items = self.items.clone();
        }
        FrameOutput {
            items: self.items,
            items_changed,
            wants_redraw_soon: self.wants_redraw_soon,
            redraw_now: self.redraw_now,
            had_input: self.had_input,
        }
    }
}

const LINE_STEP: i32 = 30;

fn byte_at(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(b, _)| b).unwrap_or(s.len())
}

#[derive(Clone, Copy)]
enum OskKind {
    Char(char, char),
    Key(Key, &'static str, &'static str),
    Space,
    Shift,
}

#[derive(Clone, Copy)]
struct OskKey {
    kind: OskKind,
    units: u8,
}

const fn ch(lo: char, hi: char) -> OskKey {
    OskKey { kind: OskKind::Char(lo, hi), units: 1 }
}

const OSK_UNITS: i64 = 11;
const OSK_ROWS: [&[OskKey]; 5] = [
    &[
        ch('1', '!'), ch('2', '@'), ch('3', '#'), ch('4', '$'), ch('5', '%'), ch('6', '^'),
        ch('7', '&'), ch('8', '*'), ch('9', '('), ch('0', ')'),
        OskKey { kind: OskKind::Key(Key::Backspace, "Bksp", "<"), units: 1 },
    ],
    &[
        ch('q', 'Q'), ch('w', 'W'), ch('e', 'E'), ch('r', 'R'), ch('t', 'T'), ch('y', 'Y'),
        ch('u', 'U'), ch('i', 'I'), ch('o', 'O'), ch('p', 'P'), ch('-', '_'),
    ],
    &[
        ch('a', 'A'), ch('s', 'S'), ch('d', 'D'), ch('f', 'F'), ch('g', 'G'), ch('h', 'H'),
        ch('j', 'J'), ch('k', 'K'), ch('l', 'L'),
        OskKey { kind: OskKind::Key(Key::Enter, "Enter", "OK"), units: 2 },
    ],
    &[
        OskKey { kind: OskKind::Shift, units: 2 },
        ch('z', 'Z'), ch('x', 'X'), ch('c', 'C'), ch('v', 'V'), ch('b', 'B'), ch('n', 'N'),
        ch('m', 'M'), ch(',', ';'), ch('.', '>'),
    ],
    &[
        ch(':', '"'), ch('/', '?'),
        OskKey { kind: OskKind::Space, units: 7 },
        OskKey { kind: OskKind::Key(Key::Left, "<", "<"), units: 1 },
        OskKey { kind: OskKind::Key(Key::Right, ">", ">"), units: 1 },
    ],
];

/// Draw-free text helper re-exported for callers laying out labels.
pub fn label_width(s: &str, theme: &Theme) -> i32 {
    text_width(s, theme.text_scale)
}
