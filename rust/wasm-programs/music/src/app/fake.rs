//! A scripted world for tests: real musicd backends (`musicd::api::App` over
//! an in-memory library, behind real `musicd::conn::Conn` HTTP connections),
//! each running its own realm (`ecm_auth::Realm` behind `ServerConn`), a
//! clock, a network that can be slow or drop connections, and a speaker that
//! plays at its rate by that clock.

use crate::net::{Net, Speaker};
use ecm_auth::{public_key, signing_key, AuthError, Realm, ServerConn, Stream};
use musicd::api::{App as Backend, Config};
use musicd::auth::Gatekeeper;
use musicd::conn::Conn;
use musicd::disk::MemDisk;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};
use std::rc::Rc;

pub const CBR: &[u8] = include_bytes!("../../../../crates/ecm-music/tests/data/tone_cbr.mp3");
pub const VBR: &[u8] = include_bytes!("../../../../crates/ecm-music/tests/data/tone_vbr.mp3");
pub const MPEG2: &[u8] = include_bytes!("../../../../crates/ecm-music/tests/data/tone_mpeg2.mp3");
pub const START: u64 = 1_800_000_000_000;

/// An ID3v1 tag.
pub fn v1(title: &str, artist: &str, album: &str) -> Vec<u8> {
    let mut t = vec![0u8; 128];
    t[..3].copy_from_slice(b"TAG");
    for (s, at) in [(title, 3), (artist, 33), (album, 63)] {
        let b = s.as_bytes();
        t[at..at + b.len().min(30)].copy_from_slice(&b[..b.len().min(30)]);
    }
    t[93..97].copy_from_slice(b"2001");
    t[127] = 52; // Electronic
    t
}

/// A song of `copies` x 2 s (the MPEG-2 22 kHz tone, 32 kbps), tagged.
pub fn long_song(copies: usize, title: &str, artist: &str, album: &str) -> Vec<u8> {
    let audio = &MPEG2[..MPEG2.len() - 128];
    let mut v = Vec::new();
    for _ in 0..copies {
        v.extend_from_slice(audio);
    }
    v.extend(v1(title, artist, album));
    v
}

/// One musicd with its own realm.
pub struct FakeServer {
    pub host: String,
    pub port: u16,
    pub app: Backend<MemDisk>,
    pub realm: Rc<RefCell<Realm>>,
    pub announced: bool,
}

impl FakeServer {
    /// A backend named `name` at `host`, key from `seed`, with `songs` (path, bytes).
    pub fn new(host: &str, name: &str, seed: [u8; 32], songs: &[(&str, Vec<u8>)]) -> FakeServer {
        let mut disk = MemDisk::default();
        for (i, (p, b)) in songs.iter().enumerate() {
            disk.put(&format!("/srv/music/{p}"), b, 100 + i as u64);
        }
        let own = public_key(&signing_key(&seed));
        let fp = ecm_auth::fingerprint(&own);
        let cfg = Config {
            name: name.into(),
            port: 7800,
            library_dir: "/srv/music".into(),
            data_dir: "/var/music".into(),
            transcode: true,
            public_browse: true,
            max_kbps: 192,
            realm_name: name.into(),
            realm_fp: fp.clone(),
            auth_endpoint: "-:7801".into(),
        };
        let mut app = Backend::new(cfg, disk, Gatekeeper::new(own, &fp, Some(own)));
        app.scan_now();
        let realm = Realm::new(&seed, name).unwrap().with_kdf(ecm_auth::MIN_KDF_ITERATIONS);
        FakeServer { host: host.into(), port: 7800, app, realm: Rc::new(RefCell::new(realm)), announced: true }
    }

    /// Make every Data saver copy now.
    pub fn make_saver(&mut self) {
        while self.app.busy() {
            self.app.step();
        }
    }

    pub fn song_id(&self, title: &str) -> String {
        self.app.lib.songs().iter().find(|s| s.title == title).unwrap_or_else(|| panic!("no song {title}")).id.clone()
    }
}

/// A connection to a realm, driving its `ServerConn` directly.
struct RealmPipe {
    realm: Rc<RefCell<Realm>>,
    conn: ServerConn,
    inbox: VecDeque<u8>,
    now: Rc<Cell<u64>>,
}

impl Stream for RealmPipe {
    fn write_all(&mut self, data: &[u8]) -> Result<(), AuthError> {
        let out = self.conn.receive(&mut self.realm.borrow_mut(), data, self.now.get());
        self.inbox.extend(out.send);
        Ok(())
    }
    fn read_exact(&mut self, buf: &mut [u8], _timeout_ms: u32) -> Result<(), AuthError> {
        if self.inbox.len() < buf.len() {
            return Err(AuthError::Timeout);
        }
        for b in buf.iter_mut() {
            *b = self.inbox.pop_front().unwrap();
        }
        Ok(())
    }
}

struct FakeConn {
    server: usize,
    conn: Conn,
    closed: bool,
}

/// The network.
pub struct FakeNet {
    pub now: Rc<Cell<u64>>,
    pub servers: Vec<FakeServer>,
    conns: BTreeMap<i32, FakeConn>,
    next: i32,
    /// The link speed in bytes per second (None: as fast as asked).
    pub rate: Option<u64>,
    allowance: f64,
    last_ms: u64,
    /// Close the next connection that carries this many more bytes (a dropped link).
    pub drop_after: Option<usize>,
    /// Hosts that refuse connections.
    pub down: Vec<String>,
    pub cellular: bool,
    pub searches: usize,
    /// Every request's first lines (method, path and Range), for tests.
    pub requests: Vec<String>,
    pub drops: usize,
}

impl FakeNet {
    pub fn new(servers: Vec<FakeServer>) -> FakeNet {
        FakeNet {
            now: Rc::new(Cell::new(START)),
            servers,
            conns: BTreeMap::new(),
            next: 0,
            rate: None,
            allowance: 0.0,
            last_ms: START,
            drop_after: None,
            down: Vec::new(),
            cellular: false,
            searches: 0,
            requests: Vec::new(),
            drops: 0,
        }
    }

    pub fn advance(&self, ms: u64) {
        self.now.set(self.now.get() + ms);
    }

    /// The `Range: bytes=a-` starts of the stream requests for song `id`.
    pub fn range_starts(&self, id: &str) -> Vec<u64> {
        self.requests
            .iter()
            .filter(|r| r.contains(&format!("/stream/{id}")))
            .filter_map(|r| r.split("Range: bytes=").nth(1)?.split('-').next()?.parse().ok())
            .collect()
    }
}

impl FakeNet {
    /// Every open connection drops (the server process died).
    pub fn drop_all(&mut self) {
        for c in self.conns.values_mut() {
            c.closed = true;
        }
    }
}

impl Net for FakeNet {
    fn connect(&mut self, host: &str, port: u16) -> Result<i32, String> {
        if self.down.iter().any(|h| h == host) {
            return Err(format!("{host}:{port} refused the connection"));
        }
        let server = self.servers.iter().position(|s| s.host == host && s.port == port).ok_or_else(|| format!("no route to {host}"))?;
        self.next += 1;
        let h = 100 + self.next;
        self.conns.insert(h, FakeConn { server, conn: Conn::new("10.0.0.50:40000", self.now.get()), closed: false });
        Ok(h)
    }

    fn send(&mut self, h: i32, data: &[u8]) -> Result<(), String> {
        let now = self.now.get();
        let c = self.conns.get_mut(&h).ok_or("closed")?;
        if c.closed {
            return Err("closed".into());
        }
        let text = String::from_utf8_lossy(data);
        let first: Vec<&str> = text.lines().filter(|l| !l.starts_with("Authorization") && !l.starts_with("X-ECM")).take(4).collect();
        self.requests.push(first.join(" | "));
        c.conn.receive(&mut self.servers[c.server].app, data, now);
        Ok(())
    }

    fn recv(&mut self, h: i32) -> Result<Vec<u8>, String> {
        let now = self.now.get();
        let c = self.conns.get_mut(&h).ok_or("closed")?;
        if c.closed {
            return Err("closed".into());
        }
        let budget = match self.rate {
            Some(r) => {
                self.allowance = (self.allowance + now.saturating_sub(self.last_ms) as f64 * r as f64 / 1000.0).min(r as f64);
                self.last_ms = now;
                self.allowance as usize
            }
            None => usize::MAX,
        };
        let app = &mut self.servers[c.server].app;
        let mut out = Vec::new();
        while out.len() < budget {
            let p = c.conn.pending(&app.disk);
            if p.is_empty() {
                break;
            }
            let n = p.len().min(budget - out.len());
            out.extend_from_slice(&p[..n]);
            c.conn.sent(n);
            c.conn.process(app, now);
        }
        if self.rate.is_some() {
            self.allowance -= out.len() as f64;
        }
        if let Some(left) = self.drop_after {
            if out.len() >= left {
                out.truncate(left);
                self.drop_after = None;
                c.closed = true;
                self.drops += 1;
                return Ok(out);
            }
            self.drop_after = Some(left - out.len());
        }
        if out.is_empty() && c.conn.finished() {
            return Err("closed".into());
        }
        Ok(out)
    }

    fn close(&mut self, h: i32) {
        self.conns.remove(&h);
    }

    fn now_ms(&mut self) -> u64 {
        self.now.get()
    }

    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        self.searches += 1;
        self.servers
            .iter()
            .filter(|s| s.announced)
            .map(|s| ecm_disco::Found { kind: "music".into(), host: ecm_disco::parse_ip(&s.host).unwrap_or([0; 4]), port: s.port, name: s.app.cfg.name.clone() })
            .collect()
    }

    fn realm(&mut self, host: &str) -> Result<Box<dyn Stream>, AuthError> {
        let s = self
            .servers
            .iter()
            .find(|s| format!("{}:{}", s.host, s.port + 1) == host)
            .ok_or_else(|| AuthError::Network(format!("can't connect to {host}")))?;
        Ok(Box::new(RealmPipe { realm: s.realm.clone(), conn: ServerConn::new("10.0.0.50:41000"), inbox: VecDeque::new(), now: self.now.clone() }))
    }

    fn cellular(&mut self) -> bool {
        self.cellular
    }
}

/// A speaker that plays at its rate by the fake clock.
pub struct FakeSpeaker {
    pub now: Rc<Cell<u64>>,
    rate: u32,
    queued: f64,
    last: u64,
    pub opened: bool,
    pub played: Vec<i16>,
    pub flushes: u32,
}

impl FakeSpeaker {
    pub fn new(now: Rc<Cell<u64>>) -> FakeSpeaker {
        let last = now.get();
        FakeSpeaker { now, rate: 22050, queued: 0.0, last, opened: false, played: Vec::new(), flushes: 0 }
    }

    fn advance(&mut self) {
        let t = self.now.get();
        let dt = t.saturating_sub(self.last) as f64;
        self.last = t;
        self.queued = (self.queued - dt * self.rate as f64 / 1000.0).max(0.0);
    }
}

impl Speaker for FakeSpeaker {
    fn open(&mut self, rate: u32) -> Result<(), String> {
        self.rate = rate;
        self.opened = true;
        self.advance();
        Ok(())
    }
    fn write(&mut self, pcm: &[i16]) {
        self.advance();
        self.queued += pcm.len() as f64;
        self.played.extend_from_slice(pcm);
    }
    fn buffered_ms(&mut self) -> i64 {
        self.advance();
        (self.queued * 1000.0 / self.rate as f64) as i64
    }
    fn flush(&mut self) {
        self.advance();
        // What was queued is not heard.
        let q = self.queued as usize;
        let keep = self.played.len().saturating_sub(q);
        self.played.truncate(keep);
        self.queued = 0.0;
        self.flushes += 1;
    }
    fn close(&mut self) {
        self.opened = false;
    }
}
