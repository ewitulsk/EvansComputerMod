//! Start menu, Settings quick menu, macOS-style All Apps overlay, and desktop
//! dialogs (`New Folder` / `Rename`) for `dev.ecm.desktop`.
//!
//! The searchable menus and dialogs run as frameless shell-owned `Window`
//! surfaces (`shell:start`, `shell:apps`, `shell:dialog`) so the compositor's
//! `FocusWindow` gives their `text_field` full keyboard and text input with no
//! wire-protocol changes.

use crate::app_catalog::{self, AppItem};
use crate::desktop::{self, Shared, SysRef, R, TASKBAR_H};
use ecm_ui::diff::SceneNode;
use ecm_ui::element::{button, column, custom, image, label, row, spacer, stack, text_field, Align, Element};
use ecm_ui::window::EventValue;
use ecm_ui::{Ctx, Theme};
use ecm_ui_proto as proto;
use ecm_ui_proto::{NodeKind, Prop};

/// Sections offered by the Settings quick menu on the taskbar.
pub const SETTINGS_QUICK_ITEMS: &[(&str, &str)] = &[
    ("Open Settings", ""),
    ("Wi-Fi", "wifi"),
    ("Appearance", "appearance"),
    ("Display", "display"),
    ("Apps", "apps"),
    ("Sound", "sound"),
    ("Bluetooth", "bluetooth"),
    ("About", "about"),
];

/// Compact Settings links shown in the Start menu's right column.
pub const START_SETTINGS_LINKS: &[(&str, &str)] = &[
    ("Settings", ""),
    ("Wi-Fi", "wifi"),
    ("Appearance", "appearance"),
    ("Display", "display"),
    ("Apps", "apps"),
    ("Sound", "sound"),
];

/// Which shell-owned frameless overlay window is currently open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlayKind {
    /// Traditional Start menu anchored above the Start button.
    Start,
    /// macOS-style searchable, paginated All Apps overlay.
    AllApps,
    /// Create a new folder inside `/home/Desktop`.
    NewFolder,
    /// Rename an existing item inside `/home/Desktop`.
    Rename { file_name: String, initial: String },
}

impl OverlayKind {
    pub fn title(&self) -> &'static str {
        match self {
            OverlayKind::Start => "shell:start",
            OverlayKind::AllApps => "shell:apps",
            OverlayKind::NewFolder | OverlayKind::Rename { .. } => "shell:dialog",
        }
    }

    pub fn focus_field(&self) -> &'static str {
        match self {
            OverlayKind::Start => "start:search",
            OverlayKind::AllApps => "apps:search",
            OverlayKind::NewFolder | OverlayKind::Rename { .. } => "dlg:input",
        }
    }
}

/// Active shell overlay window state.
#[derive(Clone, Debug)]
pub struct ShellOverlay {
    pub kind: OverlayKind,
    pub surface: u32,
    pub wid: Option<u32>,
    pub rect: R,
    pub sent: Option<R>,
    pub query: String,
    pub page: usize,
    pub error: String,
}

/// Geometry of each overlay for display `d`.
pub fn overlay_rect(d: &proto::DisplayInfo, kind: &OverlayKind) -> R {
    let wa = desktop::work_area(d);
    let compact = d.size_class == proto::SizeClass::Compact;
    match kind {
        OverlayKind::Start => {
            if compact {
                let w = (wa.2 - 8).max(180);
                let h = (wa.3 - 8).max(120);
                (4, (wa.3 - h - 4).max(0), w, h)
            } else {
                let w = 348.min(wa.2 - 12);
                let h = 284.min(wa.3 - 12);
                (4, (wa.3 - h - 4).max(0), w, h)
            }
        }
        OverlayKind::AllApps => {
            let m = if compact { 4 } else { 12 };
            let w = (wa.2 - 2 * m).max(200);
            let h = (wa.3 - 2 * m).max(140);
            (m, m, w, h)
        }
        OverlayKind::NewFolder | OverlayKind::Rename { .. } => {
            let w = 264.min(wa.2 - 16).max(180);
            let h = 114.min(wa.3 - 16).max(96);
            ((wa.2 - w) / 2, ((wa.3 - h) / 2).max(4), w, h)
        }
    }
}

/// Resolve an [`AppItem`]'s uploaded texture resource id (custom PNG first, else built-in).
pub fn icon_res(st: &desktop::State, item: &AppItem) -> Option<u32> {
    if let Some(path) = &item.custom_icon {
        if let Some(&id) = st.custom_icons.get(path) {
            return Some(id);
        }
    }
    st.icons.get(&item.icon).or_else(|| st.icons.get("app")).copied()
}

fn card_bg(w: i32, h: i32, t: &Theme, border: u32, radius: u16) -> Element {
    custom().id("card:bg").nodes(vec![SceneNode::new(
        "card",
        NodeKind::Rect,
        vec![
            Prop::W(w),
            Prop::H(h),
            Prop::Fill(t.surface),
            Prop::BorderW(1),
            Prop::BorderColor(border),
            Prop::Radius(radius),
        ],
    )])
}

fn hline(id: &str, w: i32, color: u32) -> Element {
    custom()
        .id(id)
        .width(w)
        .height(1)
        .nodes(vec![SceneNode::new("l", NodeKind::Rect, vec![Prop::W(w), Prop::H(1), Prop::Fill(color)])])
}

fn vline(id: &str, h: i32, color: u32) -> Element {
    custom()
        .id(id)
        .width(1)
        .height(h)
        .nodes(vec![SceneNode::new("l", NodeKind::Rect, vec![Prop::W(1), Prop::H(h), Prop::Fill(color)])])
}

fn truncate_to_width(text: &str, max_px: i32, font_size: u8) -> String {
    let Some(face) = ecm_ui_proto::fonts::face(proto::Font::Ui, font_size) else {
        return text.chars().take(12).collect();
    };
    if face.text_width(text) <= max_px {
        return text.to_string();
    }
    let dots = "..";
    let dots_w = face.text_width(dots);
    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        if face.text_width(&out) + dots_w > max_px {
            out.pop();
            break;
        }
    }
    out.push_str(dots);
    out
}

// ------------------------------------------------------------------ Start Menu

/// Build the Start menu view (`shell:start`).
pub fn start_menu_view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef) -> Element {
    let t = vcx.theme;
    // The Start window itself is ~348 lu wide on Medium displays (< 400 lu, so vcx.class is Compact);
    // switch to the single-column layout only on narrow displays where two columns won't fit.
    let compact = vcx.width < 320;
    let (query, items, usage, icons, custom_icons) = {
        let s = st.borrow();
        let q = s.overlay.as_ref().map_or(String::new(), |o| o.query.clone());
        (q, s.launch_items.clone(), s.usage.clone(), s.icons.clone(), s.custom_icons.clone())
    };
    let resolve_icon = |it: &AppItem| -> Option<u32> {
        it.custom_icon
            .as_ref()
            .and_then(|p| custom_icons.get(p).copied())
            .or_else(|| icons.get(&it.icon).copied())
            .or_else(|| icons.get("app").copied())
    };

    let searching = !query.trim().is_empty();
    let max_rows = if compact { 4 } else { 6 };
    let shown: Vec<AppItem> = if searching {
        app_catalog::filter_apps(&items, &usage, &query).into_iter().take(max_rows).collect()
    } else {
        app_catalog::most_used(&items, &usage, max_rows)
    };
    let match_console = searching && "console".contains(&query.trim().to_lowercase());

    // Top search bar.
    let (st_ch, st_sub, sys_sub) = (st.clone(), st.clone(), sys.clone());
    let first_match = shown.first().cloned();
    let search = text_field()
        .id("start:search")
        .placeholder("Search apps...")
        .on_change(move |cx, e| {
            if let EventValue::Text(q) = &e.value {
                if let Some(o) = &mut st_ch.borrow_mut().overlay {
                    o.query = q.clone();
                }
                cx.invalidate();
            }
        })
        .on_submit(move |cx, _| {
            if let Some(it) = &first_match {
                desktop::launch_and_close(cx, &st_sub, &sys_sub, &it.program, &it.args);
            } else if match_console {
                desktop::go_console(cx, &st_sub);
            }
        });

    // Left column: Most Used or Search Results + All Apps button.
    let mut left = column().gap(2).grow(1);
    let heading = if searching {
        format!("Results ({})", shown.len() + usize::from(match_console))
    } else {
        "Most Used".to_string()
    };
    left = left.child(label(heading).id("start:heading").style("caption").wrap(false));
    if shown.is_empty() && !match_console {
        left = left.child(label("No matching apps").id("start:empty").style("caption").wrap(false));
    } else {
        for (i, it) in shown.iter().enumerate() {
            let (st_l, sys_l) = (st.clone(), sys.clone());
            let prog = it.program.clone();
            let args = it.args.clone();
            let icon = resolve_icon(it);
            let mut r = row().gap(6).align(Align::Center);
            if let Some(res) = icon {
                r = r.child(image(res, 16, 16));
            }
            r = r.child(
                button(it.label.clone())
                    .id(format!("start:app:{i}"))
                    .style("flat")
                    .grow(1)
                    .on_click(move |cx, _| desktop::launch_and_close(cx, &st_l, &sys_l, &prog, &args)),
            );
            left = left.child(r);
        }
        if match_console {
            let st_c = st.clone();
            let mut r = row().gap(6).align(Align::Center);
            if let Some(&res) = icons.get("terminal") {
                r = r.child(image(res, 16, 16));
            }
            r = r.child(
                button("Console")
                    .id("start:search_console")
                    .style("flat")
                    .grow(1)
                    .on_click(move |cx, _| desktop::go_console(cx, &st_c)),
            );
            left = left.child(r);
        }
    }
    left = left.child(spacer());
    let (st_all, sys_all) = (st.clone(), sys.clone());
    left = left.child(
        button("All Apps >")
            .id("start:all_apps")
            .style("primary")
            .on_click(move |cx, _| desktop::open_all_apps(cx, &st_all, &sys_all)),
    );

    // Body: two columns on Medium/Wide, single column on Compact.
    let body = if compact {
        left
    } else {
        let mut right = column().gap(2).width(126);
        right = right.child(label("Places").style("caption").wrap(false));
        for (idx, (lbl, path)) in [("Home", "/home"), ("Desktop", "/home/Desktop")].iter().enumerate() {
            let (st_p, sys_p) = (st.clone(), sys.clone());
            let path = (*path).to_string();
            right = right.child(
                button(*lbl)
                    .id(format!("start:place:{idx}"))
                    .style("flat")
                    .on_click(move |cx, _| desktop::launch_and_close(cx, &st_p, &sys_p, "files", &[path.clone()])),
            );
        }
        right = right.child(hline("start:rdiv", 120, t.border));
        right = right.child(label("Settings").style("caption").wrap(false));
        for (idx, (lbl, sec)) in START_SETTINGS_LINKS.iter().enumerate() {
            let (st_s, sys_s) = (st.clone(), sys.clone());
            let sec = (*sec).to_string();
            right = right.child(
                button(*lbl)
                    .id(format!("start:set:{idx}"))
                    .style("flat")
                    .on_click(move |cx, _| desktop::open_settings_section(cx, &st_s, &sys_s, &sec)),
            );
        }
        row()
            .gap(8)
            .grow(1)
            .child(left)
            .child(vline("start:vdiv", (vcx.height - 80).max(80), t.border))
            .child(right)
    };

    // Footer: Console + Settings (on compact) + Exit GUI.
    let (st_con, st_exit) = (st.clone(), st.clone());
    let mut footer = row()
        .gap(6)
        .align(Align::Center)
        .child(button("Console").id("start:console").style("flat").on_click(move |cx, _| desktop::go_console(cx, &st_con)));
    if compact {
        let (st_s, sys_s) = (st.clone(), sys.clone());
        footer = footer.child(button("Settings").id("start:settings_compact").style("flat").on_click(move |cx, _| {
            desktop::open_settings_section(cx, &st_s, &sys_s, "");
        }));
    }
    footer = footer.child(spacer()).child(
        button("Exit GUI")
            .id("start:exit_gui")
            .style("danger")
            .on_click(move |cx, _| desktop::exit_gui(cx, &st_exit)),
    );

    let inner = column()
        .padding(8)
        .gap(6)
        .child(search)
        .child(body)
        .child(hline("start:fdiv", (vcx.width - 16).max(40), t.border))
        .child(footer);

    stack().child(card_bg(vcx.width, vcx.height, t, t.border, 6)).child(inner)
}

// ------------------------------------------------------------------ macOS-style All Apps Overlay

/// Build the macOS-style searchable, paginated All Apps overlay (`shell:apps`).
pub fn all_apps_view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef) -> Element {
    let t = vcx.theme;
    let compact = vcx.class == proto::SizeClass::Compact || vcx.width < 360;
    let (query, page, items, usage, icons, custom_icons) = {
        let s = st.borrow();
        let (q, p) = s.overlay.as_ref().map_or((String::new(), 0), |o| (o.query.clone(), o.page));
        (q, p, s.launch_items.clone(), s.usage.clone(), s.icons.clone(), s.custom_icons.clone())
    };
    let resolve_icon = |it: &AppItem| -> Option<u32> {
        it.custom_icon
            .as_ref()
            .and_then(|p| custom_icons.get(p).copied())
            .or_else(|| icons.get(&it.icon).copied())
            .or_else(|| icons.get("app").copied())
    };

    let searching = !query.trim().is_empty();
    let filtered = app_catalog::filter_apps(&items, &usage, &query);
    let top_used = if !searching && !compact {
        app_catalog::most_used(&items, &usage, 5)
    } else {
        Vec::new()
    };

    // Header: title + centered search bar + close button.
    let (st_ch, st_sub, sys_sub, st_close) = (st.clone(), st.clone(), sys.clone(), st.clone());
    let first_match = filtered.first().cloned();
    let search_w = if compact { (vcx.width - 110).max(110) } else { 220.min(vcx.width - 150) };
    let search = text_field()
        .id("apps:search")
        .placeholder("Search all apps...")
        .width(search_w)
        .on_change(move |cx, e| {
            if let EventValue::Text(q) = &e.value {
                if let Some(o) = &mut st_ch.borrow_mut().overlay {
                    o.query = q.clone();
                    o.page = 0;
                }
                cx.invalidate();
            }
        })
        .on_submit(move |cx, _| {
            if let Some(it) = &first_match {
                desktop::launch_and_close(cx, &st_sub, &sys_sub, &it.program, &it.args);
            }
        });

    let header = row()
        .gap(6)
        .align(Align::Center)
        .child(label("All Apps").id("apps:title").style("title").wrap(false))
        .child(spacer())
        .child(search)
        .child(spacer())
        .child(button("x").id("apps:close").style("flat").width(22).on_click(move |cx, _| desktop::close_overlay(cx, &st_close)));

    // Compute remaining height for the Launchpad page grid.
    let header_h = 28;
    let most_used_h = if top_used.is_empty() { 0 } else { 44 };
    let footer_h = 28;
    let pad = if compact { 12 } else { 20 };
    let grid_w = (vcx.width - pad * 2).max(140);
    let grid_h = (vcx.height - header_h - most_used_h - footer_h - pad * 2 - 12).max(60);
    let metrics = app_catalog::grid_metrics(grid_w, grid_h, compact);
    let total_pages = app_catalog::page_count(filtered.len(), metrics.per_page);
    let (clamped_page, slice) = app_catalog::page_slice(&filtered, page, metrics.per_page);
    if clamped_page != page {
        if let Some(o) = &mut st.borrow_mut().overlay {
            o.page = clamped_page;
        }
    }

    // Build custom Launchpad grid (supports crisp 24x24 icons, left-click launch, right-click "Add to Desktop", and wheel paging).
    let used_w = metrics.cols as i32 * metrics.cell_w;
    let ox = ((grid_w - used_w) / 2).max(0);
    let mut grid_nodes: Vec<SceneNode> = Vec::new();
    if slice.is_empty() {
        let msg = "No apps match your search.";
        let f = ecm_ui_proto::fonts::face(proto::Font::Ui, 12);
        let tw = f.map_or(140, |f| f.text_width(msg));
        grid_nodes.push(SceneNode::new(
            "empty",
            NodeKind::Text,
            vec![
                Prop::X(((grid_w - tw) / 2).max(0)),
                Prop::Y((grid_h / 2 - 8).max(4)),
                Prop::Text(msg.into()),
                Prop::Font(proto::Font::Ui),
                Prop::Size(12),
                Prop::Color(t.text_muted),
            ],
        ));
    } else {
        let icon_sz = if compact { 20 } else { 24 };
        let font_sz: u8 = 8;
        let face = ecm_ui_proto::fonts::face(proto::Font::Ui, font_sz);
        for (idx, it) in slice.iter().enumerate() {
            let col = (idx % metrics.cols) as i32;
            let r = (idx / metrics.cols) as i32;
            let cx0 = ox + col * metrics.cell_w;
            let cy0 = r * metrics.cell_h;
            let tile_w = metrics.cell_w - 6;
            let tile_h = metrics.cell_h - 6;
            grid_nodes.push(SceneNode::new(
                format!("tile:{idx}:bg"),
                NodeKind::Rect,
                vec![
                    Prop::X(cx0 + 3),
                    Prop::Y(cy0 + 2),
                    Prop::W(tile_w),
                    Prop::H(tile_h),
                    Prop::Fill(t.surface_alt),
                    Prop::BorderW(1),
                    Prop::BorderColor(t.border),
                    Prop::Radius(6),
                ],
            ));
            if let Some(res) = resolve_icon(it) {
                grid_nodes.push(SceneNode::new(
                    format!("tile:{idx}:img"),
                    NodeKind::Image,
                    vec![
                        Prop::X(cx0 + 3 + (tile_w - icon_sz) / 2),
                        Prop::Y(cy0 + 8),
                        Prop::W(icon_sz),
                        Prop::H(icon_sz),
                        Prop::Resource(res),
                    ],
                ));
            }
            let lbl = truncate_to_width(&it.label, tile_w - 8, font_sz);
            let tw = face.map_or(lbl.len() as i32 * 6, |f| f.text_width(&lbl));
            grid_nodes.push(SceneNode::new(
                format!("tile:{idx}:lbl"),
                NodeKind::Text,
                vec![
                    Prop::X(cx0 + 3 + ((tile_w - tw) / 2).max(2)),
                    Prop::Y(cy0 + 12 + icon_sz),
                    Prop::Text(lbl),
                    Prop::Font(proto::Font::Ui),
                    Prop::Size(font_sz),
                    Prop::Color(t.text),
                ],
            ));
        }
    }

    let slice_owned: Vec<AppItem> = slice.to_vec();
    let (st_g, sys_g) = (st.clone(), sys.clone());
    let grid_area = custom()
        .id("apps:grid")
        .width(grid_w)
        .height(grid_h)
        .grow(1)
        .nodes(grid_nodes)
        .on_pointer(move |cx, ev| {
            let EventValue::Pointer(p) = &ev.value else { return };
            if p.kind == proto::PointerKind::Scroll {
                if p.scroll_y != 0 {
                    desktop::all_apps_step_page(cx, &st_g, if p.scroll_y < 0 { 1 } else { -1 }, total_pages);
                }
                return;
            }
            if p.kind != proto::PointerKind::Down {
                return;
            }
            let rel_x = p.x - ox;
            if rel_x < 0 || p.y < 0 {
                return;
            }
            let col = (rel_x / metrics.cell_w) as usize;
            let row_i = (p.y / metrics.cell_h) as usize;
            if col >= metrics.cols || row_i >= metrics.rows {
                return;
            }
            let idx = row_i * metrics.cols + col;
            let Some(item) = slice_owned.get(idx).cloned() else { return };
            if p.button == proto::Button::Right {
                let (st_m, sys_m, item_m) = (st_g.clone(), sys_g.clone(), item.clone());
                cx.popup_menu(p.x, p.y, ["Open", "Add to Desktop"], move |cx, choice| {
                    if choice == 0 {
                        desktop::launch_and_close(cx, &st_m, &sys_m, &item_m.program, &item_m.args);
                    } else {
                        desktop::add_shortcut_to_desktop(cx, &st_m, &sys_m, &item_m);
                    }
                });
            } else if p.button == proto::Button::Left {
                desktop::launch_and_close(cx, &st_g, &sys_g, &item.program, &item.args);
            }
        });

    // Footer: Launchpad pagination (< dots >) + app count.
    let (st_prev, st_next) = (st.clone(), st.clone());
    let mut pager = row().gap(4).align(Align::Center);
    pager = pager.child(
        button("<")
            .id("apps:prev")
            .style("flat")
            .width(24)
            .enabled(clamped_page > 0)
            .on_click(move |cx, _| desktop::all_apps_step_page(cx, &st_prev, -1, total_pages)),
    );
    pager = pager.child(spacer());
    // Render clickable page dots (capped to a clean sliding window if there are > 9 pages).
    let max_dots = 9usize;
    let (dot_start, dot_end) = if total_pages <= max_dots {
        (0, total_pages)
    } else {
        let half = max_dots / 2;
        let s = clamped_page.saturating_sub(half).min(total_pages - max_dots);
        (s, s + max_dots)
    };
    for p_idx in dot_start..dot_end {
        let active = p_idx == clamped_page;
        let fill = if active { t.accent } else { t.border };
        let sz = if active { 8 } else { 6 };
        let off = (12 - sz) / 2;
        let st_d = st.clone();
        pager = pager.child(
            custom()
                .id(format!("apps:dot:{p_idx}"))
                .width(12)
                .height(12)
                .nodes(vec![SceneNode::new(
                    "d",
                    NodeKind::Rect,
                    vec![Prop::X(off), Prop::Y(off), Prop::W(sz), Prop::H(sz), Prop::Fill(fill), Prop::Radius(4)],
                )])
                .on_pointer(move |cx, ev| {
                    if let EventValue::Pointer(p) = &ev.value {
                        if p.kind == proto::PointerKind::Down && p.button == proto::Button::Left {
                            desktop::all_apps_set_page(cx, &st_d, p_idx);
                        }
                    }
                }),
        );
    }
    if total_pages > 1 {
        pager = pager.child(
            label(format!("{}/{}", clamped_page + 1, total_pages))
                .id("apps:page_label")
                .style("caption")
                .wrap(false),
        );
    }
    pager = pager.child(spacer());
    pager = pager.child(
        label(format!("{} app{}", filtered.len(), if filtered.len() == 1 { "" } else { "s" }))
            .id("apps:count")
            .style("caption")
            .wrap(false),
    );
    pager = pager.child(
        button(">")
            .id("apps:next")
            .style("flat")
            .width(24)
            .enabled(clamped_page + 1 < total_pages)
            .on_click(move |cx, _| desktop::all_apps_step_page(cx, &st_next, 1, total_pages)),
    );

    let mut content = column().padding(if compact { 6 } else { 10 }).gap(6).child(header);
    if !top_used.is_empty() {
        let mut strip = row().gap(4).align(Align::Center).child(label("Most Used:").style("caption").wrap(false));
        for (i, it) in top_used.iter().enumerate() {
            let (st_u, sys_u) = (st.clone(), sys.clone());
            let prog = it.program.clone();
            let args = it.args.clone();
            let mut r = row().gap(3).align(Align::Center);
            if let Some(res) = resolve_icon(it) {
                r = r.child(image(res, 14, 14));
            }
            let short = truncate_to_width(&it.label, 76, 8);
            r = r.child(button(short).id(format!("apps:used:{i}")).style("flat").on_click(move |cx, _| {
                desktop::launch_and_close(cx, &st_u, &sys_u, &prog, &args);
            }));
            strip = strip.child(r);
        }
        content = content.child(strip).child(hline("apps:div", grid_w, t.border));
    }
    content = content.child(grid_area).child(pager);

    stack().child(card_bg(vcx.width, vcx.height, t, t.accent, 8)).child(content)
}

// ------------------------------------------------------------------ Desktop Dialog (New Folder / Rename)

/// Build the desktop New Folder / Rename dialog (`shell:dialog`).
pub fn dialog_view(vcx: &ecm_ui::ViewCtx, st: &Shared, sys: &SysRef) -> Element {
    let t = vcx.theme;
    let (title, ok_label, error) = {
        let s = st.borrow();
        match s.overlay.as_ref().map(|o| (&o.kind, o.error.clone())) {
            Some((OverlayKind::NewFolder, err)) => ("New Folder".to_string(), "Create", err),
            Some((OverlayKind::Rename { file_name, .. }, err)) => (format!("Rename {file_name}"), "Rename", err),
            _ => ("Desktop".to_string(), "OK", String::new()),
        }
    };
    let (st_ok1, sys_ok1, st_ok2, sys_ok2, st_cancel) = (st.clone(), sys.clone(), st.clone(), sys.clone(), st.clone());
    let mut c = column()
        .padding(10)
        .gap(6)
        .child(label(title).id("dlg:title").style("title").wrap(false))
        .child(
            text_field()
                .id("dlg:input")
                .placeholder("Name")
                .on_submit(move |cx, _| desktop::submit_dialog(cx, &st_ok1, &sys_ok1)),
        );
    if !error.is_empty() {
        c = c.child(label(error).id("dlg:error").style("caption").wrap(false));
    }
    c = c.child(spacer()).child(
        row()
            .gap(6)
            .child(spacer())
            .child(button("Cancel").id("dlg:cancel").on_click(move |cx, _| desktop::close_overlay(cx, &st_cancel)))
            .child(
                button(ok_label)
                    .id("dlg:ok")
                    .style("primary")
                    .on_click(move |cx, _| desktop::submit_dialog(cx, &st_ok2, &sys_ok2)),
            ),
    );
    stack().child(card_bg(vcx.width, vcx.height, t, t.accent, 6)).child(c)
}

// ------------------------------------------------------------------ Taskbar Settings Quick Menu (Popup)

/// Build the taskbar's Settings Quick Menu popup content.
pub fn settings_popup_element(st: &Shared, sys: &SysRef, t: &Theme) -> Element {
    let (st2, sys2) = (st.clone(), sys.clone());
    let labels: Vec<&str> = SETTINGS_QUICK_ITEMS.iter().map(|(l, _)| *l).collect();
    let m = ecm_ui::element::menu(labels).id("settings_quick_menu").on_select(move |cx, e| {
        if let EventValue::Index(i) = e.value {
            st2.borrow_mut().settings_popup = None;
            if let Some((_, sec)) = SETTINGS_QUICK_ITEMS.get(i) {
                desktop::open_settings_section(cx, &st2, &sys2, sec);
            }
        }
    });
    let _ = t;
    column().padding(2).gap(0).child(m)
}

/// Measure and open/toggle the taskbar Settings Quick Menu popup.
pub fn toggle_settings_popup(cx: &mut Ctx, st: &Shared, sys: &SysRef) {
    let open = st.borrow().settings_popup;
    if let Some(p) = open {
        st.borrow_mut().settings_popup = None;
        if cx.core().window(p).is_some() {
            cx.core().close_window(p);
            return;
        }
    }
    desktop::close_overlay(cx, st);
    let theme = cx.core().theme.clone();
    let el = settings_popup_element(st, sys, &theme);
    let (w, h) = ecm_ui::window::measure(&el, &theme, proto::SizeClass::Compact, 200);
    let w = w.max(132);
    let (tb, d) = {
        let s = st.borrow();
        (s.taskbar, s.display.clone())
    };
    let x = cx
        .core()
        .window(tb)
        .and_then(|win| win.rect_of("tray:settings"))
        .map(|r| r.x + r.w - w)
        .unwrap_or(d.w - w - 4)
        .clamp(2, (d.w - w - 2).max(2));
    let h = h.min(d.h - TASKBAR_H - 8);
    let s = cx.core().create_popup(tb, x, -h - 2, w, h, ecm_ui::window::Content::Static(el));
    if let Some(win) = cx.core().window_mut(s) {
        win.close_on_select = true;
    }
    st.borrow_mut().settings_popup = Some(s);
}
