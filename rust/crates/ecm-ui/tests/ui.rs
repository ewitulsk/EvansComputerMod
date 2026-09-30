use ecm_ui::*;

fn down(x: i32, y: i32) -> InputEvent {
    InputEvent::MouseDown { x, y, button: MouseButton::Left }
}
fn up(x: i32, y: i32) -> InputEvent {
    InputEvent::MouseUp { x, y, button: MouseButton::Left }
}
fn mv(x: i32, y: i32) -> InputEvent {
    InputEvent::MouseMove { x, y }
}

struct H {
    ui: Ui,
    canvas: Canvas,
    theme: Theme,
}

impl H {
    fn new(w: i32, h: i32) -> H {
        H { ui: Ui::new(), canvas: Canvas::new(w, h), theme: Theme::default() }
    }
    fn frame<R>(&mut self, events: &[InputEvent], body: impl FnOnce(&mut Frame) -> R) -> (R, FrameOutput) {
        let mut f = self.ui.begin_frame(&mut self.canvas, events, &self.theme);
        let r = body(&mut f);
        (r, f.end())
    }
}

// ---- Rect ----------------------------------------------------------------

#[test]
fn rect_split_and_intersect() {
    let r = Rect::new(10, 20, 100, 50);
    let (t, rest) = r.split_top(10);
    assert_eq!(t, Rect::new(10, 20, 100, 10));
    assert_eq!(rest, Rect::new(10, 30, 100, 40));
    let (b, rest) = r.split_bottom(15);
    assert_eq!(b, Rect::new(10, 55, 100, 15));
    assert_eq!(rest, Rect::new(10, 20, 100, 35));
    let (l, rest) = r.split_left(30);
    assert_eq!((l, rest), (Rect::new(10, 20, 30, 50), Rect::new(40, 20, 70, 50)));
    let (rt, rest) = r.split_right(30);
    assert_eq!((rt, rest), (Rect::new(80, 20, 30, 50), Rect::new(10, 20, 70, 50)));
    // Over-large splits clamp.
    assert_eq!(r.split_top(500).1.h, 0);
    assert_eq!(r.split_left(-5).0.w, 0);

    let rows = r.rows(3);
    assert_eq!(rows.iter().map(|r| r.h).sum::<i32>(), 50);
    assert_eq!(rows[0].y, 20);
    assert_eq!(rows[2].bottom(), 70);
    let cols = r.cols(7);
    assert_eq!(cols.iter().map(|r| r.w).sum::<i32>(), 100);

    assert_eq!(r.intersect(Rect::new(0, 0, 20, 30)), Rect::new(10, 20, 10, 10));
    assert!(r.intersect(Rect::new(200, 200, 5, 5)).is_empty());
    assert!(r.contains(10, 20) && !r.contains(110, 20) && !r.contains(10, 70));
    assert_eq!(r.inset(5), Rect::new(15, 25, 90, 40));
    assert_eq!(r.inset(100).w, 0);
    assert_eq!(r.center(20, 10), Rect::new(50, 40, 20, 10));
    // Extreme values don't panic.
    let big = Rect::new(i32::MAX - 1, i32::MAX - 1, i32::MAX, i32::MAX);
    let _ = big.intersect(r);
    let _ = big.inset(-100);
    let _ = big.rows(4);
    let _ = Rect::new(0, 0, -5, -5).cols(3);
    assert!(!Rect::new(0, 0, -5, 5).contains(0, 0));
}

// ---- clicks --------------------------------------------------------------

#[test]
fn click_semantics() {
    let mut h = H::new(100, 100);
    let r = Rect::new(10, 10, 30, 20);
    let bid = WidgetId::new("b");
    // Down + up in same batch inside => click.
    let (resp, _) = h.frame(&[down(15, 15), up(16, 16)], |f| f.button(bid, r, "OK"));
    assert!(resp.clicked);
    // Down inside, up outside (same batch) => no click.
    let (resp, _) = h.frame(&[down(15, 15), up(80, 80)], |f| f.button(bid, r, "OK"));
    assert!(!resp.clicked);
    // Down in one frame, up inside later => click; pressed while held.
    let (resp, _) = h.frame(&[down(15, 15)], |f| f.button(bid, r, "OK"));
    assert!(!resp.clicked && resp.pressed);
    let (resp, _) = h.frame(&[mv(20, 20)], |f| f.button(bid, r, "OK"));
    assert!(resp.pressed && resp.hovered);
    let (resp, _) = h.frame(&[up(20, 20)], |f| f.button(bid, r, "OK"));
    assert!(resp.clicked);
    // Down in frame 1, up outside in frame 2 => no click.
    h.frame(&[down(15, 15)], |f| f.button(bid, r, "OK"));
    let (resp, _) = h.frame(&[up(90, 90)], |f| f.button(bid, r, "OK"));
    assert!(!resp.clicked);
    // Down outside, up inside => no click.
    let (resp, _) = h.frame(&[down(90, 90), up(15, 15)], |f| f.button(bid, r, "OK"));
    assert!(!resp.clicked);
    // Right click and shift modifier.
    let rc = [
        InputEvent::Modifiers(Modifiers { shift: true, ctrl: false }),
        InputEvent::MouseDown { x: 15, y: 15, button: MouseButton::Right },
        InputEvent::MouseUp { x: 15, y: 15, button: MouseButton::Right },
    ];
    let (resp, _) = h.frame(&rc, |f| f.button(bid, r, "OK"));
    assert!(resp.right_clicked && !resp.clicked && resp.shift);
}

#[test]
fn clicks_respect_clip() {
    let mut h = H::new(100, 100);
    let (resp, _) = h.frame(&[down(15, 15), up(15, 15)], |f| {
        f.push_clip(Rect::new(0, 0, 12, 100));
        let r = f.button(WidgetId::new("b"), Rect::new(10, 10, 30, 20), "x");
        f.pop_clip();
        r
    });
    assert!(!resp.clicked);
}

// ---- text field ----------------------------------------------------------

#[test]
fn text_field_editing() {
    let mut h = H::new(200, 50);
    let fid = WidgetId::new("search");
    let r = Rect::new(0, 0, 200, 16);
    let mut s = String::new();
    // Click to focus, then type in the same batch.
    let ev = [down(5, 5), up(5, 5), InputEvent::Char('a'), InputEvent::Char('b'), InputEvent::Char('c')];
    let (resp, _) = h.frame(&ev, |f| f.text_field(fid, r, &mut s, "Search"));
    assert!(resp.focused && resp.changed);
    assert_eq!(s, "abc");
    assert_eq!(h.ui.focused(), Some(fid));

    let ev = [InputEvent::Key(Key::Left), InputEvent::Key(Key::Backspace)];
    h.frame(&ev, |f| f.text_field(fid, r, &mut s, ""));
    assert_eq!(s, "ac");
    let ev = [InputEvent::Char('X'), InputEvent::Key(Key::Home), InputEvent::Char('>'), InputEvent::Key(Key::End), InputEvent::Char('!')];
    h.frame(&ev, |f| f.text_field(fid, r, &mut s, ""));
    assert_eq!(s, ">aXc!");
    let ev = [InputEvent::Key(Key::Home), InputEvent::Key(Key::Right), InputEvent::Key(Key::Delete)];
    h.frame(&ev, |f| f.text_field(fid, r, &mut s, ""));
    assert_eq!(s, ">Xc!");
    // Control chars ignored.
    h.frame(&[InputEvent::Char('\u{8}')], |f| f.text_field(fid, r, &mut s, ""));
    assert_eq!(s, ">Xc!");
    let (resp, _) = h.frame(&[InputEvent::Key(Key::Enter)], |f| f.text_field(fid, r, &mut s, ""));
    assert!(resp.submitted && resp.focused);
    // Keys consumed by the field are not visible to key_pressed.
    let (seen, _) = h.frame(&[InputEvent::Key(Key::Enter)], |f| {
        f.text_field(fid, r, &mut s, "");
        f.key_pressed(Key::Enter)
    });
    assert!(!seen);
    // Escape unfocuses.
    let (resp, _) = h.frame(&[InputEvent::Key(Key::Escape)], |f| f.text_field(fid, r, &mut s, ""));
    assert!(!resp.focused);
    assert_eq!(h.ui.focused(), None);
    // Unfocused: typing does nothing.
    h.frame(&[InputEvent::Char('z')], |f| f.text_field(fid, r, &mut s, ""));
    assert_eq!(s, ">Xc!");
    // Focus then click elsewhere unfocuses.
    h.frame(&[down(5, 5), up(5, 5)], |f| f.text_field(fid, r, &mut s, ""));
    assert_eq!(h.ui.focused(), Some(fid));
    h.frame(&[down(5, 40), up(5, 40)], |f| f.text_field(fid, r, &mut s, ""));
    assert_eq!(h.ui.focused(), None);
}

#[test]
fn text_field_long_text_scrolls_without_panic() {
    let mut h = H::new(40, 12);
    let fid = WidgetId::new("f");
    let mut s = String::new();
    h.ui.focus(fid);
    let ev: Vec<InputEvent> = "the quick brown fox jumps".chars().map(InputEvent::Char).collect();
    h.frame(&ev, |f| f.text_field(fid, Rect::new(0, 0, 40, 12), &mut s, ""));
    assert_eq!(s, "the quick brown fox jumps");
    // Tiny / degenerate rects.
    h.frame(&[InputEvent::Char('x')], |f| f.text_field(fid, Rect::new(0, 0, 0, -3), &mut s, "p"));
}

// ---- grid ----------------------------------------------------------------

#[test]
fn grid_virtualization_and_scroll() {
    let mut h = H::new(200, 100);
    let gid = WidgetId::new("grid");
    let r = Rect::new(0, 0, 106, 100); // 106 - 6 bar = 100 => 5 cols of 20
    let mut visited = Vec::new();
    let (g, _) = h.frame(&[], |f| f.grid(gid, r, 20, 20, 1000, |_, i, _| visited.push(i)));
    assert_eq!(g.cols, 5);
    assert_eq!(g.rows, 200);
    assert_eq!(visited, (0..25).collect::<Vec<_>>());
    assert_eq!(g.max_offset, 200 * 20 - 100);

    // Wheel up at the top clamps at 0.
    let (g, _) = h.frame(&[InputEvent::Scroll { x: 10, y: 10, delta: 1 }], |f| f.grid(gid, r, 20, 20, 1000, |_, _, _| {}));
    assert_eq!(g.offset, 0);
    // Wheel down 3 notches = 3 rows.
    let mut visited = Vec::new();
    let (g, _) = h.frame(&[InputEvent::Scroll { x: 10, y: 10, delta: -3 }], |f| f.grid(gid, r, 20, 20, 1000, |_, i, _| visited.push(i)));
    assert_eq!(g.offset, 60);
    assert_eq!(visited.first(), Some(&15));
    assert_eq!(visited.len(), 25);
    // Wheel outside the grid does nothing.
    let (g, _) = h.frame(&[InputEvent::Scroll { x: 150, y: 10, delta: -3 }], |f| f.grid(gid, r, 20, 20, 1000, |_, _, _| {}));
    assert_eq!(g.offset, 60);
    // Scroll far past the end clamps.
    let (g, _) = h.frame(&[InputEvent::Scroll { x: 10, y: 10, delta: -100000 }], |f| f.grid(gid, r, 20, 20, 1000, |_, _, _| {}));
    assert_eq!(g.offset, g.max_offset);
    let mut last = Vec::new();
    h.frame(&[], |f| f.grid(gid, r, 20, 20, 1000, |_, i, _| last.push(i)));
    assert_eq!(*last.last().unwrap(), 999);
    assert!(last.len() <= 30);

    // scroll_to_top.
    h.ui.scroll_to_top(gid);
    let (g, _) = h.frame(&[], |f| f.grid(gid, r, 20, 20, 1000, |_, _, _| {}));
    assert_eq!(g.offset, 0);

    // Scrollbar drag: press on the bar near the top, drag to the bottom.
    let bar_x = 103;
    let (g, _) = h.frame(&[down(bar_x, 1), mv(bar_x, 50)], |f| f.grid(gid, r, 20, 20, 1000, |_, _, _| {}));
    assert!(g.offset > 0, "drag moved the offset");
    let mid = g.offset;
    let (g, _) = h.frame(&[mv(bar_x, 99), up(bar_x, 99)], |f| f.grid(gid, r, 20, 20, 1000, |_, _, _| {}));
    assert!(g.offset > mid);
    assert_eq!(g.offset, g.max_offset);
    // After release, moves no longer drag.
    let (g2, _) = h.frame(&[mv(bar_x, 0)], |f| f.grid(gid, r, 20, 20, 1000, |_, _, _| {}));
    assert_eq!(g2.offset, g.offset);
}

#[test]
fn grid_item_placements_clip() {
    let mut h = H::new(200, 100);
    let gid = WidgetId::new("grid");
    let r = Rect::new(0, 10, 106, 50);
    h.ui.set_scroll(gid, 10); // half a row scrolled
    let (_, out) = h.frame(&[], |f| {
        f.grid(gid, r, 20, 20, 100, |f, i, cell| {
            f.item_cell(id("cell", i as u64), cell, &format!("minecraft:item{i}"), "64", false);
        })
    });
    let view = Rect::new(0, 10, 100, 50);
    assert!(!out.items.is_empty());
    for p in &out.items {
        assert_eq!(p.clip, view);
        assert!(p.rect().intersects(p.clip));
        assert_eq!(p.size, 16);
        assert_eq!(p.label, "64");
    }
    // Rows 0 (partial), 1, 2 visible; row 3 would start at y = 10 + 60 - 10 = 60 => outside.
    assert_eq!(out.items.len(), 15);
    assert!(out.items_changed);
    let (_, out2) = h.frame(&[], |f| {
        f.grid(gid, r, 20, 20, 100, |f, i, cell| {
            f.item_cell(id("cell", i as u64), cell, &format!("minecraft:item{i}"), "64", false);
        })
    });
    assert!(!out2.items_changed);

    // Direct emit fully outside clip is skipped.
    let (_, out) = h.frame(&[], |f| {
        f.push_clip(Rect::new(0, 0, 10, 10));
        f.emit_item(20, 20, 16, "x", "", 0);
        f.emit_item(5, 5, 16, "y", "", 0);
        f.pop_clip();
    });
    assert_eq!(out.items.len(), 1);
    assert_eq!(out.items[0].item, "y");
    assert_eq!(out.items[0].clip, Rect::new(0, 0, 10, 10));
}

#[test]
fn item_cell_click_and_sizes() {
    let mut h = H::new(100, 100);
    let (resp, out) = h.frame(&[down(10, 10), up(10, 10)], |f| f.item_cell(WidgetId::new("c"), Rect::new(0, 0, 36, 36), "k1", "3", true));
    assert!(resp.clicked);
    assert_eq!(out.items[0].size, 32);
    assert_eq!(out.items[0].flags & ITEM_FLAG_SELECTED, ITEM_FLAG_SELECTED);
    assert_eq!(Frame::item_icon_size(Rect::new(0, 0, 10, 10)), 8);
    assert_eq!(Frame::item_icon_size(Rect::new(0, 0, 18, 40)), 16);
}

#[test]
fn list_visits_visible_rows() {
    let mut h = H::new(100, 100);
    let mut v = Vec::new();
    h.frame(&[], |f| f.list(WidgetId::new("l"), Rect::new(0, 0, 100, 35), 10, 50, |_, i, r| v.push((i, r.w))));
    assert_eq!(v.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(v[0].1, 100 - Theme::default().scrollbar_width);
}

// ---- modal ---------------------------------------------------------------

#[test]
fn modal_blocks_input_beneath() {
    let mut h = H::new(200, 200);
    let mid = WidgetId::new("modal");
    let bid = WidgetId::new("under");
    let inner = WidgetId::new("inner");
    let under = Rect::new(0, 0, 50, 20);
    let mr = Rect::new(60, 60, 100, 80);
    h.ui.open_modal_with(mid, ModalOptions { close_on_outside_click: false, ..Default::default() });
    let mut inner_clicked = false;
    let ((ru, _), _) = h.frame(&[down(10, 10), up(10, 10)], |f| {
        let ru = f.button(bid, under, "under");
        let rm = f.modal(mid, mr, "Title", |f, c| {
            inner_clicked = f.button(inner, c, "in").clicked;
        });
        (ru, rm)
    });
    assert!(!ru.clicked && !ru.hovered);
    assert!(h.ui.is_modal_open(mid));
    // Click inside the modal content works.
    let (_, _) = h.frame(&[down(110, 110), up(110, 110)], |f| {
        f.button(bid, under, "under");
        f.modal(mid, mr, "Title", |f, c| {
            inner_clicked = f.button(inner, c, "in").clicked;
        });
    });
    assert!(inner_clicked);
    // Items beneath the modal are dropped / dimmed.
    let (_, out) = h.frame(&[], |f| {
        f.emit_item(0, 0, 16, "outside", "", 0);
        f.emit_item(80, 80, 16, "covered", "", 0);
        f.modal(mid, mr, "Title", |_, _| {});
    });
    assert_eq!(out.items.len(), 1);
    assert_eq!(out.items[0].item, "outside");
    assert!(out.items[0].flags & ITEM_FLAG_DIMMED != 0);
    // Escape closes.
    h.frame(&[InputEvent::Key(Key::Escape)], |f| {
        f.modal(mid, mr, "Title", |_, _| {});
    });
    assert!(!h.ui.is_modal_open(mid));
    // Once closed, the widget beneath works again.
    let (ru, _) = h.frame(&[down(10, 10), up(10, 10)], |f| f.button(bid, under, "under"));
    assert!(ru.clicked);
}

#[test]
fn modal_outside_click_closes_but_not_on_open_frame() {
    let mut h = H::new(200, 200);
    let mid = WidgetId::new("m");
    let open_btn = WidgetId::new("open");
    let mr = Rect::new(60, 60, 100, 80);
    // Click the opener: the same Down must not immediately close the modal.
    h.frame(&[down(5, 5), up(5, 5)], |f| {
        if f.button(open_btn, Rect::new(0, 0, 20, 20), "o").clicked {
            f.open_modal(mid);
        }
        f.modal(mid, mr, "T", |_, _| {});
    });
    assert!(h.ui.is_modal_open(mid));
    h.frame(&[down(5, 5), up(5, 5)], |f| {
        let r = f.button(open_btn, Rect::new(0, 0, 20, 20), "o");
        assert!(!r.clicked);
        f.modal(mid, mr, "T", |_, _| {});
    });
    assert!(!h.ui.is_modal_open(mid));
}

// ---- canvas / text / dirty -----------------------------------------------

fn count_lit(c: &Canvas) -> usize {
    c.pixels.chunks_exact(4).filter(|p| p[3] != 0).count()
}

#[test]
fn text_rendering() {
    let mut c = Canvas::new(40, 20);
    c.draw_text(0, 0, " ", Color::WHITE, 1);
    assert_eq!(count_lit(&c), 0);
    c.draw_text(0, 0, "a", Color::WHITE, 1);
    assert!(count_lit(&c) > 5);
    // Every printable char except space draws something; unknown chars draw a box.
    for ch in (0x21u8..0x7F).map(|b| b as char).chain(['\u{e9}']) {
        let mut c = Canvas::new(10, 12);
        c.draw_text(0, 0, &ch.to_string(), Color::WHITE, 1);
        assert!(count_lit(&c) > 0, "glyph {ch:?} empty");
    }
    // Descender goes below the cap line.
    let mut c = Canvas::new(10, 12);
    c.draw_text(0, 0, "g", Color::WHITE, 1);
    assert!((0..6).any(|x| c.get_pixel(x, 8).a != 0));
}

#[test]
fn text_width_matches_draw() {
    for scale in 1..=3 {
        for s in ["a", "Hello, World!", "W", "|", "mmmm"] {
            let mut c = Canvas::new(300, 40);
            let end = c.draw_text(2, 1, s, Color::WHITE, scale);
            let w = text_width(s, scale);
            let max_x = (0..300).filter(|&x| (0..40).any(|y| c.get_pixel(x, y).a != 0)).max().unwrap();
            assert!(max_x < 2 + w, "{s:?} x{scale}: ink {max_x} >= {}", 2 + w);
            assert_eq!(end, 2 + w + scale);
        }
        assert_eq!(text_height(scale), 9 * scale);
    }
    assert_eq!(text_width("", 1), 0);
    assert_eq!(fit_text("hello world", 1000, 1), "hello world");
    let f = fit_text("hello world", 40, 1);
    assert!(f.ends_with("..") && text_width(&f, 1) <= 40, "{f}");
    assert_eq!(fit_text("hello", 3, 1), "");
}

#[test]
fn dirty_rects() {
    let mut c = Canvas::new(100, 50);
    assert_eq!(c.take_dirty_rects(16), vec![Rect::new(0, 0, 100, 50)]);
    assert!(c.take_dirty_rects(16).is_empty());
    c.set_pixel(40, 20, Color::WHITE);
    let d = c.take_dirty_rects(16);
    assert_eq!(d.len(), 1);
    assert!(d[0].contains(40, 20));
    assert!(d[0].w <= 16 && d[0].h <= 16);
    assert!(c.take_dirty_rects(16).is_empty());
    // A horizontal strip merges into one rect; a block merges vertically too.
    c.fill_rect(Rect::new(0, 0, 100, 2), Color::WHITE);
    assert_eq!(c.take_dirty_rects(16), vec![Rect::new(0, 0, 100, 16)]);
    c.fill_rect(Rect::new(20, 20, 30, 25), Color::rgb(1, 2, 3));
    assert_eq!(c.take_dirty_rects(16), vec![Rect::new(16, 16, 48, 32)]);
    // Checkerboard of changes exceeding the cap => one bounding rect.
    let mut big = Canvas::new(512, 512);
    big.take_dirty_rects(8);
    for ty in 0..64 {
        for tx in 0..64 {
            if (tx + ty) % 2 == 0 {
                big.set_pixel(tx * 8, ty * 8, Color::WHITE);
            }
        }
    }
    let d = big.take_dirty_rects(8);
    assert_eq!(d.len(), 1);
    // Resize => full again. pixels_of extracts bytes.
    c.resize(20, 10);
    assert_eq!(c.take_dirty_rects(4), vec![Rect::new(0, 0, 20, 10)]);
    c.fill_rect(Rect::new(1, 1, 2, 2), Color::rgba(9, 8, 7, 255));
    let px = c.pixels_of(Rect::new(1, 1, 2, 2));
    assert_eq!(px, [9, 8, 7, 255].repeat(4));
    assert_eq!(c.pixels_of(Rect::new(18, 8, 10, 10)).len(), 2 * 2 * 4);
}

#[test]
fn clip_and_blend() {
    let mut c = Canvas::new(10, 10);
    c.clear(Color::BLACK);
    c.push_clip(Rect::new(2, 2, 3, 3));
    c.fill_rect(Rect::new(-100, -100, 1000, 1000), Color::WHITE);
    c.pop_clip();
    assert_eq!(c.get_pixel(1, 1), Color::BLACK);
    assert_eq!(c.get_pixel(2, 2), Color::WHITE);
    assert_eq!(c.get_pixel(5, 5), Color::BLACK);
    c.fill_rect(Rect::new(0, 0, 1, 1), Color::rgba(255, 255, 255, 128));
    let p = c.get_pixel(0, 0);
    assert!(p.r > 120 && p.r < 136 && p.a == 255);
    // Degenerate draws don't panic.
    c.draw_text(i32::MAX - 5, i32::MAX - 5, "hello", Color::WHITE, 1000);
    c.draw_text(i32::MIN, i32::MIN, "hello", Color::WHITE, 1);
    c.stroke_rect(Rect::new(3, 3, 1, 1), Color::WHITE);
}

// ---- on-screen keyboard ---------------------------------------------------

/// Centre of the key at row `r`, unit column `u` (11 units per row) in `kb`.
fn key_pos(kb: Rect, row: usize, u: i32) -> (i32, i32) {
    let rows = kb.rows(5);
    let rr = rows[row];
    let x0 = rr.x + rr.w * u / 11;
    let x1 = rr.x + rr.w * (u + 1) / 11;
    ((x0 + x1) / 2, rr.y + rr.h / 2)
}

#[test]
fn on_screen_keyboard_types_into_focused_field() {
    let mut h = H::new(220, 150);
    let fid = WidgetId::new("field");
    let kid = WidgetId::new("osk");
    let fr = Rect::new(0, 0, 220, 16);
    let kb = Rect::new(0, 50, 220, 100);
    let mut s = String::new();
    let draw = |f: &mut Frame, s: &mut String| {
        f.text_field(fid, fr, s, "type");
        f.on_screen_keyboard(kid, kb)
    };
    // Focus the field.
    h.frame(&[down(5, 5), up(5, 5)], |f| draw(f, &mut s));
    assert_eq!(h.ui.focused(), Some(fid));
    // Tap 'q' (row 1, unit 0): returns it, keeps focus, asks for immediate redraw.
    let (x, y) = key_pos(kb, 1, 0);
    let (k, out) = h.frame(&[down(x, y), up(x, y)], |f| draw(f, &mut s));
    assert_eq!(k, Some(KeyOrChar::Char('q')));
    assert!(out.redraw_now);
    assert_eq!(h.ui.focused(), Some(fid));
    // Next frame the injected char lands in the field.
    h.frame(&[], |f| draw(f, &mut s));
    assert_eq!(s, "q");
    // Shift then 'a' => 'A'; shift is one-shot.
    let (sx, sy) = key_pos(kb, 3, 0);
    h.frame(&[down(sx, sy), up(sx, sy)], |f| draw(f, &mut s));
    let (ax, ay) = key_pos(kb, 2, 0);
    h.frame(&[down(ax, ay), up(ax, ay)], |f| draw(f, &mut s));
    h.frame(&[down(ax, ay), up(ax, ay)], |f| draw(f, &mut s));
    h.frame(&[], |f| draw(f, &mut s));
    assert_eq!(s, "qAa");
    // Digit and backspace.
    let (dx, dy) = key_pos(kb, 0, 4);
    h.frame(&[down(dx, dy), up(dx, dy)], |f| draw(f, &mut s));
    h.frame(&[], |f| draw(f, &mut s));
    assert_eq!(s, "qAa5");
    let (bx, by) = key_pos(kb, 0, 10);
    h.frame(&[down(bx, by), up(bx, by)], |f| draw(f, &mut s));
    h.frame(&[], |f| draw(f, &mut s));
    assert_eq!(s, "qAa");
    // Space (row 4, units 2..9).
    let (px, py) = key_pos(kb, 4, 5);
    let (k, _) = h.frame(&[down(px, py), up(px, py)], |f| draw(f, &mut s));
    assert_eq!(k, Some(KeyOrChar::Char(' ')));
    h.frame(&[], |f| draw(f, &mut s));
    assert_eq!(s, "qAa ");
    // Tiny keyboard doesn't panic.
    h.frame(&[down(1, 1)], |f| f.on_screen_keyboard(kid, Rect::new(0, 0, 5, 3)));
}

#[test]
fn widgets_survive_tiny_rects() {
    let mut h = H::new(8, 8);
    let mut b = false;
    let mut sel = 0;
    let mut s = String::from("abc");
    let ev = [down(0, 0), up(0, 0), InputEvent::Scroll { x: 0, y: 0, delta: -5 }];
    h.frame(&ev, |f| {
        let z = Rect::new(0, 0, 0, 0);
        let n = Rect::new(-5, -5, -10, -10);
        f.button(WidgetId::new("b"), z, "x");
        f.checkbox(WidgetId::new("c"), n, &mut b, "x");
        f.tabs(WidgetId::new("t"), Rect::new(0, 0, 2, 2), &["a", "b", "c"], &mut sel);
        f.progress_bar(n, 2.0, "x");
        f.progress_bar(Rect::new(0, 0, 8, 3), f32::NAN, "");
        f.text_field(WidgetId::new("f"), Rect::new(0, 0, 3, 3), &mut s, "");
        f.grid(WidgetId::new("g"), Rect::new(0, 0, 3, 3), 0, 0, 10, |_, _, _| {});
        f.list(WidgetId::new("l"), n, -1, usize::MAX / 2, |_, _, _| {});
        f.item_cell(WidgetId::new("i"), z, "x", "1", false);
        f.label(n, "hello", Color::WHITE, Align::Center);
        f.tooltip("tip");
        f.open_modal(WidgetId::new("m"));
        f.modal(WidgetId::new("m"), n, "t", |_, _| {});
    });
    let mut c = Canvas::new(-1, -1);
    assert!(c.take_dirty_rects(0).is_empty());
}

#[test]
fn tabs_change_selection() {
    let mut h = H::new(90, 20);
    let mut sel = 0;
    let (changed, _) = h.frame(&[down(75, 5), up(75, 5)], |f| f.tabs(WidgetId::new("tabs"), Rect::new(0, 0, 90, 20), &["Items", "Cells", "Tokens"], &mut sel));
    assert!(changed);
    assert_eq!(sel, 2);
}
