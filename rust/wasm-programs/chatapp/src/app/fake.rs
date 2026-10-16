//! A scripted chat server for tests and the golden-session recorder.

use super::Net;
use std::collections::VecDeque;

/// A chat server in a test: lines the app sends, bytes it will receive.
#[derive(Default)]
pub struct FakeNet {
    pub connected: Option<(String, u16)>,
    pub sent: Vec<String>,
    pub inbox: VecDeque<Vec<u8>>,
    pub refuse: Option<String>,
    pub closed: bool,
    /// What a network search finds, and how many searches there were.
    pub found: Vec<ecm_disco::Found>,
    pub searches: usize,
}

impl FakeNet {
    pub fn server_says(&mut self, lines: &[&str]) {
        let mut b = Vec::new();
        for l in lines {
            b.extend_from_slice(l.as_bytes());
            b.push(b'\n');
        }
        self.inbox.push_back(b);
    }
}

impl Net for FakeNet {
    fn connect(&mut self, host: &str, port: u16) -> Result<(), String> {
        if let Some(e) = &self.refuse {
            return Err(e.clone());
        }
        self.connected = Some((host.into(), port));
        self.closed = false;
        Ok(())
    }
    fn send(&mut self, line: &str) -> Result<(), String> {
        if self.closed {
            return Err(String::new());
        }
        self.sent.push(line.into());
        Ok(())
    }
    fn recv(&mut self) -> Result<Vec<u8>, String> {
        if self.closed {
            return Err(String::new());
        }
        Ok(self.inbox.pop_front().unwrap_or_default())
    }
    fn close(&mut self) {
        self.closed = true;
    }
    fn default_nick(&mut self) -> String {
        "pc-0a1b".into()
    }
    fn discover(&mut self) -> Vec<ecm_disco::Found> {
        self.searches += 1;
        self.found.clone()
    }
}

