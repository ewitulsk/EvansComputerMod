//! A scripted telephonyd and in-memory files, for the apps' tests and golden recorders.

use std::collections::{BTreeMap, VecDeque};

use crate::{Event, Files, Tel};
use ecm_tel::ctl::{CallLine, LogLine, MsgLine, ThreadLine};

/// Files in memory.
#[derive(Default, Clone)]
pub struct MemFiles {
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Files for MemFiles {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String> {
        self.files.insert(path.into(), data.to_vec());
        Ok(())
    }
    fn list(&mut self, dir: &str) -> Vec<String> {
        let p = format!("{}/", dir.trim_end_matches('/'));
        self.files.keys().filter_map(|k| k.strip_prefix(&p)).filter(|r| !r.contains('/') && !r.is_empty()).map(String::from).collect()
    }
}

/// A telephonyd that answers from its own small state (calls, threads, the log),
/// records every request, and hands out queued events.
#[derive(Default)]
pub struct FakeTel {
    pub msisdn: String,
    pub registered: bool,
    pub calls: Vec<CallLine>,
    pub threads: Vec<ThreadLine>,
    pub messages: BTreeMap<String, Vec<MsgLine>>,
    pub log: Vec<LogLine>,
    pub voicemail: Vec<String>,
    pub requests: Vec<(u16, String)>,
    pub events: VecDeque<Event>,
    pub next_id: u64,
    /// No telephonyd at all.
    pub down: bool,
}

impl FakeTel {
    pub fn new(msisdn: &str) -> FakeTel {
        FakeTel { msisdn: msisdn.into(), registered: true, next_id: 1, ..Default::default() }
    }

    pub fn push_event(&mut self, name: &str, fields: &[(&str, &str)]) {
        self.events.push_back(Event::new(name, fields));
    }

    /// Requests sent so far that start with `prefix`.
    pub fn sent(&self, prefix: &str) -> Vec<String> {
        self.requests.iter().filter(|(_, r)| r.starts_with(prefix)).map(|(_, r)| r.clone()).collect()
    }
}

impl Tel for FakeTel {
    fn call(&mut self, port: u16, req: &str) -> Result<Result<Vec<String>, String>, String> {
        if self.down {
            return Err("telephonyd is not running".into());
        }
        self.requests.push((port, req.to_string()));
        let w: Vec<&str> = req.split_whitespace().collect();
        let ok = |v: Vec<String>| Ok(Ok(v));
        match w.as_slice() {
            ["status"] => ok(vec![
                format!("registration {}", if self.registered { "registered" } else { "off" }),
                format!("msisdn {}", if self.msisdn.is_empty() { "-" } else { &self.msisdn }),
                "domain cell-red.ecm".into(),
                format!("volte {}", self.registered as u8),
                format!("hd {}", self.calls.iter().any(|c| c.hd) as u8),
                format!("voicemail {}", self.voicemail.iter().filter(|v| v.ends_with(" new")).count()),
                format!("unread {}", self.threads.iter().map(|t| t.unread).sum::<u32>()),
                format!("missed {}", self.log.iter().filter(|l| l.dir == "missed").count()),
                format!("calls {}", self.calls.len()),
            ]),
            ["calls"] => ok(self.calls.iter().map(|c| c.line()).collect()),
            ["threads"] => ok(self.threads.iter().map(|t| t.line()).collect()),
            ["messages", k, ..] => ok(self.messages.get(*k).map(|v| v.iter().map(|m| m.line()).collect()).unwrap_or_default()),
            ["log", ..] => ok(self.log.iter().map(|l| l.line()).collect()),
            ["voicemail"] => ok(self.voicemail.clone()),
            ["dial", n, ..] => {
                let id = self.next_id as u32;
                self.next_id += 1;
                let number = ecm_tel::ctl::dec(n);
                self.calls.push(CallLine { id, state: "dialing".into(), number, outgoing: true, hd: true, codec: "ADPCM-WB".into(), ..Default::default() });
                ok(vec![format!("call {id}")])
            }
            ["answer", i] => {
                if let Some(c) = self.calls.iter_mut().find(|c| c.id.to_string() == *i) {
                    c.state = "active".into();
                }
                ok(vec![])
            }
            ["hangup", i] | ["decline", i] => {
                self.calls.retain(|c| c.id.to_string() != *i);
                ok(vec![])
            }
            ["sms", to, text] => {
                let id = self.next_id;
                self.next_id += 1;
                let to = ecm_tel::ctl::dec(to);
                self.messages.entry(to.clone()).or_default().push(MsgLine { id, incoming: false, from: self.msisdn.clone(), time_ms: 0, status: "sending".into(), text: ecm_tel::ctl::dec(text), ..Default::default() });
                if !self.threads.iter().any(|t| t.key == to) {
                    self.threads.insert(0, ThreadLine { key: to, unread: 0, time_ms: 0, preview: ecm_tel::ctl::dec(text) });
                }
                ok(vec![format!("sms {id}")])
            }
            ["mms", to, ..] => {
                let id = self.next_id;
                self.next_id += 1;
                let _ = to;
                ok(vec![format!("mms {id}")])
            }
            _ => ok(vec![]),
        }
    }

    fn events(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
}
