//! The storage browser: state, drawing and actions. Host-independent (the
//! network is behind [`Backend`]), so it runs and is tested natively.

use ecm_host_abi::storage::{short_count, Cell, Item, Port, Sort, TokenInfo};
use ecm_ui::{id, Align, Canvas, Frame, FrameOutput, InputEvent, Key, ModalOptions, Rect, Theme, Ui, WidgetId};

/// What the app needs from the storage network.
pub trait Backend {
    fn items(&mut self, query: &str, sort: Sort) -> Result<Vec<Item>, String>;
    fn cells(&mut self) -> Result<Vec<Cell>, String>;
    /// Decoders in reach (items come out of these).
    fn decoders(&mut self) -> Result<Vec<Port>, String>;
    /// Send up to `count` out of `decoder`; returns how many left.
    fn extract(&mut self, key: &str, count: i64, decoder: Option<&str>) -> Result<i64, String>;
    fn tokens(&mut self) -> Result<Vec<TokenInfo>, String>;
    fn lost(&mut self) -> Result<Vec<TokenInfo>, String>;
    fn claim_lost(&mut self) -> Result<i64, String>;
}

const ID_TABS: WidgetId = WidgetId(0x7461_6273);
const ID_QUIT: WidgetId = WidgetId(0x7175_6974);
const ID_SEARCH: WidgetId = WidgetId(0x7365_6172);
const ID_SORT: WidgetId = WidgetId(0x736f_7274);
const ID_KBD: WidgetId = WidgetId(0x6b62_6474);
const ID_OSK: WidgetId = WidgetId(0x6f73_6b00);
const ID_GRID: WidgetId = WidgetId(0x6772_6964);
const ID_CELLS: WidgetId = WidgetId(0x6365_6c6c);
const ID_TOKENS: WidgetId = WidgetId(0x746f_6b6e);
const ID_CLAIM: WidgetId = WidgetId(0x636c_6169);
const ID_MODAL: WidgetId = WidgetId(0x6d6f_6461);
const ID_AMOUNT: WidgetId = WidgetId(0x616d_6f75);
const ID_TAKE: WidgetId = WidgetId(0x7461_6b65);
const ID_DECODER: WidgetId = WidgetId(0x6465_636f);

/// Quick amounts in the request dialog.
const AMOUNTS: [i64; 6] = [1, 8, 16, 32, 64, 256];
/// Full stack for shift-click / middle-click.
const STACK: i64 = 64;

pub const TAB_CELLS: usize = 1;
pub const TAB_TOKENS: usize = 2;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Request { key: String, name: String, count: i64 },
    ClaimLost,
    Quit,
}

/// Everything the widgets read and write (kept apart from the Ui/Canvas so a
/// frame can borrow both).
#[derive(Default)]
pub struct View {
    pub tab: usize,
    pub query: String,
    pub sort: usize,
    pub items: Vec<Item>,
    pub cells: Vec<Cell>,
    pub decoders: Vec<Port>,
    pub decoder: usize,
    pub tokens: Vec<TokenInfo>,
    pub lost: Vec<TokenInfo>,
    pub selected: Option<Item>,
    pub amount: String,
    pub status: String,
    pub status_error: bool,
    pub touch: bool,
    pub keyboard: bool,
    pub load_error: Option<String>,
    /// The query or sort changed: re-list items.
    pub requery: bool,
}

const SORTS: [(Sort, &str); 3] = [(Sort::Count, "Count"), (Sort::Name, "Name"), (Sort::Mod, "Mod")];

pub struct App {
    pub ui: Ui,
    pub canvas: Canvas,
    pub theme: Theme,
    pub view: View,
    pub quit: bool,
    /// Data changed (refresh done) since the last frame: redraw.
    pub dirty: bool,
}

impl App {
    pub fn new(width: i32, height: i32, touch: bool) -> App {
        App {
            ui: Ui::new(),
            canvas: Canvas::new(width, height),
            theme: Theme::for_size(width, height),
            view: View { touch, ..View::default() },
            quit: false,
            dirty: true,
        }
    }

    /// Reload what's shown from the network.
    pub fn refresh(&mut self, b: &mut dyn Backend) {
        let v = &mut self.view;
        v.requery = false;
        match b.items(v.query.trim(), SORTS[v.sort % SORTS.len()].0) {
            Ok(items) => {
                v.items = items;
                v.load_error = None;
            }
            Err(e) => {
                v.items.clear();
                v.load_error = Some(e);
            }
        }
        v.cells = b.cells().unwrap_or_default();
        v.decoders = b.decoders().unwrap_or_default();
        if v.decoder >= v.decoders.len() {
            v.decoder = 0;
        }
        v.tokens = b.tokens().unwrap_or_default();
        v.lost = b.lost().unwrap_or_default();
        if let Some(sel) = &v.selected {
            if let Some(now) = v.items.iter().find(|i| i.key == sel.key) {
                v.selected = Some(now.clone());
            }
        }
        self.dirty = true;
    }

    /// One frame: draw, handle input, then carry out what was asked.
    pub fn frame(&mut self, events: &[InputEvent], b: &mut dyn Backend) -> FrameOutput {
        let mut actions = Vec::new();
        let out = {
            let mut f = self.ui.begin_frame(&mut self.canvas, events, &self.theme);
            draw(&mut f, &mut self.view, &mut actions);
            f.end()
        };
        self.dirty = false;
        for a in actions {
            self.run(a, b);
        }
        if self.view.requery {
            self.refresh(b);
        }
        out
    }

    fn run(&mut self, a: Action, b: &mut dyn Backend) {
        let v = &mut self.view;
        match a {
            Action::Quit => self.quit = true,
            Action::Request { key, name, count } => {
                let dec = v.decoders.get(v.decoder).map(|p| p.name.clone());
                match b.extract(&key, count, dec.as_deref()) {
                    Ok(0) => set_status(v, &format!("Nothing sent: the decoder is full or {} is gone", name), true),
                    Ok(n) => set_status(v, &format!("Sent {} x {}{}", n, name,
                        dec.map(|d| format!(" to {}", d)).unwrap_or_default()), false),
                    Err(e) => set_status(v, &e, true),
                }
                self.refresh(b);
            }
            Action::ClaimLost => {
                match b.claim_lost() {
                    Ok(n) => set_status(v, &format!("Put {} lost items back", n), false),
                    Err(e) => set_status(v, &e, true),
                }
                self.refresh(b);
            }
        }
    }
}

fn set_status(v: &mut View, s: &str, error: bool) {
    v.status = s.to_string();
    v.status_error = error;
}

fn draw(f: &mut Frame, v: &mut View, actions: &mut Vec<Action>) {
    let t = f.theme();
    let ch = t.control_height();
    f.clear();
    let root = f.rect();

    let (top, rest) = root.split_top(ch + 2);
    let (quit_r, tabs_r) = top.split_right(ch + 4);
    let before = v.tab;
    f.tabs(ID_TABS, tabs_r, &["Items", "Cells", "Tokens"], &mut v.tab);
    if v.tab != before {
        f.ui().unfocus();
    }
    if f.button(ID_QUIT, quit_r.inset(1), "X").clicked {
        actions.push(Action::Quit);
    }
    let (status_r, body) = rest.split_bottom(ch);
    let status_color = if v.status_error { t.danger } else { t.text_dim };
    let status = if v.status.is_empty() { summary(v) } else { v.status.clone() };
    f.label(status_r.inset_xy(2, 0), &status, status_color, Align::Left);

    match v.tab {
        TAB_CELLS => cells_view(f, v, body.inset(1)),
        TAB_TOKENS => tokens_view(f, v, body.inset(1), actions),
        _ => items_view(f, v, body.inset(1), actions),
    }
    request_modal(f, v, root, actions);

    if !v.touch && !f.blocked() && f.ui().focused().is_none() && f.key_pressed(Key::Escape) {
        actions.push(Action::Quit);
    }
}

fn summary(v: &View) -> String {
    let total: i64 = v.items.iter().map(|i| i.count).sum();
    format!("{} types, {} items in {} cells", v.items.len(), short_count(total), v.cells.len())
}

fn items_view(f: &mut Frame, v: &mut View, r: Rect, actions: &mut Vec<Action>) {
    let t = f.theme();
    let ch = t.control_height();
    let sc = t.text_scale.max(1);
    let (bar, mut grid_r) = r.split_top(ch + 2);
    let sort_w = ecm_ui::label_width("Sort: Count", t) + 8;
    let kbd_w = if v.touch { ecm_ui::label_width("Keys", t) + 8 } else { 0 };
    let (right, field_r) = bar.split_right(sort_w + kbd_w);
    let (sort_r, kbd_r) = right.split_left(sort_w);

    let resp = f.text_field(ID_SEARCH, field_r.inset(1), &mut v.query, "Search items");
    if resp.changed {
        v.requery = true;
        f.scroll_to_top(ID_GRID);
    }
    if resp.focused && v.touch {
        v.keyboard = true;
    }
    let label = format!("Sort: {}", SORTS[v.sort % SORTS.len()].1);
    if f.button(ID_SORT, sort_r.inset(1), &label).clicked {
        v.sort = (v.sort + 1) % SORTS.len();
        v.requery = true;
        f.scroll_to_top(ID_GRID);
    }
    if v.touch && f.button(ID_KBD, kbd_r.inset(1), "Keys").clicked {
        v.keyboard = !v.keyboard;
        if v.keyboard {
            f.ui().focus(ID_SEARCH);
        }
    }
    if v.touch && v.keyboard {
        let (kb, g) = grid_r.split_bottom(grid_r.h * 2 / 5);
        grid_r = g;
        f.on_screen_keyboard(ID_OSK, kb);
    }

    if v.items.is_empty() {
        let msg = match &v.load_error {
            Some(e) => e.clone(),
            None if !v.query.trim().is_empty() => "No items match".to_string(),
            None => "No items stored. Feed an Item Encoder to add some.".to_string(),
        };
        f.label(grid_r, &msg, t.text_dim, Align::Center);
        return;
    }

    let cell = (16 * sc + 6).max(20);
    let items = &v.items;
    let touch = v.touch;
    let mut open: Option<Item> = None;
    f.grid(ID_GRID, grid_r, cell, cell, items.len(), |f, i, cr| {
        let it = &items[i];
        let resp = f.item_cell(id("cell", i as u64), cr, &it.key, &short_count(it.count), false);
        if touch {
            if resp.clicked || resp.right_clicked {
                open = Some(it.clone());
            }
        } else if resp.clicked {
            let count = if resp.shift { STACK } else { 1 };
            actions.push(Action::Request { key: it.key.clone(), name: it.name.clone(), count });
        } else if resp.middle_clicked {
            actions.push(Action::Request { key: it.key.clone(), name: it.name.clone(), count: STACK });
        } else if resp.right_clicked {
            open = Some(it.clone());
        }
        if resp.hovered && !touch {
            f.tooltip(&format!("{} x{}", it.name, it.count));
        }
    });
    if let Some(it) = open {
        v.amount.clear();
        v.selected = Some(it);
        f.ui().open_modal_with(ID_MODAL, ModalOptions { close_on_outside_click: true, close_on_escape: true, close_button: true });
    }
}

fn request_modal(f: &mut Frame, v: &mut View, root: Rect, actions: &mut Vec<Action>) {
    let Some(sel) = v.selected.clone() else { return };
    let t = f.theme();
    let ch = t.control_height();
    let w = (root.w - 8).min(ch * 12).max(ch * 6);
    let rows = if v.touch && v.keyboard { 6 } else { 5 };
    let h = (root.h - 4).min(ch * rows + ch + 8);
    let mr = root.center_fit(w, h);
    let res = f.modal(ID_MODAL, mr, &sel.name, |f, c| {
        let mut out: Option<Action> = None;
        let rows = c.rows(rows as usize);
        f.label(rows[0], &format!("Stored: {}   ({})", sel.count, sel.id), t.text_dim, Align::Left);
        for (i, cr) in rows[1].cols(AMOUNTS.len()).into_iter().enumerate() {
            if f.button(id("amt", i as u64), cr.inset(1), &AMOUNTS[i].to_string()).clicked {
                out = Some(Action::Request { key: sel.key.clone(), name: sel.name.clone(), count: AMOUNTS[i] });
            }
        }
        let (take_r, field_r) = rows[2].split_right(ecm_ui::label_width("Take", t) + 10);
        let resp = f.text_field(ID_AMOUNT, field_r.inset(1), &mut v.amount, "Amount");
        let take = f.button_primary(ID_TAKE, take_r.inset(1), "Take").clicked || resp.submitted;
        if take {
            match v.amount.trim().parse::<i64>() {
                Ok(n) if n > 0 => out = Some(Action::Request { key: sel.key.clone(), name: sel.name.clone(), count: n }),
                _ => set_status(v, "Type a positive amount", true),
            }
        }
        let dec_label = match v.decoders.get(v.decoder) {
            Some(p) => format!("Out: {}", p.name),
            None => "No decoder in reach".to_string(),
        };
        if v.decoders.len() > 1 {
            if f.button(ID_DECODER, rows[3].inset(1), &dec_label).clicked {
                v.decoder = (v.decoder + 1) % v.decoders.len();
            }
        } else {
            let color = if v.decoders.is_empty() { t.danger } else { t.text_dim };
            f.label(rows[3], &dec_label, color, Align::Left);
        }
        if f.button(id("all", 0), rows[4].inset(1), &format!("All ({})", short_count(sel.count))).clicked {
            out = Some(Action::Request { key: sel.key.clone(), name: sel.name.clone(), count: sel.count.max(1) });
        }
        if v.touch && v.keyboard && rows.len() > 5 {
            f.on_screen_keyboard(id("osk2", 0), rows[5]);
        }
        out
    });
    match res {
        None => v.selected = None,
        Some(Some(a)) => {
            actions.push(a);
            f.close_modal();
            v.selected = None;
        }
        Some(None) => {}
    }
}

fn cells_view(f: &mut Frame, v: &mut View, r: Rect) {
    let t = f.theme();
    let ch = t.control_height();
    if v.cells.is_empty() {
        f.label(r, "No cells in reach. Put a Storage Cell in a Drive or a Storage Module.", t.text_dim, Align::Center);
        return;
    }
    let cells = &v.cells;
    f.list(ID_CELLS, r, ch * 2 + 4, cells.len(), |f, i, row| {
        let c = &cells[i];
        let (head, bars) = row.inset(1).split_top(ch);
        f.label(head, &format!("{} cell {}  in {} slot {}  ({} items)", c.tier, c.short, c.device, c.slot + 1, short_count(c.items)),
            t.text, Align::Left);
        let halves = bars.cols(2);
        let bytes = if c.bytes_total > 0 { c.bytes_used as f32 / c.bytes_total as f32 } else { 0.0 };
        let types = if c.types_total > 0 { c.types_used as f32 / c.types_total as f32 } else { 0.0 };
        f.progress_bar(halves[0].inset(1), bytes, &format!("{}/{} B", c.bytes_used, c.bytes_total));
        f.progress_bar(halves[1].inset(1), types, &format!("{}/{} types", c.types_used, c.types_total));
    });
}

fn tokens_view(f: &mut Frame, v: &mut View, r: Rect, actions: &mut Vec<Action>) {
    let t = f.theme();
    let ch = t.control_height();
    let mut rows: Vec<String> = Vec::new();
    for tk in &v.tokens {
        rows.push(format!("{} x{}  expires in {}s  {}..", tk.name, tk.count, tk.expires_in, &tk.token[..tk.token.len().min(14)]));
    }
    let (list_r, claim_r) = if v.lost.is_empty() {
        (r, Rect::EMPTY)
    } else {
        let (claim, list) = r.split_bottom(ch + 2);
        (list, claim)
    };
    if !v.lost.is_empty() {
        let n: i64 = v.lost.iter().map(|l| l.count).sum();
        if f.button_primary(ID_CLAIM, claim_r.inset(1), &format!("Claim {} lost items", short_count(n))).clicked {
            actions.push(Action::ClaimLost);
        }
    }
    if rows.is_empty() {
        f.label(list_r, "No unspent tokens from these cells", t.text_dim, Align::Center);
        return;
    }
    f.list(ID_TOKENS, list_r, ch, rows.len(), |f, i, row| {
        f.label(row.inset_xy(2, 0), &rows[i], t.text, Align::Left);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecm_ui::MouseButton;

    #[derive(Default)]
    struct Fake {
        items: Vec<Item>,
        extracted: Vec<(String, i64, Option<String>)>,
        queries: Vec<String>,
    }

    impl Backend for Fake {
        fn items(&mut self, query: &str, _sort: Sort) -> Result<Vec<Item>, String> {
            self.queries.push(query.to_string());
            Ok(self.items.iter().filter(|i| i.name.to_lowercase().contains(&query.to_lowercase())).cloned().collect())
        }
        fn cells(&mut self) -> Result<Vec<Cell>, String> {
            Ok(vec![Cell { id: "c".into(), short: "abcd1234".into(), tier: "1k".into(), device: "drive".into(), bytes_total: 1024, types_total: 63, ..Cell::default() }])
        }
        fn decoders(&mut self) -> Result<Vec<Port>, String> {
            Ok(vec![Port { name: "back".into(), kind: "decoder".into(), id: "d".into() }])
        }
        fn extract(&mut self, key: &str, count: i64, decoder: Option<&str>) -> Result<i64, String> {
            self.extracted.push((key.to_string(), count, decoder.map(str::to_string)));
            Ok(count)
        }
        fn tokens(&mut self) -> Result<Vec<TokenInfo>, String> {
            Ok(vec![])
        }
        fn lost(&mut self) -> Result<Vec<TokenInfo>, String> {
            Ok(vec![])
        }
        fn claim_lost(&mut self) -> Result<i64, String> {
            Ok(0)
        }
    }

    fn fake() -> Fake {
        Fake {
            items: vec![
                Item { key: "k1".into(), id: "minecraft:iron_ingot".into(), name: "Iron Ingot".into(), mod_id: "minecraft".into(), count: 100 },
                Item { key: "k2".into(), id: "minecraft:diamond".into(), name: "Diamond".into(), mod_id: "minecraft".into(), count: 5 },
            ],
            ..Fake::default()
        }
    }

    fn click(x: i32, y: i32, button: MouseButton) -> Vec<InputEvent> {
        vec![InputEvent::MouseMove { x, y }, InputEvent::MouseDown { x, y, button }, InputEvent::MouseUp { x, y, button }]
    }

    fn first_item_center(out: &FrameOutput) -> (i32, i32) {
        let p = &out.items[0];
        (p.x + p.size / 2, p.y + p.size / 2)
    }

    #[test]
    fn lists_items_as_overlays() {
        let mut b = fake();
        let mut app = App::new(320, 200, false);
        app.refresh(&mut b);
        let out = app.frame(&[], &mut b);
        assert_eq!(out.items.len(), 2);
        assert_eq!(out.items[0].item, "k1");
        assert_eq!(out.items[0].label, "100");
    }

    #[test]
    fn click_requests_one_and_shift_click_a_stack() {
        let mut b = fake();
        let mut app = App::new(320, 200, false);
        app.refresh(&mut b);
        let out = app.frame(&[], &mut b);
        let (x, y) = first_item_center(&out);
        app.frame(&click(x, y, MouseButton::Left), &mut b);
        assert_eq!(b.extracted, vec![("k1".to_string(), 1, Some("back".to_string()))]);
        let mut ev = vec![InputEvent::Modifiers(ecm_ui::Modifiers { shift: true, ctrl: false })];
        ev.extend(click(x, y, MouseButton::Left));
        app.frame(&ev, &mut b);
        assert_eq!(b.extracted[1].1, STACK);
    }

    #[test]
    fn touch_opens_dialog_and_amount_buttons_request() {
        let mut b = fake();
        let mut app = App::new(320, 200, true);
        app.refresh(&mut b);
        let out = app.frame(&[], &mut b);
        let (x, y) = first_item_center(&out);
        app.frame(&click(x, y, MouseButton::Left), &mut b);
        assert!(b.extracted.is_empty(), "a tap should open the dialog, not request");
        assert!(app.view.selected.is_some());
        // Frame again so the modal lays out, then find the "16" button by scanning the dialog row.
        let _ = app.frame(&[], &mut b);
        let mut hit = false;
        'scan: for yy in (40..170).step_by(4) {
            for xx in (60..260).step_by(4) {
                let before = b.extracted.len();
                let snapshot = app.view.selected.clone();
                app.frame(&click(xx, yy, MouseButton::Left), &mut b);
                if b.extracted.len() > before {
                    hit = true;
                    break 'scan;
                }
                if app.view.selected.is_none() {
                    // Clicked outside/close: reopen and keep scanning.
                    app.view.selected = snapshot;
                    app.ui.open_modal_with(ID_MODAL, ModalOptions { close_on_outside_click: true, close_on_escape: true, close_button: true });
                    let _ = app.frame(&[], &mut b);
                }
            }
        }
        assert!(hit, "no amount button requested anything");
        assert_eq!(b.extracted[0].0, "k1");
    }

    #[test]
    fn typing_in_search_requeries() {
        let mut b = fake();
        let mut app = App::new(320, 200, false);
        app.refresh(&mut b);
        app.frame(&[], &mut b);
        // Click the search field (top-left under the tabs) and type.
        let ch = app.theme.control_height();
        let mut ev = click(10, ch + 2 + ch / 2 + 1, MouseButton::Left);
        ev.extend([InputEvent::Char('d'), InputEvent::Char('i')]);
        app.frame(&ev, &mut b);
        assert_eq!(app.view.query, "di");
        assert_eq!(b.queries.last().unwrap(), "di");
        let out = app.frame(&[], &mut b);
        assert_eq!(out.items.len(), 1);
        assert_eq!(out.items[0].item, "k2");
    }

    #[test]
    fn cells_and_tokens_tabs_draw() {
        let mut b = fake();
        let mut app = App::new(320, 200, false);
        app.refresh(&mut b);
        app.view.tab = TAB_CELLS;
        let out = app.frame(&[], &mut b);
        assert!(out.items.is_empty());
        app.view.tab = TAB_TOKENS;
        app.frame(&[], &mut b);
    }

    #[test]
    fn escape_quits_on_the_terminal() {
        let mut b = fake();
        let mut app = App::new(320, 200, false);
        app.refresh(&mut b);
        app.frame(&[InputEvent::Key(Key::Escape)], &mut b);
        assert!(app.quit);
    }
}

