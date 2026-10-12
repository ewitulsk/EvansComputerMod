//! `settings`: Wi-Fi (scan and the one-step join of the `wifi` command, with
//! the same fix-it messages), cellular modems, the display panel, the
//! theme / shell / autostart (written to `/etc/ui.toml`; the desktop sends
//! the new theme, so every open app restyles live), Apps (installed apps,
//! their permissions, the files they were given and the types they open by
//! default: `ecm_apps_ui::settings::AppsSection` over `app_ctl`) and About.
//!
//! The layout adapts to the size class: a sidebar of sections beside the
//! page on medium and wide windows (wider on wide), and on compact ones a
//! header with the section's name and a Sections menu above the page.
//!
//!   settings [wifi|cellular|display|appearance|apps|about]

use crate::sys::{self, CellState, DisplayState, Network, Sys, WifiState};
use crate::uitoml;
use ecm_apps_ui::appctl::AppCtl;
use ecm_apps_ui::settings::AppsSection;
use ecm_ui::element::{button, column, label, list, responsive, row, scroll_view, spacer, text_field, toggle, Align, Element};
use ecm_ui::window::EventValue;
use ecm_ui::{App, Ctx, Theme};
use ecm_ui_proto as proto;
use std::cell::RefCell;
use std::rc::Rc;

/// The sections, in order.
pub const SECTIONS: &[&str] = &["Wi-Fi", "Cellular", "Display", "Appearance", "Apps", "About"];

/// The Apps section's index in [`SECTIONS`].
pub const APPS: usize = 4;

/// The shells Settings offers (program / app id, label).
pub const SHELLS: &[(&str, &str)] = &[("dev.ecm.desktop", "Desktop"), ("test-shell", "Test shell (tiling)")];

/// A section by its command-line name.
pub fn section_named(s: &str) -> Option<usize> {
    Some(match s {
        "wifi" | "wi-fi" => 0,
        "cellular" | "cell" => 1,
        "display" => 2,
        "appearance" | "theme" => 3,
        "apps" | "permissions" => APPS,
        "about" => 5,
        _ => return None,
    })
}

/// What Settings shows.
#[derive(Default)]
pub struct State {
    pub section: usize,
    pub wifi: WifiState,
    pub networks: Vec<Network>,
    pub scanned: bool,
    pub scan_error: Option<String>,
    pub chosen: Option<usize>,
    /// Progress lines of the last join.
    pub log: Vec<String>,
    /// The last join's outcome: Ok(final line) or Err(problem text).
    pub result: Option<Result<String, String>>,
    pub cells: Vec<CellState>,
    pub display: Option<DisplayState>,
    pub gui: Option<proto::DisplayInfo>,
    pub theme: String,
    pub shell: String,
    pub autostart: bool,
    pub notice: String,
    /// Settings -> Apps (the app manager through `app_ctl`).
    pub apps: Option<AppsSection>,
}

type Shared = Rc<RefCell<State>>;
type SysRef = Rc<RefCell<Box<dyn Sys>>>;

fn read_toml(sys: &SysRef) -> String {
    sys::read_file(&**sys.borrow(), uitoml::PATH).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default()
}

/// Set a key in /etc/ui.toml.
fn write_toml(sys: &SysRef, key: &str, value: &str) -> Result<(), String> {
    let text = uitoml::set(&read_toml(sys), key, value);
    let s = sys.borrow();
    let _ = std::fs::create_dir_all(sys::host_path(&s.root(), "/etc"));
    sys::write_file(&**s, uitoml::PATH, text.as_bytes())
}

/// Re-read everything Settings shows from the computer.
pub fn refresh(st: &Shared, sys: &SysRef) {
    let toml = read_toml(sys);
    let (wifi, cells, display) = {
        let mut s = sys.borrow_mut();
        (s.wifi(), s.cells(), s.display())
    };
    let mut s = st.borrow_mut();
    s.wifi = wifi;
    s.cells = cells;
    s.display = display;
    s.theme = uitoml::get(&toml, "theme").unwrap_or_else(|| "ecm-default".into());
    s.shell = uitoml::get(&toml, "shell").unwrap_or_else(|| "dev.ecm.desktop".into());
    s.autostart = uitoml::get(&toml, "autostart").as_deref() == Some("true");
}

fn scan(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    st.borrow_mut().notice = "Scanning...".into();
    cx.invalidate();
    let _ = cx.core().render();
    let r = sys.borrow_mut().wifi_scan();
    let mut s = st.borrow_mut();
    s.notice.clear();
    s.scanned = true;
    s.chosen = None;
    match r {
        Ok(n) => {
            s.networks = n;
            s.scan_error = None;
        }
        Err(e) => {
            s.networks.clear();
            s.scan_error = Some(e);
        }
    }
    cx.set_selected("nets", None);
}

fn join(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    let Some(net) = ({
        let s = st.borrow();
        s.chosen.and_then(|i| s.networks.get(i).cloned())
    }) else {
        return;
    };
    let pass = cx.text("pass");
    let password = if net.secured() && !pass.is_empty() { Some(pass) } else { None };
    {
        let mut s = st.borrow_mut();
        s.log = vec![format!("Joining '{}'...", net.ssid)];
        s.result = None;
    }
    cx.invalidate();
    let _ = cx.core().render();
    let r = {
        let mut progress = |line: &str| {
            st.borrow_mut().log.push(line.to_string());
            cx.invalidate();
            let _ = cx.core().render();
        };
        sys.borrow_mut().wifi_connect(&net.ssid, password.as_deref(), &mut progress)
    };
    let wifi = sys.borrow_mut().wifi();
    let mut s = st.borrow_mut();
    s.result = Some(r);
    s.wifi = wifi;
}

/// The `wifi` command's fix-it text, worded for this window.
pub fn for_window(problem: &str) -> String {
    problem
        .trim_end()
        .trim_start_matches("wifi: ")
        .replace("Then run this command again.", "Then press Scan again.")
        .replace("Run this command again", "Press Scan again")
}

fn kv(k: &str, v: impl Into<String>) -> Element {
    kv_w(k, v, 96)
}

/// A key and value with a key column `w` wide.
fn kv_w(k: &str, v: impl Into<String>, w: i32) -> Element {
    row().gap(8).child(label(k).width(w).style("caption").wrap(false)).child(label(v.into()))
}

/// A page's title (compact windows show it in the header instead).
fn title(s: &str) -> Element {
    label(s).style("title").wrap(false).visible_in(false, true, true)
}

fn wifi_page(s: &State, st: &Shared, sys: &SysRef, compact: bool) -> Element {
    let mut c = column().gap(8).child(title("Wi-Fi"));
    let w = &s.wifi;
    if let Some(p) = &w.problem {
        c = c.child(label(for_window(p)).id("wifi_problem"));
    } else if w.connected {
        let sig = w.signal_dbm.map(|d| format!(", {d} dBm")).unwrap_or_default();
        c = c
            .child(label(format!("Connected to '{}'{}", w.ssid, sig)).id("wifi_status"))
            .child(label(format!("Address {}", w.address.clone().unwrap_or_else(|| "none yet".into()))).style("caption"));
    } else {
        c = c.child(label("Not connected. Scan, pick a network and join it.").id("wifi_status"));
    }
    let (a, ya, b, yb) = (st.clone(), sys.clone(), st.clone(), sys.clone());
    let mut actions = row().gap(6).child(button("Scan").id("scan").on_click(move |cx, _| scan(cx, &a, &ya)));
    if w.connected {
        actions = actions.child(button("Disconnect").id("disconnect").on_click(move |cx, _| {
            let r = yb.borrow_mut().wifi_disconnect();
            let wifi = yb.borrow_mut().wifi();
            let mut s = b.borrow_mut();
            s.wifi = wifi;
            s.notice = match r {
                Ok(()) => "Disconnected".into(),
                Err(e) => e,
            };
            cx.invalidate();
        }));
    }
    if !s.notice.is_empty() {
        actions = actions.child(label(s.notice.clone()).style("caption").wrap(false));
    }
    c = c.child(actions);
    if let Some(e) = &s.scan_error {
        c = c.child(label(for_window(e)).id("scan_error"));
    } else if s.scanned && s.networks.is_empty() {
        c = c.child(label("No Wi-Fi networks in range.").style("caption"));
    }
    if !s.networks.is_empty() {
        let rows: Vec<Element> = s
            .networks
            .iter()
            .map(|n| {
                let sec = match n.security.as_str() {
                    "open" => "open",
                    "wpa2-psk" => "WPA2",
                    _ => "unsupported",
                };
                let bars: String = (0..4).map(|i| if i < n.bars() { '|' } else { '.' }).collect();
                let mut r = row().gap(8).child(label(n.ssid.clone()).wrap(false)).child(spacer());
                if !compact {
                    r = r.child(label(format!("{} dBm  ch {}", n.signal_dbm, n.channel)).style("caption").wrap(false));
                }
                r.child(label(sec).style("caption").wrap(false)).child(label(bars).wrap(false).font(proto::Font::Mono))
            })
            .collect();
        let ch = st.clone();
        c = c.child(list().id("nets").children(rows).on_select(move |cx, e| {
            if let EventValue::Index(i) = e.value {
                ch.borrow_mut().chosen = Some(i);
                cx.focus("pass");
            }
        }));
    }
    if let Some(n) = s.chosen.and_then(|i| s.networks.get(i)) {
        let (j, yj, k, yk) = (st.clone(), sys.clone(), st.clone(), sys.clone());
        let mut r = row().gap(6).align(Align::Center);
        if n.secured() {
            r = r.child(text_field().id("pass").placeholder("Password").password(true).on_submit(move |cx, _| join(cx, &k, &yk)));
        } else {
            r = r.child(label(format!("'{}' is open (no password).", n.ssid)).style("caption")).child(spacer());
        }
        c = c.child(r.child(button(format!("Join {}", n.ssid)).id("join").style("primary").on_click(move |cx, _| join(cx, &j, &yj))));
    }
    for (i, l) in s.log.iter().enumerate() {
        c = c.child(label(l.clone()).id(format!("log{i}")).style("caption"));
    }
    match &s.result {
        Some(Ok(done)) => c = c.child(label(done.clone()).id("join_ok")),
        Some(Err(p)) => c = c.child(label(for_window(p)).id("join_error").color(0xe5534bff)),
        None => {}
    }
    c
}

/// Signal bars as text: `||||`, `||..`.
fn bar_text(bars: u8) -> String {
    (0..4).map(|i| if i < bars { '|' } else { '.' }).collect()
}

/// Send `cmd` to celld; the answer (or the problem) becomes the page's notice.
fn cell_command(cx: &mut Ctx, st: &Shared, sys: &SysRef, cmd: &str) {
    let r = sys.borrow_mut().cell_command(cmd);
    let cells = sys.borrow_mut().cells();
    let mut s = st.borrow_mut();
    s.cells = cells;
    s.notice = match r {
        Ok(a) => {
            let a = a.trim();
            a.strip_prefix("ok ").map(|x| format!("celld: {x}")).unwrap_or_else(|| a.to_string())
        }
        Err(e) => e,
    };
    cx.invalidate();
}

fn cell_page(s: &State, st: &Shared, sys: &SysRef, compact: bool) -> Element {
    let mut c = column().gap(if compact { 4 } else { 6 }).child(title("Cellular"));
    if s.cells.is_empty() {
        c = c.child(label("This computer has no cellular modem.").id("no_modem")).child(
            label("Fit an LTE Modem or mmWave Modem in a module bay (right-click the computer with a Module Expansion Card, then the modem) and give it a SIM card (an ePhone has a SIM tray).")
                .style("caption"),
        );
    }
    for m in &s.cells {
        if !m.daemon {
            // A v1 modem engine (server config modem.v1Engine).
            c = c
                .child(label(format!("{} ({} modem)", m.name, m.tech)).wrap(false))
                .child(kv("State", format!("{} - {}", m.state, m.message)))
                .child(kv("SIM", m.sim.clone()))
                .child(kv("Carrier", if m.carrier.is_empty() { "-".to_string() } else { m.carrier.clone() }))
                .child(kv("Signal", format!("{} ({} of 4 bars)", m.signal, m.bars)));
            continue;
        }
        let kw = if compact { 56 } else { 96 };
        let tech = match m.tech.as_str() {
            "lte" => "LTE",
            "mmwave" => "mmWave",
            o => o,
        };
        c = c
            .child(
                row()
                    .gap(8)
                    .align(Align::Center)
                    .child(label(format!("{} ({} modem)", m.name, tech)).wrap(false))
                    .child(spacer())
                    .child(label(if m.carrier.is_empty() { "No service".to_string() } else { m.carrier.clone() }).id("cell_carrier").wrap(false))
                    .child(label(bar_text(m.bars)).id("cell_bars").wrap(false).font(proto::Font::Mono)),
            )
            .child(kv_w("State", m.message.clone(), kw).id("cell_state"))
            .child(kv_w("SIM", m.sim.clone(), kw).id("cell_sim"))
            .child(kv_w("Cell", m.cell.clone(), kw))
            .child(kv_w("Signal", m.signal.clone(), kw))
            .child(kv_w("Address", m.address.clone(), kw))
            .child(kv_w("Data", m.data.clone(), kw).id("cell_data"));
        let (r, yr) = (st.clone(), sys.clone());
        c = c.child(toggle("Data roaming").id("roaming").checked(m.roaming).on_change(move |cx, e| {
            if let EventValue::Bool(on) = e.value {
                cell_command(cx, &r, &yr, if on { "roaming on" } else { "roaming off" });
            }
        }));
        if !m.cells.is_empty() {
            c = c.child(label("Cells heard").style("caption"));
            let rows: Vec<Element> = m
                .cells
                .iter()
                .map(|(t, serving)| {
                    let mut r = row().gap(6).child(label(t.clone()).wrap(false));
                    if *serving {
                        r = r.child(spacer()).child(label("serving").style("caption").wrap(false));
                    }
                    r
                })
                .collect();
            c = c.child(list().id("cells").children(rows));
        }
    }
    let (a, ya) = (st.clone(), sys.clone());
    let mut actions = row().gap(6).align(Align::Center).child(button("Refresh").id("refresh_cell").on_click(move |cx, _| {
        refresh(&a, &ya);
        cx.invalidate();
    }));
    if let Some(m) = s.cells.iter().find(|m| m.daemon && m.state != "no daemon") {
        let (b, yb) = (st.clone(), sys.clone());
        let off = m.state == "down";
        actions = actions.child(button(if off { "Turn on" } else { "Turn off" }).id("cell_power").on_click(move |cx, _| {
            cell_command(cx, &b, &yb, if off { "up" } else { "down" });
        }));
    }
    if !s.notice.is_empty() && s.section == 1 {
        actions = actions.child(label(s.notice.clone()).id("cell_notice").style("caption").wrap(false));
    }
    c.child(actions)
}

fn display_page(s: &State) -> Element {
    let mut c = column().gap(6).child(title("Display"));
    match &s.display {
        Some(d) => {
            c = c
                .child(kv("Panel", d.tier_name()))
                .child(kv("Resolution", format!("{} x {} pixels", d.width, d.height)))
                .child(kv("Refresh", format!("{} Hz", d.refresh_hz)))
                .child(kv("Colours", if d.colors.is_empty() { "-".to_string() } else { d.colors.clone() }))
                .child(kv("Console", format!("{} x {} characters", d.text_cols, d.text_rows)))
                .child(kv("Touch", if d.touch { "yes" } else { "no" }));
        }
        None => c = c.child(label("No display panel: the Terminal runs headless.").id("no_panel")),
    }
    if let Some(g) = &s.gui {
        c = c.child(kv("GUI", format!("{} x {} lu, scale {}, {}", g.w, g.h, g.scale, g.size_class.name())));
    }
    c.child(label("Swap the panel by right-clicking the Terminal's screen with another one.").style("caption"))
}

fn appearance_page(s: &State, st: &Shared, sys: &SysRef) -> Element {
    let theme_i = Theme::NAMES.iter().position(|n| *n == s.theme);
    let (a, ya, b, yb, c2, yc) = (st.clone(), sys.clone(), st.clone(), sys.clone(), st.clone(), sys.clone());
    let themes = list().id("themes").children(Theme::NAMES.iter().map(|n| label(Theme::label(n)).wrap(false))).on_select(move |cx, e| {
        if let EventValue::Index(i) = e.value {
            let name = Theme::NAMES[i];
            let r = write_toml(&ya, "theme", &uitoml::quote(name));
            let mut s = a.borrow_mut();
            s.theme = name.to_string();
            s.notice = match r {
                Ok(()) => format!("Theme: {} (every open app restyles)", Theme::label(name)),
                Err(e) => format!("Can't write {}: {e}", uitoml::PATH),
            };
            cx.invalidate();
        }
    });
    let themes = match theme_i {
        Some(i) => themes.selected(i),
        None => themes,
    };
    let shell_i = SHELLS.iter().position(|(id, _)| *id == s.shell || id.rsplit('.').next() == Some(s.shell.as_str()));
    let shells = list().id("shells").children(SHELLS.iter().map(|(_, l)| label(*l).wrap(false))).on_select(move |cx, e| {
        if let EventValue::Index(i) = e.value {
            let (id, l) = SHELLS[i];
            let r = write_toml(&yb, "shell", &uitoml::quote(id));
            let mut s = b.borrow_mut();
            s.shell = id.to_string();
            s.notice = match r {
                Ok(()) => format!("Shell: {l}. It starts with the GUI next time (gui stop, then gui)."),
                Err(e) => format!("Can't write {}: {e}", uitoml::PATH),
            };
            cx.invalidate();
        }
    });
    let shells = match shell_i {
        Some(i) => shells.selected(i),
        None => shells,
    };
    let mut col = column()
        .gap(8)
        .child(title("Appearance"))
        .child(label("Theme").style("caption"))
        .child(themes)
        .child(label("Shell").style("caption"))
        .child(shells)
        .child(toggle("Start the GUI when the computer boots").id("autostart").checked(s.autostart).on_change(move |cx, e| {
            if let EventValue::Bool(on) = e.value {
                let r = write_toml(&yc, "autostart", if on { "true" } else { "false" });
                let mut s = c2.borrow_mut();
                s.autostart = on;
                s.notice = match r {
                    Ok(()) => format!("Autostart {}", if on { "on" } else { "off" }),
                    Err(e) => format!("Can't write {}: {e}", uitoml::PATH),
                };
                cx.invalidate();
            }
        }));
    if !s.notice.is_empty() {
        col = col.child(label(s.notice.clone()).id("notice").style("caption"));
    }
    col
}

fn about_page(s: &State) -> Element {
    let mut c = column()
        .gap(6)
        .child(title("About"))
        .child(label("Terminal OS with the ECM desktop"))
        .child(kv("Protocol", format!("UI protocol v{}", proto::VERSION)))
        .child(kv("Fonts", format!("metrics version {}", proto::fonts::METRICS_VERSION)))
        .child(kv("Shell", s.shell.clone()))
        .child(kv("Theme", Theme::label(&s.theme)));
    if let Some(d) = &s.display {
        c = c.child(kv("Panel", d.tier_name()));
    }
    c.child(label("Apps: Terminal, Files, Text Editor, Settings, Notes, Visual Programmer, App Store.").style("caption"))
}

fn apps_page(vcx: &ecm_ui::ViewCtx, s: &State) -> Element {
    let c = column().gap(8).child(title("Apps"));
    match &s.apps {
        // Beside the sidebar the page is narrower than the window: the list and the app
        // go side by side only when the page has room for both (wide windows).
        Some(a) => c.child(a.view_with(vcx, vcx.width >= 760)),
        None => c.child(label("No app manager on this computer.").style("caption")),
    }
}

fn page(vcx: &ecm_ui::ViewCtx, s: &State, st: &Shared, sys: &SysRef, compact: bool) -> Element {
    let body = match s.section {
        0 => wifi_page(s, st, sys, compact),
        1 => cell_page(s, st, sys, compact),
        2 => display_page(s),
        3 => appearance_page(s, st, sys),
        APPS => apps_page(vcx, s),
        _ => about_page(s),
    };
    // grow: the page takes what the sidebar leaves (and no more).
    scroll_view(body.padding(if compact { 8 } else { 12 })).id("page").grow(1)
}

fn select_section(cx: &mut Ctx, st: &Shared, sys: &SysRef, i: usize) {
    refresh(st, sys);
    if i == APPS {
        if let Some(a) = st.borrow().apps.clone() {
            a.reload();
        }
    }
    let mut s = st.borrow_mut();
    s.section = i.min(SECTIONS.len() - 1);
    s.notice.clear();
    cx.set_selected("sections", Some(s.section));
}

fn view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef) -> Element {
    let s = st.borrow();
    let sidebar = |w: i32| {
        let (a, ya) = (st.clone(), sys.clone());
        list()
            .id("sections")
            .width(w)
            .selected(s.section)
            .children(SECTIONS.iter().map(|n| label(*n).wrap(false)))
            .on_select(move |cx, e| {
                if let EventValue::Index(i) = e.value {
                    select_section(cx, &a, &ya, i);
                }
            })
    };
    let divider = |h: i32| ecm_ui::element::custom().width(1).height(h).nodes(vec![ecm_ui::diff::SceneNode::new(
        "line",
        ecm_ui_proto::NodeKind::Rect,
        vec![ecm_ui_proto::Prop::W(1), ecm_ui_proto::Prop::H(h), ecm_ui_proto::Prop::Fill(vcx.theme.border)],
    )]);
    let (m, ym) = (st.clone(), sys.clone());
    let compact = column()
        .gap(0)
        .child(
            row()
                .gap(6)
                .padding(6)
                .align(Align::Center)
                .child(label(SECTIONS[s.section]).style("title").wrap(false))
                .child(spacer())
                .child(button("Sections").id("sections_menu").on_click(move |cx, _| {
                    let (m2, ym2) = (m.clone(), ym.clone());
                    let (w, _) = cx.window_size();
                    cx.popup_menu((w - 130).max(0), 26, SECTIONS.iter().copied(), move |cx, i| select_section(cx, &m2, &ym2, i));
                })),
        )
        .child(page(vcx, &s, st, sys, true));
    let medium = row().gap(0).align(Align::Stretch).child(column().padding(6).child(sidebar(120))).child(divider(vcx.height)).child(page(vcx, &s, st, sys, false));
    let wide = row().gap(0).align(Align::Stretch).child(column().padding(8).child(sidebar(170))).child(divider(vcx.height)).child(page(vcx, &s, st, sys, false));
    responsive(compact, medium, wide)
}

/// The app manager Settings -> Apps talks to: `app_ctl` on the computer; on
/// the host (tests without one) an absent one, so the section says so.
pub fn default_app_ctl() -> Box<dyn AppCtl> {
    #[cfg(target_arch = "wasm32")]
    {
        Box::new(ecm_apps_ui::appctl::HostCtl)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        Box::new(ecm_apps_ui::appctl::FakeCtl { absent: true, ..Default::default() })
    }
}

/// The Settings app, opened at `section` (a name from [`section_named`]).
pub fn app(conn: Box<dyn ecm_ui::Connection>, sys: Box<dyn Sys>, section: Option<&str>) -> Result<App, ecm_ui::ConnError> {
    app_with(conn, sys, section, default_app_ctl())
}

/// [`app`] with the app manager Settings -> Apps uses (tests and golden sessions pass a `FakeCtl`).
pub fn app_with(conn: Box<dyn ecm_ui::Connection>, sys: Box<dyn Sys>, section: Option<&str>, ctl: Box<dyn AppCtl>) -> Result<App, ecm_ui::ConnError> {
    let mut app = App::with_connection("dev.ecm.settings", conn)?;
    let sys: SysRef = Rc::new(RefCell::new(sys));
    let st: Shared = Rc::new(RefCell::new(State {
        gui: Some(app.core.welcome.display.clone()),
        apps: Some(AppsSection::new(ctl)),
        ..Default::default()
    }));
    st.borrow_mut().section = section.and_then(section_named).unwrap_or(0);
    refresh(&st, &sys);
    let (s1, y1) = (st.clone(), sys.clone());
    app.window("Settings", move |vcx| view(vcx, &s1, &y1));
    // Keep the status pages current while open.
    let (s2, y2) = (st.clone(), sys.clone());
    app.every(3000, move |_| {
        let before = {
            let s = s2.borrow();
            (s.wifi.clone(), s.cells.clone())
        };
        let (wifi, cells) = {
            let mut y = y2.borrow_mut();
            (y.wifi(), y.cells())
        };
        let mut s = s2.borrow_mut();
        let changed = before != (wifi.clone(), cells.clone());
        s.wifi = wifi;
        s.cells = cells;
        changed
    });
    let s3 = st.clone();
    app.on_message(move |cx, m| {
        if let proto::Message::DisplayChanged(d) = m {
            s3.borrow_mut().gui = Some(d.display.clone());
            cx.invalidate();
        }
    });
    Ok(app)
}
