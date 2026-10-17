//! A scripted network of Discord backends and a scripted sound card, for tests and
//! the golden-session recorder. Each fake backend is the real backend state machine
//! (`ecm_discord::server::Backend`), so the app talks to the real protocol.

use super::{Audio, Net};
use ecm_discord::server::{Backend, Out};
use std::collections::{BTreeMap, VecDeque};

/// One backend on the fake network.
pub struct FakeBackend {
    pub host: String,
    pub port: u16,
    pub backend: Backend,
    /// Lines to clients that haven't been read yet, per connection handle.
    pub pending: BTreeMap<i32, Vec<u8>>,
    /// Found by a network search.
    pub announced: bool,
    /// Datagrams the backend sent to the app's voice socket.
    pub udp_to_app: VecDeque<Vec<u8>>,
}

/// A network with any number of backends.
#[derive(Default)]
pub struct FakeNet {
    pub backends: Vec<FakeBackend>,
    /// Handle -> backend index.
    pub conns: BTreeMap<i32, usize>,
    next: i32,
    pub now: u64,
    /// Searches made.
    pub searches: usize,
    pub refuse: Vec<String>,
    /// The voice socket: (backend index) it talks to.
    pub voice_to: Option<usize>,
    pub voice_sent: Vec<Vec<u8>>,
    /// Lines every handle sent (tests).
    pub sent: Vec<(i32, String)>,
}

impl FakeNet {
    /// Add a backend called `name` at `host` (port 7777); `announced` = found by a search.
    pub fn add(&mut self, host: &str, name: &str, announced: bool) -> usize {
        self.backends.push(FakeBackend {
            host: host.into(),
            port: 7777,
            backend: Backend::new(name, 7778, 7),
            pending: BTreeMap::new(),
            announced,
            udp_to_app: VecDeque::new(),
        });
        self.backends.len() - 1
    }

    fn perform(&mut self, bi: usize, outs: Vec<Out>) {
        for o in outs {
            match o {
                Out::Send(id, line) => {
                    let b = &mut self.backends[bi];
                    b.pending.entry(id as i32).or_default().extend_from_slice(format!("{line}\n").as_bytes());
                }
                Out::Udp(_, _, data) => self.backends[bi].udp_to_app.push_back(data),
                Out::Close(_) | Out::Log(_) | Out::Save => {}
            }
        }
    }

    /// Another user on backend `bi` (a second app, the chat command): their lines in, what they got out.
    pub fn user(&mut self, bi: usize, handle: i32, lines: &str) -> String {
        if !self.conns.contains_key(&handle) {
            self.conns.insert(handle, bi);
            let outs = self.backends[bi].backend.connect(handle as u32, "10.0.0.99:4000");
            self.perform(bi, outs);
        }
        let now = self.now;
        let outs = self.backends[bi].backend.receive(handle as u32, lines.as_bytes(), now);
        self.perform(bi, outs);
        String::from_utf8(self.backends[bi].pending.remove(&handle).unwrap_or_default()).unwrap()
    }

    /// Another user's voice datagram to backend `bi` (from 10.0.0.99:6000).
    pub fn user_udp(&mut self, bi: usize, data: &[u8]) -> Vec<Vec<u8>> {
        let now = self.now;
        let outs = self.backends[bi].backend.udp([10, 0, 0, 99], 6000, data, now);
        let mut to_user = Vec::new();
        for o in outs {
            match o {
                Out::Udp([10, 0, 0, 99], 6000, d) => to_user.push(d),
                o => self.perform(bi, vec![o]),
            }
        }
        to_user
    }
}

impl Net for FakeNet {
    fn connect(&mut self, host: &str, port: u16) -> Result<i32, String> {
        if self.refuse.iter().any(|h| h == host) {
            return Err(format!("{host}:{port} refused the connection (is discordd running there?)"));
        }
        let Some(bi) = self.backends.iter().position(|b| b.host == host && b.port == port) else {
            return Err(format!("no route to {host}"));
        };
        self.next += 1;
        let h = 100 + self.next;
        self.conns.insert(h, bi);
        let outs = self.backends[bi].backend.connect(h as u32, "10.0.0.50:40000");
        self.perform(bi, outs);
        Ok(h)
    }
    fn send(&mut self, h: i32, line: &str) -> Result<(), String> {
        let Some(&bi) = self.conns.get(&h) else { return Err(String::new()) };
        self.sent.push((h, line.to_string()));
        let now = self.now;
        let outs = self.backends[bi].backend.receive(h as u32, format!("{line}\n").as_bytes(), now);
        self.perform(bi, outs);
        Ok(())
    }
    fn recv(&mut self, h: i32) -> Result<Vec<u8>, String> {
        let Some(&bi) = self.conns.get(&h) else { return Err(String::new()) };
        Ok(self.backends[bi].pending.remove(&h).unwrap_or_default())
    }
    fn close(&mut self, h: i32) {
        if let Some(bi) = self.conns.remove(&h) {
            let now = self.now;
            let outs = self.backends[bi].backend.disconnect(h as u32, now);
            self.perform(bi, outs);
        }
    }
    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        self.searches += 1;
        self.backends
            .iter()
            .filter(|b| b.announced)
            .map(|b| ecm_disco::Found {
                kind: "discord".into(),
                host: ecm_disco::parse_ip(&b.host).unwrap_or([0, 0, 0, 0]),
                port: b.port,
                name: b.backend.name.clone(),
            })
            .collect()
    }
    fn default_nick(&mut self) -> String {
        "pc-0a1b".into()
    }
    fn now_ms(&mut self) -> u64 {
        self.now
    }
    fn voice_open(&mut self, host: &str, _port: u16) -> Result<(), String> {
        self.voice_to = self.backends.iter().position(|b| b.host == host);
        self.voice_to.map(|_| ()).ok_or_else(|| "no route".into())
    }
    fn voice_send(&mut self, data: &[u8]) {
        self.voice_sent.push(data.to_vec());
        if let Some(bi) = self.voice_to {
            let now = self.now;
            let outs = self.backends[bi].backend.udp([10, 0, 0, 50], 50000, data, now);
            // Forwarded to the other user (10.0.0.99) is dropped here; tests read voice_sent.
            for o in outs {
                if !matches!(o, Out::Udp(..)) {
                    self.perform(bi, vec![o]);
                }
            }
        }
    }
    fn voice_recv(&mut self) -> Vec<Vec<u8>> {
        match self.voice_to {
            Some(bi) => self.backends[bi].udp_to_app.drain(..).collect(),
            None => Vec::new(),
        }
    }
    fn voice_close(&mut self) {
        self.voice_to = None;
    }
}

/// A sound card in a test: what the microphone will capture, what was played.
#[derive(Default)]
pub struct FakeAudio {
    pub open: bool,
    pub mic: VecDeque<i16>,
    pub played: Vec<i16>,
    pub queued: i64,
    pub headphones: bool,
    pub fail: Option<String>,
}

impl Audio for FakeAudio {
    fn open(&mut self) -> Result<(), String> {
        if let Some(e) = &self.fail {
            return Err(e.clone());
        }
        self.open = true;
        Ok(())
    }
    fn capture(&mut self) -> Vec<i16> {
        self.mic.drain(..).collect()
    }
    fn play(&mut self, pcm: &[i16]) {
        self.played.extend_from_slice(pcm);
    }
    fn queued_ms(&mut self) -> i64 {
        self.queued
    }
    fn headphones(&mut self) -> bool {
        self.headphones
    }
    fn close(&mut self) {
        self.open = false;
    }
}
