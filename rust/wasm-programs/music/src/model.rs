//! The app's state: the backends, what the pages show, the queue, the
//! settings. Plain data fed by the driver (`app.rs`) from musicd's JSON
//! answers; the view's handlers only queue [`Action`]s here.

use crate::engine::Track;
use ecm_apps_ui::signin::SignInScreen;
use ecm_music::http::Json;
use std::cell::RefCell;
use std::rc::Rc;

/// A song as the backend describes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Song {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub artist_id: String,
    pub album: String,
    pub album_id: String,
    pub duration_ms: u64,
    pub art: bool,
    pub liked: bool,
    /// Left the library (still in a playlist).
    pub missing: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub song_count: u64,
    pub art: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Artist {
    pub id: String,
    pub name: String,
    pub song_count: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub public: bool,
    pub song_count: u64,
    pub mine: bool,
    pub songs: Vec<Song>,
}

fn s(j: &Json, k: &str) -> String {
    j.get(k).and_then(Json::as_str).unwrap_or("").to_string()
}

fn n(j: &Json, k: &str) -> u64 {
    j.get(k).and_then(Json::as_u64).unwrap_or(0)
}

fn b(j: &Json, k: &str) -> bool {
    j.get(k).and_then(Json::as_bool).unwrap_or(false)
}

/// The items of a JSON array (or of `{"items": [...]}`) under `k`.
pub fn list<'a>(j: &'a Json, k: &str) -> &'a [Json] {
    match j.get(k) {
        Some(Json::Arr(a)) => a,
        Some(o) => o.get("items").and_then(Json::as_array).unwrap_or(&[]),
        None => &[],
    }
}

impl Song {
    pub fn from_json(j: &Json) -> Song {
        Song {
            id: s(j, "id"),
            title: s(j, "title"),
            artist: s(j, "artist"),
            artist_id: s(j, "artist_id"),
            album: s(j, "album"),
            album_id: s(j, "album_id"),
            duration_ms: n(j, "duration_ms"),
            art: b(j, "art"),
            liked: b(j, "liked"),
            missing: b(j, "missing"),
        }
    }

    pub fn track(&self, host: &str, port: u16) -> Track {
        Track {
            id: self.id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
            art: self.art,
            host: host.to_string(),
            port,
        }
    }

    pub fn songs(js: &[Json]) -> Vec<Song> {
        js.iter().map(Song::from_json).collect()
    }
}

impl Album {
    pub fn from_json(j: &Json) -> Album {
        Album { id: s(j, "id"), name: s(j, "name"), artist: s(j, "artist"), song_count: n(j, "song_count"), art: b(j, "art") }
    }
}

impl Artist {
    pub fn from_json(j: &Json) -> Artist {
        Artist { id: s(j, "id"), name: s(j, "name"), song_count: n(j, "song_count") }
    }
}

impl Playlist {
    pub fn from_json(j: &Json) -> Playlist {
        Playlist {
            id: s(j, "id"),
            name: s(j, "name"),
            owner: s(j, "owner"),
            public: b(j, "public"),
            song_count: n(j, "song_count"),
            mine: b(j, "mine"),
            songs: Song::songs(list(j, "songs")),
        }
    }
}

/// What `/info` said.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub name: String,
    pub backend_fp: String,
    pub songs: u64,
    pub transcode: bool,
    pub public_browse: bool,
    pub realm_name: String,
    pub realm_fp: String,
    /// Where to sign in: `host:port`.
    pub realm_host: String,
}

impl Info {
    /// From `/info` of the backend at `host`.
    pub fn from_json(j: &Json, host: &str) -> Info {
        let auth = j.get("auth").cloned().unwrap_or(Json::Null);
        let ep = s(&auth, "endpoint");
        // "-:<port>": the backend runs its own accounts, on the same host.
        let realm_host = match ep.strip_prefix("-:") {
            Some(port) => format!("{host}:{port}"),
            None => ep,
        };
        Info {
            name: s(j, "name"),
            backend_fp: s(j, "backend_fp"),
            songs: n(j, "songs"),
            transcode: b(j, "transcode"),
            public_browse: j.get("public_browse").and_then(Json::as_bool).unwrap_or(true),
            realm_name: s(&auth, "name"),
            realm_fp: s(&auth, "fp"),
            realm_host,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Answered a network search.
    Found,
    /// Added by the player, or remembered.
    Saved,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Backend {
    pub host: String,
    pub port: u16,
    pub name: String,
    pub origin: Origin,
    pub info: Option<Info>,
    pub error: Option<String>,
    /// Signed in as (display name), if so.
    pub user: Option<String>,
}

impl Backend {
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Tab {
    Home,
    Search,
    Library,
    NowPlaying,
    Settings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Main,
    Album(String),
    Artist(String),
    Playlist(String),
    Liked,
    AddBackend,
    SignIn,
    /// Pick a playlist to add this song to.
    AddTo(Song),
    NewPlaylist(Option<Song>),
    Rename(String),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QualitySetting {
    /// Data saver on cellular, Original on Wi-Fi or a cable.
    Auto,
    Original,
    Saver,
}

impl QualitySetting {
    pub fn name(self) -> &'static str {
        match self {
            QualitySetting::Auto => "auto",
            QualitySetting::Original => "original",
            QualitySetting::Saver => "saver",
        }
    }
    pub fn parse(s: &str) -> Option<QualitySetting> {
        [QualitySetting::Auto, QualitySetting::Original, QualitySetting::Saver].into_iter().find(|q| q.name() == s)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Repeat {
    Off,
    All,
    One,
}

/// What the player asked for; the driver does it.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Discover,
    AddBackend(String),
    Select(usize),
    SignIn(usize),
    SignOut(usize),
    Tab(Tab),
    Open(Page),
    Back,
    Search(String),
    /// Play `songs` (of the current backend) from `index`.
    Play { songs: Vec<Song>, index: usize },
    PlayNext(Song),
    Enqueue(Song),
    PlayQueued(usize),
    Unqueue(usize),
    Toggle,
    Next,
    Previous,
    Seek(u64),
    Shuffle,
    Repeat,
    Like(String, bool),
    NewPlaylist { name: String, song: Option<Song> },
    AddToPlaylist { playlist: String, song: Song },
    RemoveFromPlaylist { playlist: String, index: usize },
    MoveInPlaylist { playlist: String, from: usize, to: usize },
    RenamePlaylist { playlist: String, name: String },
    SetPublic { playlist: String, public: bool },
    DeletePlaylist(String),
    Quality(QualitySetting),
    ForgetPin(String),
}

/// The detail page's data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Detail {
    pub title: String,
    pub subtitle: String,
    pub songs: Vec<Song>,
    pub albums: Vec<Album>,
    pub playlist: Option<Playlist>,
}

/// Grouped search results.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Results {
    /// (kind, the item's name, a subtitle, the item's id), and the song if it is one.
    pub top: Option<(String, String, String, String, Option<Song>)>,
    pub songs: Vec<Song>,
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub playlists: Vec<Playlist>,
}

impl Results {
    pub fn from_json(j: &Json) -> Results {
        let top = j.get("top").filter(|t| !t.is_null()).map(|t| {
            let kind = s(t, "kind");
            let item = t.get("item").cloned().unwrap_or(Json::Null);
            let (name, sub) = match kind.as_str() {
                "song" => (s(&item, "title"), s(&item, "artist")),
                "album" => (s(&item, "name"), s(&item, "artist")),
                "playlist" => (s(&item, "name"), s(&item, "owner")),
                _ => (s(&item, "name"), "Artist".to_string()),
            };
            let song = (kind == "song").then(|| Song::from_json(&item));
            (kind, name, sub, s(&item, "id"), song)
        });
        Results {
            top,
            songs: Song::songs(list(j, "songs")),
            artists: list(j, "artists").iter().map(Artist::from_json).collect(),
            albums: list(j, "albums").iter().map(Album::from_json).collect(),
            playlists: list(j, "playlists").iter().map(Playlist::from_json).collect(),
        }
    }
}

/// What the Now Playing screen shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Now {
    pub track: Option<Track>,
    pub playing: bool,
    pub buffering: bool,
    pub position_ms: u64,
    pub status: String,
    /// The 160x160 cover (an image resource), 0 if none.
    pub art: u32,
    /// The visualizer's canvas, 0 until it first draws.
    pub viz: u32,
}

pub struct Model {
    pub backends: Vec<Backend>,
    pub current: Option<usize>,
    pub tab: Tab,
    pub page: Page,
    /// Pages to go back to.
    pub history: Vec<Page>,
    pub searching: bool,
    // Home
    pub recent: Vec<Song>,
    pub my_playlists: Vec<Playlist>,
    pub new_songs: Vec<Song>,
    // Search
    pub query: String,
    pub results: Results,
    // Library
    pub liked: Vec<Song>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub detail: Detail,
    // Playback
    pub queue: Vec<Track>,
    pub pos: Option<usize>,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub quality: QualitySetting,
    pub now: Now,
    // Accounts
    pub signin: Option<Rc<RefCell<SignInScreen>>>,
    pub signin_backend: Option<usize>,
    /// Pinned realms: (fp, host, name), for Settings > Accounts.
    pub pins: Vec<(String, String, String)>,
    pub notice: Option<String>,
    pub logo: u32,
    actions: Vec<Action>,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            backends: Vec::new(),
            current: None,
            tab: Tab::Home,
            page: Page::Main,
            history: Vec::new(),
            searching: false,
            recent: Vec::new(),
            my_playlists: Vec::new(),
            new_songs: Vec::new(),
            query: String::new(),
            results: Results::default(),
            liked: Vec::new(),
            albums: Vec::new(),
            artists: Vec::new(),
            detail: Detail::default(),
            queue: Vec::new(),
            pos: None,
            shuffle: false,
            repeat: Repeat::Off,
            quality: QualitySetting::Auto,
            now: Now::default(),
            signin: None,
            signin_backend: None,
            pins: Vec::new(),
            notice: None,
            logo: 0,
            actions: Vec::new(),
        }
    }
}

impl Model {
    pub fn act(&mut self, a: Action) {
        self.actions.push(a);
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    pub fn has_actions(&self) -> bool {
        !self.actions.is_empty()
    }

    pub fn backend(&self) -> Option<&Backend> {
        self.current.and_then(|i| self.backends.get(i))
    }

    /// Add (or find) a backend; its index.
    pub fn add_backend(&mut self, host: &str, port: u16, name: &str, origin: Origin) -> usize {
        if let Some(i) = self.backends.iter().position(|b| b.host == host && b.port == port) {
            if !name.is_empty() && self.backends[i].name.is_empty() {
                self.backends[i].name = name.to_string();
            }
            return i;
        }
        self.backends.push(Backend { host: host.into(), port, name: name.into(), origin, info: None, error: None, user: None });
        if self.current.is_none() {
            self.current = Some(self.backends.len() - 1);
        }
        self.backends.len() - 1
    }

    pub fn open(&mut self, p: Page) {
        if self.page != p {
            let old = std::mem::replace(&mut self.page, p);
            self.history.push(old);
        }
    }

    pub fn back(&mut self) {
        self.page = self.history.pop().unwrap_or(Page::Main);
        if self.page == Page::SignIn {
            self.page = Page::Main;
        }
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.pos.and_then(|p| self.queue.get(p))
    }

    /// The index after `pos` (shuffle picks with `random`), honouring Repeat All.
    pub fn next_index(&self, random: u64) -> Option<usize> {
        let p = self.pos?;
        let n = self.queue.len();
        if n == 0 {
            return None;
        }
        if self.repeat == Repeat::One {
            return Some(p);
        }
        if self.shuffle && n > 1 {
            let mut r = (random % (n as u64 - 1)) as usize;
            if r >= p {
                r += 1;
            }
            return Some(r);
        }
        if p + 1 < n {
            Some(p + 1)
        } else if self.repeat == Repeat::All {
            Some(0)
        } else {
            None
        }
    }

    /// Settings text (data folder): backends and the quality.
    pub fn settings_text(&self) -> String {
        let mut s = format!("quality {}\n", self.quality.name());
        for b in self.backends.iter().filter(|b| b.origin == Origin::Saved) {
            s.push_str(&format!("backend {} {}\n", b.endpoint(), b.name));
        }
        s
    }

    pub fn load_settings(&mut self, text: &str) {
        for l in text.lines() {
            let mut w = l.splitn(3, ' ');
            match (w.next(), w.next()) {
                (Some("quality"), Some(q)) => self.quality = QualitySetting::parse(q).unwrap_or(QualitySetting::Auto),
                (Some("backend"), Some(ep)) => {
                    if let Some((h, p)) = ep.rsplit_once(':') {
                        if let Ok(p) = p.parse() {
                            self.add_backend(h, p, w.next().unwrap_or(""), Origin::Saved);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// What is saved on suspend: the queue and the position.
    pub fn saved_state(&self) -> Json {
        let q: Vec<Json> = self
            .queue
            .iter()
            .map(|t| {
                Json::obj([
                    ("id", Json::from(&t.id)),
                    ("title", Json::from(&t.title)),
                    ("artist", Json::from(&t.artist)),
                    ("album", Json::from(&t.album)),
                    ("duration_ms", Json::from(t.duration_ms)),
                    ("art", Json::from(t.art)),
                    ("host", Json::from(&t.host)),
                    ("port", Json::from(t.port as u32)),
                ])
            })
            .collect();
        Json::obj([("queue", Json::from(q)), ("pos", Json::from(self.pos.map(|p| p as u64))), ("position_ms", Json::from(self.now.position_ms))])
    }

    /// Restore [`Model::saved_state`]; the position to resume at.
    pub fn restore_state(&mut self, j: &Json) -> Option<u64> {
        let q = j.get("queue")?.as_array()?;
        self.queue = q
            .iter()
            .map(|t| Track {
                id: s(t, "id"),
                title: s(t, "title"),
                artist: s(t, "artist"),
                album: s(t, "album"),
                duration_ms: n(t, "duration_ms"),
                art: b(t, "art"),
                host: s(t, "host"),
                port: n(t, "port") as u16,
            })
            .collect();
        self.pos = j.get("pos").and_then(Json::as_u64).map(|p| p as usize).filter(|p| *p < self.queue.len());
        self.pos?;
        let ms = n(j, "position_ms");
        self.now.track = self.current_track().cloned();
        self.now.position_ms = ms;
        Some(ms)
    }
}

/// `m:ss`.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_order_settings_and_saved_state() {
        let mut m = Model::default();
        m.queue = (0..4).map(|i| Track { id: format!("s{i}"), host: "h".into(), port: 7800, ..Track::default() }).collect();
        m.pos = Some(3);
        assert_eq!(m.next_index(0), None);
        m.repeat = Repeat::All;
        assert_eq!(m.next_index(0), Some(0));
        m.repeat = Repeat::One;
        assert_eq!(m.next_index(0), Some(3));
        m.repeat = Repeat::Off;
        m.shuffle = true;
        for r in 0..20 {
            assert_ne!(m.next_index(r), Some(3), "shuffle never repeats the current song");
        }
        m.add_backend("10.0.0.5", 7800, "Radio", Origin::Saved);
        m.quality = QualitySetting::Saver;
        let mut m2 = Model::default();
        m2.load_settings(&m.settings_text());
        assert_eq!((m2.quality, m2.backends[0].endpoint(), m2.backends[0].name.as_str()), (QualitySetting::Saver, "10.0.0.5:7800".to_string(), "Radio"));
        m.now.position_ms = 61_000;
        let mut m3 = Model::default();
        assert_eq!(m3.restore_state(&m.saved_state()), Some(61_000));
        assert_eq!((m3.queue.len(), m3.pos), (4, Some(3)));
        assert_eq!(clock(61_000), "1:01");
    }

    #[test]
    fn info_resolves_a_local_realm_to_the_backend_host() {
        let j = Json::parse(r#"{"name":"R","backend_fp":"ab","auth":{"name":"R","fp":"ab","endpoint":"-:7801"},"public_browse":true}"#).unwrap();
        assert_eq!(Info::from_json(&j, "10.0.0.5").realm_host, "10.0.0.5:7801");
        let j = Json::parse(r#"{"auth":{"endpoint":"10.0.8.1:7600"}}"#).unwrap();
        assert_eq!(Info::from_json(&j, "10.0.0.5").realm_host, "10.0.8.1:7600");
    }
}
