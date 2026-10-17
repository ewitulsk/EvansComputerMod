//! The HTTP API: [`App::handle`] turns one request into one [`Reply`], with
//! no socket I/O (the poll loop in `main.rs` does that), so every endpoint is
//! tested on the host against an in-memory disk.
//!
//! Background work (the incremental library scan and the Data saver
//! transcodes) runs a slice at a time in [`App::step`], so the loop keeps
//! answering while it goes on.

use crate::auth::{AuthFail, Gatekeeper};
use crate::disk::Disk;
use ecm_auth::{Claims, CLOCK_SKEW_MS};
use ecm_music::http::{self, Json, Request};
use ecm_music::library::{self, Album, Artist, FileStat, Library, ScanReport, Song, Sort};
use ecm_music::search::{Hit, HitKind, Index};
use ecm_music::store::{Playlist, Store, StoreError};
use ecm_music::transcode::{self, Mp3Decoder, SAVER_MIME, SAVER_RATE};
use std::collections::{HashSet, VecDeque};

/// Bytes of MP3 decoded per background step (about half a second of audio).
const TRANSCODE_STEP: usize = 16 * 1024;
const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 500;

/// What the daemon was started with.
#[derive(Clone, Debug)]
pub struct Config {
    pub name: String,
    pub port: u16,
    pub library_dir: String,
    pub data_dir: String,
    pub transcode: bool,
    pub public_browse: bool,
    pub max_kbps: u32,
    /// The realm whose accounts are accepted.
    pub realm_name: String,
    pub realm_fp: String,
    /// Where apps sign in: `-:<PORT+1>` (this host) or `host:port`.
    pub auth_endpoint: String,
}

impl Config {
    pub fn index_file(&self) -> String {
        format!("{}/index", self.data_dir)
    }
    pub fn store_file(&self) -> String {
        format!("{}/store", self.data_dir)
    }
    pub fn trigger_file(&self) -> String {
        format!("{}/rescan", self.data_dir)
    }
    pub fn status_file(&self) -> String {
        format!("{}/status", self.data_dir)
    }
    pub fn art_file(&self, song_id: &str, size: u32) -> String {
        format!("{}/art/{song_id}-{size}.png", self.data_dir)
    }
    pub fn cache_file(&self, song_id: &str) -> String {
        format!("{}/cache/{song_id}.ecms", self.data_dir)
    }
    pub fn song_file(&self, s: &Song) -> String {
        format!("{}/{}", self.library_dir, s.path)
    }
}

/// Per-connection state: the peer and the token it proved.
#[derive(Clone, Debug, Default)]
pub struct ConnCtx {
    pub peer_ip: String,
    verified: Option<(String, Claims)>,
}

impl ConnCtx {
    pub fn new(peer_ip: &str) -> ConnCtx {
        ConnCtx { peer_ip: peer_ip.to_string(), verified: None }
    }
}

/// A response body: bytes, or a region of a file streamed in chunks.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Bytes(Vec<u8>),
    File { path: String, start: u64, len: u64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub status: u16,
    /// The status line and headers, ending in the blank line.
    pub head: Vec<u8>,
    pub body: Body,
}

impl Reply {
    pub fn bytes(status: u16, content_type: &str, body: Vec<u8>, extra: &[(&str, &str)]) -> Reply {
        Reply { status, head: http::head(status, content_type, body.len() as u64, extra), body: Body::Bytes(body) }
    }

    pub fn json(status: u16, j: &Json) -> Reply {
        Reply::bytes(status, "application/json", j.to_string().into_bytes(), &[])
    }

    pub fn error(status: u16, code: &str, message: &str) -> Reply {
        Reply::json(status, &Json::obj([("error", Json::from(message)), ("code", Json::from(code))]))
    }

    fn auth(f: AuthFail) -> Reply {
        Reply::error(f.status(), f.code(), &f.message())
    }

    fn store(e: StoreError) -> Reply {
        let code = match e {
            StoreError::NotFound => "not_found",
            StoreError::NotOwner => "not_owner",
            StoreError::BadName => "bad_name",
            StoreError::BadIndex => "bad_index",
            StoreError::TooMany => "too_many",
        };
        Reply::error(e.status(), code, e.message())
    }

    fn not_found(what: &str) -> Reply {
        Reply::error(404, "not_found", &format!("no such {what}"))
    }

    fn bad(message: &str) -> Reply {
        Reply::error(400, "bad_request", message)
    }

    /// The body as text (tests).
    pub fn text(&self) -> String {
        match &self.body {
            Body::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
            Body::File { .. } => String::new(),
        }
    }
}

struct ScanJob {
    files: Vec<FileStat>,
    todo: Vec<FileStat>,
    next: usize,
    scanned: Vec<Song>,
}

struct TranscodeJob {
    id: String,
    mp3: Vec<u8>,
    pos: usize,
    dec: Mp3Decoder,
    pcm: Vec<i16>,
}

pub struct App<D: Disk> {
    pub cfg: Config,
    pub disk: D,
    pub lib: Library,
    pub store: Store,
    pub gate: Gatekeeper,
    index: Option<Index>,
    scan: Option<ScanJob>,
    queue: VecDeque<String>,
    job: Option<TranscodeJob>,
    failed: HashSet<String>,
    pub last_scan: Option<ScanReport>,
    /// Lines for the log, taken by the loop.
    pub log: Vec<String>,
}

fn limit_offset(req: &Request) -> (usize, usize) {
    (req.param_num("limit", DEFAULT_LIMIT).clamp(1, MAX_LIMIT), req.param_num("offset", 0))
}

/// The song ids of a JSON array.
fn ids(v: Option<&Json>) -> Option<Vec<String>> {
    v?.as_array()?.iter().map(|j| j.as_str().map(str::to_string)).collect()
}

impl<D: Disk> App<D> {
    /// Load the saved index and store (or start empty).
    pub fn new(cfg: Config, mut disk: D, gate: Gatekeeper) -> App<D> {
        let mut log = Vec::new();
        let lib = match disk.read_text(&cfg.index_file()).map(|t| Library::load(&t, Some(cfg.max_kbps))) {
            Some(Ok(l)) => l,
            Some(Err(e)) => {
                log.push(format!("{}: {e}; rescanning everything", cfg.index_file()));
                Library::new(cfg.max_kbps)
            }
            None => Library::new(cfg.max_kbps),
        };
        let store = match disk.read_text(&cfg.store_file()).map(|t| Store::load(&t)) {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                // Keep the unreadable file for the operator; start a new one.
                let bad = disk.read(&cfg.store_file()).unwrap_or_default();
                disk.write(&format!("{}.bad", cfg.store_file()), &bad);
                log.push(format!("{}: {e}; kept as store.bad, starting empty", cfg.store_file()));
                Store::new()
            }
            None => Store::new(),
        };
        App::with(cfg, disk, gate, lib, store, log)
    }
    fn with(cfg: Config, disk: D, gate: Gatekeeper, lib: Library, store: Store, log: Vec<String>) -> App<D> {
        let mut app =
            App { cfg, disk, lib, store, gate, index: None, scan: None, queue: VecDeque::new(), job: None, failed: HashSet::new(), last_scan: None, log };
        app.queue_transcodes();
        app
    }

    // ------------------------------------------------------------------ background work

    /// Is there background work (the loop then polls without waiting)?
    pub fn busy(&self) -> bool {
        self.scan.is_some() || self.job.is_some() || !self.queue.is_empty()
    }

    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// Start a scan if the library folder changed (new, changed or removed files).
    pub fn start_scan(&mut self) -> bool {
        if self.scan.is_some() {
            return false;
        }
        let files: Vec<FileStat> = self.disk.list(&self.cfg.library_dir).into_iter().filter(|f| library::is_mp3_path(&f.path)).collect();
        let todo: Vec<FileStat> = self.lib.changed(&files).into_iter().cloned().collect();
        let listed: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let removed = self.lib.songs().iter().filter(|s| !listed.contains(s.path.as_str())).count();
        if todo.is_empty() && removed == 0 {
            return false;
        }
        self.scan = Some(ScanJob { files, todo, next: 0, scanned: Vec::new() });
        true
    }

    /// Run a whole scan now (`musicd scan`).
    pub fn scan_now(&mut self) -> ScanReport {
        self.start_scan();
        while self.scan.is_some() {
            self.scan_step();
        }
        self.last_scan.clone().unwrap_or_default()
    }

    /// Reload the index file (after `musicd scan` wrote it).
    pub fn reload_index(&mut self) {
        if let Some(Ok(l)) = self.disk.read_text(&self.cfg.index_file()).map(|t| Library::load(&t, Some(self.cfg.max_kbps))) {
            self.lib = l;
            self.index = None;
            self.queue_transcodes();
        }
    }

    /// One slice of background work: a file of the scan, or 16 KB of a transcode.
    pub fn step(&mut self) {
        if self.scan.is_some() {
            self.scan_step();
        } else if self.cfg.transcode {
            self.transcode_step();
        }
    }

    fn scan_step(&mut self) {
        let Some(job) = self.scan.as_mut() else { return };
        if let Some(st) = job.todo.get(job.next).cloned() {
            job.next += 1;
            let path = format!("{}/{}", self.cfg.library_dir, st.path);
            match self.disk.read(&path).and_then(|b| Song::from_file(&st, &b)) {
                Some((song, tags)) => {
                    if let Some(p) = tags.picture {
                        match ecm_music::art::thumbnails(&p.data) {
                            Ok([small, large]) => {
                                self.disk.write(&self.cfg.art_file(&song.id, 64), &small);
                                self.disk.write(&self.cfg.art_file(&song.id, 160), &large);
                            }
                            Err(e) => self.log.push(format!("{}: cover art: {e}", st.path)),
                        }
                    }
                    // A changed file needs a new Data saver copy.
                    self.disk.remove(&self.cfg.cache_file(&song.id));
                    self.failed.remove(&song.id);
                    if song.bitrate_kbps > self.cfg.max_kbps {
                        self.log.push(format!("{}: {} kbps is above --max-kbps {}", st.path, song.bitrate_kbps, self.cfg.max_kbps));
                    }
                    self.scan.as_mut().unwrap().scanned.push(song);
                }
                None => self.log.push(format!("{}: not an MP3 musicd can read; skipped", st.path)),
            }
            return;
        }
        let job = self.scan.take().unwrap();
        let mut scanned = job.scanned;
        for s in &mut scanned {
            s.art = s.art.take().filter(|_| self.disk.size(&self.cfg.art_file(&library::song_id(&s.path), 64)).is_some());
        }
        let report = self.lib.apply(&job.files, scanned);
        let index = self.lib.save();
        self.disk.write(&self.cfg.index_file(), index.as_bytes());
        self.index = None;
        self.log.push(format!(
            "scan: {} songs ({} new, {} changed, {} removed, {} unreadable)",
            self.lib.len(),
            report.added,
            report.changed,
            report.removed,
            report.skipped
        ));
        self.last_scan = Some(report);
        self.queue_transcodes();
    }

    /// Queue every song without a Data saver copy.
    fn queue_transcodes(&mut self) {
        if !self.cfg.transcode {
            return;
        }
        let ids: Vec<String> = self
            .lib
            .songs()
            .iter()
            .map(|s| s.id.clone())
            .filter(|id| !self.failed.contains(id) && !self.queue.contains(id) && self.disk.size(&self.cfg.cache_file(id)).is_none())
            .collect();
        self.queue.extend(ids);
    }

    fn transcode_step(&mut self) {
        if self.job.is_none() {
            let Some(id) = self.queue.pop_front() else { return };
            if self.disk.size(&self.cfg.cache_file(&id)).is_some() {
                return;
            }
            let Some(song) = self.lib.song(&id) else { return };
            let Some(mp3) = self.disk.read(&self.cfg.song_file(song)) else {
                self.failed.insert(id);
                return;
            };
            self.job = Some(TranscodeJob { id, mp3, pos: 0, dec: Mp3Decoder::new(SAVER_RATE), pcm: Vec::new() });
            return;
        }
        let job = self.job.as_mut().unwrap();
        let end = (job.pos + TRANSCODE_STEP).min(job.mp3.len());
        job.dec.push(&job.mp3[job.pos..end], &mut job.pcm);
        job.pos = end;
        if job.pos < job.mp3.len() {
            return;
        }
        let mut job = self.job.take().unwrap();
        job.dec.finish(&mut job.pcm);
        if job.pcm.is_empty() {
            self.log.push(format!("Data saver: {} has no audio musicd can decode", job.id));
            self.failed.insert(job.id);
            return;
        }
        let file = transcode::encode_saver(&job.pcm);
        if !self.disk.write(&self.cfg.cache_file(&job.id), &file) {
            self.log.push(format!("Data saver: can't write {}", self.cfg.cache_file(&job.id)));
            self.failed.insert(job.id);
        }
    }

    /// Transcode `id` next.
    fn hurry(&mut self, id: &str) -> f64 {
        if let Some(j) = &self.job {
            if j.id == id {
                return j.pos as f64 / j.mp3.len().max(1) as f64;
            }
        }
        self.queue.retain(|q| q != id);
        self.queue.push_front(id.to_string());
        0.0
    }

    /// Songs still waiting for their Data saver copy.
    pub fn transcode_backlog(&self) -> usize {
        self.queue.len() + self.job.is_some() as usize
    }

    fn save_store(&mut self) {
        let text = self.store.save();
        if !self.disk.write(&self.cfg.store_file(), text.as_bytes()) {
            self.log.push(format!("can't write {}", self.cfg.store_file()));
        }
    }

    fn index(&mut self) -> &Index {
        if self.index.is_none() {
            let store = &self.store;
            self.index = Some(Index::build(&self.lib.search_docs(|id| store.play_count(id)), &store.search_docs()));
        }
        self.index.as_ref().unwrap()
    }

    // ------------------------------------------------------------------ JSON shapes

    pub fn song_json(&self, s: &Song, user: Option<&str>) -> Json {
        Json::obj([
            ("id", Json::from(&s.id)),
            ("title", Json::from(&s.title)),
            ("artist", Json::from(&s.artist)),
            ("artist_id", Json::from(&s.artist_id)),
            ("album", Json::from(&s.album)),
            ("album_id", Json::from(&s.album_id)),
            ("album_artist", Json::from(&s.album_artist)),
            ("genre", Json::from(&s.genre)),
            ("track", Json::from(s.track)),
            ("year", Json::from(s.year)),
            ("duration_ms", Json::from(s.duration_ms)),
            ("bitrate_kbps", Json::from(s.bitrate_kbps)),
            ("vbr", Json::from(s.vbr)),
            ("sample_rate", Json::from(s.sample_rate)),
            ("channels", Json::from(s.channels as u32)),
            ("art", Json::from(s.art.is_some())),
            ("liked", Json::from(user.is_some_and(|u| self.store.is_liked(u, &s.id)))),
            ("plays", Json::from(self.store.play_count(&s.id))),
        ])
    }

    /// A song by id, or `{"id": .., "missing": true}` for one that left the library.
    fn song_or_missing(&self, id: &str, user: Option<&str>) -> Json {
        match self.lib.song(id) {
            Some(s) => self.song_json(s, user),
            None => Json::obj([("id", Json::from(id)), ("missing", Json::from(true))]),
        }
    }

    pub fn album_json(&self, a: &Album) -> Json {
        Json::obj([
            ("id", Json::from(&a.id)),
            ("name", Json::from(&a.name)),
            ("artist", Json::from(&a.artist)),
            ("artist_id", Json::from(&a.artist_id)),
            ("year", Json::from(a.year)),
            ("genre", Json::from(&a.genre)),
            ("song_count", Json::from(a.songs.len())),
            ("duration_ms", Json::from(a.duration_ms)),
            ("art", Json::from(a.art_song.is_some())),
            ("added", Json::from(a.added)),
        ])
    }

    pub fn artist_json(&self, a: &Artist) -> Json {
        let art = a.albums.iter().any(|al| self.lib.album(al).is_some_and(|al| al.art_song.is_some()));
        Json::obj([
            ("id", Json::from(&a.id)),
            ("name", Json::from(&a.name)),
            ("album_count", Json::from(a.albums.len())),
            ("song_count", Json::from(a.songs.len())),
            ("art", Json::from(art)),
        ])
    }

    pub fn playlist_json(&self, p: &Playlist, user: Option<&str>, songs: bool) -> Json {
        let duration: u64 = p.songs.iter().filter_map(|s| self.lib.song(s)).map(|s| s.duration_ms).sum();
        let mut j = Json::obj([
            ("id", Json::from(&p.id)),
            ("name", Json::from(&p.name)),
            ("owner", Json::from(&p.owner)),
            ("public", Json::from(p.public)),
            ("song_count", Json::from(p.songs.len())),
            ("duration_ms", Json::from(duration)),
            ("created", Json::from(p.created)),
            ("updated", Json::from(p.updated)),
            ("mine", Json::from(user == Some(p.owner.as_str()))),
        ]);
        if songs {
            j = j.with("songs", p.songs.iter().map(|s| self.song_or_missing(s, user)).collect::<Vec<_>>());
        }
        j
    }

    fn hit_json(&self, h: &Hit, user: Option<&str>) -> Option<Json> {
        match h.kind {
            HitKind::Song => self.lib.song(&h.id).map(|s| self.song_json(s, user)),
            HitKind::Artist => self.lib.artist(&h.id).map(|a| self.artist_json(a)),
            HitKind::Album => self.lib.album(&h.id).map(|a| self.album_json(a)),
            HitKind::Playlist => self.store.playlist(user, &h.id).ok().map(|p| self.playlist_json(p, user, false)),
        }
    }

    pub fn info_json(&self) -> Json {
        let auth = Json::obj([
            ("name", Json::from(&self.cfg.realm_name)),
            ("fp", Json::from(&self.cfg.realm_fp)),
            ("endpoint", Json::from(&self.cfg.auth_endpoint)),
            ("local", Json::from(self.cfg.auth_endpoint.starts_with("-:"))),
            ("ready", Json::from(self.gate.realm_pub().is_some())),
        ]);
        let ep = self.cfg.auth_endpoint.strip_prefix("-:").map(|_| None).unwrap_or(Some(self.cfg.auth_endpoint.clone()));
        let realm_line = ecm_auth::RealmRef { name: self.cfg.realm_name.clone(), fp: self.cfg.realm_fp.clone(), endpoint: ep }.to_line();
        Json::obj([
            ("name", Json::from(&self.cfg.name)),
            ("api", Json::from(1u32)),
            ("backend_fp", Json::from(self.gate.audience())),
            ("songs", Json::from(self.lib.len())),
            ("albums", Json::from(self.lib.albums().len())),
            ("artists", Json::from(self.lib.artists().len())),
            ("transcode", Json::from(self.cfg.transcode)),
            ("public_browse", Json::from(self.cfg.public_browse)),
            ("auth", auth),
            ("realm", Json::from(realm_line)),
            ("formats", Json::obj([("original", Json::from("audio/mpeg")), ("saver", Json::from(SAVER_MIME))])),
            ("scanning", Json::from(self.scan.is_some())),
        ])
    }

    // ------------------------------------------------------------------ auth

    /// The signed-in user, if the request carries a token.
    fn optional_user(&mut self, req: &Request, ctx: &mut ConnCtx, now: u64) -> Result<Option<Claims>, AuthFail> {
        let Some(token) = req.bearer() else { return Ok(None) };
        if let Some((t, c)) = &ctx.verified {
            if t == token && now <= c.expires_ms + CLOCK_SKEW_MS {
                return Ok(Some(c.clone()));
            }
        }
        let (Some(nonce), Some(proof)) = (req.header("x-ecm-nonce"), req.header("x-ecm-proof")) else {
            return Err(AuthFail::ProofRequired);
        };
        let c = self.gate.verify(token, nonce, proof, &ctx.peer_ip, now)?;
        ctx.verified = Some((token.to_string(), c.clone()));
        Ok(Some(c))
    }

    fn user(&mut self, req: &Request, ctx: &mut ConnCtx, now: u64) -> Result<Claims, Reply> {
        match self.optional_user(req, ctx, now) {
            Ok(Some(c)) => Ok(c),
            Ok(None) => Err(Reply::auth(AuthFail::Required)),
            Err(f) => Err(Reply::auth(f)),
        }
    }

    /// For browsing: anyone with --public-browse on, else a signed-in user.
    fn viewer(&mut self, req: &Request, ctx: &mut ConnCtx, now: u64) -> Result<Option<String>, Reply> {
        match self.optional_user(req, ctx, now) {
            Ok(Some(c)) => Ok(Some(c.username)),
            Ok(None) if self.cfg.public_browse => Ok(None),
            Ok(None) => Err(Reply::auth(AuthFail::Required)),
            Err(f) => Err(Reply::auth(f)),
        }
    }

    // ------------------------------------------------------------------ routing

    /// Answer one request. `body` is the request body (Content-Length bytes).
    pub fn handle(&mut self, req: &Request, body: &[u8], ctx: &mut ConnCtx, now: u64) -> Reply {
        let segs = req.segments();
        let s: Vec<&str> = segs.iter().map(String::as_str).collect();
        let method = if req.method == "HEAD" { "GET" } else { req.method.as_str() };
        macro_rules! viewer {
            () => {
                match self.viewer(req, ctx, now) {
                    Ok(v) => v,
                    Err(r) => return r,
                }
            };
        }
        macro_rules! user {
            () => {
                match self.user(req, ctx, now) {
                    Ok(c) => c.username,
                    Err(r) => return r,
                }
            };
        }
        let json_body = || -> Result<Json, Reply> {
            if body.is_empty() {
                return Ok(Json::obj(Vec::<(String, Json)>::new()));
            }
            let text = std::str::from_utf8(body).map_err(|_| Reply::bad("the body is not UTF-8"))?;
            Json::parse(text).map_err(|e| Reply::bad(&format!("bad JSON: {e}")))
        };
        match (method, s.as_slice()) {
            ("GET", ["info"] | []) => Reply::json(200, &self.info_json()),
            ("GET", ["nonce"]) => {
                let n = self.gate.issue_nonce(ecm_auth::random(), now);
                Reply::json(200, &Json::obj([("nonce", Json::from(n)), ("expires_ms", Json::from(crate::auth::NONCE_MS)), ("audience", Json::from(self.gate.audience()))]))
            }
            ("GET", ["search"]) => {
                let v = viewer!();
                let (limit, offset) = limit_offset(req);
                let q = req.param("q").unwrap_or("").to_string();
                self.search(&q, v.as_deref(), limit, offset)
            }
            ("GET", ["songs", id]) => {
                let v = viewer!();
                match self.lib.song(id) {
                    Some(song) => Reply::json(200, &self.song_json(song, v.as_deref())),
                    None => Reply::not_found("song"),
                }
            }
            ("GET", ["albums"]) => {
                viewer!();
                let (limit, offset) = limit_offset(req);
                let sort = Sort::parse(req.param("sort").unwrap_or("az")).unwrap_or(Sort::Az);
                let items: Vec<Json> = self.lib.browse_albums(sort, limit, offset).iter().map(|a| self.album_json(a)).collect();
                Reply::json(200, &Json::obj([("total", Json::from(self.lib.albums().len())), ("items", Json::from(items))]))
            }
            ("GET", ["albums", id]) => {
                let v = viewer!();
                let Some(a) = self.lib.album(id) else { return Reply::not_found("album") };
                let songs: Vec<Json> = self.lib.album_songs(id).iter().map(|s| self.song_json(s, v.as_deref())).collect();
                Reply::json(200, &self.album_json(a).with("songs", songs))
            }
            ("GET", ["artists"]) => {
                viewer!();
                let (limit, offset) = limit_offset(req);
                let items: Vec<Json> = self.lib.artists().iter().skip(offset).take(limit).map(|a| self.artist_json(a)).collect();
                Reply::json(200, &Json::obj([("total", Json::from(self.lib.artists().len())), ("items", Json::from(items))]))
            }
            ("GET", ["artists", id]) => {
                let v = viewer!();
                let Some(a) = self.lib.artist(id) else { return Reply::not_found("artist") };
                let albums: Vec<Json> = a.albums.iter().filter_map(|al| self.lib.album(al)).map(|al| self.album_json(al)).collect();
                let songs: Vec<Json> = a.songs.iter().filter_map(|s| self.lib.song(s)).map(|s| self.song_json(s, v.as_deref())).collect();
                Reply::json(200, &self.artist_json(a).with("albums", albums).with("songs", songs))
            }
            ("GET", ["genres"]) => {
                viewer!();
                let g: Vec<Json> = self.lib.genres().iter().map(|(n, c)| Json::obj([("name", Json::from(n)), ("songs", Json::from(*c))])).collect();
                Reply::json(200, &Json::obj([("genres", Json::from(g))]))
            }
            ("GET", ["genres", name]) => {
                let v = viewer!();
                let (limit, offset) = limit_offset(req);
                let all = self.lib.genre_songs(name);
                let songs: Vec<Json> = all.iter().skip(offset).take(limit).map(|s| self.song_json(s, v.as_deref())).collect();
                Reply::json(200, &Json::obj([("name", Json::from(*name)), ("total", Json::from(all.len())), ("songs", Json::from(songs))]))
            }
            ("GET", ["browse"]) => {
                let v = viewer!();
                let (limit, offset) = limit_offset(req);
                let sort_name = req.param("sort").unwrap_or("new");
                let Some(sort) = Sort::parse(sort_name) else { return Reply::bad("sort is new or az") };
                let kind = req.param("kind").unwrap_or("songs");
                let (total, items): (usize, Vec<Json>) = match kind {
                    "songs" => (self.lib.len(), self.lib.browse(sort, limit, offset).iter().map(|s| self.song_json(s, v.as_deref())).collect()),
                    "albums" => (self.lib.albums().len(), self.lib.browse_albums(sort, limit, offset).iter().map(|a| self.album_json(a)).collect()),
                    _ => return Reply::bad("kind is songs or albums"),
                };
                Reply::json(
                    200,
                    &Json::obj([("sort", Json::from(if sort == Sort::New { "new" } else { "az" })), ("kind", Json::from(kind)), ("total", Json::from(total)), ("items", Json::from(items))]),
                )
            }
            ("GET", ["stream", id]) => {
                user!();
                self.stream(req, id)
            }
            ("GET", ["art", id, file]) => {
                viewer!();
                self.art(id, file)
            }
            ("GET", ["playlists"]) => {
                let v = viewer!();
                let list: Vec<&Playlist> = match (req.param("owner"), v.as_deref()) {
                    (Some(owner), _) => self.store.playlists_of(owner).into_iter().filter(|p| p.public || v.as_deref() == Some(owner)).collect(),
                    (None, Some(me)) => self.store.playlists_of(me),
                    (None, None) => self.store.visible_playlists(None),
                };
                let items: Vec<Json> = list.iter().map(|p| self.playlist_json(p, v.as_deref(), false)).collect();
                Reply::json(200, &Json::obj([("playlists", Json::from(items))]))
            }
            ("POST", ["playlists"]) => {
                let me = user!();
                let b = match json_body() {
                    Ok(b) => b,
                    Err(r) => return r,
                };
                let name = b.get("name").and_then(Json::as_str).unwrap_or("");
                let public = b.get("public").and_then(Json::as_bool).unwrap_or(false);
                let songs = ids(b.get("songs")).unwrap_or_default();
                if let Some(bad) = songs.iter().find(|s| self.lib.song(s).is_none()) {
                    return Reply::error(400, "unknown_song", &format!("no such song {bad}"));
                }
                let id = match self.store.create_playlist(&me, name, public, now) {
                    Ok(id) => id,
                    Err(e) => return Reply::store(e),
                };
                if !songs.is_empty() {
                    if let Err(e) = self.store.set_songs(&me, &id, &songs, now) {
                        self.store.delete_playlist(&me, &id).ok();
                        return Reply::store(e);
                    }
                }
                self.after_playlist_change();
                let p = self.store.playlist(Some(&me), &id).unwrap();
                Reply::json(201, &self.playlist_json(p, Some(&me), true))
            }
            ("GET", ["playlists", id]) => {
                let v = viewer!();
                match self.store.playlist(v.as_deref(), id) {
                    Ok(p) => Reply::json(200, &self.playlist_json(p, v.as_deref(), true)),
                    Err(e) => Reply::store(e),
                }
            }
            ("PUT", ["playlists", id]) => {
                let me = user!();
                let b = match json_body() {
                    Ok(b) => b,
                    Err(r) => return r,
                };
                self.edit_playlist(&me, id, &b, now)
            }
            ("DELETE", ["playlists", id]) => {
                let me = user!();
                match self.store.delete_playlist(&me, id) {
                    Ok(()) => {
                        self.after_playlist_change();
                        Reply::bytes(204, "", Vec::new(), &[])
                    }
                    Err(e) => Reply::store(e),
                }
            }
            ("POST", ["playlists", id, "songs"]) => {
                let me = user!();
                let b = match json_body() {
                    Ok(b) => b,
                    Err(r) => return r,
                };
                let edit = Json::obj([("add", b.get("songs").cloned().unwrap_or(Json::Null)), ("at", b.get("at").cloned().unwrap_or(Json::Null))]);
                self.edit_playlist(&me, id, &edit, now)
            }
            ("DELETE", ["playlists", id, "songs", index]) => {
                let me = user!();
                let Ok(i) = index.parse::<usize>() else { return Reply::bad("the position is a number") };
                self.edit_playlist(&me, id, &Json::obj([("remove", Json::Arr(vec![Json::from(i)]))]), now)
            }
            ("GET", ["likes"]) => {
                let me = user!();
                let (limit, offset) = limit_offset(req);
                let liked = self.store.liked(&me);
                let songs: Vec<Json> = liked.iter().skip(offset).take(limit).map(|s| self.song_or_missing(s, Some(&me))).collect();
                Reply::json(200, &Json::obj([("total", Json::from(liked.len())), ("songs", Json::from(songs))]))
            }
            ("POST" | "DELETE", ["likes", id]) => {
                let me = user!();
                let like = method == "POST";
                if like && self.lib.song(id).is_none() {
                    return Reply::not_found("song");
                }
                if like {
                    self.store.like(&me, id);
                } else {
                    self.store.unlike(&me, id);
                }
                self.save_store();
                Reply::json(200, &Json::obj([("id", Json::from(*id)), ("liked", Json::from(like))]))
            }
            ("POST", ["played", id]) => {
                let me = user!();
                if self.lib.song(id).is_none() {
                    return Reply::not_found("song");
                }
                self.store.played(&me, id);
                let plays = self.store.play_count(id);
                if let Some(ix) = self.index.as_mut() {
                    ix.set_plays(id, plays);
                }
                self.save_store();
                Reply::json(200, &Json::obj([("id", Json::from(*id)), ("plays", Json::from(plays)), ("user_plays", Json::from(self.store.user_play_count(&me, id)))]))
            }
            ("GET", ["recent"]) => {
                let me = user!();
                let songs: Vec<Json> = self.store.recent(&me).iter().map(|s| self.song_or_missing(s, Some(&me))).collect();
                Reply::json(200, &Json::obj([("songs", Json::from(songs))]))
            }
            (_, ["info" | "nonce" | "search" | "songs" | "albums" | "artists" | "genres" | "browse" | "stream" | "art" | "playlists" | "likes" | "played" | "recent", ..]) => {
                Reply::error(405, "method_not_allowed", &format!("{} is not allowed here", req.method))
            }
            _ => Reply::not_found("endpoint"),
        }
    }

    fn after_playlist_change(&mut self) {
        self.save_store();
        // Playlist names are searchable: rebuild on the next search.
        self.index = None;
    }

    fn search(&mut self, q: &str, user: Option<&str>, limit: usize, offset: usize) -> Reply {
        let r = self.index().search(q, user, limit, offset);
        let group = |hits: &[Hit], total: usize| -> Json {
            Json::obj([("total", Json::from(total)), ("items", Json::from(hits.iter().filter_map(|h| self.hit_json(h, user)).collect::<Vec<_>>()))])
        };
        let top = r.top.as_ref().and_then(|h| self.hit_json(h, user).map(|j| Json::obj([("kind", Json::from(h.kind.as_str())), ("item", j)])));
        Reply::json(
            200,
            &Json::obj([
                ("query", Json::from(q)),
                ("top", top.unwrap_or(Json::Null)),
                ("songs", group(&r.songs, r.total_songs)),
                ("artists", group(&r.artists, r.total_artists)),
                ("albums", group(&r.albums, r.total_albums)),
                ("playlists", group(&r.playlists, r.total_playlists)),
            ]),
        )
    }

    /// `PUT /playlists/<id>`: any of name, public, songs (replace), add (+ at),
    /// remove (positions), move {from, to}, applied in that order.
    fn edit_playlist(&mut self, me: &str, id: &str, b: &Json, now: u64) -> Reply {
        // Visibility and ownership first, so a stranger learns nothing from validation errors.
        match self.store.playlist(Some(me), id) {
            Ok(p) if p.owner == me => {}
            Ok(_) => return Reply::store(StoreError::NotOwner),
            Err(e) => return Reply::store(e),
        }
        let check = |list: &[String], lib: &Library| list.iter().find(|s| lib.song(s).is_none()).cloned();
        let mut r: Result<(), StoreError> = Ok(());
        if let Some(n) = b.get("name") {
            match n.as_str() {
                Some(n) => r = r.and_then(|_| self.store.rename(me, id, n, now)),
                None => return Reply::bad("name is a string"),
            }
        }
        if let Some(p) = b.get("public") {
            match p.as_bool() {
                Some(p) => r = r.and_then(|_| self.store.set_public(me, id, p, now)),
                None => return Reply::bad("public is true or false"),
            }
        }
        if let Some(v) = b.get("songs").filter(|v| !v.is_null()) {
            let Some(list) = ids(Some(v)) else { return Reply::bad("songs is a list of song ids") };
            if let Some(bad) = check(&list, &self.lib) {
                return Reply::error(400, "unknown_song", &format!("no such song {bad}"));
            }
            r = r.and_then(|_| self.store.set_songs(me, id, &list, now));
        }
        if let Some(v) = b.get("add").filter(|v| !v.is_null()) {
            let Some(list) = ids(Some(v)) else { return Reply::bad("add is a list of song ids") };
            if let Some(bad) = check(&list, &self.lib) {
                return Reply::error(400, "unknown_song", &format!("no such song {bad}"));
            }
            let at = b.get("at").and_then(Json::as_u64).map(|n| n as usize);
            r = r.and_then(|_| self.store.add_songs(me, id, &list, at, now));
        }
        if let Some(v) = b.get("remove").filter(|v| !v.is_null()) {
            let Some(mut pos): Option<Vec<usize>> = v.as_array().and_then(|a| a.iter().map(|j| j.as_u64().map(|n| n as usize)).collect()) else {
                return Reply::bad("remove is a list of positions");
            };
            pos.sort_unstable();
            pos.dedup();
            for p in pos.into_iter().rev() {
                r = r.and_then(|_| self.store.remove_at(me, id, p, now).map(|_| ()));
            }
        }
        if let Some(m) = b.get("move").filter(|v| !v.is_null()) {
            let (Some(from), Some(to)) = (m.get("from").and_then(Json::as_u64), m.get("to").and_then(Json::as_u64)) else {
                return Reply::bad("move is {\"from\": n, \"to\": n}");
            };
            r = r.and_then(|_| self.store.move_song(me, id, from as usize, to as usize, now));
        }
        if let Err(e) = r {
            self.after_playlist_change();
            return Reply::store(e);
        }
        self.after_playlist_change();
        let p = self.store.playlist(Some(me), id).unwrap();
        Reply::json(200, &self.playlist_json(p, Some(me), true))
    }

    fn stream(&mut self, req: &Request, id: &str) -> Reply {
        let Some(song) = self.lib.song(id) else { return Reply::not_found("song") };
        let duration = song.duration_ms.to_string();
        let (path, mime) = match req.param("q").unwrap_or("original") {
            "original" => (self.cfg.song_file(song), "audio/mpeg"),
            "saver" => {
                if !self.cfg.transcode {
                    return Reply::error(404, "no_transcode", "this server serves Original quality only");
                }
                let cache = self.cfg.cache_file(id);
                if self.disk.size(&cache).is_none() {
                    if self.failed.contains(id) {
                        return Reply::error(500, "transcode_failed", "this song can't be made into Data saver; play Original");
                    }
                    let progress = self.hurry(id);
                    let j = Json::obj([
                        ("error", Json::from("the Data saver copy is being made")),
                        ("code", Json::from("preparing")),
                        ("retry_ms", Json::from(2000u32)),
                        ("progress", Json::from((progress * 100.0).round() / 100.0)),
                    ]);
                    return Reply::bytes(503, "application/json", j.to_string().into_bytes(), &[("Retry-After", "2")]);
                }
                (cache, SAVER_MIME)
            }
            _ => return Reply::bad("q is original or saver"),
        };
        let Some(total) = self.disk.size(&path) else { return Reply::error(404, "file_missing", "the song's file is gone; rescanning") };
        let a = http::range_answer(req.header("range"), total, mime, &[("X-ECM-Duration-Ms", &duration), ("Cache-Control", "no-store")]);
        let body = if a.len > 0 { Body::File { path, start: a.start, len: a.len } } else { Body::Bytes(Vec::new()) };
        Reply { status: a.status, head: a.head, body }
    }

    fn art(&self, id: &str, file: &str) -> Reply {
        let size = match file {
            "64.png" => 64,
            "160.png" => 160,
            _ => return Reply::not_found("picture size (64.png or 160.png)"),
        };
        let album_art = |al: &str| self.lib.album(al).and_then(|a| a.art_song.clone());
        let song = if let Some(s) = self.lib.song(id) {
            s.art.as_ref().map(|_| s.id.clone()).or_else(|| album_art(&s.album_id))
        } else if self.lib.album(id).is_some() {
            album_art(id)
        } else if let Some(a) = self.lib.artist(id) {
            a.albums.iter().find_map(|al| album_art(al))
        } else {
            None
        };
        match song.and_then(|s| self.disk.read(&self.cfg.art_file(&s, size))) {
            Some(png) => Reply::bytes(200, "image/png", png, &[("Cache-Control", "max-age=3600")]),
            None => Reply::not_found("cover art"),
        }
    }

    /// `musicd status` and the status file.
    pub fn status_lines(&self) -> Vec<String> {
        let mut v = vec![
            format!("{}: {} songs, {} albums, {} artists, {} genres", self.cfg.name, self.lib.len(), self.lib.albums().len(), self.lib.artists().len(), self.lib.genres().len()),
            format!("library {}  data {}", self.cfg.library_dir, self.cfg.data_dir),
            format!("accounts: {} {} ({})", self.cfg.realm_name, ecm_auth::short_fingerprint(&self.cfg.realm_fp), self.cfg.auth_endpoint),
        ];
        if self.cfg.transcode {
            let cached = self.lib.songs().iter().filter(|s| self.disk.size(&self.cfg.cache_file(&s.id)).is_some()).count();
            v.push(format!("Data saver: {cached} of {} songs ready", self.lib.len()));
        } else {
            v.push("Data saver: off (--transcode off)".into());
        }
        if self.lib.warnings().is_empty() {
            v.push(format!("no files above {} kbps", self.cfg.max_kbps));
        } else {
            v.push(format!("{} file(s) above {} kbps (about {} KB/s per listener):", self.lib.warnings().len(), self.cfg.max_kbps, self.cfg.max_kbps / 8));
            for w in self.lib.warnings() {
                v.push(format!("  {} kbps  {}", w.kbps, w.path));
            }
        }
        v
    }
}
