//! The App Store (docs/gui/APPS.md "App Store"): browse the configured
//! repositories, search, an app's page (summary, version, publisher with a
//! verified / unverified badge, size, the permissions it asks for in plain
//! words, screenshots), install / update / remove with progress, the
//! Updates and Installed tabs, and the Repositories page (add a repository
//! by its URL, remove one: what `pkg repo add` / `pkg repo remove` do, for
//! phones without a terminal).
//!
//! [`Store`] is the state and the logic, with no I/O. [`StoreApp`] runs it:
//! an `ecm-ui` window whose view is [`view::view`], the repository client
//! (`ecm_pkg::client`, the same code as `pkg`) and the app manager
//! ([`AppCtl`]). Handlers only change the state or queue a [`Cmd`]; the
//! driver does the I/O between loop turns, redrawing while a download runs.

pub mod view;

use crate::appctl::{self, AppCtl, PermLine, PermState};
use crate::perms::{self, PermText};
use crate::png::{Images, Img};
use ecm_pkg::account::{self, Account, DevicePoll, DeviceStart, MemorySecrets, Secrets, Summary};
use ecm_pkg::client::{Available, Catalog, Client, Installed, Net, Plan, TrustHint};
use ecm_pkg::http::Url;
use ecm_pkg::manifest::Kind;
use ecm_pkg::Repo;
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::App;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tab {
    Discover = 0,
    Updates = 1,
    Installed = 2,
    /// The signed-in player's favorites (shown only when signed in).
    Favorites = 3,
}

impl Tab {
    pub fn from_index(i: usize) -> Tab {
        match i {
            1 => Tab::Updates,
            2 => Tab::Installed,
            3 => Tab::Favorites,
            _ => Tab::Discover,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum JobKind {
    Install,
    Update,
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub kind: JobKind,
    pub id: String,
}

/// The job being done, with download progress.
#[derive(Clone, Debug, PartialEq)]
pub struct Running {
    pub job: Job,
    pub done: usize,
    pub total: Option<usize>,
}

impl Running {
    pub fn percent(&self) -> u32 {
        match self.total {
            Some(t) if t > 0 => ((self.done as u64 * 100) / t as u64).min(100) as u32,
            _ => 0,
        }
    }
}

/// I/O the driver does for a handler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    /// Read the indexes again (`true`: fetch them from the repositories).
    Refresh(bool),
    /// Launch an installed app.
    Open(String),
    /// Load an app page's pictures and permission answers.
    Detail(String),
    /// Add a repository (`pkg repo add <name> <url>`), then fetch every index.
    AddRepo { name: String, url: String },
    /// Remove a repository (`pkg repo remove <name>`).
    RemoveRepo(String),
    /// Start signing in to the repository that has accounts (a code to enter in the browser).
    SignIn,
    /// Stop waiting for the code to be approved.
    CancelSignIn,
    SignOut,
    /// Like or favorite (`what`) a package, or take it back.
    Mark { id: String, what: &'static str, on: bool },
    /// Read the signed-in player's favorites.
    Favorites,
}

/// What the button on an app's card does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Update { from: String },
    Open,
    /// Installed and nothing to open: a program, a library or a source package.
    Installed,
    /// In the player's favorites, but no repository here offers it.
    NotOffered,
    /// Queued or running (`percent` of the download).
    Busy { percent: u32, label: String },
    /// The repository pins a key and the package isn't signed.
    Refused,
}

/// One app as a card or row.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub version: String,
    pub publisher: String,
    pub trust: String,
    pub verified: bool,
    pub action: Action,
    /// `app`, `program`, `library` or `source`.
    pub kind: Kind,
    /// Likes at the repository, when it counts them and they were read.
    pub likes: Option<u64>,
}

/// An app's page.
#[derive(Clone, Debug, PartialEq)]
pub struct Detail {
    pub card: Card,
    /// From the repository (None: installed only).
    pub available: Option<Available>,
    pub installed_version: Option<String>,
    pub size: Option<u64>,
    pub repo: Option<String>,
    /// (permission, words, the current answer if installed).
    pub permissions: Vec<(String, PermText, Option<PermState>)>,
    pub screenshots: Vec<String>,
    pub can_remove: bool,
    /// What the package needs installed first, as written (`dev.x.lib >= 1.0.0`).
    pub depends: Vec<String>,
    /// For a program or a library: it is not sandboxed. Shown before installing.
    pub warning: Option<String>,
    /// Likes, downloads and the player's own marks, from a repository with accounts.
    pub social: Option<Summary>,
    /// The player is signed in: Like and Favorite can be shown.
    pub signed_in: bool,
}

/// A sign-in in progress: the code to enter on the repository's page.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Login {
    pub code: String,
    pub url: String,
}

/// Apps the store never offers to remove (it is one).
pub const SYSTEM_APPS: &[&str] = &[crate::STORE_ID];

#[derive(Default)]
pub struct Store {
    pub catalog: Catalog,
    pub installed: Vec<Installed>,
    pub repos: Vec<Repo>,
    pub tab: Option<Tab>,
    /// The app whose page is open.
    pub detail: Option<String>,
    pub query: String,
    pub status: String,
    pub queue: VecDeque<Job>,
    pub running: Option<Running>,
    pub cmds: Vec<Cmd>,
    /// Pictures by key (`icon:<id>`, `shot:<id>:<member>`), filled by the driver.
    pub pics: HashMap<String, Img>,
    /// Permission answers of installed apps (`perm list <id>`).
    pub answers: HashMap<String, Vec<PermLine>>,
    /// Pids of apps the store opened.
    pub opened: Vec<(String, String)>,
    /// The Repositories page is open (it replaces the list, like an app's page).
    pub repos_page: bool,
    /// Repositories found on the network that aren't configured (docs/DISCOVERY.md): their
    /// apps are in the catalog too, and the Repositories page offers to add them.
    pub found: Vec<Repo>,
    /// The first repository that has accounts (https:// or host://), if any.
    pub account_repo: Option<Repo>,
    /// The handle signed in there.
    pub account: Option<String>,
    pub login: Option<Login>,
    /// What the repository says about packages whose page was opened.
    pub social: HashMap<String, Summary>,
    pub favorites: Vec<Summary>,
}

/// The words shown before a program or library is installed.
pub fn unconfined_warning(kind: Kind) -> Option<String> {
    kind.unconfined().then(|| {
        format!(
            "This is a {}, not an app: it is not sandboxed and can read and change everything on this computer. Install it only if you trust who made it.",
            kind.name()
        )
    })
}

/// The first configured repository that can have accounts.
pub fn account_repo(repos: &[Repo]) -> Option<Repo> {
    repos.iter().find(|r| Url::parse(&r.url).map(|u| u.private()).unwrap_or(false)).cloned()
}

impl Store {
    pub fn tab(&self) -> Tab {
        self.tab.unwrap_or(Tab::Discover)
    }

    pub fn installed_version(&self, id: &str) -> Option<&str> {
        self.installed.iter().find(|i| i.manifest.id == id).map(|i| i.manifest.version.as_str())
    }

    fn job_of(&self, id: &str) -> Option<(Action, bool)> {
        if let Some(r) = &self.running {
            if r.job.id == id {
                let label = match r.job.kind {
                    JobKind::Remove => "Removing".to_string(),
                    _ if r.total.is_some() => format!("{}%", r.percent()),
                    _ => "Starting".to_string(),
                };
                return Some((Action::Busy { percent: r.percent(), label }, true));
            }
        }
        self.queue.iter().any(|j| j.id == id).then(|| (Action::Busy { percent: 0, label: "Waiting".into() }, true))
    }

    fn action_for(&self, id: &str, available: Option<&Available>) -> Action {
        if let Some((a, _)) = self.job_of(id) {
            return a;
        }
        match available {
            Some(a) if a.trust() == TrustHint::Refused && self.installed_version(id).is_none() => Action::Refused,
            Some(a) => match Plan::of(self.installed_version(id), &a.entry.version) {
                Plan::Install => Action::Install,
                Plan::Update { from } => Action::Update { from },
                Plan::UpToDate | Plan::Newer { .. } if a.entry.kind() == Kind::App => Action::Open,
                Plan::UpToDate | Plan::Newer { .. } => Action::Installed,
            },
            None if self.installed.iter().any(|i| i.manifest.id == id && i.manifest.kind != Kind::App) => Action::Installed,
            None => Action::Open,
        }
    }

    fn card_of_available(&self, a: &Available) -> Card {
        let t = a.trust();
        Card {
            id: a.entry.id.clone(),
            name: a.entry.name.clone(),
            summary: a.entry.summary.clone(),
            version: a.entry.version.clone(),
            publisher: if a.entry.publisher.is_empty() { a.repo.name.clone() } else { a.entry.publisher.clone() },
            trust: t.label().into(),
            verified: t.verified(),
            action: self.action_for(&a.entry.id, Some(a)),
            kind: a.entry.kind(),
            likes: self.social.get(&a.entry.id).map(|s| s.likes),
        }
    }

    fn card_of_installed(&self, i: &Installed) -> Card {
        let m = &i.manifest;
        let trust = i.origin.as_ref().map(|o| o.trust.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| "system app".into());
        Card {
            id: m.id.clone(),
            name: m.name.clone(),
            summary: m.summary.clone(),
            version: m.version.clone(),
            publisher: m.publisher.clone(),
            verified: trust == "verified publisher" || trust == "system app",
            trust,
            action: self.action_for(&m.id, self.catalog.get(&m.id)),
            kind: m.kind,
            likes: self.social.get(&m.id).map(|s| s.likes),
        }
    }

    /// Discover: the catalog, filtered by the search.
    pub fn discover(&self) -> Vec<Card> {
        self.catalog.search(&self.query).into_iter().map(|a| self.card_of_available(a)).collect()
    }

    /// Installed apps a repository has a newer version of.
    pub fn updates(&self) -> Vec<Card> {
        self.catalog.updates(&self.installed).into_iter().map(|(_, a)| self.card_of_available(a)).collect()
    }

    pub fn installed_cards(&self) -> Vec<Card> {
        self.installed.iter().map(|i| self.card_of_installed(i)).collect()
    }

    /// The player's favorites: installable where a repository here offers them.
    pub fn favorite_cards(&self) -> Vec<Card> {
        self.favorites
            .iter()
            .map(|f| match self.catalog.get(&f.id) {
                Some(a) => {
                    let mut c = self.card_of_available(a);
                    c.likes = Some(f.likes);
                    c
                }
                None => Card {
                    id: f.id.clone(),
                    name: if f.name.is_empty() { f.id.clone() } else { f.name.clone() },
                    summary: f.summary.clone(),
                    version: f.version.clone(),
                    publisher: f.owner.clone(),
                    trust: String::new(),
                    verified: false,
                    action: if self.installed_version(&f.id).is_some() { Action::Installed } else { Action::NotOffered },
                    kind: Kind::parse(&f.kind).unwrap_or(Kind::App),
                    likes: Some(f.likes),
                },
            })
            .collect()
    }

    pub fn cards(&self) -> Vec<Card> {
        match self.tab() {
            Tab::Discover => self.discover(),
            Tab::Updates => self.updates(),
            Tab::Installed => self.installed_cards(),
            Tab::Favorites => self.favorite_cards(),
        }
    }

    pub fn detail_of(&self, id: &str) -> Option<Detail> {
        let available = self.catalog.get(id).cloned();
        let installed = self.installed.iter().find(|i| i.manifest.id == id);
        let card = match (&available, installed) {
            (Some(a), _) => self.card_of_available(a),
            (None, Some(i)) => self.card_of_installed(i),
            (None, None) => return None,
        };
        let declared: Vec<String> = match (installed, &available) {
            (Some(i), _) => i.manifest.permissions.clone(),
            (None, Some(a)) => a.entry.permissions.clone(),
            _ => Vec::new(),
        };
        let answers = self.answers.get(id);
        let permissions = declared
            .iter()
            .map(|p| {
                let state = installed.map(|_| {
                    answers.and_then(|a| a.iter().find(|l| &l.permission == p)).map_or(PermState::Ask, |l| l.state)
                });
                (p.clone(), perms::describe(p), state)
            })
            .collect();
        Some(Detail {
            screenshots: available.as_ref().map(|a| a.entry.screenshots.clone()).unwrap_or_default(),
            size: available.as_ref().map(|a| a.entry.size),
            repo: available.as_ref().map(|a| a.repo.name.clone()),
            installed_version: installed.map(|i| i.manifest.version.clone()),
            can_remove: installed.is_some() && !SYSTEM_APPS.contains(&id),
            depends: match (&available, installed) {
                (Some(a), _) => a.entry.depends.clone(),
                (None, Some(i)) => i.manifest.depends.iter().map(|d| d.to_string()).collect(),
                _ => Vec::new(),
            },
            warning: if installed.is_none() { unconfined_warning(card.kind) } else { None },
            social: self.social.get(id).cloned(),
            signed_in: self.account.is_some(),
            card,
            available,
            permissions,
        })
    }

    // ------------------------------------------------------------ actions (from handlers)

    pub fn set_tab(&mut self, t: Tab) {
        // Favorites only exists while signed in.
        let t = if t == Tab::Favorites && self.account.is_none() { Tab::Discover } else { t };
        self.tab = Some(t);
        self.detail = None;
        if t == Tab::Favorites {
            self.cmds.push(Cmd::Favorites);
        }
    }

    pub fn sign_in(&mut self) {
        self.cmds.push(Cmd::SignIn);
    }

    pub fn cancel_sign_in(&mut self) {
        self.cmds.push(Cmd::CancelSignIn);
    }

    pub fn sign_out(&mut self) {
        self.cmds.push(Cmd::SignOut);
    }

    /// Like or favorite (`what`) a package, or take it back.
    pub fn mark(&mut self, id: &str, what: &'static str, on: bool) {
        self.cmds.push(Cmd::Mark { id: id.to_string(), what, on });
    }

    pub fn open_detail(&mut self, id: &str) {
        self.repos_page = false;
        self.detail = Some(id.to_string());
        self.cmds.push(Cmd::Detail(id.to_string()));
    }

    pub fn close_detail(&mut self) {
        self.detail = None;
    }

    /// Queue an install, update or remove (once per app).
    pub fn enqueue(&mut self, kind: JobKind, id: &str) {
        if self.running.as_ref().is_some_and(|r| r.job.id == id) || self.queue.iter().any(|j| j.id == id) {
            return;
        }
        self.queue.push_back(Job { kind, id: id.to_string() });
    }

    /// The card button: install, update or open.
    pub fn act(&mut self, id: &str) {
        let card = self.detail_of(id).map(|d| d.card.action);
        match card {
            Some(Action::Install) => self.enqueue(JobKind::Install, id),
            Some(Action::Update { .. }) => self.enqueue(JobKind::Update, id),
            Some(Action::Open) => self.cmds.push(Cmd::Open(id.to_string())),
            _ => {}
        }
    }

    pub fn update_all(&mut self) {
        for c in self.updates() {
            self.enqueue(JobKind::Update, &c.id);
        }
    }

    pub fn refresh(&mut self) {
        self.cmds.push(Cmd::Refresh(true));
    }

    pub fn open_repos(&mut self) {
        self.detail = None;
        self.repos_page = true;
    }

    pub fn close_repos(&mut self) {
        self.repos_page = false;
    }

    /// The Add button: a repository by URL; an empty name is made from the URL's host.
    pub fn add_repo(&mut self, name: &str, url: &str) {
        let url = url.trim();
        if url.is_empty() {
            self.status = "Type the repository's address, e.g. http://10.0.30.1:8080/".into();
            return;
        }
        let url = if url.contains("://") { url.to_string() } else { format!("http://{url}") };
        let name = match name.trim() {
            "" => repo_name_for(&url),
            n => n.to_string(),
        };
        self.cmds.push(Cmd::AddRepo { name, url });
    }

    pub fn remove_repo(&mut self, name: &str) {
        self.cmds.push(Cmd::RemoveRepo(name.to_string()));
    }
}

pub use ecm_pkg::repo_name_for;

/// Read the repositories, the configured ones and then `found` (on the network): fetch
/// every index (a repository that can't be reached falls back to its cached index and is reported).
pub fn load_catalog<N: Net>(client: &mut Client<N>, fetch: bool, found: &[Repo]) -> (Catalog, Vec<String>) {
    let mut repos = client.repos();
    repos.extend(found.iter().cloned());
    let mut indexes = Vec::new();
    let mut errors = Vec::new();
    for r in repos {
        let got = if fetch { client.fetch_index(&r) } else { client.index_of(&r) };
        match got {
            Ok(ix) => indexes.push((r, ix)),
            Err(e) => {
                let cached = client.cached_index(&r);
                errors.push(format!("{}: {}{}", r.name, e, if cached.is_some() { " (showing the saved list)" } else { "" }));
                if let Some(ix) = cached {
                    indexes.push((r, ix));
                }
            }
        }
    }
    (Catalog::merge(indexes), errors)
}

/// Do one job. `Ok` is the message to show.
pub fn run_job<N: Net>(job: &Job, store: &Store, client: &mut Client<N>, ctl: &mut dyn AppCtl, progress: &mut dyn FnMut(usize, Option<usize>)) -> Result<String, String> {
    let msg = match job.kind {
        JobKind::Install | JobKind::Update => {
            let a = store.catalog.get(&job.id).cloned().ok_or_else(|| format!("{} is not in any repository", job.id))?;
            let r = client.install_available(&a, progress)?;
            match &r.replaced {
                Some(old) => format!("Updated {} {} -> {} ({})", r.manifest.name, old, r.manifest.version, r.trust.label()),
                None => format!("Installed {} {} ({})", r.manifest.name, r.manifest.version, r.trust.label()),
            }
        }
        JobKind::Remove => {
            if SYSTEM_APPS.contains(&job.id.as_str()) {
                return Err(format!("{} is a system app", job.id));
            }
            let name = store.installed.iter().find(|i| i.manifest.id == job.id).map(|i| i.manifest.name.clone()).unwrap_or(job.id.clone());
            client.remove(&job.id, false)?;
            format!("Removed {name} (its data is kept)")
        }
    };
    // The app manager picks up the change at once (no file watch).
    if let Err(e) = ctl.call(&["reindex"]) {
        return Ok(format!("{msg}; the app manager didn't reindex: {e}"));
    }
    Ok(msg)
}

/// The App Store app: the window, the repository client and the app manager.
pub struct StoreApp<N: Net, C: AppCtl> {
    pub app: App,
    pub store: Rc<RefCell<Store>>,
    pub client: Client<N>,
    pub ctl: C,
    pub images: Images,
    pub window: u32,
    /// Where the sign-in is kept (the computer's secret store in the program).
    pub secrets: Box<dyn Secrets>,
    /// Shows the sign-in page's link to the player outside the game (their chat), if it can.
    pub show_link: fn(&str, &str),
    /// A device sign-in being waited for, and when to ask again.
    pending: Option<(DeviceStart, std::time::Instant, std::time::Instant)>,
}

impl<N: Net, C: AppCtl> StoreApp<N, C> {
    /// Connect, open the window and read the repositories (fetching them when `fetch`).
    pub fn new(conn: Box<dyn Connection>, client: Client<N>, ctl: C, fetch: bool) -> Result<Self, ConnError> {
        let mut app = App::with_connection(crate::STORE_ID, conn)?;
        let store = Rc::new(RefCell::new(Store::default()));
        let s = store.clone();
        let window = app.window("App Store", move |cx| view::view(&s, cx));
        store.borrow_mut().cmds.push(Cmd::Refresh(fetch));
        let mut me = StoreApp {
            app,
            store,
            client,
            ctl,
            images: Images::new(),
            window,
            secrets: Box::new(MemorySecrets::default()),
            show_link: |_, _| {},
            pending: None,
        };
        me.process();
        Ok(me)
    }

    /// Show an app's page right away (`store --app <id>`).
    pub fn show(&mut self, id: &str) {
        self.store.borrow_mut().open_detail(id);
        self.process();
    }

    /// One loop turn: draw, wait up to `timeout_ms` for input, handle it, do queued work.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let busy = !self.store.borrow().cmds.is_empty() || !self.store.borrow().queue.is_empty();
        // While a sign-in waits for the player's browser, wake up to ask whether it is approved.
        let timeout = match (&self.pending, busy) {
            (_, true) => 0,
            (Some((d, _, _)), false) => {
                let wait = (d.interval * 1000) as i32;
                if timeout_ms < 0 { wait } else { timeout_ms.min(wait) }
            }
            (None, false) => timeout_ms,
        };
        if !self.app.step(timeout) {
            return false;
        }
        self.process();
        self.poll_sign_in();
        true
    }

    fn account_base(&self) -> Option<Url> {
        self.store.borrow().account_repo.as_ref().and_then(|r| Url::parse(&r.url).ok())
    }

    fn signed_in(&self) -> Option<Account> {
        account::load(self.secrets.as_ref(), &self.account_base()?)
    }

    /// Ask the repository whether the sign-in code was approved (at its interval).
    pub fn poll_sign_in(&mut self) {
        let Some((d, started, next)) = self.pending.clone() else { return };
        let now = std::time::Instant::now();
        if now < next {
            return;
        }
        let Some(base) = self.account_base() else {
            self.pending = None;
            return;
        };
        let outcome = if started.elapsed().as_secs() > d.expires_in { Ok(DevicePoll::Expired) } else { account::device_poll(&mut self.client.net, &base, &d.device_code) };
        let mut done = true;
        let status = match outcome {
            Ok(DevicePoll::Pending) => {
                done = false;
                self.pending = Some((d.clone(), started, now + std::time::Duration::from_secs(d.interval)));
                None
            }
            Ok(DevicePoll::Granted(a)) => match account::save(self.secrets.as_mut(), &a) {
                Ok(()) => {
                    self.store.borrow_mut().account = Some(a.handle.clone());
                    Some(format!("Signed in as {}", a.handle))
                }
                Err(e) => Some(format!("Signed in, but the sign-in could not be kept: {e}")),
            },
            Ok(DevicePoll::Denied) => Some("The sign-in was denied on the web page".into()),
            Ok(DevicePoll::Expired) => Some("The sign-in code ran out; try again".into()),
            Err(e) => Some(format!("Couldn't sign in: {e}")),
        };
        if done {
            self.pending = None;
            self.store.borrow_mut().login = None;
        }
        if let Some(m) = status {
            self.store.borrow_mut().status = m;
            let _ = self.redraw();
        }
    }

    pub fn run(&mut self) {
        while self.step(-1) {}
    }

    /// Do the queued commands and jobs.
    pub fn process(&mut self) {
        loop {
            let cmd = {
                let mut s = self.store.borrow_mut();
                if s.cmds.is_empty() {
                    None
                } else {
                    Some(s.cmds.remove(0))
                }
            };
            match cmd {
                Some(c) => self.command(c),
                None => break,
            }
        }
        loop {
            let job = self.store.borrow_mut().queue.pop_front();
            let Some(job) = job else { break };
            self.job(job);
        }
    }

    fn command(&mut self, c: Cmd) {
        match c {
            Cmd::Refresh(fetch) => {
                // Looking on the network goes with fetching the lists (opening, Refresh); a
                // refresh after an install keeps what was found.
                let found = if fetch {
                    self.client.found_repos()
                } else {
                    let repos = self.client.repos();
                    let mut f = self.store.borrow().found.clone();
                    f.retain(|x| !repos.iter().any(|r| r.url == x.url || r.name == x.name));
                    f
                };
                let (catalog, errors) = load_catalog(&mut self.client, fetch, &found);
                let installed = self.client.installed();
                let repos = self.client.repos();
                {
                    let mut s = self.store.borrow_mut();
                    let n = repos.len() + found.len();
                    s.status = if n == 0 {
                        "No repositories yet: add one under Repos (or pkg repo add <name> http://<host>:8080/)".into()
                    } else if !errors.is_empty() {
                        errors.join("; ")
                    } else {
                        let mut t = format!("{} apps from {} {}", catalog.apps.len(), n, if n == 1 { "repository" } else { "repositories" });
                        if !found.is_empty() {
                            t.push_str(&format!(" ({} found on the network)", found.len()));
                        }
                        t
                    };
                    s.catalog = catalog;
                    s.installed = installed;
                    s.account_repo = account_repo(&repos);
                    s.repos = repos;
                    s.found = found;
                }
                let who = self.signed_in().map(|a| a.handle);
                self.store.borrow_mut().account = who;
                self.load_icons();
            }
            Cmd::AddRepo { name, url } => match self.client.add_repo(&name, &url, None) {
                Ok(r) => {
                    self.store.borrow_mut().repos_page = false;
                    self.command(Cmd::Refresh(true));
                    let mut s = self.store.borrow_mut();
                    let n = s.catalog.apps.iter().filter(|a| a.repo.name == r.name).count();
                    if !s.status.contains(&r.name) {
                        s.status = format!("Added {} ({}): {} {}", r.name, r.url, n, if n == 1 { "app" } else { "apps" });
                    }
                }
                Err(e) => self.store.borrow_mut().status = format!("Couldn't add the repository: {e}"),
            },
            Cmd::RemoveRepo(name) => {
                match self.client.remove_repo(&name) {
                    Ok(()) => {
                        self.command(Cmd::Refresh(false));
                        self.store.borrow_mut().status = format!("Removed the repository {name}");
                    }
                    Err(e) => self.store.borrow_mut().status = format!("Couldn't remove {name}: {e}"),
                }
            }
            Cmd::Open(id) => {
                let name = self.store.borrow().detail_of(&id).map(|d| d.card.name).unwrap_or(id.clone());
                let r = self.ctl.call(&["launch", &id]);
                let mut s = self.store.borrow_mut();
                match r {
                    Ok(pid) => {
                        s.status = format!("Opened {name}");
                        s.opened.push((id, pid.trim().to_string()));
                    }
                    Err(e) => s.status = format!("Can't open {name}: {e}"),
                }
            }
            Cmd::SignIn => {
                let Some(base) = self.account_base() else {
                    self.store.borrow_mut().status = "No repository here has accounts (the official repository does).".into();
                    return;
                };
                match account::device_start(&mut self.client.net, &base, "App Store", &["social", "publish"]) {
                    Ok(d) => {
                        (self.show_link)(&d.verification_uri_complete, "Sign in to the package repository");
                        let mut s = self.store.borrow_mut();
                        s.login = Some(Login { code: d.user_code.clone(), url: d.verification_uri.clone() });
                        s.status = "Waiting for you to approve the code in your browser...".into();
                        let now = std::time::Instant::now();
                        self.pending = Some((d.clone(), now, now + std::time::Duration::from_secs(d.interval)));
                    }
                    Err(e) => self.store.borrow_mut().status = format!("Couldn't start signing in: {e}"),
                }
            }
            Cmd::CancelSignIn => {
                self.pending = None;
                let mut s = self.store.borrow_mut();
                s.login = None;
                s.status = "Sign-in cancelled".into();
            }
            Cmd::SignOut => {
                if let Some(base) = self.account_base() {
                    account::logout(&mut self.client.net, self.secrets.as_mut(), &base);
                }
                let mut s = self.store.borrow_mut();
                s.account = None;
                s.favorites.clear();
                for v in s.social.values_mut() {
                    v.liked = false;
                    v.favorited = false;
                }
                if s.tab == Some(Tab::Favorites) {
                    s.tab = Some(Tab::Discover);
                }
                s.status = "Signed out".into();
            }
            Cmd::Mark { id, what, on } => {
                let (Some(base), who) = (self.account_base(), self.signed_in()) else { return };
                match account::mark(&mut self.client.net, &base, who.as_ref(), &id, what, on) {
                    Ok(()) => {
                        // Read the counts back: someone else may have liked it meanwhile.
                        if let Ok(sum) = account::detail(&mut self.client.net, &base, who.as_ref(), &id) {
                            self.store.borrow_mut().social.insert(id.clone(), sum);
                        }
                        let showing_favorites = self.store.borrow().tab == Some(Tab::Favorites);
                        if what == "favorite" && showing_favorites {
                            self.command(Cmd::Favorites);
                        }
                    }
                    Err(e) => self.store.borrow_mut().status = e,
                }
            }
            Cmd::Favorites => {
                let (Some(base), who) = (self.account_base(), self.signed_in()) else { return };
                match account::mine(&mut self.client.net, &base, who.as_ref(), "favorites") {
                    Ok(list) => self.store.borrow_mut().favorites = list,
                    Err(e) => self.store.borrow_mut().status = format!("Couldn't read your favorites: {e}"),
                }
            }
            Cmd::Detail(id) => {
                // Likes and downloads, from a repository that counts them (one small request).
                let from_account_repo = {
                    let s = self.store.borrow();
                    match (&s.account_repo, s.catalog.get(&id)) {
                        (Some(r), Some(a)) => Url::parse(&r.url).ok().map(|u| u.origin()) == a.url().ok().map(|u| u.origin()),
                        _ => false,
                    }
                };
                if from_account_repo {
                    if let Some(base) = self.account_base() {
                        let who = self.signed_in();
                        if let Ok(sum) = account::detail(&mut self.client.net, &base, who.as_ref(), &id) {
                            self.store.borrow_mut().social.insert(id.clone(), sum);
                        }
                    }
                }
                if self.store.borrow().installed_version(&id).is_some() {
                    if let Ok(r) = self.ctl.call(&["perm", "list", &id]) {
                        let (lines, _) = appctl::parse_perm_list(&r);
                        self.store.borrow_mut().answers.insert(id.clone(), lines);
                    }
                }
                let available = self.store.borrow().catalog.get(&id).cloned();
                if let Some(a) = available {
                    for shot in &a.entry.screenshots {
                        let key = format!("shot:{id}:{shot}");
                        if self.images.known(&key) {
                            continue;
                        }
                        match self.client.fetch_asset(&a, shot) {
                            Ok(b) => {
                                if let Some(img) = self.images.load(&mut self.app.core, &key, &b) {
                                    self.store.borrow_mut().pics.insert(key, img);
                                }
                            }
                            Err(_) => self.images.missing(&key),
                        }
                    }
                }
            }
        }
    }

    /// Icons: installed apps' from their folders, the others' from their repository.
    fn load_icons(&mut self) {
        let (installed, available): (Vec<(String, String)>, Vec<Available>) = {
            let s = self.store.borrow();
            (
                s.installed.iter().filter(|i| !i.manifest.icon.is_empty()).map(|i| (i.manifest.id.clone(), i.manifest.icon.clone())).collect(),
                s.catalog.apps.iter().filter(|a| !a.entry.icon.is_empty()).take(64).cloned().collect(),
            )
        };
        for (id, icon) in installed {
            let key = format!("icon:{id}");
            if self.images.get(&key).is_some() {
                continue;
            }
            if let Some(b) = self.client.installed_file(&id, &icon) {
                self.images.forget_missing(&key);
                if let Some(img) = self.images.load(&mut self.app.core, &key, &b) {
                    self.store.borrow_mut().pics.insert(key, img);
                }
            }
        }
        for a in available {
            let key = format!("icon:{}", a.entry.id);
            if self.images.known(&key) {
                continue;
            }
            match self.client.fetch_asset(&a, &a.entry.icon) {
                Ok(b) => {
                    if let Some(img) = self.images.load(&mut self.app.core, &key, &b) {
                        self.store.borrow_mut().pics.insert(key, img);
                    }
                }
                Err(_) => self.images.missing(&key),
            }
        }
    }

    fn job(&mut self, job: Job) {
        self.store.borrow_mut().running = Some(Running { job: job.clone(), done: 0, total: None });
        let _ = self.redraw();
        let result = {
            let snapshot = Store { catalog: self.store.borrow().catalog.clone(), installed: self.store.borrow().installed.clone(), ..Default::default() };
            let store = self.store.clone();
            let app = &mut self.app;
            let mut last = 0u32;
            let mut progress = |done: usize, total: Option<usize>| {
                let pct = {
                    let mut s = store.borrow_mut();
                    if let Some(r) = s.running.as_mut() {
                        r.done = done;
                        r.total = total;
                    }
                    s.running.as_ref().map_or(0, |r| r.percent())
                };
                // Redraw when the bar moves by a few percent, not for every packet.
                if pct >= last + 5 || pct == 100 {
                    last = pct;
                    app.core.mark_all_dirty();
                    let _ = app.core.render();
                }
            };
            run_job(&job, &snapshot, &mut self.client, &mut self.ctl, &mut progress)
        };
        let installed = self.client.installed();
        {
            let mut s = self.store.borrow_mut();
            s.running = None;
            s.installed = installed;
            s.status = match result {
                Ok(m) => m,
                Err(e) => format!("Couldn't {} {}: {}", match job.kind { JobKind::Remove => "remove", JobKind::Update => "update", JobKind::Install => "install" }, job.id, e),
            };
            if job.kind == JobKind::Remove {
                s.pics.remove(&format!("icon:{}", job.id));
            }
        }
        if self.store.borrow().installed_version(&job.id).is_some() {
            self.store.borrow_mut().cmds.push(Cmd::Detail(job.id.clone()));
        }
        self.load_icons();
        let _ = self.redraw();
    }

    fn redraw(&mut self) -> Result<(), ConnError> {
        self.app.core.mark_all_dirty();
        self.app.core.render()
    }
}

#[cfg(test)]
mod tests;
