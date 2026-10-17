//! The window. Desktop (medium and wide): `[<] [>] [Reload] [Home] [address] [Go] [Menu]`
//! over the page and a status line. Phone (compact): `[address] [Go]` over the
//! page and `[<] [>] [Reload] [Home] [Menu]` at the bottom, in thumb reach.
//! Menu: bookmark this page, Bookmarks, History, set as home page.

use crate::model::{ClickResult, Model, Nav, Press, DRAG_SLOP, SCROLL_STEP};
use crate::scene;
use ecm_ui::prelude::*;
use ecm_ui::window::{self, keys};
use ecm_ui::Theme;
use ecm_ui_proto::PointerKind;
use ecm_web::style::UaColors;
use ecm_web::Url;
use std::cell::RefCell;
use std::rc::Rc;

pub type M = Rc<RefCell<Model>>;

/// The page's colours from the shell theme (unstyled pages look native).
pub fn ua_colors(t: &Theme) -> UaColors {
    UaColors { text: t.text, background: t.bg, link: t.accent, muted: t.text_muted, border: t.border, surface: t.surface, button: t.button }
}

fn nav_button(m: &M, id: &str, text: &str, nav: Nav, enabled: bool) -> Element {
    let m = m.clone();
    button(text).id(id).enabled(enabled).on_click(move |_, _| {
        m.borrow_mut().pending = Some(nav.clone());
    })
}

fn go(m: &M, cx: &mut Ctx) {
    let text = cx.text("url");
    m.borrow_mut().go_input(&text);
    cx.focus("page");
}

pub const MENU: [&str; 5] = ["Bookmark this page", "Bookmarks", "History", "Set as home page", "Console"];

fn menu_button(m: &M) -> Element {
    let m = m.clone();
    button("Menu").id("menu").on_click(move |cx, _| {
        let w = cx.window();
        let at = cx.core().window(w).and_then(|win| win.rect_of("menu")).map_or((0, 0), |r| (r.x, r.y + r.h));
        let mm = m.clone();
        cx.popup_menu(at.0, at.1, MENU, move |cx, i| {
            menu_action(&mm, i);
            cx.invalidate();
        });
    })
}

/// What a Menu entry does.
pub fn menu_action(m: &M, i: usize) {
    let mut mm = m.borrow_mut();
    match i {
        0 => {
            mm.bookmark();
        }
        1 => mm.go(Url::parse("about:bookmarks").expect("about")),
        2 => mm.go(Url::parse("about:history").expect("about")),
        3 => {
            mm.set_home();
        }
        4 => mm.show_console = !mm.show_console,
        _ => {}
    }
}

fn address_field(m: &M) -> Element {
    let m = m.clone();
    text_field().id("url").placeholder("Address").grow(1).on_submit(move |cx, _| go(&m, cx))
}

fn go_button(m: &M) -> Element {
    let m = m.clone();
    button("Go").id("go").style("primary").on_click(move |cx, _| go(&m, cx))
}

/// A key on the page or anywhere in the window. True if it was used.
pub fn page_key(m: &M, cx: &mut Ctx, key: u16, mods: u16) -> bool {
    let alt = mods & (ecm_ui_proto::mods::ALT) != 0;
    let ctrl = mods & (ecm_ui_proto::mods::CTRL) != 0;
    let shift = mods & (ecm_ui_proto::mods::SHIFT) != 0;
    let mut mm = m.borrow_mut();
    // The page's scripts see the key (Enter in a field: edit_key sends it with the form's default).
    if !(mm.focus.is_some() && matches!(key, keys::ENTER | keys::KP_ENTER)) && !ctrl && !alt {
        mm.key_event(key, shift);
    }
    // A focused form field takes its editing keys first.
    if !ctrl && !alt && mm.focus.is_some() && mm.edit_key(key, shift) {
        drop(mm);
        cx.invalidate();
        return true;
    }
    let page = mm.view.1 - 24.0;
    let used = match key {
        keys::LEFT if alt => {
            mm.pending = Some(Nav::Back);
            true
        }
        keys::RIGHT if alt => {
            mm.pending = Some(Nav::Forward);
            true
        }
        keys::F5 => {
            mm.pending = Some(Nav::Reload);
            true
        }
        0x0F if ctrl => {
            // Ctrl+L: the address bar.
            drop(mm);
            cx.focus("url");
            return true;
        }
        keys::DOWN => mm.scroll_by(SCROLL_STEP),
        keys::UP => mm.scroll_by(-SCROLL_STEP),
        keys::PAGE_DOWN | keys::SPACE => mm.scroll_by(page),
        keys::PAGE_UP => mm.scroll_by(-page),
        keys::HOME if !ctrl => {
            mm.scroll_to(0.0);
            true
        }
        keys::END => {
            let end = mm.max_scroll();
            mm.scroll_to(end);
            true
        }
        keys::BACKSPACE => {
            mm.pending = Some(Nav::Back);
            true
        }
        _ => false,
    };
    drop(mm);
    if used {
        cx.invalidate();
    }
    used
}

/// Text typed on the page: into the focused field.
fn on_page_text(m: &M, cx: &mut Ctx, e: &UiEvent) {
    if let EventValue::Text(t) = &e.value {
        if m.borrow_mut().type_text(t) {
            cx.invalidate();
        }
    }
}

/// The page area's top-left in the window (popups are placed in window coordinates).
fn page_origin(cx: &mut Ctx) -> (i32, i32) {
    let w = cx.window();
    cx.core().window(w).and_then(|win| win.rect_of("page")).map_or((0, 0), |r| (r.x, r.y))
}

fn on_page_pointer(m: &M, cx: &mut Ctx, e: &UiEvent) {
    let EventValue::Pointer(p) = &e.value else {
        if let EventValue::Key { key, mods, down: true, .. } = e.value {
            page_key(m, cx, key, mods);
        }
        return;
    };
    let mut mm = m.borrow_mut();
    let redraw = match p.kind {
        PointerKind::Scroll => mm.scroll_by(p.scroll_y as f32 * SCROLL_STEP),
        PointerKind::Down => {
            let s = mm.scroll;
            mm.press = Some(Press { x: p.x, y: p.y, scroll0: s, dragging: false });
            false
        }
        PointerKind::Move => {
            let drag = mm.press.as_ref().map(|pr| (pr.y, pr.x, pr.scroll0, pr.dragging));
            match drag {
                Some((y0, x0, s0, dragging)) => {
                    if dragging || (p.y - y0).abs() > DRAG_SLOP || (p.x - x0).abs() > DRAG_SLOP {
                        if let Some(pr) = mm.press.as_mut() {
                            pr.dragging = true;
                        }
                        let target = s0 - (p.y - y0) as f32;
                        let before = mm.scroll;
                        mm.scroll_to(target);
                        mm.scroll != before
                    } else {
                        false
                    }
                }
                None => mm.hover_at(p.x, p.y),
            }
        }
        PointerKind::Up => {
            let press = mm.press.take();
            if press.is_some_and(|pr| !pr.dragging) {
                if let ClickResult::Choose { select, labels } = mm.click(p.x, p.y) {
                    drop(mm);
                    let (ox, oy) = page_origin(cx);
                    let mc = m.clone();
                    cx.popup_menu(ox + p.x, oy + p.y, labels, move |cx, i| {
                        mc.borrow_mut().choose(select, i);
                        cx.invalidate();
                    });
                    cx.invalidate();
                    return;
                }
            }
            true
        }
        _ => false,
    };
    drop(mm);
    if redraw {
        cx.invalidate();
    }
}

/// The window's elements for its current size.
pub fn view(m: &M, cx: &ViewCtx) -> Element {
    {
        let ua = ua_colors(cx.theme);
        let mut mm = m.borrow_mut();
        if mm.ua != ua {
            mm.ua = ua;
            mm.cache = None;
        }
    }
    let compact = cx.class == SizeClass::Compact;
    let (can_back, can_fwd) = {
        let mm = m.borrow();
        (mm.can_back(), mm.can_forward())
    };
    let home = Nav::Go { url: m.borrow().home(), push: true };
    let top;
    let bottom;
    if compact {
        top = row().gap(4).padding(4).child(address_field(m)).child(go_button(m));
        bottom = Some(
            row()
                .gap(4)
                .padding(4)
                .child(nav_button(m, "back", "<", Nav::Back, can_back).grow(1))
                .child(nav_button(m, "forward", ">", Nav::Forward, can_fwd).grow(1))
                .child(nav_button(m, "reload", "Reload", Nav::Reload, true).grow(1))
                .child(nav_button(m, "home", "Home", home, true).grow(1))
                .child(menu_button(m).grow(1)),
        );
    } else {
        top = row()
            .gap(4)
            .padding(4)
            .child(nav_button(m, "back", "<", Nav::Back, can_back))
            .child(nav_button(m, "forward", ">", Nav::Forward, can_fwd))
            .child(nav_button(m, "reload", "Reload", Nav::Reload, true))
            .child(nav_button(m, "home", "Home", home, true))
            .child(address_field(m))
            .child(go_button(m))
            .child(menu_button(m));
        let status = m.borrow().status_text();
        bottom = Some(row().padding(2).child(label(if status.is_empty() { " ".to_string() } else { status }).id("status").style("caption")));
    }
    // The page area is what the bars leave.
    let (_, top_h) = window::measure(&top, cx.theme, cx.class, cx.width);
    let bottom_h = bottom.as_ref().map_or(0, |b| window::measure(b, cx.theme, cx.class, cx.width).1);
    let pw = cx.width.max(1);
    let ph = (cx.height - top_h - bottom_h).max(1);
    let nodes = {
        let mut mm = m.borrow_mut();
        mm.render(pw as f32, ph as f32);
        let scroll = mm.scroll.round() as i32;
        match &mm.cache {
            Some(c) => scene::page_nodes(&c.rendered.list, pw, ph, scroll, (cx.theme.text_muted & 0xffff_ff00) | 0xa0, &mm.images),
            None => vec![],
        }
    };
    let pm = m.clone();
    let page = custom().id("page").grow(1).nodes(nodes).on_pointer(move |cx, e| on_page_pointer(&pm, cx, e));
    let tm = m.clone();
    let page = page.on_text(move |cx, e| on_page_text(&tm, cx, e));
    let km = m.clone();
    let page = page.on_key(move |cx, e| {
        if let EventValue::Key { key, mods, down: true, .. } = e.value {
            page_key(&km, cx, key, mods);
        }
    });
    let mut col = column().child(top).child(page);
    let (console, show_console, dialog) = {
        let mm = m.borrow();
        (mm.console.clone(), mm.show_console, mm.dialog.clone())
    };
    if show_console {
        col = col.child(console_pane(&console, compact));
    }
    if let Some(b) = bottom {
        col = col.child(b);
    }
    match dialog {
        Some(d) => stack().child(col).child(dialog_card(m, &d)),
        None => col,
    }
}

/// The page scripts' console: its last lines.
fn console_pane(lines: &[String], compact: bool) -> Element {
    let shown = if compact { 4 } else { 6 };
    let start = lines.len().saturating_sub(shown);
    let mut c = column().id("console").padding(4).gap(2).fill(0x0000_0040);
    if lines.is_empty() {
        c = c.child(label("Console: nothing logged").style("caption"));
    }
    for l in &lines[start..] {
        let t: String = l.chars().take(200).collect();
        c = c.child(label(t).font(ecm_ui_proto::Font::Mono).size(16).wrap(false));
    }
    c
}

/// A script's `alert` (OK) or `confirm` (OK, Cancel).
fn dialog_card(m: &M, d: &crate::model::Dialog) -> Element {
    let ok = {
        let m = m.clone();
        button("OK").id("dialog_ok").style("primary").on_click(move |cx, _| {
            m.borrow_mut().dialog_answer = Some(true);
            cx.invalidate();
        })
    };
    let mut buttons = row().gap(8).child(spacer().grow(1));
    if d.ask {
        let m = m.clone();
        buttons = buttons.child(button("Cancel").id("dialog_cancel").on_click(move |cx, _| {
            m.borrow_mut().dialog_answer = Some(false);
            cx.invalidate();
        }));
    }
    buttons = buttons.child(ok);
    dialog("The page says").id("dialog").child(column().gap(8).child(label(d.text.clone()).wrap(true)).child(buttons))
}
