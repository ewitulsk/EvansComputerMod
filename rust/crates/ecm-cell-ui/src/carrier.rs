//! **Carrier Console** (`dev.ecm.carrier`): the window over `cored`'s admin socket.
//!
//! Pages: **Subscribers** (each SIM with its label, address, usage against its
//! quota, revoked or online; New SIM writes the blank card in the Card
//! Reader/Writer; the chosen subscriber can be revoked or reinstated, given a
//! quota, or released), **Sites** (configured sites: connected or not, from
//! where, last heartbeat; Add site makes a credential for a new one), **Phones**
//! (live sessions: which site and cell, which address), **Handovers** (the
//! core's handover log) and **Core** (carrier, pool, ports, counters).
//!
//! * **compact** (< 400 lu): tabs with short titles, one page at a time.
//! * **medium**: the same tabs.
//! * **wide** (>= 800 lu): a sidebar (pages, the core in brief) and the page.

use crate::widgets::{card, kv_rows, message, pill, AMBER, GREEN, GREY, RED};
use crate::{fmt_bytes, fmt_secs, Admin, Daemon, POLL_MS};
use cored::daemon::{parse_sessions, parse_sites, parse_subs, SessionView, SiteView, SubView};
use ecm_cell::admin::value;
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::element::{button, column, label, list, panel, row, scroll_view, spacer, tabs, text_field, Element};
use ecm_ui::{Align, App, ViewCtx};
use ecm_ui_proto::SizeClass;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Subscribers,
    Sites,
    Phones,
    Handovers,
    Numbers,
    Ims,
    Core,
}

impl Page {
    pub const ALL: [Page; 7] = [Page::Subscribers, Page::Sites, Page::Phones, Page::Handovers, Page::Numbers, Page::Ims, Page::Core];
    fn title(self, compact: bool) -> &'static str {
        match (self, compact) {
            (Page::Subscribers, true) => "SIMs",
            (Page::Subscribers, false) => "Subscribers",
            (Page::Sites, _) => "Sites",
            (Page::Phones, _) => "Phones",
            (Page::Handovers, true) => "Log",
            (Page::Handovers, false) => "Handovers",
            (Page::Numbers, true) => "Nums",
            (Page::Numbers, false) => "Numbers",
            (Page::Ims, _) => "IMS",
            (Page::Core, _) => "Core",
        }
    }
}

#[derive(Debug)]
pub struct Model {
    pub daemon: Daemon,
    /// `status` (key value lines), parsed.
    pub status: Vec<(String, String)>,
    pub subs: Vec<SubView>,
    pub sites: Vec<SiteView>,
    pub sessions: Vec<SessionView>,
    pub handovers: Vec<String>,
    pub page: Page,
    pub message: Option<(bool, String)>,
    pub selected: Option<String>,
    /// Form fields.
    pub new_label: String,
    pub new_quota: String,
    pub quota: String,
    pub new_site: String,
    /// Telephony (docs/TELEPHONY.md): `blocks` (block, used, free) and `numbers` (number, IMSI, label).
    pub blocks: Vec<(String, u32, u32)>,
    pub numbers: Vec<(String, String, String)>,
    pub selected_number: Option<String>,
    pub new_block: String,
    pub number_for: String,
    pub move_to: String,
    pub ims_pool: String,
    pub ims_domain: String,
    /// The last credential made with Add site: (site, secret).
    pub credential: Option<(String, String)>,
    pub queue: Vec<String>,
    pub refresh: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            daemon: Daemon::Unknown,
            status: Vec::new(),
            subs: Vec::new(),
            sites: Vec::new(),
            sessions: Vec::new(),
            handovers: Vec::new(),
            page: Page::Subscribers,
            message: None,
            selected: None,
            new_label: String::new(),
            new_quota: String::new(),
            quota: String::new(),
            new_site: String::new(),
            blocks: Vec::new(),
            numbers: Vec::new(),
            selected_number: None,
            new_block: String::new(),
            number_for: String::new(),
            move_to: String::new(),
            ims_pool: String::new(),
            ims_domain: String::new(),
            credential: None,
            queue: Vec::new(),
            refresh: true,
        }
    }
}

impl Model {
    pub fn get(&self, k: &str) -> &str {
        self.status.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str()).unwrap_or("")
    }

    pub fn selected_sub(&self) -> Option<&SubView> {
        self.selected.as_ref().and_then(|s| self.subs.iter().find(|x| &x.imsi == s))
    }

    /// New SIM: `sim-new [label] [quota]`.
    pub fn write_sim(&mut self) {
        let label = self.new_label.trim().replace(' ', "-");
        let quota = self.new_quota.trim();
        let q: u64 = match quota {
            "" => 0,
            q => match q.parse() {
                Ok(v) => v,
                Err(_) => {
                    self.message = Some((false, "The quota is a number of bytes (0 or empty = unlimited)".into()));
                    return;
                }
            },
        };
        self.queue.push(match label.as_str() {
            "" => format!("sim-new {q}"),
            l => format!("sim-new {l} {q}"),
        });
    }

    pub fn add_site(&mut self) {
        let id = self.new_site.trim().to_string();
        if id.is_empty() {
            self.message = Some((false, "Type the new site's id first".into()));
            return;
        }
        self.queue.push(format!("site-new {id}"));
    }

    /// Numbers page: `block-add`, `number <who>` (the next free number), and the chosen number's
    /// `number-release`, `port-code` (a port-out code for its subscriber) and `number-move`.
    pub fn on_number(&mut self, cmd: &str) {
        let field = |s: &str| s.trim().to_string();
        match cmd {
            "block-add" => match field(&self.new_block) {
                b if b.len() == 7 && b.as_bytes()[3] == b'-' => self.queue.push(format!("block-add {b}")),
                _ => self.message = Some((false, "A block is NPA-NXX, like 214-555".into())),
            },
            "number" => match field(&self.number_for) {
                w if w.is_empty() => self.message = Some((false, "Type a subscriber (label or IMSI) first".into())),
                w => self.queue.push(format!("number {w}")),
            },
            _ => {
                let Some(n) = self.selected_number.clone() else {
                    self.message = Some((false, "Choose a number first".into()));
                    return;
                };
                let imsi = self.numbers.iter().find(|x| x.0 == n).map(|x| x.1.clone()).unwrap_or_default();
                match cmd {
                    "release" => self.queue.push(format!("number-release {imsi}")),
                    "port-code" => self.queue.push(format!("port-code {imsi}")),
                    "move" => match field(&self.move_to) {
                        w if w.is_empty() => self.message = Some((false, "Type the SIM to move it to (label or IMSI)".into())),
                        w => self.queue.push(format!("number-move {n} {w}")),
                    },
                    _ => {}
                }
            }
        }
    }

    /// IMS page: `ims pool=<p> domain=<d>` (on, or changed) and `ims-off`.
    pub fn on_ims(&mut self, on: bool) {
        if !on {
            self.queue.push("ims-off".into());
            return;
        }
        let pool = self.ims_pool.trim().to_string();
        if pool.is_empty() {
            self.message = Some((false, "Type the IMS pool (like 10.250.0.0/22)".into()));
            return;
        }
        let domain = self.ims_domain.trim();
        self.queue.push(if domain.is_empty() { format!("ims pool={pool}") } else { format!("ims pool={pool} domain={domain}") });
    }

    pub fn on_selected(&mut self, cmd: &str) {
        let Some(imsi) = self.selected.clone() else {
            self.message = Some((false, "Choose a subscriber first".into()));
            return;
        };
        match cmd {
            "quota" => match self.quota.trim().parse::<u64>() {
                Ok(q) => self.queue.push(format!("quota {imsi} {q}")),
                Err(_) => self.message = Some((false, "The quota is a number of bytes (0 = unlimited)".into())),
            },
            c => self.queue.push(format!("{c} {imsi}")),
        }
    }
}

pub type M = Rc<RefCell<Model>>;

pub struct ConsoleApp<A: Admin> {
    pub app: App,
    pub model: M,
    pub admin: A,
    pub window: u32,
    last_poll: Option<u64>,
}

impl<A: Admin> ConsoleApp<A> {
    pub fn new(conn: Box<dyn Connection>, admin: A) -> Result<Self, ConnError> {
        let mut app = App::with_connection(cored::APP_ID, conn)?;
        let model: M = Rc::new(RefCell::new(Model::default()));
        let v = model.clone();
        let window = app.window("Carrier Console", move |cx| view(&v, cx));
        Ok(ConsoleApp { app, model, admin, window, last_poll: None })
    }

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

    pub fn process(&mut self, now_ms: u64) {
        let mut changed = false;
        let cmds: Vec<String> = std::mem::take(&mut self.model.borrow_mut().queue);
        if !cmds.is_empty() {
            // "Writing..." before a command that may wait for the card writer
            self.redraw();
        }
        for c in cmds {
            let r = self.admin.call(&c);
            changed = true;
            let mut m = self.model.borrow_mut();
            m.refresh = true;
            match r {
                Ok(Ok(body)) => m.message = Some((true, done_text(&c, &body))),
                Ok(Err(e)) => m.message = Some((false, e)),
                Err(e) => m.daemon = Daemon::Down(e),
            }
            if c.starts_with("site-new ") {
                if let Some((true, _)) = &m.message {
                    let id = c.split_whitespace().nth(1).unwrap_or("").to_string();
                    let secret = m.message.as_ref().and_then(|(_, t)| t.rsplit(' ').next()).unwrap_or("").to_string();
                    m.credential = Some((id, secret));
                    m.new_site.clear();
                }
            }
            if c.starts_with("sim-new") && matches!(m.message, Some((true, _))) {
                m.new_label.clear();
                m.new_quota.clear();
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

    fn poll(&mut self) -> bool {
        let mut m = self.model.borrow_mut();
        m.refresh = false;
        let before = (m.daemon.clone(), m.status.clone(), m.subs.clone(), m.sites.clone(), m.sessions.clone(), m.handovers.clone());
        let (blocks, numbers) = (m.blocks.clone(), m.numbers.clone());
        match self.admin.call("status") {
            Ok(Ok(body)) => {
                m.daemon = Daemon::Running;
                m.sessions = parse_sessions(&body);
                m.status = body
                    .iter()
                    .filter(|l| !l.starts_with("site ") && !l.starts_with("session "))
                    .map(|l| {
                        let (k, v) = l.split_once(' ').unwrap_or((l, ""));
                        (k.to_string(), v.to_string())
                    })
                    .collect();
                if let Ok(Ok(b)) = self.admin.call("subs") {
                    m.subs = parse_subs(&b);
                }
                if let Ok(Ok(b)) = self.admin.call("sites") {
                    m.sites = parse_sites(&b);
                }
                if let Ok(Ok(b)) = self.admin.call("handovers 30") {
                    m.handovers = b;
                }
                if let Ok(Ok(b)) = self.admin.call("blocks") {
                    m.blocks = parse_blocks(&b);
                }
                if let Ok(Ok(b)) = self.admin.call("numbers") {
                    m.numbers = b
                        .iter()
                        .filter_map(|l| {
                            let w: Vec<&str> = l.split_whitespace().collect();
                            (w.len() >= 4 && w[0] == "num").then(|| (w[1].to_string(), w[2].to_string(), if w[3] == "-" { String::new() } else { w[3].to_string() }))
                        })
                        .collect();
                }
            }
            Ok(Err(e)) | Err(e) => m.daemon = Daemon::Down(e),
        }
        if m.selected.as_ref().is_some_and(|s| !m.subs.iter().any(|x| &x.imsi == s)) {
            m.selected = None;
        }
        if m.selected_number.as_ref().is_some_and(|n| !m.numbers.iter().any(|x| &x.0 == n)) {
            m.selected_number = None;
        }
        before != (m.daemon.clone(), m.status.clone(), m.subs.clone(), m.sites.clone(), m.sessions.clone(), m.handovers.clone()) || (blocks, numbers) != (m.blocks.clone(), m.numbers.clone())
    }

    fn redraw(&mut self) {
        self.app.core.mark_all_dirty();
        let _ = self.app.core.render();
    }
}

/// What the message line says after a command worked.
fn done_text(cmd: &str, body: &[String]) -> String {
    let w: Vec<&str> = cmd.split_whitespace().collect();
    match w.first().copied().unwrap_or("") {
        "sim-new" => {
            let imsi = value(body, "imsi").unwrap_or("?");
            let label = value(body, "label").filter(|l| *l != "-").map(|l| format!(" ({l})")).unwrap_or_default();
            let where_ = if value(body, "ejected") == Some("1") { "it was pushed out of the reader" } else { "take it out of the reader" };
            format!("Wrote SIM {imsi}{label}: {where_}")
        }
        "site-new" => format!("Site {} added. Its credential: {}", w.get(1).unwrap_or(&""), value(body, "secret").unwrap_or("")),
        "port-code" => format!("Port-out code: {} (give it to the subscriber; it moves the number to another carrier)", body.first().and_then(|l| l.rsplit(' ').next()).unwrap_or("?")),
        _ => body.first().map(|l| capitalize(l)).unwrap_or_else(|| "Done".into()),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

// ------------------------------------------------------------ the window

const LINE: i32 = 15;
const TITLE: i32 = 20;
const TABS_H: i32 = LINE + 8 + 1;

pub fn view(m: &M, cx: &ViewCtx) -> Element {
    let st = m.borrow();
    let pad = if cx.class == SizeClass::Compact { 6 } else { 10 };
    let body_h = cx.height - 2 * pad - (TITLE + 4 + 16 + 8);
    let body = if let Daemon::Down(why) = &st.daemon {
        not_running(cx, why)
    } else if cx.class == SizeClass::Wide {
        wide(m, &st, cx, body_h)
    } else {
        tabbed(m, &st, cx, body_h)
    };
    column().gap(8).padding(pad).child(header(&st, cx)).child(body)
}

fn header(st: &Model, cx: &ViewCtx) -> Element {
    let title = if st.get("carrier").is_empty() { "Carrier Console".to_string() } else { format!("Carrier Console  {}", st.get("carrier")) };
    let mut pills = row().gap(6);
    match &st.daemon {
        Daemon::Down(_) => pills = pills.child(pill("cored not running", GREY)),
        Daemon::Unknown => pills = pills.child(pill("Connecting", GREY)),
        Daemon::Running => {
            let up = st.sites.iter().filter(|s| s.connected).count();
            let col = if st.sites.is_empty() { GREY } else if up == st.sites.len() { GREEN } else if up > 0 { AMBER } else { RED };
            pills = pills.child(pill(&format!("{up}/{} sites", st.sites.len()), col));
            pills = pills.child(pill(&format!("{} phones", st.sessions.len()), if st.sessions.is_empty() { GREY } else { GREEN }));
            if cx.class != SizeClass::Compact {
                pills = pills.child(label(format!("pool {} · gateway {}", st.get("pool"), st.get("gateway"))).style("caption").wrap(false));
            }
        }
    }
    column().gap(4).child(label(title).style("title").wrap(false)).child(pills)
}

fn not_running(cx: &ViewCtx, why: &str) -> Element {
    card(
        cx,
        "cored is not running",
        column()
            .gap(6)
            .child(label("This computer's carrier core isn't answering. In a terminal:"))
            .child(label("cored init <carrier> <pool>   (once)").color(cx.theme.accent))
            .child(label("cored start &").color(cx.theme.accent))
            .child(label(format!("({why})")).style("caption")),
    )
}

fn sub_line(s: &SubView, compact: bool) -> Element {
    let name = if s.label.is_empty() { s.imsi.clone() } else { format!("{}  {}", s.label, s.imsi) };
    let usage = if s.quota > 0 { format!("{} of {}", fmt_bytes(s.used), fmt_bytes(s.quota)) } else { fmt_bytes(s.used) };
    let mut tags = row().gap(4);
    if s.revoked {
        tags = tags.child(pill("revoked", RED));
    } else if s.online {
        tags = tags.child(pill("online", GREEN));
    }
    if s.quota > 0 && s.used >= s.quota {
        tags = tags.child(pill("over quota", AMBER));
    }
    if compact {
        column()
            .gap(1)
            .child(row().gap(6).child(label(name).wrap(false)).child(spacer()).child(tags))
            .child(label(format!("{} · {}", if s.ip == "-" { "no address yet" } else { &s.ip }, usage)).style("caption").wrap(false))
    } else {
        row()
            .gap(8)
            .child(label(name).width(200).wrap(false))
            .child(label(s.ip.clone()).width(90).wrap(false))
            .child(label(usage).grow(1).wrap(false))
            .child(tags)
    }
}

fn subscribers(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let compact = cx.class == SizeClass::Compact;
    let mut c = column().gap(8);
    // New SIM
    let (l1, l2, go) = (m.clone(), m.clone(), m.clone());
    let new_sim = column()
        .gap(4)
        .child(label("Put a blank SIM card in the Card Reader/Writer, then:").style("caption"))
        .child(
            row()
                .gap(6)
                .child(text_field().id("new_label").placeholder("Label (alice)").text(st.new_label.clone()).grow(1).on_change(move |cx, _| {
                    let t = cx.text("new_label");
                    l1.borrow_mut().new_label = t;
                }))
                .child(text_field().id("new_quota").placeholder("Quota bytes").text(st.new_quota.clone()).width(if compact { 80 } else { 110 }).on_change(move |cx, _| {
                    let t = cx.text("new_quota");
                    l2.borrow_mut().new_quota = t;
                })),
        )
        .child(row().child(button("Write SIM").style("primary").id("write_sim").on_click(move |_, _| go.borrow_mut().write_sim())).child(spacer()));
    c = c.child(card(cx, "New SIM", new_sim));
    // the list
    let sel = st.selected.as_ref().and_then(|s| st.subs.iter().position(|x| &x.imsi == s));
    let mut lst = column().gap(4);
    if st.subs.is_empty() {
        lst = lst.child(label("No subscribers yet. Write a SIM above.").style("caption"));
    } else {
        let imsis: Vec<String> = st.subs.iter().map(|s| s.imsi.clone()).collect();
        let mm = m.clone();
        let mut l = list().id("subs").children(st.subs.iter().map(|s| sub_line(s, compact))).on_select(move |cx, _| {
            let i = cx.selected("subs");
            let mut md = mm.borrow_mut();
            md.selected = i.and_then(|i| imsis.get(i).cloned());
            let q = md.selected_sub().map(|s| s.quota.to_string()).unwrap_or_default();
            md.quota = q;
        });
        if let Some(i) = sel {
            l = l.selected(i);
        }
        lst = lst.child(l);
    }
    // actions on the chosen one
    if let Some(s) = st.selected_sub() {
        let (r, q, qs, rel) = (m.clone(), m.clone(), m.clone(), m.clone());
        let revoked = s.revoked;
        lst = lst.child(label(format!("{}{}", s.imsi, if s.label.is_empty() { String::new() } else { format!(" ({})", s.label) })).style("caption")).child(
            row()
                .gap(6)
                .child(button(if revoked { "Reinstate" } else { "Revoke" }).id("revoke").style(if revoked { "primary" } else { "danger" }).on_click(move |_, _| {
                    r.borrow_mut().on_selected(if revoked { "unrevoke" } else { "revoke" })
                }))
                .child(text_field().id("quota").placeholder("Quota bytes").text(st.quota.clone()).width(90).on_change(move |cx, _| {
                    let t = cx.text("quota");
                    q.borrow_mut().quota = t;
                }))
                .child(button("Set quota").id("set_quota").on_click(move |_, _| qs.borrow_mut().on_selected("quota")))
                .child(button("Release").id("release").enabled(s.online).on_click(move |_, _| rel.borrow_mut().on_selected("release"))),
        );
    }
    c = c.child(card(cx, &format!("Subscribers ({})", st.subs.len()), lst));
    c.child(message(cx, &st.message))
}

fn sites(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let mut c = column().gap(6);
    if st.sites.is_empty() {
        c = c.child(label("No sites yet. Add one below and put its credential in the site's settings.").style("caption"));
    }
    for s in &st.sites {
        let state = if s.connected { pill("connected", GREEN) } else { pill("not connected", GREY) };
        let detail = if s.connected {
            format!(
                "from {} · cells {} · heartbeat {:.1} s ago · up {}{}",
                s.peer,
                s.cells,
                s.heartbeat_ms.unwrap_or(0) as f64 / 1000.0,
                fmt_secs(s.up_s.unwrap_or(0)),
                if s.rejected > 0 { format!(" · {} rejected", s.rejected) } else { String::new() }
            )
        } else {
            "waiting for the site to connect".into()
        };
        c = c.child(column().gap(1).child(row().gap(6).child(label(s.id.clone()).wrap(false)).child(state)).child(label(detail).style("caption")));
    }
    let (a, go) = (m.clone(), m.clone());
    let mut add = column()
        .gap(4)
        .child(
            row()
                .gap(6)
                .child(text_field().id("new_site").placeholder("Site id (site-b)").text(st.new_site.clone()).grow(1).on_change(move |cx, _| {
                    let t = cx.text("new_site");
                    a.borrow_mut().new_site = t;
                }))
                .child(button("Add site").id("add_site").on_click(move |_, _| go.borrow_mut().add_site())),
        );
    if let Some((id, secret)) = &st.credential {
        add = add.child(label(format!("On {id}'s computer: celld-site config set site_id {id} secret {secret}")).style("caption"));
    }
    column()
        .gap(8)
        .child(card(cx, &format!("Sites ({})", st.sites.len()), c))
        .child(card(cx, "Add a site", add))
        .child(message(cx, &st.message))
}

fn phones(st: &Model, cx: &ViewCtx) -> Element {
    let compact = cx.class == SizeClass::Compact;
    let mut c = column().gap(4);
    if st.sessions.is_empty() {
        c = c.child(label("No phones attached.").style("caption"));
    }
    for s in &st.sessions {
        let label_of = st.subs.iter().find(|x| x.imsi == s.imsi).map(|x| x.label.clone()).filter(|l| !l.is_empty());
        let who = label_of.map(|l| format!("{l}  {}", s.imsi)).unwrap_or_else(|| s.imsi.clone());
        let at = if s.site == "-" { "idle".to_string() } else { format!("{} cell {}", s.site, s.cell) };
        if compact {
            c = c.child(column().gap(1).child(label(who).wrap(false)).child(label(format!("{} · {} · {} · {}", s.ip, at, s.state, fmt_bytes(s.used))).style("caption").wrap(false)));
        } else {
            c = c.child(
                row()
                    .gap(8)
                    .child(label(who).width(200).wrap(false))
                    .child(label(s.ip.clone()).width(90).wrap(false))
                    .child(label(at).width(110).wrap(false))
                    .child(label(s.state.clone()).width(70).wrap(false))
                    .child(label(fmt_bytes(s.used)).style("caption").wrap(false)),
            );
        }
    }
    card(cx, &format!("Phones ({})", st.sessions.len()), c)
}

fn handovers(st: &Model, cx: &ViewCtx) -> Element {
    let mut c = column().gap(3);
    if st.handovers.is_empty() {
        c = c.child(label("No handovers yet.").style("caption"));
    }
    for l in st.handovers.iter().rev() {
        c = c.child(label(l.clone()).wrap(true));
    }
    card(cx, "Handover log", c)
}

fn core_card(st: &Model, cx: &ViewCtx) -> Element {
    let rows = vec![
        ("Carrier", st.get("carrier").to_string()),
        ("Phone pool", st.get("pool").to_string()),
        ("Gateway", format!("{} on {}", st.get("gateway"), st.get("tun"))),
        ("Sites port", format!("{} (TCP), tunnels {} (UDP)", st.get("listen"), st.get("tunnel_port"))),
        ("Database", st.get("db").to_string()),
        ("Subscribers", st.get("subscribers").to_string()),
        ("Sites", format!("{} connected of {}", st.get("sites_connected"), st.get("sites_configured"))),
        ("Rejected", format!("{} tunnel datagrams", st.get("tunnel_rejects"))),
        ("Uptime", fmt_secs(st.get("uptime_s").parse().unwrap_or(0))),
    ];
    card(
        cx,
        "Core",
        column()
            .gap(6)
            .child(kv_rows(&rows, 80))
            .child(label("Settings live in /etc/cored.toml (cored config set <key> <value>); the network in the database.").style("caption")),
    )
}

/// `block <NPA-NXX> <used> used <free> free` lines.
fn parse_blocks(body: &[String]) -> Vec<(String, u32, u32)> {
    body.iter()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            (w.len() >= 6 && w[0] == "block").then(|| (w[1].to_string(), w[2].parse().unwrap_or(0), w[4].parse().unwrap_or(0)))
        })
        .collect()
}

/// `+12145550142` as `(214) 555-0142`.
fn pretty_number(n: &str) -> String {
    let d = n.strip_prefix("+1").unwrap_or("");
    if d.len() == 10 && d.bytes().all(|b| b.is_ascii_digit()) {
        format!("({}) {}-{}", &d[..3], &d[3..6], &d[6..])
    } else {
        n.to_string()
    }
}

fn numbers_page(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    // blocks
    let mut blocks = column().gap(4);
    if st.blocks.is_empty() {
        blocks = blocks.child(label("No number blocks. Add one (NPA-NXX: an area code and an exchange, 10,000 numbers).").style("caption"));
    }
    for (b, used, free) in &st.blocks {
        let rm = m.clone();
        let cmd = format!("block-remove {b}");
        blocks = blocks.child(
            row()
                .gap(8)
                .child(label(b.clone()).width(80).wrap(false))
                .child(label(format!("{used} used · {free} free")).style("caption").grow(1).wrap(false))
                .child(button("Remove").id(format!("block_rm_{b}")).enabled(*used == 0).on_click(move |_, _| rm.borrow_mut().queue.push(cmd.clone()))),
        );
    }
    let (nb, add) = (m.clone(), m.clone());
    blocks = blocks.child(
        row()
            .gap(6)
            .child(text_field().id("new_block").placeholder("214-555").text(st.new_block.clone()).grow(1).on_change(move |cx, _| {
                let t = cx.text("new_block");
                nb.borrow_mut().new_block = t;
            }))
            .child(button("Add block").id("add_block").on_click(move |_, _| add.borrow_mut().on_number("block-add"))),
    );
    // numbers
    let mut nums = column().gap(4);
    let without = st.subs.len().saturating_sub(st.numbers.len());
    if st.numbers.is_empty() {
        nums = nums.child(label("No numbers given out yet.").style("caption"));
    } else {
        let keys: Vec<String> = st.numbers.iter().map(|x| x.0.clone()).collect();
        let mm = m.clone();
        let sel = st.selected_number.as_ref().and_then(|n| keys.iter().position(|k| k == n));
        let mut l = list()
            .id("numbers")
            .children(st.numbers.iter().map(|(n, imsi, lab)| {
                row()
                    .gap(8)
                    .child(label(pretty_number(n)).width(110).wrap(false))
                    .child(label(if lab.is_empty() { imsi.clone() } else { format!("{lab}  {imsi}") }).style("caption").grow(1).wrap(false))
            }))
            .on_select(move |cx, _| {
                let i = cx.selected("numbers");
                mm.borrow_mut().selected_number = i.and_then(|i| keys.get(i).cloned());
            });
        if let Some(i) = sel {
            l = l.selected(i);
        }
        nums = nums.child(l);
    }
    if let Some(n) = &st.selected_number {
        let (rel, port, mv, mt) = (m.clone(), m.clone(), m.clone(), m.clone());
        nums = nums.child(label(pretty_number(n)).style("caption")).child(
            row()
                .gap(6)
                .child(button("Release").id("number_release").style("danger").on_click(move |_, _| rel.borrow_mut().on_number("release")))
                .child(button("Port-out code").id("port_code").on_click(move |_, _| port.borrow_mut().on_number("port-code")))
                .child(text_field().id("move_to").placeholder("Move to SIM").text(st.move_to.clone()).width(110).on_change(move |cx, _| {
                    let t = cx.text("move_to");
                    mt.borrow_mut().move_to = t;
                }))
                .child(button("Move").id("number_move").on_click(move |_, _| mv.borrow_mut().on_number("move"))),
        );
    }
    let (nf, give) = (m.clone(), m.clone());
    let give_row = column()
        .gap(4)
        .child(label(format!("{without} subscriber(s) without a number. New SIMs get one by themselves.")).style("caption"))
        .child(
            row()
                .gap(6)
                .child(text_field().id("number_for").placeholder("Label or IMSI").text(st.number_for.clone()).grow(1).on_change(move |cx, _| {
                    let t = cx.text("number_for");
                    nf.borrow_mut().number_for = t;
                }))
                .child(button("Give a number").id("give_number").on_click(move |_, _| give.borrow_mut().on_number("number"))),
        );
    column()
        .gap(8)
        .child(card(cx, &format!("Number blocks ({})", st.blocks.len()), blocks))
        .child(card(cx, &format!("Numbers ({})", st.numbers.len()), nums))
        .child(card(cx, "Give a number", give_row))
        .child(message(cx, &st.message))
}

fn ims_page(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    let on = st.get("ims") == "on";
    let state = if on { pill("IMS on", GREEN) } else { pill("IMS off", GREY) };
    let rows = if on {
        vec![
            ("Domain", st.get("ims_domain").to_string()),
            ("IMS pool", st.get("ims_pool").to_string()),
            ("P-CSCF", st.get("pcscf").to_string()),
            ("MMSC", st.get("mmsc").to_string()),
            ("HSS port", st.get("hss_port").to_string()),
            ("Country code", st.get("cc").to_string()),
            ("Policy drops", format!("{} packets outside the IMS network", st.get("ims_policy_drops"))),
        ]
    } else {
        vec![("Calls", "phones get no IMS bearer: data only".to_string()), ("HSS port", st.get("hss_port").to_string())]
    };
    let info = column()
        .gap(6)
        .child(state)
        .child(kv_rows(&rows, 90))
        .child(label("imsd, smscd, mmscd and mrfd run beside cored (imsd status, ...); see man telephony.").style("caption"));
    let (p, d, go, off) = (m.clone(), m.clone(), m.clone(), m.clone());
    let form = column()
        .gap(4)
        .child(label(if on { "Change the IMS network:" } else { "Give phones an IMS bearer (calls and texts):" }).style("caption"))
        .child(
            row()
                .gap(6)
                .child(text_field().id("ims_pool").placeholder("10.250.0.0/22").text(st.ims_pool.clone()).grow(1).on_change(move |cx, _| {
                    let t = cx.text("ims_pool");
                    p.borrow_mut().ims_pool = t;
                }))
                .child(text_field().id("ims_domain").placeholder("carrier.ecm").text(st.ims_domain.clone()).grow(1).on_change(move |cx, _| {
                    let t = cx.text("ims_domain");
                    d.borrow_mut().ims_domain = t;
                })),
        )
        .child(
            row()
                .gap(6)
                .child(button(if on { "Apply" } else { "Turn on" }).style("primary").id("ims_on").on_click(move |_, _| go.borrow_mut().on_ims(true)))
                .child(button("Turn off").style("danger").id("ims_off").enabled(on).on_click(move |_, _| off.borrow_mut().on_ims(false))),
        );
    column().gap(8).child(card(cx, "IMS", info)).child(card(cx, "Settings", form)).child(message(cx, &st.message))
}

fn page(m: &M, st: &Model, cx: &ViewCtx) -> Element {
    match st.page {
        Page::Subscribers => subscribers(m, st, cx),
        Page::Sites => sites(m, st, cx),
        Page::Phones => phones(st, cx),
        Page::Handovers => handovers(st, cx),
        Page::Numbers => numbers_page(m, st, cx),
        Page::Ims => ims_page(m, st, cx),
        Page::Core => core_card(st, cx),
    }
}

fn tabbed(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let compact = cx.class == SizeClass::Compact;
    let sel = Page::ALL.iter().position(|p| *p == st.page).unwrap_or(0);
    let mut pages: Vec<Element> = (0..Page::ALL.len()).map(|_| spacer()).collect();
    pages[sel] = scroll_view(page(m, st, cx)).id(format!("page_{sel}")).height((h - TABS_H).max(60));
    let t = m.clone();
    tabs(Page::ALL.iter().map(|p| p.title(compact))).id("tabs").selected(sel).grow(1).children(pages).on_select(move |cx, _| {
        let i = cx.selected("tabs").unwrap_or(0);
        t.borrow_mut().page = Page::ALL[i.min(Page::ALL.len() - 1)];
    })
}

fn wide(m: &M, st: &Model, cx: &ViewCtx, h: i32) -> Element {
    let sel = Page::ALL.iter().position(|p| *p == st.page).unwrap_or(0);
    let t = m.clone();
    let nav = list()
        .id("nav")
        .selected(sel)
        .children(Page::ALL.iter().map(|p| {
            let n = match p {
                Page::Subscribers => st.subs.len().to_string(),
                Page::Sites => format!("{}/{}", st.sites.iter().filter(|s| s.connected).count(), st.sites.len()),
                Page::Phones => st.sessions.len().to_string(),
                Page::Numbers => st.numbers.len().to_string(),
                Page::Ims => if st.get("ims") == "on" { "on".into() } else { "off".into() },
                _ => String::new(),
            };
            row().gap(6).child(label(p.title(false)).wrap(false)).child(spacer()).child(label(n).style("caption").wrap(false))
        }))
        .on_select(move |cx, _| {
            let i = cx.selected("nav").unwrap_or(0);
            t.borrow_mut().page = Page::ALL[i.min(Page::ALL.len() - 1)];
        });
    let brief = column()
        .gap(3)
        .child(label(format!("Carrier {}", st.get("carrier"))).style("caption"))
        .child(label(format!("Pool {}", st.get("pool"))).style("caption"))
        .child(label(format!("Up {}", fmt_secs(st.get("uptime_s").parse().unwrap_or(0)))).style("caption"));
    let side = panel().fill(cx.theme.surface).radius(6).padding(8).child(column().gap(12).width(170).child(nav).child(brief));
    let main = scroll_view(page(m, st, cx)).id(format!("page_{sel}")).height(h).grow(1);
    row().gap(12).align(Align::Start).child(side).child(main)
}

#[cfg(test)]
mod tests;
