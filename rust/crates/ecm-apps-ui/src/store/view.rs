//! The App Store's window at the three size classes:
//!
//! * **compact** (phones, < 400 lu): tabs on top, one app per row; an app's
//!   page replaces the list (with a Back button).
//! * **medium**: tabs on top, apps as cards two to a row; the page replaces the list.
//! * **wide** (>= 800 lu): a sidebar (Discover / Updates / Installed and the
//!   repositories), cards three to a row, and the open app's page beside them.

use super::{Action, Card, Detail, JobKind, Store, Tab};
use crate::appctl::PermState;
use crate::widgets::{icon, pill, progress_bar, size_text, trust_badge, GREEN, RED};
use ecm_ui::element::{button, column, grid, image, label, list, panel, row, scroll_view, spacer, tabs, text_field, Element};
use ecm_ui::{Align, ViewCtx};
use ecm_ui_proto::SizeClass;
use std::cell::RefCell;
use std::rc::Rc;

type S = Rc<RefCell<Store>>;

const TABS: [&str; 3] = ["Discover", "Updates", "Installed"];

pub fn view(s: &S, cx: &ViewCtx) -> Element {
    let st = s.borrow();
    match cx.class {
        SizeClass::Compact => compact(s, &st, cx),
        SizeClass::Medium => medium(s, &st, cx),
        SizeClass::Wide => wide(s, &st, cx),
    }
}

fn tab_titles(st: &Store) -> Vec<String> {
    let n = st.updates().len();
    vec![TABS[0].to_string(), if n > 0 { format!("Updates ({n})") } else { TABS[1].to_string() }, TABS[2].to_string()]
}

fn header(s: &S, compact: bool) -> Element {
    let r = s.clone();
    row()
        .gap(8)
        .child(label("App Store").style("title").wrap(false))
        .child(spacer())
        .child(button(if compact { "Refresh" } else { "Refresh lists" }).id("refresh").on_click(move |_, _| r.borrow_mut().refresh()))
}

fn search(s: &S, st: &Store) -> Element {
    let q = s.clone();
    text_field().id("search").placeholder("Search apps").text(st.query.clone()).on_change(move |cx, _| {
        let t = cx.text("search");
        q.borrow_mut().query = t;
    })
}

fn status(st: &Store, cx: &ViewCtx) -> Element {
    let mut c = column().gap(4);
    if let Some(r) = &st.running {
        let what = match r.job.kind {
            JobKind::Install => "Installing",
            JobKind::Update => "Updating",
            JobKind::Remove => "Removing",
        };
        let name = st.detail_of(&r.job.id).map(|d| d.card.name).unwrap_or(r.job.id.clone());
        let amount = match r.total {
            Some(t) => format!("{} of {}", size_text(r.done as u64), size_text(t as u64)),
            None => String::new(),
        };
        c = c.child(row().gap(6).child(label(format!("{what} {name}")).wrap(false)).child(label(amount).style("caption").wrap(false)));
        c = c.child(progress_bar(cx.theme, r.percent(), (cx.width - 40).clamp(60, 300)).id("progress"));
    }
    if !st.status.is_empty() {
        c = c.child(label(st.status.clone()).style("caption").id("status"));
    }
    c
}

fn action_button(s: &S, card: &Card) -> Element {
    action_button_as(s, card, "act")
}

/// The same button with its own id prefix (the page shows it next to the card's).
fn action_button_as(s: &S, card: &Card, prefix: &str) -> Element {
    let id = card.id.clone();
    let key = format!("{prefix}:{}", card.id);
    match &card.action {
        Action::Install => {
            let s = s.clone();
            button("Install").style("primary").id(key).on_click(move |_, _| s.borrow_mut().act(&id))
        }
        Action::Update { .. } => {
            let s = s.clone();
            button("Update").style("primary").id(key).on_click(move |_, _| s.borrow_mut().act(&id))
        }
        Action::Open => {
            let s = s.clone();
            button("Open").id(key).on_click(move |_, _| s.borrow_mut().act(&id))
        }
        Action::Busy { label: l, .. } => button(l.clone()).id(key).enabled(false),
        Action::Refused => button("Unsigned").id(key).enabled(false),
    }
}

fn open_link(s: &S, card: &Card, text: Element) -> Element {
    let s = s.clone();
    let id = card.id.clone();
    text.id(format!("name:{}", card.id)).on_click(move |_, _| s.borrow_mut().open_detail(&id))
}

fn app_icon(st: &Store, cx: &ViewCtx, card: &Card, size: i32) -> Element {
    icon(st.pics.get(&format!("icon:{}", card.id)).copied(), &card.name, size, cx.theme)
}

/// A list row (compact).
fn app_row(s: &S, st: &Store, cx: &ViewCtx, card: &Card) -> Element {
    row()
        .gap(8)
        .child(app_icon(st, cx, card, 32))
        .child(
            column()
                .gap(2)
                .grow(1)
                .child(open_link(s, card, label(card.name.clone()).wrap(false)))
                .child(label(card.summary.clone()).style("caption")),
        )
        .child(action_button(s, card))
}

/// A card (medium, wide).
fn app_card(s: &S, st: &Store, cx: &ViewCtx, card: &Card) -> Element {
    let pub_line = if card.publisher.is_empty() { format!("v{}", card.version) } else { format!("{} · v{}", card.publisher, card.version) };
    panel().fill(cx.theme.surface).radius(6).padding(8).border(1, cx.theme.border).child(
        column()
            .gap(6)
            .child(
                row()
                    .gap(8)
                    .child(app_icon(st, cx, card, 32))
                    .child(
                        column()
                            .gap(2)
                            .grow(1)
                            .child(open_link(s, card, label(card.name.clone()).wrap(false)))
                            .child(label(pub_line).style("caption").wrap(false)),
                    ),
            )
            .child(label(card.summary.clone()).style("caption"))
            .child(row().gap(6).child(trust_badge(card.verified, short_trust(card)).align(Align::Center)).child(spacer()).child(action_button(s, card))),
    )
}

fn short_trust(card: &Card) -> &'static str {
    if card.verified {
        if card.trust == "system app" {
            "System app"
        } else {
            "Verified"
        }
    } else {
        "Unverified"
    }
}

fn empty_text(st: &Store) -> &'static str {
    match st.tab() {
        Tab::Discover if !st.query.is_empty() => "No apps match the search.",
        Tab::Discover => "No apps yet. Add a repository with pkg repo add, then Refresh.",
        Tab::Updates => "Everything is up to date.",
        Tab::Installed => "No apps installed.",
    }
}

/// The list of the current tab: rows (compact) or a grid of cards.
fn apps(s: &S, st: &Store, cx: &ViewCtx, cols: u16, h: i32) -> Element {
    let cards = st.cards();
    let mut body = column().gap(8);
    if st.tab() == Tab::Discover {
        body = body.child(search(s, st));
    }
    if st.tab() == Tab::Updates && !cards.is_empty() {
        let u = s.clone();
        body = body.child(row().child(spacer()).child(button("Update all").id("update_all").on_click(move |_, _| u.borrow_mut().update_all())));
    }
    if cards.is_empty() {
        body = body.child(label(empty_text(st)).style("caption"));
    } else if cols <= 1 {
        let mut l = column().gap(10);
        for c in &cards {
            l = l.child(app_row(s, st, cx, c));
        }
        body = body.child(l);
    } else {
        body = body.child(grid(cols).gap(8).children(cards.iter().map(|c| app_card(s, st, cx, c))));
    }
    scroll_view(body).id("apps").height(h.max(60))
}

/// An app's page.
fn detail(s: &S, st: &Store, cx: &ViewCtx, d: &Detail, back: bool, shots_w: i32, h: i32) -> Element {
    let card = &d.card;
    let mut c = column().gap(10);
    if back {
        let b = s.clone();
        c = c.child(row().child(button("< Back").id("back").on_click(move |_, _| b.borrow_mut().close_detail())));
    }
    let mut buttons = row().gap(6).child(action_button_as(s, card, "page"));
    if d.can_remove && !matches!(card.action, Action::Busy { .. }) {
        let r = s.clone();
        let id = card.id.clone();
        buttons = buttons.child(button("Remove").style("danger").id("remove").on_click(move |_, _| r.borrow_mut().enqueue(JobKind::Remove, &id)));
    }
    let mut facts = vec![format!("Version {}", card.version)];
    if let Some(v) = &d.installed_version {
        if *v != card.version {
            facts.push(format!("installed {v}"));
        } else {
            facts.push("installed".into());
        }
    }
    if let Some(n) = d.size {
        facts.push(size_text(n));
    }
    if let Some(r) = &d.repo {
        facts.push(format!("from {r}"));
    }
    c = c.child(
        row()
            .gap(10)
            .align(Align::Start)
            .child(app_icon(st, cx, card, 64))
            .child(
                column()
                    .gap(4)
                    .grow(1)
                    .child(label(card.name.clone()).style("title"))
                    .child(label(if card.publisher.is_empty() { "Unknown publisher".to_string() } else { card.publisher.clone() }))
                    .child(trust_badge(card.verified, &badge_text(card)))
                    .child(label(facts.join(" · ")).style("caption")),
            ),
    );
    c = c.child(buttons);
    if !card.summary.is_empty() {
        c = c.child(label(card.summary.clone()));
    }
    // Permissions in plain words.
    c = c.child(label("Permissions").style("title"));
    if d.permissions.is_empty() {
        c = c.child(label("This app asks for no special permissions.").style("caption"));
    }
    for (_, words, state) in &d.permissions {
        let mut r = row().gap(6).child(column().gap(1).grow(1).child(label(words.title.clone())).child(label(words.detail.clone()).style("caption")));
        if let Some(st) = state {
            r = r.child(match st {
                PermState::Allow => pill("Allowed", GREEN),
                PermState::Deny => pill("Denied", RED),
                PermState::Ask => pill("Asks first", cx.theme.border),
            });
        }
        c = c.child(r);
    }
    if d.installed_version.is_none() && !d.permissions.is_empty() {
        c = c.child(label("You are asked before it uses any of these.").style("caption"));
    }
    // Screenshots.
    let shots: Vec<Element> = d
        .screenshots
        .iter()
        .filter_map(|m| st.pics.get(&format!("shot:{}:{}", card.id, m)))
        .map(|img| {
            let (w, h) = crate::png::fit(img.w, img.h, shots_w - 6, 180);
            // A frame, so the picture doesn't melt into the page.
            panel().border(1, cx.theme.border).padding(2).align(Align::Start).child(image(img.resource, w, h))
        })
        .collect();
    if !shots.is_empty() {
        c = c.child(label("Screenshots").style("title"));
        c = c.child(column().gap(8).children(shots));
    }
    scroll_view(c).id("detail").height(h.max(60))
}

fn badge_text(card: &Card) -> String {
    match card.trust.as_str() {
        "verified publisher" => "Verified publisher".into(),
        "system app" => "System app".into(),
        t if t.contains("signed") && !t.starts_with("unsigned") => "Unverified publisher (signed, key not pinned)".into(),
        _ => "Unverified publisher".into(),
    }
}

fn tabbed(s: &S, st: &Store, body: Element) -> Element {
    let t = s.clone();
    let titles = tab_titles(st);
    let sel = st.tab() as usize;
    let mut pages: Vec<Element> = (0..3).map(|_| spacer()).collect();
    pages[sel] = body;
    tabs(titles).id("tabs").selected(sel).grow(1).children(pages).on_select(move |cx, _| {
        let i = cx.selected("tabs").unwrap_or(0);
        t.borrow_mut().set_tab(Tab::from_index(i));
    })
}

/// Height of the status block (progress and message) with its gap.
fn status_h(st: &Store, cx: &ViewCtx) -> i32 {
    let mut h = 0;
    if st.running.is_some() {
        h += LINE + 4 + 6 + 4;
    }
    if !st.status.is_empty() {
        // Captions wrap on narrow windows.
        let per_line = ((cx.width - 40) / 5).max(20) as usize;
        h += 12 * (st.status.len() / per_line + 1) as i32;
    }
    h + 8
}

/// Body line height, button height and tab header height (default theme).
const LINE: i32 = 15;
const BUTTON: i32 = LINE + 8;
const TABS_H: i32 = LINE + 8 + 1;

fn compact(s: &S, st: &Store, cx: &ViewCtx) -> Element {
    let mut c = column().gap(8).padding(8).child(header(s, true));
    let top = 8 + BUTTON + 8;
    match st.detail.as_ref().and_then(|id| st.detail_of(id)) {
        Some(d) => {
            let h = cx.height - top - status_h(st, cx) - 8;
            c = c.child(status(st, cx)).child(detail(s, st, cx, &d, true, cx.width - 32, h))
        }
        None => {
            let h = cx.height - top - TABS_H - status_h(st, cx) - 8;
            c = c.child(tabbed(s, st, column().gap(8).child(status(st, cx)).child(apps(s, st, cx, 1, h))))
        }
    }
    c
}

fn medium(s: &S, st: &Store, cx: &ViewCtx) -> Element {
    let mut c = column().gap(8).padding(10).child(header(s, false));
    let top = 10 + BUTTON + 8;
    match st.detail.as_ref().and_then(|id| st.detail_of(id)) {
        Some(d) => {
            let h = cx.height - top - status_h(st, cx) - 10;
            c = c.child(status(st, cx)).child(detail(s, st, cx, &d, true, cx.width - 40, h))
        }
        None => {
            let h = cx.height - top - TABS_H - status_h(st, cx) - 10;
            c = c.child(tabbed(s, st, column().gap(8).child(status(st, cx)).child(apps(s, st, cx, 2, h))))
        }
    }
    c
}

fn wide(s: &S, st: &Store, cx: &ViewCtx) -> Element {
    let nav = s.clone();
    let titles = tab_titles(st);
    let mut side = column()
        .gap(8)
        .width(170)
        .child(label("App Store").style("title"))
        .child(
            list()
                .id("nav")
                .selected(st.tab() as usize)
                .children(titles.into_iter().map(label))
                .on_select(move |cx, _| {
                    let i = cx.selected("nav").unwrap_or(0);
                    nav.borrow_mut().set_tab(Tab::from_index(i));
                }),
        )
        .child(label("Repositories").style("caption"));
    if st.repos.is_empty() {
        side = side.child(label("None yet").style("caption"));
    }
    for r in &st.repos {
        side = side.child(label(format!("{}{}", r.name, if r.key.is_some() { " (pinned)" } else { "" })).wrap(false));
    }
    let rf = s.clone();
    side = side.child(button("Refresh lists").id("refresh").on_click(move |_, _| rf.borrow_mut().refresh()));
    let d = st.detail.as_ref().and_then(|id| st.detail_of(id));
    let detail_w = 300;
    let cols = if d.is_some() { 2 } else { 3 };
    let main = column().gap(8).grow(1).child(status(st, cx)).child(apps(s, st, cx, cols, cx.height - 24 - status_h(st, cx)));
    let mut r = row().gap(12).padding(12).align(Align::Stretch).grow(1).child(side).child(main);
    if let Some(d) = d {
        r = r.child(
            panel()
                .fill(cx.theme.surface)
                .radius(6)
                .padding(10)
                .width(detail_w)
                .align(Align::Stretch)
                .child(column().gap(6).child(close_row(s)).child(detail(s, st, cx, &d, false, detail_w - 30, cx.height - 24 - 20 - BUTTON - 6))),
        );
    }
    column().child(r)
}

fn close_row(s: &S) -> Element {
    let c = s.clone();
    row().child(spacer()).child(button("Close").id("close_detail").on_click(move |_, _| c.borrow_mut().close_detail()))
}
