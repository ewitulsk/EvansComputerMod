//! Admin sockets for tests and the golden-image recorder.
//!
//! * [`Scripted`]: canned replies (a running network as the recorder shows it).
//! * [`CoreAdmin`]: the real `cored` daemon logic (`cored::daemon::Daemon`) over an
//!   in-memory database, with a [`FakeReader`] for the Card Reader/Writer.
//! * [`SiteAdmin`]: the real `celld-site` daemon logic with an in-memory config file.

use crate::Admin;
use cored::daemon::{Card, CardWriter, Daemon as CoreDaemon};
use ecm_cell::core::{CoreOut, Entropy};
use ecm_cell::db::Db;

/// Canned replies: the first entry whose command is a prefix of the request answers.
#[derive(Default)]
pub struct Scripted {
    pub replies: Vec<(String, Result<Vec<String>, String>)>,
    pub sent: Vec<String>,
    /// Not running: every call fails like a refused connection.
    pub down: bool,
}

impl Scripted {
    pub fn on(&mut self, cmd: &str, body: &str) -> &mut Self {
        self.replies.retain(|(c, _)| c != cmd);
        self.replies.insert(0, (cmd.into(), Ok(body.lines().map(String::from).collect())));
        self
    }
    pub fn fail(&mut self, cmd: &str, err: &str) -> &mut Self {
        self.replies.retain(|(c, _)| c != cmd);
        self.replies.insert(0, (cmd.into(), Err(err.into())));
        self
    }
}

impl Admin for Scripted {
    fn call(&mut self, cmd: &str) -> Result<Result<Vec<String>, String>, String> {
        if self.down {
            return Err("not running".into());
        }
        self.sent.push(cmd.into());
        Ok(self
            .replies
            .iter()
            .find(|(c, _)| cmd == c || cmd.starts_with(&format!("{c} ")))
            .map(|(_, r)| r.clone())
            .unwrap_or_else(|| Err(format!("unknown command '{cmd}'"))))
    }
}

/// Deterministic randomness.
pub struct Counter(pub u8);
impl Entropy for Counter {
    fn fill(&mut self, b: &mut [u8]) {
        for x in b {
            self.0 = self.0.wrapping_add(41);
            *x = self.0;
        }
    }
}

/// A Card Reader/Writer: the card in it, and what was written.
#[derive(Default)]
pub struct FakeReader {
    pub card: Option<Card>,
    pub written: Vec<(String, String, String)>,
}

impl CardWriter for FakeReader {
    fn card(&mut self) -> Result<Card, String> {
        Ok(self.card.clone().unwrap_or(Card::Empty))
    }
    fn write(&mut self, imsi: &str, carrier: &str, key: &str, _msisdn: &str) -> Result<bool, String> {
        if self.card != Some(Card::Blank) {
            return Err("the card is not blank".into());
        }
        self.written.push((imsi.into(), carrier.into(), key.into()));
        self.card = Some(Card::Sim { imsi: imsi.into(), carrier: carrier.into() });
        Ok(false)
    }
}

/// The real `cored` command handling over an in-memory database.
pub struct CoreAdmin {
    pub d: CoreDaemon,
    pub reader: FakeReader,
    /// The database file as `cored` would have written it last.
    pub db_text: String,
    pub now_ms: u64,
}

impl CoreAdmin {
    pub fn new(db_text: &str) -> CoreAdmin {
        let db = Db::parse(db_text).expect("db");
        CoreAdmin {
            d: CoreDaemon::new(db, cored::config::Conf::default(), Box::new(Counter(3)), Box::new(Counter(77)), 0),
            reader: FakeReader::default(),
            db_text: db_text.into(),
            now_ms: 0,
        }
    }
}

impl Admin for CoreAdmin {
    fn call(&mut self, cmd: &str) -> Result<Result<Vec<String>, String>, String> {
        let (r, _) = self.d.handle(cmd, self.now_ms, &mut self.reader);
        for o in self.d.take_out(self.now_ms) {
            if let CoreOut::SaveDb(t) = o {
                self.db_text = t;
            }
        }
        Ok(r.map(|b| b.lines().map(String::from).collect()))
    }
}

/// The real `celld-site` command handling with an in-memory config file.
pub struct SiteAdmin {
    pub d: celld_site::daemon::Daemon,
    pub file: String,
    pub now_ms: u64,
}

impl SiteAdmin {
    pub fn new(conf_text: &str) -> SiteAdmin {
        let conf = celld_site::config::Conf::parse(conf_text).expect("config");
        SiteAdmin { d: celld_site::daemon::Daemon::new(conf, Box::new(Counter(5)), 0).expect("site"), file: conf_text.into(), now_ms: 0 }
    }
}

impl Admin for SiteAdmin {
    fn call(&mut self, cmd: &str) -> Result<Result<Vec<String>, String>, String> {
        let mut file = String::new();
        let (r, _) = self.d.handle(cmd, self.now_ms, &mut |t| {
            file = t.to_string();
            Ok(())
        });
        if !file.is_empty() {
            self.file = file;
        }
        Ok(r.map(|b| b.lines().map(String::from).collect()))
    }
}

/// The settings file of the sample site.
pub const SITE_CONF: &str = "site_id = site-a
secret = 5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e
carrier = cell-red
core = 10.0.1.1
cell_id = 1
channel = 3
";

/// A site with two phones and a neighbour (the golden images).
pub fn busy_site() -> Scripted {
    let mut s = Scripted::default();
    s.on(
        "status",
        "site site-a\ncarrier cell-red\ncell 1\nprofile lte\nchannel 3\npower auto (43)\naddress a0000001\nradio first SDR\ncore 10.0.1.1:7700\nlink up\nuptime_s 754\nphones 2\nul_bytes 48213\ndl_bytes 391044\nbackhaul_rejects 0\nul_mask 0110\nradio_tx_frames 8812\nradio_rx_frames 2310\nradio_beacons 7540\n\
ue b0000011 001010000000417 connected 100.65.2.2 -71.5 11 2 48213\n\
ue b0000012 001010000000932 connected 100.65.2.3 -88.0 6 3 1022\n\
nb 2 lte 3 0001",
    );
    s.on("config", SITE_CONF);
    s
}


/// A core with three subscribers, two sites (one up), two phones and a handover (the golden images).
pub fn busy_core() -> Scripted {
    let mut s = Scripted::default();
    s.on(
        "status",
        "carrier cell-red\npool 100.65.2.0/24\ngateway 100.65.2.1\ndb /etc/cored.db\nlisten 7700\ntunnel_port 7701\ntun cell0\nuptime_s 1834\nsubscribers 3\nsites_configured 2\nsites_connected 1\nsessions 2\ntunnel_rejects 0\n\
site site-a 10.0.1.21 1 1830 412 0\n\
session 291 001010000000417 100.65.2.2 active site-a 1 48213 0\n\
session 292 001010000000932 100.65.2.3 active site-a 1 1022 5000000",
    );
    s.on(
        "subs",
        "sub 001010000000417 100.65.2.2 48213 0 0 alice 1\nsub 001010000000932 100.65.2.3 1022 5000000 0 bob 1\nsub 001010000000588 - 0 0 1 lost-phone 0",
    );
    s.on("sites", "site site-a 1 10.0.1.21 1 412 1830 0\nsite site-b 0 - - - - 0");
    s.on("handovers", "[   912.3] handover 001010000000417 cell 2 -> 1 (ip 100.65.2.2 kept)");
    s
}

