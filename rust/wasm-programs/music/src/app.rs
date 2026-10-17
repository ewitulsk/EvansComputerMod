//! [`MusicApp`]: the window, the backends (found on the network, saved, typed),
//! their accounts, the API calls, and playback through the [`Engine`].
//!
//! Handlers only queue [`Action`]s on the [`Model`]; between loop turns the
//! driver does the I/O: the search for backends (on open and every minute),
//! `/info` of each, the sign-in screen's steps over the realm connection,
//! the API calls the actions need, and an engine turn. While a song plays it
//! keeps the `audio` background mode on and the Now Playing card (the
//! MediaSession) up to date about once a second.
//!
//! API calls wait for their answer (they are small); the song itself streams
//! without ever waiting, a piece per turn.

use crate::engine::{Engine, Event, Quality, State, Track};
use crate::httpc::{self, Reader};
use crate::model::{list, Action, Album, Artist, Detail, Info, Model, Origin, Page, Playlist, QualitySetting, Repeat, Results, Song, Tab};
use crate::net::{Net, Speaker};
use crate::view;
use ecm_apps_ui::signin::{Action as SignInAction, SignInScreen};
use ecm_auth::store::{AccountStore, Files, PinCheck};
use ecm_auth::AuthError;
use ecm_music::http::{percent_encode, Json};
use ecm_ui::conn::{ConnError, Connection};
use ecm_ui::{App, MediaSession, MediaState};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Poll this often while a song plays or loads, ms.
pub const PLAY_POLL_MS: i32 = 20;
/// The network is searched again this often, ms.
pub const RESEARCH_MS: u64 = 60_000;
/// A proven token is reused this long (musicd accepts a proof for 10 minutes).
pub const PROOF_MS: u64 = 9 * 60_000;
/// A token is fetched again after this long (they last an hour).
pub const TOKEN_MS: u64 = 50 * 60_000;
/// The Now Playing card is refreshed this often while playing, ms.
pub const MEDIA_MS: u64 = 1000;
/// How long a pause keeps the audio background mode (Play from home can still reach Music), ms.
pub const PAUSE_GRACE_MS: u64 = 60_000;
/// An API call waits this long for its answer, ms.
pub const API_WAIT_MS: u64 = 8000;

/// Any [`Files`] (the app's accounts folder, or memory in tests).
pub struct AnyFiles(pub Box<dyn Files>);

impl Files for AnyFiles {
    fn read(&self, name: &str) -> Option<String> {
        self.0.read(name)
    }
    fn write(&mut self, name: &str, text: &str) -> Result<(), AuthError> {
        self.0.write(name, text)
    }
    fn remove(&mut self, name: &str) {
        self.0.remove(name)
    }
}

fn split_ep(ep: &str) -> (String, u16) {
    match ep.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().unwrap_or(7800)),
        None => (ep.to_string(), 7800),
    }
}

pub struct MusicApp<N: Net, S: Speaker> {
    pub app: App,
    pub model: Rc<RefCell<Model>>,
    pub net: N,
    pub spk: S,
    pub engine: Engine,
    pub accounts: AccountStore<AnyFiles>,
    pub window: u32,
    data_dir: Option<String>,
    saved: String,
    /// API connection per backend endpoint.
    api: HashMap<String, i32>,
    /// Proven auth headers per backend endpoint: (headers, made at).
    proofs: HashMap<String, (String, u64)>,
    /// Tokens per backend endpoint: (token, fetched at).
    tokens: HashMap<String, (String, u64)>,
    last_search: u64,
    search_wanted: bool,
    last_media: u64,
    media_sig: String,
    audio_mode: bool,
    /// When playback paused (the audio mode is kept for [PAUSE_GRACE_MS]).
    paused_at: Option<u64>,
    art_rgba: Option<(String, (u16, u16, Vec<u8>))>,
    restore: Option<u64>,
    /// Cover image resources by song id (uploaded once).
    art_ids: HashMap<String, u32>,
    viz: crate::viz::Visualizer,
    /// The visualizer's canvas (a motion canvas), 0 until first shown.
    viz_canvas: u32,
    upcoming: Option<usize>,
    rng: u64,
    /// Songs reported played (tests, the log).
    pub played: Vec<String>,
    /// MediaSessions sent (tests).
    pub sessions: Vec<MediaSession>,
    /// The visualizer frames' pts (tests).
    pub viz_commits: Vec<u64>,
}

impl<N: Net, S: Speaker> MusicApp<N, S> {
    /// Open the window. `data_dir` keeps the settings; `files` is the accounts folder.
    pub fn new(conn: Box<dyn Connection>, net: N, spk: S, data_dir: Option<String>, files: Box<dyn Files>) -> Result<Self, ConnError> {
        let mut app = App::with_connection(crate::APP_ID, conn)?;
        let mut m = Model::default();
        if let Some(t) = data_dir.as_ref().and_then(|d| std::fs::read_to_string(format!("{d}/settings")).ok()) {
            m.load_settings(&t);
        }
        if let Ok(id) = app.core.image_png(32, 32, crate::ICON_PNG) {
            m.logo = id;
        }
        let saved = app.restore().and_then(|b| std::str::from_utf8(b).ok()).and_then(|t| Json::parse(t).ok());
        let restore = saved.and_then(|j| m.restore_state(&j));
        let model = Rc::new(RefCell::new(m));
        let v = model.clone();
        let window = app.window("Music", move |cx| view::view(&v, cx));
        for (slot, name, a) in [(1u8, "Play/Pause", Action::Toggle), (2, "Previous", Action::Previous), (3, "Next", Action::Next)] {
            let h = model.clone();
            app.hotkey(slot, name, move |_| h.borrow_mut().act(a.clone()));
        }
        let s = model.clone();
        app.on_suspend(move || s.borrow().saved_state().to_string().into_bytes());
        let md = model.clone();
        app.on_media(move |_, action, pos| {
            let mut m = md.borrow_mut();
            match action {
                "play" if !m.now.playing => m.act(Action::Toggle),
                "pause" if m.now.playing => m.act(Action::Toggle),
                "toggle" => m.act(Action::Toggle),
                "next" => m.act(Action::Next),
                "previous" => m.act(Action::Previous),
                "seek" => m.act(Action::Seek(pos as u64)),
                _ => {}
            }
        });
        let saved = model.borrow().settings_text();
        let mut a = MusicApp {
            app,
            model,
            net,
            spk,
            engine: Engine::new(),
            accounts: AccountStore::new(AnyFiles(files)),
            window,
            data_dir,
            saved,
            api: HashMap::new(),
            proofs: HashMap::new(),
            tokens: HashMap::new(),
            last_search: 0,
            search_wanted: true,
            last_media: 0,
            media_sig: String::new(),
            audio_mode: false,
            paused_at: None,
            art_rgba: None,
            restore,
            art_ids: HashMap::new(),
            viz: crate::viz::Visualizer::new(),
            viz_canvas: 0,
            upcoming: None,
            rng: 0x2545_f491_4f6c_dd1d,
            played: Vec::new(),
            sessions: Vec::new(),
            viz_commits: Vec::new(),
        };
        a.refresh_pins();
        Ok(a)
    }

    /// One loop turn: draw, wait for input, then the I/O.
    pub fn step(&mut self, timeout_ms: i32) -> bool {
        let wait = {
            let m = self.model.borrow();
            if m.has_actions() || self.search_wanted || m.backends.iter().any(|b| b.info.is_none() && b.error.is_none()) {
                0
            } else if matches!(self.engine.state, State::Playing | State::Buffering) {
                PLAY_POLL_MS
            } else if timeout_ms < 0 {
                1000
            } else {
                timeout_ms.min(1000)
            }
        };
        if !self.app.step(wait) {
            return false;
        }
        self.process();
        !self.app.core.is_closed()
    }

    pub fn run(&mut self) {
        while self.step(-1) {}
        self.engine.close(&mut self.net, &mut self.spk);
        for (_, h) in self.api.drain() {
            self.net.close(h);
        }
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// The I/O between loop turns.
    pub fn process(&mut self) {
        let now = self.net.now_ms();
        let mut changed = false;
        let shown = self.app.core.window(self.window).is_some_and(|w| w.configured);
        if shown && (self.search_wanted || (self.last_search != 0 && now.saturating_sub(self.last_search) >= RESEARCH_MS)) {
            self.search_wanted = false;
            self.model.borrow_mut().searching = true;
            self.redraw();
            let found = self.net.discover();
            self.last_search = now.max(1);
            let mut m = self.model.borrow_mut();
            m.searching = false;
            for f in found.iter().filter(|f| f.kind == ecm_disco::types::MUSIC) {
                let h = format!("{}.{}.{}.{}", f.host[0], f.host[1], f.host[2], f.host[3]);
                m.add_backend(&h, f.port, &f.name, Origin::Found);
            }
            changed = true;
        }
        changed |= self.fetch_infos();
        changed |= self.signin_steps();
        let actions = self.model.borrow_mut().take_actions();
        for a in actions {
            self.perform(a);
            changed = true;
        }
        self.try_restore();
        changed |= self.play_turn(now);
        self.save_settings();
        if changed {
            self.redraw();
        }
    }

    // ------------------------------------------------------------------ HTTP

    fn http(&mut self, ep: &str, method: &str, path: &str, headers: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
        let mut last = String::from("no answer");
        for _ in 0..2 {
            let h = match self.api.get(ep) {
                Some(h) => *h,
                None => {
                    let (host, port) = split_ep(ep);
                    let h = self.net.connect(&host, port)?;
                    self.api.insert(ep.to_string(), h);
                    h
                }
            };
            if self.net.send(h, &httpc::request(method, path, ep, headers, body)).is_err() {
                self.net.close(h);
                self.api.remove(ep);
                continue;
            }
            let mut r = Reader::new();
            let mut got = Vec::new();
            let start = self.net.now_ms();
            let mut idle = 0;
            loop {
                match self.net.recv(h) {
                    Ok(d) if d.is_empty() => {
                        idle += 1;
                        if self.net.now_ms().saturating_sub(start) > API_WAIT_MS || idle > 20_000 {
                            self.net.close(h);
                            self.api.remove(ep);
                            return Err(format!("{ep} didn't answer"));
                        }
                        self.net.wait(h, 20);
                    }
                    Ok(d) => {
                        idle = 0;
                        got.extend(r.push(&d)?);
                        if r.done() {
                            let status = r.head.as_ref().unwrap().status;
                            if r.head.as_ref().and_then(|hd| hd.header("connection")).is_some_and(|c| c.eq_ignore_ascii_case("close")) {
                                self.net.close(h);
                                self.api.remove(ep);
                            }
                            return Ok((status, got));
                        }
                    }
                    Err(e) => {
                        last = e;
                        self.net.close(h);
                        self.api.remove(ep);
                        break;
                    }
                }
            }
        }
        Err(if last.is_empty() { format!("the connection to {ep} closed") } else { last })
    }

    fn backend_index(&self, host: &str, port: u16) -> Option<usize> {
        self.model.borrow().backends.iter().position(|b| b.host == host && b.port == port)
    }

    /// Auth headers for backend `b` (a proven token), made when needed.
    fn auth_headers(&mut self, b: usize) -> Result<String, String> {
        let now = self.net.now_ms();
        let (ep, info, name) = {
            let m = self.model.borrow();
            let be = &m.backends[b];
            (be.endpoint(), be.info.clone(), be.name.clone())
        };
        if let Some((h, at)) = self.proofs.get(&ep) {
            if now.saturating_sub(*at) < PROOF_MS {
                return Ok(h.clone());
            }
        }
        let info = info.ok_or_else(|| format!("{name} hasn't answered yet"))?;
        if self.accounts.login(&info.realm_fp).is_none() {
            return Err(format!("Sign in to {} first", if name.is_empty() { &ep } else { &name }));
        }
        let token = match self.tokens.get(&ep) {
            Some((t, at)) if now.saturating_sub(*at) < TOKEN_MS => t.clone(),
            _ => {
                let stream = self.net.realm(&info.realm_host).map_err(|e| e.to_string())?;
                let (t, _) = self.accounts.token(stream, &info.realm_fp, &info.backend_fp).map_err(|e| e.to_string())?;
                self.tokens.insert(ep.clone(), (t.clone(), now));
                t
            }
        };
        let (status, body) = self.http(&ep, "GET", "/nonce", "", b"")?;
        let j = Json::parse(&String::from_utf8_lossy(&body)).map_err(|e| format!("/nonce: {e}"))?;
        let nonce = j.get("nonce").and_then(Json::as_str).filter(|_| status == 200).ok_or("no nonce from the server")?.to_string();
        let n: [u8; 32] = ecm_auth::unhex_n(&nonce).ok_or("a bad nonce from the server")?;
        let proof = self.accounts.prove(&info.realm_fp, &n, &info.backend_fp).map_err(|e| e.to_string())?;
        let headers = format!("Authorization: Bearer {token}\r\nX-ECM-Nonce: {nonce}\r\nX-ECM-Proof: {}\r\n", ecm_auth::hex(&proof));
        self.proofs.insert(ep, (headers.clone(), now));
        Ok(headers)
    }

    fn forget_auth(&mut self, ep: &str) {
        self.proofs.remove(ep);
        self.tokens.remove(ep);
    }

    /// A JSON API call on backend `b`. Signed-in calls carry the proven token.
    fn call(&mut self, b: usize, method: &str, path: &str, body: Option<Json>, need_auth: bool) -> Result<Json, String> {
        let (ep, signed_in, public) = {
            let m = self.model.borrow();
            let be = &m.backends[b];
            (be.endpoint(), be.user.is_some(), be.info.as_ref().is_none_or(|i| i.public_browse))
        };
        let body = body.map(|j| j.to_string().into_bytes()).unwrap_or_default();
        for attempt in 0..2 {
            let headers = if signed_in || need_auth || !public {
                match self.auth_headers(b) {
                    Ok(h) => h,
                    Err(e) if need_auth || !public => return Err(e),
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            };
            let (status, bytes) = self.http(&ep, method, path, &headers, &body)?;
            if status == 401 && attempt == 0 && !headers.is_empty() {
                self.forget_auth(&ep);
                continue;
            }
            if status == 204 {
                return Ok(Json::Null);
            }
            let j = Json::parse(&String::from_utf8_lossy(&bytes)).unwrap_or(Json::Null);
            if status >= 400 {
                return Err(j.get("error").and_then(Json::as_str).map(str::to_string).unwrap_or_else(|| format!("the server answered {status}")));
            }
            return Ok(j);
        }
        Err("not signed in".into())
    }

    fn current_call(&mut self, method: &str, path: &str, body: Option<Json>, need_auth: bool) -> Result<Json, String> {
        let b = self.model.borrow().current.ok_or("No music server yet")?;
        self.call(b, method, path, body, need_auth)
    }

    // ------------------------------------------------------------------ backends and accounts

    fn signed_in_as(&self, info: &Info) -> Option<String> {
        let pins = self.accounts.pins();
        if pins.check_pin(&info.realm_name, &info.realm_host, &info.realm_fp) != PinCheck::Pinned {
            return None;
        }
        self.accounts.login(&info.realm_fp).map(|l| l.display_name)
    }

    /// `/info` of every backend that hasn't answered yet (one per turn).
    fn fetch_infos(&mut self) -> bool {
        let next = self.model.borrow().backends.iter().position(|b| b.info.is_none() && b.error.is_none());
        let Some(b) = next else { return false };
        let (ep, host) = {
            let m = self.model.borrow();
            (m.backends[b].endpoint(), m.backends[b].host.clone())
        };
        match self.http(&ep, "GET", "/info", "", b"") {
            Ok((200, body)) => match Json::parse(&String::from_utf8_lossy(&body)) {
                Ok(j) => {
                    let info = Info::from_json(&j, &host);
                    let user = self.signed_in_as(&info);
                    let first = {
                        let mut m = self.model.borrow_mut();
                        let be = &mut m.backends[b];
                        if be.name.is_empty() {
                            be.name = info.name.clone();
                        }
                        be.info = Some(info);
                        be.user = user;
                        m.current == Some(b)
                    };
                    if first {
                        self.load_home();
                    }
                }
                Err(e) => self.model.borrow_mut().backends[b].error = Some(format!("not a music server ({e})")),
            },
            Ok((s, _)) => self.model.borrow_mut().backends[b].error = Some(format!("answered {s}")),
            Err(e) => self.model.borrow_mut().backends[b].error = Some(e),
        }
        true
    }

    fn refresh_pins(&mut self) {
        let pins = self.accounts.pins().pins.into_iter().map(|p| (p.fp, p.host, p.name)).collect();
        let mut m = self.model.borrow_mut();
        m.pins = pins;
        let users: Vec<Option<String>> = m.backends.iter().map(|b| b.info.as_ref().and_then(|i| self.signed_in_as(i))).collect();
        for (b, u) in m.backends.iter_mut().zip(users) {
            b.user = u;
        }
    }

    /// Open the sign-in screen for backend `b`'s realm.
    fn open_signin(&mut self, b: usize) {
        let Some(info) = self.model.borrow().backends.get(b).and_then(|x| x.info.clone()) else {
            self.model.borrow_mut().notice = Some("That server hasn't answered yet.".into());
            return;
        };
        let host = info.realm_host.clone();
        let screen = match self.net.realm(&host).and_then(|s| self.accounts.fingerprint(s, &host)) {
            Ok((si, check)) => {
                if si.realm_fp != info.realm_fp {
                    // The backend trusts one key; whoever answers at its sign-in address has another.
                    self.model.borrow_mut().notice = Some(format!(
                        "{} trusts the accounts key {}, but {} answers with {}: not signing in there.",
                        info.name,
                        ecm_auth::short_fingerprint(&info.realm_fp),
                        host,
                        ecm_auth::short_fingerprint(&si.realm_fp)
                    ));
                    return;
                }
                match self.accounts.login(&si.realm_fp).filter(|_| check == PinCheck::Pinned) {
                    Some(l) => SignInScreen::signed_in(&si.realm_name, &host, &si.realm_fp, &l.username, &l.display_name),
                    None => SignInScreen::new(&si.realm_name, &host, &si.realm_fp, &check),
                }
            }
            Err(AuthError::RealmChanged { name, host: h, old_fp, new_fp }) => {
                SignInScreen::new(&name, &host, &new_fp, &PinCheck::Changed { old_fp, name: name.clone(), host: h })
            }
            Err(e) => {
                self.model.borrow_mut().notice = Some(format!("Can't reach the sign-in of {}: {e}", info.name));
                return;
            }
        };
        let mut m = self.model.borrow_mut();
        m.signin = Some(Rc::new(RefCell::new(screen)));
        m.signin_backend = Some(b);
        m.open(Page::SignIn);
    }

    /// Do what the sign-in screen asked for.
    fn signin_steps(&mut self) -> bool {
        let (screen, b) = {
            let m = self.model.borrow();
            match (&m.signin, m.signin_backend) {
                (Some(s), Some(b)) => (s.clone(), b),
                _ => return false,
            }
        };
        let Some(action) = screen.borrow_mut().take_action() else { return false };
        let host = screen.borrow().host.clone();
        let close = |me: &mut Self| {
            let mut m = me.model.borrow_mut();
            m.signin = None;
            m.signin_backend = None;
            if m.page == Page::SignIn {
                m.back();
            }
        };
        match action {
            SignInAction::SignIn { user, pass } => {
                let r = self.net.realm(&host).and_then(|s| self.accounts.sign_in(s, &host, &user, &pass));
                self.after_signin(&screen, b, r);
            }
            SignInAction::SignUp { user, display, pass } => {
                let r = self.net.realm(&host).and_then(|s| self.accounts.sign_up(s, &host, &user, &pass, Some(&display)));
                self.after_signin(&screen, b, r);
            }
            SignInAction::Cancel => close(self),
            SignInAction::ForgetPin(fp) => {
                let _ = self.accounts.remove_pin(&fp);
                self.proofs.clear();
                self.tokens.clear();
                self.refresh_pins();
                close(self);
            }
        }
        true
    }

    fn after_signin(&mut self, screen: &Rc<RefCell<SignInScreen>>, b: usize, r: Result<ecm_auth::Login, AuthError>) {
        match r {
            Ok(l) => {
                screen.borrow_mut().succeeded(&l.username, &l.display_name);
                let ep = self.model.borrow().backends[b].endpoint();
                self.forget_auth(&ep);
                self.refresh_pins();
                if self.model.borrow().current == Some(b) {
                    self.load_home();
                }
            }
            Err(e) => screen.borrow_mut().failed(&e),
        }
    }

    // ------------------------------------------------------------------ pages

    fn load_home(&mut self) {
        let signed_in = self.model.borrow().backend().is_some_and(|b| b.user.is_some());
        let new = self.current_call("GET", "/browse?sort=new&limit=20", None, false).map(|j| Song::songs(list(&j, "items"))).unwrap_or_default();
        let (recent, pls) = if signed_in {
            (
                self.current_call("GET", "/recent", None, true).map(|j| Song::songs(list(&j, "songs"))).unwrap_or_default(),
                self.current_call("GET", "/playlists", None, true).map(|j| list(&j, "playlists").iter().map(Playlist::from_json).collect()).unwrap_or_default(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let mut m = self.model.borrow_mut();
        m.new_songs = new;
        m.recent = recent;
        m.my_playlists = pls;
    }

    fn load_library(&mut self) {
        let albums = self.current_call("GET", "/albums?limit=200", None, false).map(|j| list(&j, "items").iter().map(Album::from_json).collect()).unwrap_or_default();
        let artists = self.current_call("GET", "/artists?limit=200", None, false).map(|j| list(&j, "items").iter().map(Artist::from_json).collect()).unwrap_or_default();
        let signed_in = self.model.borrow().backend().is_some_and(|b| b.user.is_some());
        let pls = if signed_in {
            self.current_call("GET", "/playlists", None, true).map(|j| list(&j, "playlists").iter().map(Playlist::from_json).collect()).unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut m = self.model.borrow_mut();
        m.albums = albums;
        m.artists = artists;
        m.my_playlists = pls;
    }

    fn load_page(&mut self, p: &Page) -> Result<(), String> {
        match p {
            Page::Album(id) => {
                let j = self.current_call("GET", &format!("/albums/{}", percent_encode(id)), None, false)?;
                let d = Detail {
                    title: j.get("name").and_then(Json::as_str).unwrap_or("").into(),
                    subtitle: j.get("artist").and_then(Json::as_str).unwrap_or("").into(),
                    songs: Song::songs(list(&j, "songs")),
                    ..Detail::default()
                };
                self.model.borrow_mut().detail = d;
            }
            Page::Artist(id) => {
                let j = self.current_call("GET", &format!("/artists/{}", percent_encode(id)), None, false)?;
                let d = Detail {
                    title: j.get("name").and_then(Json::as_str).unwrap_or("").into(),
                    subtitle: "Artist".into(),
                    songs: Song::songs(list(&j, "songs")),
                    albums: list(&j, "albums").iter().map(Album::from_json).collect(),
                    playlist: None,
                };
                self.model.borrow_mut().detail = d;
            }
            Page::Playlist(id) => {
                let j = self.current_call("GET", &format!("/playlists/{}", percent_encode(id)), None, false)?;
                self.model.borrow_mut().detail = Detail { playlist: Some(Playlist::from_json(&j)), ..Detail::default() };
            }
            Page::Liked => {
                let j = self.current_call("GET", "/likes?limit=500", None, true)?;
                self.model.borrow_mut().liked = Song::songs(list(&j, "songs"));
            }
            Page::AddTo(_) => {
                let j = self.current_call("GET", "/playlists", None, true)?;
                self.model.borrow_mut().my_playlists = list(&j, "playlists").iter().map(Playlist::from_json).collect();
            }
            _ => {}
        }
        Ok(())
    }

    fn notice(&mut self, r: Result<(), String>) {
        if let Err(e) = r {
            self.model.borrow_mut().notice = Some(e);
        }
    }

    fn set_liked(&mut self, id: &str, on: bool) {
        let mut m = self.model.borrow_mut();
        let m = &mut *m;
        for list in [&mut m.recent, &mut m.new_songs, &mut m.results.songs, &mut m.detail.songs, &mut m.liked] {
            for s in list.iter_mut().filter(|s| s.id == id) {
                s.liked = on;
            }
        }
        if let Some(p) = m.detail.playlist.as_mut() {
            for s in p.songs.iter_mut().filter(|s| s.id == id) {
                s.liked = on;
            }
        }
    }

    fn perform(&mut self, a: Action) {
        self.model.borrow_mut().notice = None;
        match a {
            Action::Discover => self.search_wanted = true,
            Action::AddBackend(addr) => {
                let addr = addr.trim().to_string();
                if addr.is_empty() || addr.contains(' ') {
                    self.model.borrow_mut().notice = Some("Type an address like 10.0.0.5 or radio.village:7800".into());
                    return;
                }
                let (h, p) = match addr.rsplit_once(':') {
                    Some((h, p)) => match p.parse() {
                        Ok(p) => (h.to_string(), p),
                        Err(_) => {
                            self.model.borrow_mut().notice = Some(format!("bad port in {addr}"));
                            return;
                        }
                    },
                    None => (addr.clone(), 7800),
                };
                let mut m = self.model.borrow_mut();
                let i = m.add_backend(&h, p, "", Origin::Saved);
                m.backends[i].origin = Origin::Saved;
                m.backends[i].error = None;
                m.current = Some(i);
                m.back();
            }
            Action::Select(i) => {
                self.model.borrow_mut().current = Some(i);
                if self.model.borrow().backends[i].info.is_none() {
                    self.model.borrow_mut().backends[i].error = None;
                } else {
                    self.load_home();
                }
            }
            Action::SignIn(i) => self.open_signin(i),
            Action::SignOut(i) => {
                let info = self.model.borrow().backends[i].info.clone();
                if let Some(info) = info {
                    let stream = self.net.realm(&info.realm_host).ok();
                    let _ = self.accounts.logout(stream, &info.realm_fp);
                    let ep = self.model.borrow().backends[i].endpoint();
                    self.forget_auth(&ep);
                    self.refresh_pins();
                    self.load_home();
                }
            }
            Action::Tab(t) => {
                {
                    let mut m = self.model.borrow_mut();
                    m.tab = t;
                    m.page = Page::Main;
                    m.history.clear();
                }
                match t {
                    Tab::Home => self.load_home(),
                    Tab::Library => self.load_library(),
                    Tab::Settings => self.refresh_pins(),
                    _ => {}
                }
            }
            Action::Open(p) => {
                self.model.borrow_mut().open(p.clone());
                let r = self.load_page(&p);
                self.notice(r);
            }
            Action::Back => {
                self.model.borrow_mut().back();
                let p = self.model.borrow().page.clone();
                let _ = self.load_page(&p);
            }
            Action::Search(q) => {
                self.model.borrow_mut().query = q.clone();
                if q.trim().is_empty() {
                    self.model.borrow_mut().results = Results::default();
                    return;
                }
                match self.current_call("GET", &format!("/search?q={}&limit=20", percent_encode(q.trim())), None, false) {
                    Ok(j) => self.model.borrow_mut().results = Results::from_json(&j),
                    Err(e) => self.model.borrow_mut().notice = Some(e),
                }
            }
            Action::Play { songs, index } => {
                let Some((host, port)) = self.model.borrow().backend().map(|b| (b.host.clone(), b.port)) else { return };
                {
                    let mut m = self.model.borrow_mut();
                    m.queue = songs.iter().map(|s| s.track(&host, port)).collect();
                    m.pos = Some(index.min(m.queue.len().saturating_sub(1)));
                }
                self.start(true, 0);
            }
            Action::PlayNext(s) | Action::Enqueue(s) if self.model.borrow().pos.is_none() => {
                let host = self.model.borrow().backend().map(|b| (b.host.clone(), b.port));
                if let Some((h, p)) = host {
                    let mut m = self.model.borrow_mut();
                    m.queue.push(s.track(&h, p));
                    m.pos = Some(m.queue.len() - 1);
                    drop(m);
                    self.start(true, 0);
                }
            }
            Action::PlayNext(s) => {
                let host = self.model.borrow().backend().map(|b| (b.host.clone(), b.port));
                if let Some((h, p)) = host {
                    let mut m = self.model.borrow_mut();
                    let at = m.pos.map_or(0, |p| p + 1);
                    m.queue.insert(at, s.track(&h, p));
                    m.notice = Some(format!("{} plays next", s.title));
                }
                self.upcoming = None;
                self.prefetch_next();
            }
            Action::Enqueue(s) => {
                let host = self.model.borrow().backend().map(|b| (b.host.clone(), b.port));
                if let Some((h, p)) = host {
                    let mut m = self.model.borrow_mut();
                    m.queue.push(s.track(&h, p));
                    m.notice = Some(format!("{} added to the queue", s.title));
                }
                self.prefetch_next();
            }
            Action::PlayQueued(i) => {
                if i < self.model.borrow().queue.len() {
                    self.model.borrow_mut().pos = Some(i);
                    self.start(true, 0);
                }
            }
            Action::Unqueue(i) => {
                let current = {
                    let mut m = self.model.borrow_mut();
                    if i >= m.queue.len() {
                        return;
                    }
                    m.queue.remove(i);
                    match m.pos {
                        Some(p) if p == i => true,
                        Some(p) if p > i => {
                            m.pos = Some(p - 1);
                            false
                        }
                        _ => false,
                    }
                };
                if current {
                    let n = self.model.borrow().queue.len();
                    if i < n {
                        self.start(true, 0);
                    } else {
                        self.stop();
                    }
                }
                self.upcoming = None;
                self.prefetch_next();
            }
            Action::Toggle => {
                if self.engine.track().is_none() {
                    let pos = self.model.borrow().now.position_ms;
                    if self.model.borrow().pos.is_some() {
                        self.restore = None;
                        self.start(true, pos);
                    }
                } else if self.engine.state == State::Ended {
                    self.start(true, 0);
                } else {
                    self.engine.toggle(&mut self.spk);
                    if !self.engine.playing() {
                        self.viz_flush();
                    }
                }
            }
            Action::Next => self.advance(),
            Action::Previous => {
                if self.engine.position_ms() > 3000 || self.model.borrow().pos == Some(0) {
                    self.engine.seek(&mut self.net, &mut self.spk, 0);
                    self.viz_flush();
                } else {
                    let p = self.model.borrow().pos;
                    if let Some(p) = p {
                        self.model.borrow_mut().pos = Some(p.saturating_sub(1));
                    }
                }
                if self.engine.track().map(|t| t.id.clone()) != self.model.borrow().current_track().map(|t| t.id.clone()) {
                    self.start(true, 0);
                }
            }
            Action::Seek(ms) => {
                self.engine.seek(&mut self.net, &mut self.spk, ms);
                self.viz_flush();
            }
            Action::Shuffle => {
                let mut m = self.model.borrow_mut();
                m.shuffle = !m.shuffle;
                drop(m);
                self.upcoming = None;
                self.prefetch_next();
            }
            Action::Repeat => {
                let mut m = self.model.borrow_mut();
                m.repeat = match m.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                };
                drop(m);
                self.upcoming = None;
                self.prefetch_next();
            }
            Action::Like(id, on) => {
                let r = self.current_call(if on { "POST" } else { "DELETE" }, &format!("/likes/{}", percent_encode(&id)), None, true);
                match r {
                    Ok(_) => self.set_liked(&id, on),
                    Err(e) => self.model.borrow_mut().notice = Some(e),
                }
            }
            Action::NewPlaylist { name, song } => {
                let mut body = Json::obj([("name", Json::from(name.trim()))]);
                if let Some(s) = &song {
                    body = body.with("songs", vec![Json::from(&s.id)]);
                }
                match self.current_call("POST", "/playlists", Some(body), true) {
                    Ok(j) => {
                        let p = Playlist::from_json(&j);
                        let mut m = self.model.borrow_mut();
                        m.notice = Some(match &song {
                            Some(s) => format!("Added {} to {}", s.title, p.name),
                            None => format!("Made {}", p.name),
                        });
                        m.my_playlists.insert(0, p);
                        m.back();
                        if matches!(m.page, Page::AddTo(_)) {
                            m.back();
                        }
                    }
                    Err(e) => self.model.borrow_mut().notice = Some(e),
                }
            }
            Action::AddToPlaylist { playlist, song } => {
                let body = Json::obj([("songs", Json::from(vec![Json::from(&song.id)]))]);
                match self.current_call("POST", &format!("/playlists/{}/songs", percent_encode(&playlist)), Some(body), true) {
                    Ok(j) => {
                        let p = Playlist::from_json(&j);
                        let mut m = self.model.borrow_mut();
                        m.notice = Some(format!("Added {} to {}", song.title, p.name));
                        if let Some(x) = m.my_playlists.iter_mut().find(|x| x.id == p.id) {
                            *x = p;
                        }
                        m.back();
                    }
                    Err(e) => self.model.borrow_mut().notice = Some(e),
                }
            }
            Action::RemoveFromPlaylist { playlist, index } => {
                let r = self.current_call("DELETE", &format!("/playlists/{}/songs/{index}", percent_encode(&playlist)), None, true);
                self.playlist_answer(r);
            }
            Action::MoveInPlaylist { playlist, from, to } => {
                let body = Json::obj([("move", Json::obj([("from", Json::from(from)), ("to", Json::from(to))]))]);
                let r = self.current_call("PUT", &format!("/playlists/{}", percent_encode(&playlist)), Some(body), true);
                self.playlist_answer(r);
            }
            Action::SetPublic { playlist, public } => {
                let r = self.current_call("PUT", &format!("/playlists/{}", percent_encode(&playlist)), Some(Json::obj([("public", Json::from(public))])), true);
                self.playlist_answer(r);
            }
            Action::RenamePlaylist { playlist, name } => {
                let r = self.current_call("PUT", &format!("/playlists/{}", percent_encode(&playlist)), Some(Json::obj([("name", Json::from(name.trim()))])), true);
                if r.is_ok() {
                    self.model.borrow_mut().back();
                }
                self.playlist_answer(r);
            }
            Action::DeletePlaylist(id) => match self.current_call("DELETE", &format!("/playlists/{}", percent_encode(&id)), None, true) {
                Ok(_) => {
                    let mut m = self.model.borrow_mut();
                    m.my_playlists.retain(|p| p.id != id);
                    m.detail = Detail::default();
                    m.back();
                }
                Err(e) => self.model.borrow_mut().notice = Some(e),
            },
            Action::Quality(q) => {
                self.model.borrow_mut().quality = q;
                self.model.borrow_mut().notice = Some("The new quality applies from the next song.".into());
                self.prefetch_next();
            }
            Action::ForgetPin(fp) => {
                let _ = self.accounts.remove_pin(&fp);
                self.proofs.clear();
                self.tokens.clear();
                self.refresh_pins();
                self.model.borrow_mut().notice = Some("Forgotten: the next sign-in there is a first contact again.".into());
            }
        }
    }

    fn playlist_answer(&mut self, r: Result<Json, String>) {
        match r {
            Ok(j) => {
                let p = Playlist::from_json(&j);
                let mut m = self.model.borrow_mut();
                if let Some(x) = m.my_playlists.iter_mut().find(|x| x.id == p.id) {
                    *x = Playlist { songs: Vec::new(), ..p.clone() };
                }
                m.detail.playlist = Some(p);
            }
            Err(e) => self.model.borrow_mut().notice = Some(e),
        }
    }

    // ------------------------------------------------------------------ playback

    /// The quality for a song from backend `b` now (a setting change applies here: at the next song).
    fn quality_for(&mut self, b: usize) -> Quality {
        let transcode = self.model.borrow().backends.get(b).and_then(|x| x.info.as_ref()).is_some_and(|i| i.transcode);
        if !transcode {
            return Quality::Original;
        }
        match self.model.borrow().quality {
            QualitySetting::Original => Quality::Original,
            QualitySetting::Saver => Quality::Saver,
            QualitySetting::Auto => {
                if self.net.cellular() {
                    Quality::Saver
                } else {
                    Quality::Original
                }
            }
        }
    }

    /// Load the queue's current song.
    fn start(&mut self, play: bool, at_ms: u64) {
        let Some(t) = self.model.borrow().current_track().cloned() else { return };
        let Some(b) = self.backend_index(&t.host, t.port) else {
            self.model.borrow_mut().notice = Some(format!("{}:{} is not a known music server", t.host, t.port));
            return;
        };
        let auth = match self.auth_headers(b) {
            Ok(h) => h,
            Err(e) => {
                self.model.borrow_mut().notice = Some(e);
                return;
            }
        };
        let q = self.quality_for(b);
        self.viz_flush();
        self.engine.load(&mut self.net, &mut self.spk, t.clone(), q, &auth, at_ms, play);
        self.upcoming = None;
        self.prefetch_next();
        self.load_art(b, &t);
        let mut m = self.model.borrow_mut();
        m.now.track = Some(t);
        m.now.position_ms = at_ms;
    }

    fn stop(&mut self) {
        self.viz_flush();
        self.engine.stop(&mut self.net, &mut self.spk);
        let mut m = self.model.borrow_mut();
        m.now = Default::default();
        m.pos = None;
    }

    /// Fetch the start of the song after this one.
    fn prefetch_next(&mut self) {
        if self.engine.track().is_none() {
            return;
        }
        let r = self.random();
        let idx = match self.upcoming {
            Some(i) => Some(i),
            None => self.model.borrow().next_index(r),
        };
        self.upcoming = idx;
        let Some(t) = idx.and_then(|i| self.model.borrow().queue.get(i).cloned()) else { return };
        if Some(&t) == self.engine.track() {
            return;
        }
        let Some(b) = self.backend_index(&t.host, t.port) else { return };
        let Ok(auth) = self.auth_headers(b) else { return };
        let q = self.quality_for(b);
        self.engine.prefetch(&mut self.net, t, q, &auth);
    }

    /// The next song (after Next or the end of a song).
    fn advance(&mut self) {
        let r = self.random();
        let next = self.upcoming.or_else(|| self.model.borrow().next_index(r));
        match next {
            Some(i) => {
                self.model.borrow_mut().pos = Some(i);
                self.start(true, 0);
            }
            None => {
                self.engine.pause(&mut self.spk);
                self.engine.state = State::Ended;
                self.viz_flush();
                self.model.borrow_mut().now.playing = false;
            }
        }
    }

    /// After a cold boot: the saved song, paused at the saved place, once its backend answered.
    fn try_restore(&mut self) {
        let Some(ms) = self.restore else { return };
        let Some(t) = self.model.borrow().current_track().cloned() else {
            self.restore = None;
            return;
        };
        let ready = self.backend_index(&t.host, t.port).is_some_and(|b| self.model.borrow().backends[b].user.is_some());
        if ready {
            self.restore = None;
            self.start(false, ms);
        } else if self.backend_index(&t.host, t.port).is_none() {
            let mut m = self.model.borrow_mut();
            m.add_backend(&t.host, t.port, "", Origin::Saved);
        }
    }

    fn load_art(&mut self, b: usize, t: &Track) {
        self.art_rgba = None;
        self.model.borrow_mut().now.art = 0;
        if !t.art {
            return;
        }
        let ep = self.model.borrow().backends[b].endpoint();
        let headers = self.proofs.get(&ep).map(|p| p.0.clone()).unwrap_or_default();
        // One 64x64 picture for the Now Playing card and the window. (160x160 images work on
        // the platform, UiLargeResourceTest and gui_large_image; a stall seen with them came
        // from the old blocking MP3 decode in this loop, since sliced.)
        let Ok((200, png)) = self.http(&ep, "GET", &format!("/art/{}/64.png", percent_encode(&t.id)), &headers, b"") else { return };
        let Ok(img) = ecm_apps_ui::png::decode(&png) else { return };
        let id = match self.art_ids.get(&t.id) {
            Some(&id) => Some(id),
            None => self.app.core.image_rgba(img.w as i32, img.h as i32, &img.px).ok(),
        };
        if let Some(id) = id {
            self.art_ids.insert(t.id.clone(), id);
            self.model.borrow_mut().now.art = id;
        }
        self.art_rgba = Some((t.id.clone(), (img.w as u16, img.h as u16, img.px)));
    }
    /// Is the Now Playing tab on screen with something playing?
    fn viz_shown(&self) -> bool {
        let m = self.model.borrow();
        m.tab == Tab::NowPlaying && m.page == Page::Main && self.engine.playing() && self.app.core.window(self.window).is_some_and(|w| w.configured && w.activated)
    }

    /// Paint the frames the speaker's new audio completes, each committed for when it plays.
    fn viz_frames(&mut self) {
        let written = self.engine.take_written();
        if written.is_empty() {
            return;
        }
        if self.viz_canvas == 0 {
            self.viz_canvas = self.app.core.canvas(crate::viz::W as i32, crate::viz::H as i32, proto::PixelFormat::Rgba8888);
            // Moving bars: the pipeline may send them lossy.
            self.app.core.set_content(self.viz_canvas, 2);
            self.model.borrow_mut().now.viz = self.viz_canvas;
            self.app.core.mark_all_dirty();
        }
        let now_us = self.spk.display_now_us();
        for w in &written {
            for (pts, px) in self.viz.push(w, now_us) {
                if pts < now_us {
                    continue; // already late: its sound is out
                }
                let _ = self.app.core.put_canvas_at(self.viz_canvas, &px, &[], pts);
                self.viz_commits.push(pts);
            }
        }
    }

    /// Drop the visualizer's queued frames (pause, seek, a new song), as the speaker's are.
    fn viz_flush(&mut self) {
        self.viz.reset();
        if self.viz_canvas != 0 {
            let _ = self.app.core.queue_flush();
        }
    }

    /// An engine turn, then the Now Playing state, the background mode and the card.
    fn play_turn(&mut self, now: u64) -> bool {
        // The visualizer runs only while it is on screen (and never in the background).
        let viz_on = self.viz_shown();
        self.engine.keep_written = viz_on;
        let events = self.engine.tick(&mut self.net, &mut self.spk, now);
        if viz_on {
            self.viz_frames();
        }
        let mut changed = !events.is_empty();
        for e in events {
            match e {
                Event::Ended(_) => self.advance(),
                Event::Played(id) => {
                    if let Some(t) = self.engine.track().cloned() {
                        if let Some(b) = self.backend_index(&t.host, t.port) {
                            if self.call(b, "POST", &format!("/played/{}", percent_encode(&id)), None, true).is_ok() {
                                self.played.push(id);
                            }
                        }
                    }
                }
                Event::NeedAuth { host, port } => {
                    if let Some(b) = self.backend_index(&host, port) {
                        let ep = format!("{host}:{port}");
                        self.forget_auth(&ep);
                        if let Ok(h) = self.auth_headers(b) {
                            self.engine.set_auth(&host, port, &h);
                        }
                    }
                }
                Event::Fallback(_) => self.model.borrow_mut().notice = Some("The Data saver copy of this song isn't ready yet: playing Original.".into()),
                Event::Failed(e) => self.model.borrow_mut().notice = Some(format!("Can't play: {e}")),
                Event::Started(_) => {}
            }
        }
        let playing = self.engine.playing();
        let state = self.engine.state.clone();
        {
            let mut m = self.model.borrow_mut();
            if let Some(t) = self.engine.track() {
                m.now.track = Some(t.clone());
                let pos = self.engine.position_ms();
                if pos / 1000 != m.now.position_ms / 1000 {
                    changed = true;
                }
                m.now.position_ms = pos;
            }
            let buffering = playing && state == State::Buffering;
            let status = match &state {
                State::Buffering if playing && self.engine.failures() > 0 => "Buffering... (reconnecting)".to_string(),
                State::Buffering if playing => "Buffering...".to_string(),
                State::Failed(e) => format!("Can't play: {e}"),
                _ => String::new(),
            };
            if m.now.playing != playing || m.now.buffering != buffering || m.now.status != status {
                changed = true;
            }
            m.now.playing = playing;
            m.now.buffering = buffering;
            m.now.status = status;
        }
        // A pause keeps the audio mode a little while (PAUSE_GRACE_MS), so Play from the Now
        // Playing card or a hotkey at home still reaches Music; after that it lets go.
        if playing {
            self.paused_at = None;
        } else if self.audio_mode && self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
        let want = playing || (self.engine.track().is_some() && self.paused_at.is_some_and(|t| now.saturating_sub(t) < PAUSE_GRACE_MS));
        if want != self.audio_mode {
            self.audio_mode = want;
            self.app.core.background_mode("audio", want);
        }
        self.media(now, &state);
        changed
    }

    fn media(&mut self, now: u64, state: &State) {
        let Some(t) = self.engine.track().cloned() else {
            if !self.media_sig.is_empty() {
                self.media_sig.clear();
                self.app.core.media_session(&MediaSession::ended());
                self.sessions.push(MediaSession::ended());
            }
            return;
        };
        let ms = match state {
            State::Playing => MediaState::Playing,
            State::Buffering if self.engine.playing() => MediaState::Buffering,
            _ => MediaState::Paused,
        };
        let sig = format!("{}|{:?}", t.id, ms);
        if sig == self.media_sig && !(ms == MediaState::Playing && now.saturating_sub(self.last_media) >= MEDIA_MS) {
            return;
        }
        self.media_sig = sig;
        self.last_media = now;
        let art = self.art_rgba.as_ref().filter(|(id, _)| *id == t.id).map(|(_, a)| a.clone());
        let s = MediaSession {
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            art,
            state: ms,
            position_ms: self.engine.position_ms() as u32,
            duration_ms: t.duration_ms as u32,
            actions: vec![if ms == MediaState::Playing { "pause" } else { "play" }.into(), "next".into(), "previous".into(), "seek".into()],
        };
        self.app.core.media_session(&s);
        self.sessions.push(s);
    }

    fn save_settings(&mut self) {
        let text = self.model.borrow().settings_text();
        if text == self.saved {
            return;
        }
        self.saved = text.clone();
        if let Some(dir) = &self.data_dir {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(format!("{dir}/settings"), text);
        }
    }

    fn redraw(&mut self) {
        self.app.core.mark_all_dirty();
        let _ = self.app.core.render();
        if !self.app.core.window(self.window).is_none_or(|w| w.activated) && !self.engine.playing() {
            self.app.core.send(Message::BackgroundDone(proto::BackgroundDone));
        }
    }
}

#[cfg(test)]
pub mod fake;

#[cfg(test)]
mod tests;
