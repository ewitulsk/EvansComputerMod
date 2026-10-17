//! The window. Desktop (medium and wide): `[<] [>] [Reload] [Home] [address] [Go]`
//! over the page and a status line. Phone (compact): `[address] [Go]` over the
//! page and `[Back] [Fwd] [Reload] [Home]` at the bottom, in thumb reach.

use crate::model::{Model, Nav, Press, DRAG_SLOP, HOME, SCROLL_STEP};
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
    let mut mm = m.borrow_mut();
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
                if let Some(u) = mm.link_at(p.x, p.y) {
                    mm.go(u);
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
    let home = Nav::Go { url: Url::parse(HOME).expect("home"), push: true };
    let top;
    let bottom;
    if compact {
        top = row().gap(4).padding(4).child(address_field(m)).child(go_button(m));
        bottom = Some(
            row()
                .gap(4)
                .padding(4)
                .child(nav_button(m, "back", "Back", Nav::Back, can_back).grow(1))
                .child(nav_button(m, "forward", "Fwd", Nav::Forward, can_fwd).grow(1))
                .child(nav_button(m, "reload", "Reload", Nav::Reload, true).grow(1))
                .child(nav_button(m, "home", "Home", home, true).grow(1)),
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
            .child(go_button(m));
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
            Some(c) => scene::page_nodes(&c.rendered.list, pw, ph, scroll, (cx.theme.text_muted & 0xffff_ff00) | 0xa0),
            None => vec![],
        }
    };
    let pm = m.clone();
    let page = custom().id("page").grow(1).nodes(nodes).on_pointer(move |cx, e| on_page_pointer(&pm, cx, e));
    let km = m.clone();
    let page = page.on_key(move |cx, e| {
        if let EventValue::Key { key, mods, down: true, .. } = e.value {
            page_key(&km, cx, key, mods);
        }
    });
    let mut col = column().child(top).child(page);
    if let Some(b) = bottom {
        col = col.child(b);
    }
    col
}
