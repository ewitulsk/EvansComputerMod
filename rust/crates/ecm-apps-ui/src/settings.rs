//! Settings → Apps, as a section the Settings app (`ecm-apps`, P3) shows next
//! to Wi-Fi, Display and the others: installed apps; for the chosen one its
//! permissions with allow toggles (and "ask again"), the files it was given
//! one at a time (revocable), and which file types it opens by default.
//! Everything goes through `app_ctl` ([`AppCtl`]), so the Settings app needs
//! the `packages` permission (system apps get it).
//!
//! ```ignore
//! let apps = AppsSection::new(Box::new(HostCtl));
//! // in the Settings window's view, for the "Apps" entry:
//! apps.view(cx)
//! ```

use crate::appctl::{self, AppCtl, AppInfo, FileGrant, Handler, InstalledApp, PermLine, PermState};
use crate::perms;
use crate::widgets::pill;
use ecm_ui::element::{button, column, label, list, row, spacer, toggle, Element};
use ecm_ui::ViewCtx;
use ecm_ui_proto::SizeClass;
use std::cell::RefCell;
use std::rc::Rc;

/// The pane's state.
#[derive(Default, Debug)]
pub struct AppsPane {
    pub apps: Vec<InstalledApp>,
    pub selected: Option<String>,
    pub info: Option<AppInfo>,
    pub perms: Vec<PermLine>,
    pub files: Vec<FileGrant>,
    /// For each type the selected app opens: the apps that open it.
    pub handlers: Vec<(String, Vec<Handler>)>,
    pub status: String,
}

impl AppsPane {
    /// Read the installed apps (and the selected one again).
    pub fn load(&mut self, ctl: &mut dyn AppCtl) {
        match ctl.call(&["list"]) {
            Ok(r) => {
                self.apps = appctl::parse_list(&r);
                self.apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                self.status.clear();
            }
            Err(e) => {
                self.apps.clear();
                self.status = e;
            }
        }
        if let Some(id) = self.selected.clone() {
            if self.apps.iter().any(|a| a.id == id) {
                self.select(ctl, &id);
            } else {
                self.selected = None;
                self.info = None;
            }
        }
    }

    pub fn select(&mut self, ctl: &mut dyn AppCtl, id: &str) {
        self.selected = Some(id.to_string());
        self.info = ctl.call(&["info", id]).ok().map(|r| appctl::parse_info(id, &r));
        let (p, f) = ctl.call(&["perm", "list", id]).map(|r| appctl::parse_perm_list(&r)).unwrap_or_default();
        self.perms = p.into_iter().filter(|l| l.app == id).collect();
        self.files = f.into_iter().filter(|g| g.app == id).collect();
        // Declared permissions without a line yet are "ask".
        if let Some(i) = &self.info {
            for d in &i.permissions {
                if !self.perms.iter().any(|l| &l.permission == d) {
                    self.perms.push(PermLine { app: id.into(), permission: d.clone(), state: PermState::Ask });
                }
            }
        }
        self.handlers = self
            .info
            .as_ref()
            .map(|i| i.open_with.clone())
            .unwrap_or_default()
            .into_iter()
            .map(|ext| {
                let hs = ctl.call(&["handlers", &format!("/file{ext}")]).map(|r| appctl::parse_handlers(&r)).unwrap_or_default();
                (ext, hs)
            })
            .collect();
    }

    fn after(&mut self, ctl: &mut dyn AppCtl, r: Result<String, String>, ok: String) {
        self.status = match r {
            Ok(_) => ok,
            Err(e) => e,
        };
        if let Some(id) = self.selected.clone() {
            let keep = std::mem::take(&mut self.status);
            self.select(ctl, &id);
            self.status = keep;
        }
    }

    /// Allow or deny a permission (remembered).
    pub fn set_permission(&mut self, ctl: &mut dyn AppCtl, perm: &str, allow: bool) {
        let Some(id) = self.selected.clone() else { return };
        let r = ctl.call(&["perm", if allow { "grant" } else { "deny" }, &id, perm]);
        let words = perms::describe(perm).title;
        self.after(ctl, r, format!("{}: {}", words, if allow { "allowed" } else { "denied" }));
    }

    /// Forget the answer: the app is asked again next time.
    pub fn reset_permission(&mut self, ctl: &mut dyn AppCtl, perm: &str) {
        let Some(id) = self.selected.clone() else { return };
        let r = ctl.call(&["perm", "reset", &id, perm]);
        self.after(ctl, r, format!("{}: asks again", perms::describe(perm).title));
    }

    /// Take back a file given through the picker or "open with".
    pub fn revoke_file(&mut self, ctl: &mut dyn AppCtl, path: &str) {
        let Some(id) = self.selected.clone() else { return };
        let r = ctl.call(&["perm", "revoke", &id, path]);
        self.after(ctl, r, format!("{path}: access removed"));
    }

    /// Make `app` (None: nobody, ask each time) the default for `ext`.
    pub fn set_default(&mut self, ctl: &mut dyn AppCtl, ext: &str, app: Option<&str>) {
        let r = ctl.call(&["default", ext, app.unwrap_or("-")]);
        self.after(ctl, r, match app {
            Some(a) => format!("{ext} files open with {a}"),
            None => format!("{ext} files: ask each time"),
        });
    }
}

/// The section: the pane and the app manager it talks to.
#[derive(Clone)]
pub struct AppsSection {
    pub pane: Rc<RefCell<AppsPane>>,
    pub ctl: Rc<RefCell<Box<dyn AppCtl>>>,
}

impl AppsSection {
    pub fn new(ctl: Box<dyn AppCtl>) -> AppsSection {
        let s = AppsSection { pane: Rc::new(RefCell::new(AppsPane::default())), ctl: Rc::new(RefCell::new(ctl)) };
        s.reload();
        s
    }

    /// The title in the Settings list.
    pub fn title(&self) -> &'static str {
        "Apps"
    }

    pub fn reload(&self) {
        let mut c = self.ctl.borrow_mut();
        self.pane.borrow_mut().load(&mut **c);
    }

    /// Run `f` on the pane with the app manager (for handlers).
    pub fn with(&self, f: impl FnOnce(&mut AppsPane, &mut dyn AppCtl)) {
        let mut c = self.ctl.borrow_mut();
        f(&mut self.pane.borrow_mut(), &mut **c);
    }

    /// The section's content: the app list and the chosen app (side by side
    /// when wide, one under the other when compact).
    pub fn view(&self, cx: &ViewCtx) -> Element {
        let p = self.pane.borrow();
        let sel = p.apps.iter().position(|a| Some(&a.id) == p.selected.as_ref());
        let me = self.clone();
        let mut apps = list().id("apps_list").children(p.apps.iter().map(|a| {
            row().gap(6).child(label(a.name.clone()).wrap(false)).child(spacer()).child(label(a.version.clone()).style("caption").wrap(false))
        }));
        if let Some(i) = sel {
            apps = apps.selected(i);
        }
        let apps = apps.on_select(move |cx, _| {
            let i = cx.selected("apps_list").unwrap_or(0);
            let id = me.pane.borrow().apps.get(i).map(|a| a.id.clone());
            if let Some(id) = id {
                me.with(|p, c| p.select(c, &id));
            }
        });
        let left = column().gap(6).child(label("Installed apps").style("title")).child(if p.apps.is_empty() {
            label(if p.status.is_empty() { "No apps installed." } else { p.status.as_str() }).style("caption")
        } else {
            apps
        });
        let right = self.detail(&p);
        if cx.class == SizeClass::Compact {
            column().gap(12).child(left).child(right)
        } else {
            row().gap(12).align(ecm_ui::Align::Start).child(left.width(180)).child(right.grow(1))
        }
    }

    fn detail(&self, p: &AppsPane) -> Element {
        let Some(info) = &p.info else {
            return column().child(label("Choose an app to see what it may do.").style("caption"));
        };
        let mut c = column().gap(8).child(label(if info.name.is_empty() { info.id.clone() } else { info.name.clone() }).style("title"));
        let mut facts = vec![format!("{} {}", info.id, info.version)];
        if !info.trust.is_empty() {
            facts.push(info.trust.clone());
        }
        c = c.child(label(facts.join(" · ")).style("caption"));
        c = c.child(label("Permissions").style("title"));
        if p.perms.is_empty() {
            c = c.child(label("It asks for no special permissions.").style("caption"));
        }
        for l in &p.perms {
            let words = perms::describe(&l.permission);
            let me = self.clone();
            let perm = l.permission.clone();
            let t = toggle("").id(format!("perm:{}", l.permission)).checked(l.state == PermState::Allow).on_change(move |cx, e| {
                let on = cx.checked(e.id.as_deref().unwrap_or(""));
                me.with(|p, c| p.set_permission(c, &perm, on));
            });
            let mut r = row().gap(6).child(column().gap(1).grow(1).child(label(words.title)).child(label(words.detail).style("caption")));
            r = r.child(match l.state {
                PermState::Allow => pill("Allowed", crate::widgets::GREEN),
                PermState::Deny => pill("Denied", crate::widgets::RED),
                PermState::Ask => pill("Asks first", 0x454c_58ff),
            });
            if l.state != PermState::Ask {
                let me = self.clone();
                let perm = l.permission.clone();
                r = r.child(button("Ask again").id(format!("reset:{}", l.permission)).on_click(move |_, _| me.with(|p, c| p.reset_permission(c, &perm))));
            }
            c = c.child(r.child(t));
        }
        if !p.files.is_empty() {
            c = c.child(label("Files it was given").style("title"));
            for g in &p.files {
                let me = self.clone();
                let path = g.path.clone();
                c = c.child(
                    row()
                        .gap(6)
                        .child(label(g.path.clone()).wrap(false))
                        .child(label(if g.write { "read and write" } else { "read only" }).style("caption").wrap(false))
                        .child(spacer())
                        .child(button("Remove").id(format!("revoke:{}", g.path)).on_click(move |_, _| me.with(|p, c| p.revoke_file(c, &path)))),
                );
            }
        }
        if !p.handlers.is_empty() {
            c = c.child(label("Opens by default").style("title"));
            for (ext, hs) in &p.handlers {
                let is_default = hs.iter().any(|h| h.id == info.id && h.is_default);
                let others = hs.iter().filter(|h| h.id != info.id).count();
                let me = self.clone();
                let e2 = ext.clone();
                let id = info.id.clone();
                let tg = toggle(format!("{ext} files")).id(format!("default:{ext}")).checked(is_default || (others == 0 && hs.iter().any(|h| h.id == info.id))).enabled(others > 0).on_change(move |cx, ev| {
                    let on = cx.checked(ev.id.as_deref().unwrap_or(""));
                    me.with(|p, c| p.set_default(c, &e2, if on { Some(&id) } else { None }));
                });
                c = c.child(row().gap(6).child(tg).child(label(if others == 0 { "the only app for these" } else { "" }).style("caption").wrap(false)));
            }
        }
        if !p.status.is_empty() {
            c = c.child(label(p.status.clone()).style("caption").id("apps_status"));
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appctl::FakeCtl;

    fn ctl() -> FakeCtl {
        let mut f = FakeCtl::default();
        f.add("dev.ecm.editor", "Text Editor", "1.0.0", &["files:documents"], &[".txt", ".md"]);
        f.add("dev.ecm.notes", "Notes", "1.0.0", &["network", "notifications"], &[".txt"]);
        f.grants.push(FileGrant { app: "dev.ecm.notes".into(), path: "/home/a.txt".into(), write: true });
        f
    }

    #[test]
    fn lists_apps_and_their_permissions() {
        let mut f = ctl();
        let mut p = AppsPane::default();
        p.load(&mut f);
        assert_eq!(p.apps.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["Notes", "Text Editor"]);
        p.select(&mut f, "dev.ecm.notes");
        assert_eq!(p.perms.len(), 2);
        assert!(p.perms.iter().all(|l| l.state == PermState::Ask));
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.handlers.len(), 1);
        assert_eq!(p.handlers[0].1.len(), 2);
    }

    #[test]
    fn toggles_revoke_and_defaults_go_through_app_ctl() {
        let mut f = ctl();
        let mut p = AppsPane::default();
        p.load(&mut f);
        p.select(&mut f, "dev.ecm.notes");
        p.set_permission(&mut f, "network", true);
        assert!(f.log.contains(&"perm grant dev.ecm.notes network".to_string()));
        assert_eq!(p.perms.iter().find(|l| l.permission == "network").unwrap().state, PermState::Allow);
        assert_eq!(p.status, "Use the network: allowed");
        p.set_permission(&mut f, "notifications", false);
        assert_eq!(p.perms.iter().find(|l| l.permission == "notifications").unwrap().state, PermState::Deny);
        p.reset_permission(&mut f, "network");
        assert_eq!(p.perms.iter().find(|l| l.permission == "network").unwrap().state, PermState::Ask);
        p.revoke_file(&mut f, "/home/a.txt");
        assert!(p.files.is_empty());
        p.set_default(&mut f, ".txt", Some("dev.ecm.notes"));
        assert_eq!(f.defaults.get(".txt").map(String::as_str), Some("dev.ecm.notes"));
        assert!(p.handlers[0].1.iter().any(|h| h.id == "dev.ecm.notes" && h.is_default));
        p.set_default(&mut f, ".txt", None);
        assert!(f.defaults.is_empty());
        // An app removed meanwhile drops out of the selection.
        f.apps.remove("dev.ecm.notes");
        p.load(&mut f);
        assert!(p.selected.is_none() && p.info.is_none());
    }

    #[test]
    fn without_an_app_manager_it_says_so() {
        let mut f = FakeCtl { absent: true, ..Default::default() };
        let mut p = AppsPane::default();
        p.load(&mut f);
        assert_eq!(p.status, "no app manager on this computer");
    }
}
