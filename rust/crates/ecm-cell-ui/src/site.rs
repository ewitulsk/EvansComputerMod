//! **Cell Site** (`dev.ecm.cellsite`): the window over `celld-site`'s admin socket.
//!
//! * **compact** (< 400 lu): the site's name and core link on top, then tabs:
//!   Overview (radio, core link, traffic), Phones (each phone with its state,
//!   address, signal and CQI; the neighbours heard), Settings.
//! * **medium**: the same tabs, the overview's cards two to a row.
//! * **wide** (>= 800 lu): the overview, the phones and neighbours, and the
//!   settings side by side.
//!
//! Settings are the keys of `/etc/celld-site.toml`; Save sends `set` for each
//! changed one (the daemon checks and writes them), Restart applies them.

use crate::widgets::{card, kv_rows, message, pill, AMBER, GREEN, GREY, RED};
use crate::{fmt_bytes, fmt_secs, Admin, Daemon, POLL_MS};
use celld_site::config::Conf;
use celld_site::daemon::StatusView;
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::element::{button, column, label, list, row, scroll_view, spacer, tabs, text_field, Element};
use ecm_ui::{Align, App, ViewCtx};
use ecm_ui_proto::SizeClass;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Overview,
    Phones,
    Settings,
}

impl Page {
    const ALL: [Page; 3] = [Page::Overview, Page::Phones, Page::Settings];
    fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Phones => "Phones",
            Page::Settings => "Settings",
        }
    }
}

/// The settings the form edits (keys of `/etc/celld-site.toml`) and their labels.
pub const FIELDS: &[(&str, &str)] = &[
    ("site_id", "Site id"),
    ("carrier", "Carrier"),
    ("core", "Core address"),
    ("cell_id", "Cell id"),
    ("profile", "Profile (lte, mmwave)"),
    ("channel", "Channel"),
    ("power", "Power (dBm or auto)"),
];

#[derive(Debug)]
pub struct Model {
    pub daemon: Daemon,
    pub status: StatusView,
    pub page: Page,
    pub message: Option<(bool, String)>,
    /// The settings as saved, and the form's values.
    pub saved: Conf,
    pub form: BTreeMap<String, String>,
    pub loaded: bool,
    /// Admin commands handlers queued for the next turn.
    pub queue: Vec<String>,
    pub refresh: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            daemon: Daemon::Unknown,
            status: StatusView::default(),
            page: Page::Overview,
            message: None,
            saved: Conf::default(),
            form: BTreeMap::new(),
            loaded: false,
            queue: Vec::new(),
            refresh: true,
        }
    }
}

impl Model {
    /// Save: a `set` for every field that differs from the saved settings.
    pub fn save(&mut self) {
        let mut n = 0;
        for (k, _) in FIELDS.iter().copied().chain([("secret", "")]) {
            let v = self.form.get(k).cloned().unwrap_or_default();
            if k == "secret" && v.is_empty() {
                continue;
            }
            if v != self.saved.get(k) {
                self.queue.push(format!("set {k} {v}"));
                n += 1;
            }
        }
        if n == 0 {
            self.message = Some((true, "Nothing changed".into()));
        }
    }

    fn load_form(&mut self) {
        self.form = FIELDS.iter().map(|(k, _)| (k.to_string(), self.saved.get(k))).collect();
        self.form.insert("secret".into(), String::new());
    }
}

pub type M = Rc<RefCell<Model>>;

pub struct SiteApp<A: Admin> {
    pub app: App,
    pub model: M,
    pub admin: A,
    pub window: u32,
    last_poll: Option<u64>,
}

impl<A: Admin> SiteApp<A> {
    pub fn new(conn: Box<dyn Connection>, admin: A) -> Result<Self, ConnError> {
        let mut app = App::with_connection(celld_site::APP_ID, conn)?;
        let model: M = Rc::new(RefCell::new(Model::default()));
        let v = model.clone();
        let window = app.window("Cell Site", move |cx| view(&v, cx));
        Ok(SiteApp { app, model, admin, window, last_poll: None })
    }

    /// One loop turn: draw, wait for input (at most a quarter second, so polling goes on), then the I/O.
    pub fn step(&mut self, timeout_ms: i32, now_ms: u64) -> bool {
        let wait = if self.model.borrow().queue.is_empty() { if timeout_ms < 0 { 250 } else { timeout_ms.min(250) } } else { 0 };
        if !self.app.step(wait) {
            return false;
        }
        self.process(now_ms);
        !self.app.core.is_closed()
    }

    pub fn run(&mut self) {
        let start = std::time::Instant::now();
        while self.step(-1, start.elapsed().as_millis() as u64) {}
    }

    /// Run queued commands; poll the daemon when due.
    pub fn process(&mut self, now_ms: u64) {
        let mut changed = false;
        let cmds: Vec<String> = std::mem::take(&mut self.model.borrow_mut().queue);
        let mut saved = Vec::new();
        for c in cmds {
            let r = self.admin.call(&c);
            changed = true;
            let mut m = self.model.borrow_mut();
            match r {
                Ok(Ok(_)) => {
                    if let Some(k) = c.strip_prefix("set ").and_then(|r| r.split_whitespace().next()) {
                        saved.push(k.to_string());
                    } else if c == "restart" {
                        m.message = Some((true, "Restarting the site with the saved settings".into()));
                    }
                }
                Ok(Err(e)) => m.message = Some((false, e)),
                Err(e) => m.daemon = Daemon::Down(e),
            }
            m.refresh = true;
        }
        if !saved.is_empty() {
            let mut m = self.model.borrow_mut();
            m.loaded = false;
            if !matches!(m.message, Some((false, _))) {
                m.message = Some((true, format!("Saved {}. Restart the site to apply.", saved.join(", "))));
            }
        }
        let due = self.model.borrow().refresh || self.last_poll.is_none_or(|t| now_ms.saturating_sub(t) >= POLL_MS);
        if due {
            self.last_poll = Some(now_ms);
            changed |= self.poll();
        }
        if changed {
            self.redraw();
        }
    }

    /// Ask the daemon for its status (and the settings when not loaded yet). True if anything changed.
    fn poll(&mut self) -> bool {
        let mut m = self.model.borrow_mut();
        m.refresh = false;
        let before = (m.daemon.clone(), m.status.clone());
        match self.admin.call("status") {
            Ok(Ok(body)) => {
                m.daemon = Daemon::Running;
                m.status = StatusView::parse(&body);
            }
            Ok(Err(e)) => m.daemon = Daemon::Down(e),
            Err(e) => m.daemon = Daemon::Down(e),
        }
        let mut changed = before != (m.daemon.clone(), m.status.clone());
        if m.daemon == Daemon::Running && !m.loaded {
            if let Ok(Ok(body)) = self.admin.call("config") {
                if let Ok(c) = Conf::parse(&body.join("\n")) {
                    m.saved = c;
                    m.loaded = true;
                    m.load_form();
                    changed = true;
                }
            }
        }
        changed
    }

    fn redraw(&mut self) {
        self.app.core.mark_all_dirty();
        let _ = self.app.core.render();
    }
}

// ------------------------------------------------------------ the window

const LINE: i32 = 15;
const TITLE: i32 = 20;
const TABS_H: i32 = LINE + 8 + 1;

pub fn view(m: &M, cx: &ViewCtx) -> Element {
    let st = m.borrow();
    let pad = if cx.class == SizeClass::Compact { 6 } else { 10 };
    let head = header(&st, cx);
    let body_h = cx.height - 2 * pad - (TITLE + 4 + 16 + 8);
    let body = if let Daemon::Down(why) = &st.daemon {
        not_running(cx, why)
    } else {
        match cx.class {
            SizeClass::Wide => wide(m, &st, cx, body_h),
            _ => tabbed(m, &st, cx, body_h),
        }
    };
    column().gap(8).padding(pad).child(head).child(body)
}

fn header(st: &Model, cx: &ViewCtx) -> Element {
    let s = &st.status;
    let name = if s.get("site").is_empty() { "Cell Site".to_string() } else { format!("Cell Site  {}", s.get("site")) };
    let (link_text, link_col) = match (st.daemon.clone(), s.get("link")) {
        (Daemon::Down(_), _) => ("celld-site not running", GREY),
        (_, "up") => ("Core link up", GREEN),
        (_, "registering") => ("Registering", AMBER),
        (Daemon::Unknown, _) => ("Connecting", GREY),
        _ => ("Core link down", RED),
    };
    let mut pills = row().gap(6).child(pill(link_text, link_col));
    if st.daemon == Daemon::Running {
        pills = pills.child(pill(&format!("{} phones", s.get("phones")), if s.get("phones") == "0" { GREY } else { GREEN }));
        if cx.class != SizeClass::Compact {
            pills = pills.child(label(format!("{} · {} · cell {}", s.get("carrier"), radio_line(s), s.get("cell"))).style("caption").wrap(false));
        }
    }
    column().gap(4).child(label(name).style("title").wrap(false)).child(pills)
}

fn radio_line(s: &StatusView) -> String {
    let prof = s.get("profile").to_uppercase().replace("MMWAVE", "mmWave");
    format!("{} channel {}", prof, s.get("channel"))
}

fn not_running(cx: &ViewCtx, why: &str) -> Element {
    card(
        cx,
        "celld-site is not running",
        column()
            .gap(6)
            .child(label("This computer's cell site program isn't answering. Start it in a terminal:"))
            .child(label("celld-site start &").color(cx.theme.accent))
            .child(label("Configure it first with celld-site config set (celld-site config keys lists the settings).").style("caption"))
            .child(label(format!("({why})")).style("caption")),
    )
}

fn overview_cards(st: &Model, cx: &ViewCtx) -> Vec<Element> {
    let s = &st.status;
    let mut radio = vec![
        ("Profile", radio_line(s)),
        ("Power", if s.get("power").starts_with("auto") { format!("{} dBm (auto)", s.get("power").trim_start_matches("auto (").trim_end_matches(')')) } else { format!("{} dBm", s.get("power")) }),
        ("Cell", s.get("cell").to_string()),
        ("Address", s.get("address").to_string()),
        ("Radio", s.get("radio").to_string()),
    ];
    if !s.get("radio_tx_frames").is_empty() {
        radio.push(("Frames", format!("{} sent, {} received", s.get("radio_tx_frames"), s.get("radio_rx_frames"))));
    }
    if !s.get("radio_beacons").is_empty() {
        radio.push(("Beacons", s.get("radio_beacons").to_string()));
    }
    let mut core = vec![("Core", s.get("core").to_string()), ("Link", s.get("link").to_string())];
    if !s.get("link_error").is_empty() {
        core.push(("Last error", s.get("link_error").to_string()));
    }
    core.push(("Carrier", s.get("carrier").to_string()));
    core.push(("Rejected", format!("{} sealed messages", s.get("backhaul_rejects"))));
    let up: u64 = s.get("ul_bytes").parse().unwrap_or(0);
    let down: u64 = s.get("dl_bytes").parse().unwrap_or(0);
    let traffic = vec![
        ("Phones", s.get("phones").to_string()),
        ("Uplink", fmt_bytes(up)),
        ("Downlink", fmt_bytes(down)),
        ("Uptime", fmt_secs(s.get("uptime_s").parse().unwrap_or(0))),
        ("Sub-channels", s.get("ul_mask").to_string()),
    ];
    vec![card(cx, "Radio", kv_rows(&radio, 70)), card(cx, "Core link", kv_rows(&core, 70)), card(cx, "Traffic", kv_rows(&traffic, 80))]
}

fn phones(st: &Model, cx: &ViewCtx) -> Element {
    let s = &st.status;
    // one line per phone where there is room (medium); two lines when narrow (compact, the wide layout's middle column)
    let compact = cx.class != SizeClass::Medium;
    let mut c = column().gap(6);
    if s.ues.is_empty() {
        c = c.child(label("No phones on this cell.").style("caption"));
    } else {
        let rows = s.ues.iter().map(|u| {
            let who = if u.imsi == "-" { format!("phone {}", u.addr) } else { u.imsi.clone() };
            let signal = format!("{:.1} dBm · CQI {} · sub {}", u.rssi_dbm, u.cqi, u.subchannel);
            if compact {
                column()
                    .gap(1)
                    .child(row().gap(6).child(label(who).wrap(false)).child(spacer()).child(label(u.state.clone()).style("caption").wrap(false)))
                    .child(label(format!("{} · {} · {}", if u.ip == "-" { "no address" } else { &u.ip }, signal, fmt_bytes(u.used))).style("caption").wrap(false))
            } else {
                row()
                    .gap(8)
                    .child(label(who).width(130).wrap(false))
                    .child(label(u.state.clone()).width(90).wrap(false))
                    .child(label(u.ip.clone()).width(90).wrap(false))
                    .child(label(signal).grow(1).wrap(false))
                    .child(label(fmt_bytes(u.used)).style("caption").wrap(false))
            }
        });
        c = c.child(list().id("phones").children(rows));
    }
    c = c.child(label("Neighbours heard").style("title"));
    if s.nbs.is_empty() {
        c = c.child(label("None yet (phones report the cells they hear).").style("caption"));
    }
    for n in &s.nbs {
        c = c.child(label(format!("cell {} · {} channel {} · sub-channels {}", n.cell, n.profile, n.channel, n.ul_mask)).wrap(false));
    }
    card(cx, &format!("Phones ({})", s.ues.len()), c)
}

fn settings(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let mut c = column().gap(4);
    for (k, title) in FIELDS {
        let mm = m.clone();
        let key = k.to_string();
        let id = format!("f_{k}");
        let id2 = id.clone();
        c = c.child(label(*title).style("caption")).child(
            text_field().id(id).text(st.form.get(*k).cloned().unwrap_or_default()).on_change(move |cx, _| {
                let t = cx.text(&id2);
                mm.borrow_mut().form.insert(key.clone(), t);
            }),
        );
    }
    let ms = m.clone();
    c = c.child(label("Credential (32 hex digits; empty keeps it)").style("caption")).child(
        text_field().id("f_secret").password(true).placeholder("unchanged").text(st.form.get("secret").cloned().unwrap_or_default()).on_change(move |cx, _| {
            let t = cx.text("f_secret");
            ms.borrow_mut().form.insert("secret".into(), t);
        }),
    );
    let a = m.clone();
    let b = m.clone();
    c = c.child(
        row()
            .gap(6)
            .child(button("Save").style("primary").id("save").enabled(st.loaded).on_click(move |_, _| a.borrow_mut().save()))
            .child(button("Restart site").id("restart").enabled(st.daemon == Daemon::Running).on_click(move |_, _| b.borrow_mut().queue.push("restart".into()))),
    );
    c = c.child(message(cx, &st.message));
    card(cx, "Settings", c)
}

fn tabbed(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let sel = Page::ALL.iter().position(|p| *p == st.page).unwrap_or(0);
    let page_h = (h - TABS_H).max(60);
    let body = match st.page {
        Page::Overview => {
            let cards = overview_cards(st, cx);
            if cx.class == SizeClass::Medium {
                let mut it = cards.into_iter();
                let (a, b, c) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
                column().gap(8).child(row().gap(8).align(Align::Start).child(a.grow(1)).child(b.grow(1))).child(c)
            } else {
                column().gap(8).children(cards)
            }
        }
        Page::Phones => phones(st, cx),
        Page::Settings => settings(m, st, cx),
    };
    let mut pages: Vec<Element> = (0..3).map(|_| spacer()).collect();
    pages[sel] = scroll_view(body).id(format!("page_{sel}")).height(page_h);
    let t = m.clone();
    tabs(Page::ALL.iter().map(|p| p.title())).id("tabs").selected(sel).grow(1).children(pages).on_select(move |cx, _| {
        let i = cx.selected("tabs").unwrap_or(0);
        t.borrow_mut().page = Page::ALL[i.min(2)];
    })
}

fn wide(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let left = scroll_view(column().gap(8).children(overview_cards(st, cx))).id("overview").height(h).width(300);
    let mid = scroll_view(phones(st, cx)).id("phones_view").height(h).grow(1);
    let right = scroll_view(settings(m, st, cx)).id("settings").height(h).width(280);
    row().gap(10).align(Align::Start).child(left).child(mid).child(right)
}

#[cfg(test)]
mod tests;
