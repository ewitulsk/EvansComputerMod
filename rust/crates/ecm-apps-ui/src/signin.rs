//! The sign-in screen Discord and Music share (plans/background-apps/MUSIC_AND_AUTH.md,
//! "Realm identity and pinning"). It only shows and collects: the app performs the
//! [`Action`]s with its own `ecm_auth::AccountStore` over its own socket and reports
//! back with [`SignInScreen::failed`] / [`SignInScreen::succeeded`].
//!
//! ```ignore
//! let screen = Rc::new(RefCell::new(SignInScreen::new(&info.realm_name, &host, &info.realm_fp, &check)));
//! app.window("Sign in", { let s = screen.clone(); move |_| signin::view(&s) });
//! // after each step:
//! if let Some(action) = screen.borrow_mut().take_action() { /* sign in / up, forget, close */ }
//! ```
//!
//! States: a realm met for the first time (its name, address and fingerprint, so the
//! player can compare it with the owner's), a pinned realm (sign in or create an
//! account), busy, signed in, and a **changed key**: the warning, both fingerprints and
//! how to remove the old pin by hand. That state has no button that signs in or trusts
//! the new key (decided: a changed key is refused until the pin is removed).

use crate::widgets::RED;
use ecm_auth::{display_fingerprint, AuthError, PinCheck};
use ecm_ui::element::{button, column, label, row, spacer, tabs, text_field, Element};
use ecm_ui_proto::Font;
use std::cell::RefCell;
use std::rc::Rc;

/// Sign in or make an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    SignIn,
    SignUp,
}

/// What the player asked for; the app does it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    SignIn { user: String, pass: String },
    SignUp { user: String, display: String, pass: String },
    /// Close the screen without signing in.
    Cancel,
    /// Remove this realm's pin (and the login saved for it), by hand.
    ForgetPin(String),
}

/// Where the screen is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The form (first contact or a pinned realm).
    Form,
    /// The app is signing in.
    Busy,
    SignedIn { user: String, display: String },
    /// The realm's key is not the pinned one: refused.
    Changed { old_fp: String, new_fp: String },
}

/// The screen's state.
#[derive(Clone, Debug)]
pub struct SignInScreen {
    pub realm_name: String,
    pub host: String,
    /// The key the realm presented.
    pub fp: String,
    /// False on first contact: the fingerprint is shown to compare.
    pub pinned: bool,
    pub stage: Stage,
    pub mode: Mode,
    pub error: Option<String>,
    /// Where this app lets the player remove a pin (shown in the changed-key warning).
    pub forget_hint: String,
    action: Option<Action>,
}

impl SignInScreen {
    /// For a realm the app just reached: `check` is what its pins said.
    pub fn new(realm_name: &str, host: &str, fp: &str, check: &PinCheck) -> SignInScreen {
        let stage = match check {
            PinCheck::Changed { old_fp, .. } => Stage::Changed { old_fp: old_fp.clone(), new_fp: fp.to_string() },
            _ => Stage::Form,
        };
        SignInScreen {
            realm_name: realm_name.into(),
            host: host.into(),
            fp: fp.into(),
            pinned: *check == PinCheck::Pinned,
            stage,
            mode: Mode::SignIn,
            error: None,
            forget_hint: "Settings > Accounts in this app (Forget)".into(),
            action: None,
        }
    }

    /// The screen for a realm the player is signed in to.
    pub fn signed_in(realm_name: &str, host: &str, fp: &str, user: &str, display: &str) -> SignInScreen {
        let mut s = SignInScreen::new(realm_name, host, fp, &PinCheck::Pinned);
        s.stage = Stage::SignedIn { user: user.into(), display: display.into() };
        s
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if self.stage == Stage::Form {
            self.mode = mode;
            self.error = None;
        }
    }

    /// The form was sent. Checks it here first (so the network isn't asked for nothing);
    /// true if it became an action (and the screen is busy).
    pub fn submit(&mut self, user: &str, display: &str, pass: &str, pass2: &str) -> bool {
        if self.stage != Stage::Form {
            return false;
        }
        let checked = (|| -> Result<Action, String> {
            let user = ecm_auth::username(user).map_err(|e| e.to_string())?;
            if pass.is_empty() {
                return Err("Type your password.".into());
            }
            Ok(match self.mode {
                Mode::SignIn => Action::SignIn { user, pass: pass.into() },
                Mode::SignUp => {
                    if pass != pass2 {
                        return Err("The passwords are different.".into());
                    }
                    let display = if display.trim().is_empty() { user.clone() } else { ecm_auth::display_name(display).map_err(|e| e.to_string())? };
                    Action::SignUp { user, display, pass: pass.into() }
                }
            })
        })();
        match checked {
            Ok(a) => {
                self.action = Some(a);
                self.stage = Stage::Busy;
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    pub fn cancel(&mut self) {
        self.action = Some(Action::Cancel);
    }

    /// "Forget this realm": only offered for a realm whose key is pinned and unchanged.
    pub fn forget(&mut self) {
        if !matches!(self.stage, Stage::Changed { .. }) && self.pinned {
            self.action = Some(Action::ForgetPin(self.fp.clone()));
        }
    }

    /// The next thing for the app to do.
    pub fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }

    /// The app's attempt failed: back to the form with the reason (a changed key turns
    /// into the warning).
    pub fn failed(&mut self, e: &AuthError) {
        if let AuthError::RealmChanged { old_fp, new_fp, .. } = e {
            self.stage = Stage::Changed { old_fp: old_fp.clone(), new_fp: new_fp.clone() };
            self.error = None;
            return;
        }
        self.stage = Stage::Form;
        self.error = Some(match e {
            AuthError::WrongPassword => "Wrong username or password.".into(),
            AuthError::NeedsNewPassword => "This account was reset: choose a new password and sign in again.".into(),
            e => {
                let t = e.to_string();
                let mut c = t.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        });
    }

    /// Signed in (the realm is pinned now).
    pub fn succeeded(&mut self, user: &str, display: &str) {
        self.pinned = true;
        self.error = None;
        self.stage = Stage::SignedIn { user: user.into(), display: display.into() };
    }
}

type S = Rc<RefCell<SignInScreen>>;

/// The realm's name, address and key, as one block.
fn identity(s: &SignInScreen) -> Element {
    let mut c = column()
        .gap(2)
        .child(label(s.realm_name.clone()).style("title").id("realm"))
        .child(label(s.host.clone()).style("caption").id("host"))
        .child(label(format!("Key {}", display_fingerprint(&s.fp))).font(Font::Mono).size(8).id("fingerprint"));
    if !s.pinned {
        c = c.child(
            label("First time here: check that this key is the one the owner of this server gives out. It is remembered after you sign in.")
                .style("caption")
                .id("first_contact"),
        );
    }
    c
}

/// The screen's content (fits a phone: one column).
pub fn view(s: &S) -> Element {
    let st = s.borrow();
    let mut col = column().gap(8).padding(12).id("signin");
    match &st.stage {
        Stage::Changed { old_fp, new_fp } => {
            col = col
                .child(label("Sign-in refused").style("title"))
                .child(
                    label(format!("The identity of {} has changed. This could be someone pretending to be it.", st.realm_name))
                        .color(RED)
                        .id("changed"),
                )
                .child(label(format!("Remembered key: {}", display_fingerprint(old_fp))).font(Font::Mono).size(8).id("old_fp"))
                .child(label(format!("Key now: {}", display_fingerprint(new_fp))).font(Font::Mono).size(8).id("new_fp"))
                .child(label(st.host.clone()).style("caption"))
                .child(
                    label(format!(
                        "Your saved sign-in is not offered to it. If the owner of {} told you its key changed, remove the remembered key in {} and connect again.",
                        st.realm_name, st.forget_hint
                    ))
                    .style("caption")
                    .id("how_to_forget"),
                );
            let c = s.clone();
            col = col.child(row().child(spacer()).child(button("Close").id("cancel").on_click(move |_, _| c.borrow_mut().cancel())));
        }
        Stage::SignedIn { user, display } => {
            let (f, c) = (s.clone(), s.clone());
            col = col
                .child(identity(&st))
                .child(label(format!("Signed in as {display} (@{user})")).id("signed_in"))
                .child(
                    row()
                        .gap(8)
                        .child(button("Forget this server's key").style("danger").id("forget").on_click(move |_, _| f.borrow_mut().forget()))
                        .child(spacer())
                        .child(button("Done").style("primary").id("cancel").on_click(move |_, _| c.borrow_mut().cancel())),
                );
        }
        Stage::Form | Stage::Busy => {
            let busy = st.stage == Stage::Busy;
            let up = st.mode == Mode::SignUp;
            let m = s.clone();
            col = col.child(identity(&st)).child(
                tabs(["Sign in", "Create account"]).id("mode").selected(up as usize).on_select(move |cx, _| {
                    let mode = if cx.selected("mode") == Some(1) { Mode::SignUp } else { Mode::SignIn };
                    m.borrow_mut().set_mode(mode);
                }),
            );
            col = col.child(text_field().id("user").placeholder("Username").enabled(!busy));
            if up {
                col = col.child(text_field().id("display").placeholder("Display name (optional)").enabled(!busy));
            }
            let submit = {
                let s = s.clone();
                move |cx: &mut ecm_ui::Ctx, _: &ecm_ui::UiEvent| {
                    let (u, d, p, p2) = (cx.text("user"), cx.text("display"), cx.text("pass"), cx.text("pass2"));
                    if s.borrow_mut().submit(&u, &d, &p, &p2) {
                        cx.set_text("pass", "");
                        cx.set_text("pass2", "");
                    }
                }
            };
            col = col.child(text_field().id("pass").placeholder("Password").password(true).enabled(!busy).on_submit(submit.clone()));
            if up {
                col = col.child(text_field().id("pass2").placeholder("Password again").password(true).enabled(!busy).on_submit(submit.clone()));
            }
            if let Some(e) = &st.error {
                col = col.child(label(e.clone()).color(RED).id("error"));
            }
            if busy {
                col = col.child(label(if up { "Creating your account..." } else { "Signing in..." }).style("caption").id("busy"));
            }
            let c = s.clone();
            let mut buttons = row().gap(8);
            if st.pinned {
                let f = s.clone();
                buttons = buttons.child(button("Forget").id("forget").enabled(!busy).on_click(move |_, _| f.borrow_mut().forget()));
            }
            buttons = buttons
                .child(spacer())
                .child(button("Cancel").id("cancel").on_click(move |_, _| c.borrow_mut().cancel()))
                .child(button(if up { "Create account" } else { "Sign in" }).style("primary").id("submit").enabled(!busy).on_click(submit));
            col = col.child(buttons);
        }
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "3fa19c0d00000000000000000000beef";
    const OLD: &str = "aaaabbbb00000000000000000000cccc";

    fn texts(e: &Element, out: &mut Vec<String>) {
        if let Some(t) = &e.props.text {
            out.push(t.clone());
        }
        out.extend(e.props.items.iter().cloned());
        for c in &e.children {
            texts(c, out);
        }
    }

    fn ids(e: &Element, out: &mut Vec<String>) {
        if let Some(i) = &e.id {
            out.push(i.clone());
        }
        for c in &e.children {
            ids(c, out);
        }
    }

    fn shown(s: &SignInScreen) -> (String, Vec<String>) {
        let rc = Rc::new(RefCell::new(s.clone()));
        let v = view(&rc);
        let (mut t, mut i) = (Vec::new(), Vec::new());
        texts(&v, &mut t);
        ids(&v, &mut i);
        (t.join("\n"), i)
    }

    #[test]
    fn first_contact_shows_the_key_and_signs_in() {
        let mut s = SignInScreen::new("Tech Village", "10.0.8.1:7600", FP, &PinCheck::FirstContact);
        let (t, ids) = shown(&s);
        assert!(t.contains("Tech Village") && t.contains("10.0.8.1:7600") && t.contains("Key 3F:A1:9C:0D:"), "{t}");
        assert!(t.contains("First time here"));
        assert!(ids.contains(&"pass".into()) && !ids.contains(&"pass2".into()) && !ids.contains(&"forget".into()), "{ids:?}");
        // Checked here first: a bad username or no password doesn't reach the app.
        assert!(!s.submit("bad name", "", "pw", ""));
        assert!(s.error.as_deref().unwrap().contains("username"));
        assert!(!s.submit("alice", "", "", ""));
        assert_eq!(s.error.as_deref(), Some("Type your password."));
        assert!(s.take_action().is_none());
        assert!(s.submit("Alice", "", "hunter2", ""));
        assert_eq!(s.take_action(), Some(Action::SignIn { user: "alice".into(), pass: "hunter2".into() }));
        assert_eq!(s.stage, Stage::Busy);
        assert!(shown(&s).0.contains("Signing in..."));
        assert!(!s.submit("alice", "", "again", ""), "no second request while busy");
        // A wrong password: back to the form with the reason.
        s.failed(&AuthError::WrongPassword);
        assert_eq!((s.stage.clone(), s.error.as_deref()), (Stage::Form, Some("Wrong username or password.")));
        assert!(shown(&s).0.contains("Wrong username or password."));
        s.submit("alice", "", "hunter2", "");
        s.succeeded("alice", "Alice A.");
        assert!(s.pinned);
        let (t, ids) = shown(&s);
        assert!(t.contains("Signed in as Alice A. (@alice)") && !t.contains("First time here"), "{t}");
        assert!(ids.contains(&"forget".into()));
        s.forget();
        assert_eq!(s.take_action(), Some(Action::ForgetPin(FP.into())));
    }

    #[test]
    fn creating_an_account_asks_twice() {
        let mut s = SignInScreen::new("Lab", "10.0.0.5:7779", FP, &PinCheck::Pinned);
        assert!(shown(&s).1.contains(&"forget".to_string()), "a pinned realm can be forgotten from here");
        s.set_mode(Mode::SignUp);
        let (t, ids) = shown(&s);
        assert!(ids.contains(&"pass2".into()) && ids.contains(&"display".into()));
        assert!(t.contains("Create account"));
        assert!(!s.submit("bo", "Bo", "one", "two"));
        assert_eq!(s.error.as_deref(), Some("The passwords are different."));
        assert!(!s.submit("bo", &"x".repeat(40), "pw", "pw"));
        assert!(s.submit("bo", "", "pw", "pw"));
        assert_eq!(s.take_action(), Some(Action::SignUp { user: "bo".into(), display: "bo".into(), pass: "pw".into() }));
        s.failed(&AuthError::UsernameTaken("bo".into()));
        assert_eq!(s.error.as_deref(), Some("The username bo is taken"));
        assert_eq!(s.mode, Mode::SignUp, "still on Create account");
        s.cancel();
        assert_eq!(s.take_action(), Some(Action::Cancel));
    }

    #[test]
    fn a_changed_key_is_refused_with_no_way_to_trust_it() {
        let check = PinCheck::Changed { old_fp: OLD.into(), name: "Tech Village".into(), host: "10.0.8.1:7600".into() };
        let mut s = SignInScreen::new("Tech Village", "10.0.8.1:7600", FP, &check);
        let (t, ids) = shown(&s);
        assert!(t.contains("The identity of Tech Village has changed. This could be someone pretending to be it."), "{t}");
        assert!(t.contains("Remembered key: AA:AA:BB:BB:") && t.contains("Key now: 3F:A1:9C:0D:"));
        assert!(t.contains("Settings > Accounts"));
        // Only Close: no sign-in, no trust, no forget button on this screen.
        assert_eq!(ids.iter().filter(|i| ["submit", "forget", "user", "pass"].contains(&i.as_str())).count(), 0, "{ids:?}");
        assert!(!s.submit("alice", "", "pw", ""));
        s.forget();
        assert_eq!(s.take_action(), None, "forgetting happens by hand elsewhere, not from the warning");
        s.set_mode(Mode::SignUp);
        assert_eq!(s.mode, Mode::SignIn);
        // A sign-in that runs into a changed key turns into the warning too.
        let mut s = SignInScreen::new("Tech Village", "10.0.8.1:7600", OLD, &PinCheck::Pinned);
        s.submit("alice", "", "pw", "");
        assert!(matches!(s.take_action(), Some(Action::SignIn { .. })));
        s.failed(&AuthError::RealmChanged { name: "Tech Village".into(), host: "h".into(), old_fp: OLD.into(), new_fp: FP.into() });
        assert_eq!(s.stage, Stage::Changed { old_fp: OLD.into(), new_fp: FP.into() });
        s.forget();
        assert!(s.take_action().is_none());
    }

    #[test]
    fn other_errors_read_as_sentences() {
        let mut s = SignInScreen::new("Lab", "h", FP, &PinCheck::Pinned);
        s.submit("a", "", "p", "");
        s.failed(&AuthError::Locked { wait_s: 30 });
        assert_eq!(s.error.as_deref(), Some("Too many tries: wait 30 s"));
        s.failed(&AuthError::Network("can't connect to h:7600".into()));
        assert_eq!(s.error.as_deref(), Some("Network: can't connect to h:7600"));
        s.failed(&AuthError::NeedsNewPassword);
        assert!(s.error.as_deref().unwrap().contains("reset"));
    }
}
