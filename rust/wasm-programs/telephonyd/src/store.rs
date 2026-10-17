//! The phone's texts and call history: `/var/telephony/messages.log` and
//! `/var/telephony/calls.log`, append-only lines replayed at start (a status
//! change is a new line, so a crash loses nothing). MMS parts are files under
//! `/home/Messages/<id>/`.
//!
//! ```text
//! msg <id> <thread> <in|out> <from> <time> <status> <sms|mms> <text> <files>    (fields as ecm_tel::ctl)
//! st <id> <status>                     sending -> sent -> delivered, failed:<why>, received -> read
//! net <id> <network id>                the SMS id or MMS message id the network gave it
//! del <id> | delthread <thread>
//! call <id> <in|out|missed> <number> <start> <seconds> <reason>
//! ```

use std::collections::BTreeMap;

use ecm_tel::ctl::{dec, enc, LogLine, MsgLine, ThreadLine};

pub const DIR: &str = "/var/telephony";
pub const MESSAGES: &str = "/var/telephony/messages.log";
pub const CALLS: &str = "/var/telephony/calls.log";
pub const PARTS: &str = "/home/Messages";

#[derive(Default)]
pub struct Store {
    pub msgs: BTreeMap<u64, (String, MsgLine)>,
    /// Network id (SMS id as `sms:<n>`, MMS message id as `mms:<id>`) -> message.
    pub net: BTreeMap<String, u64>,
    pub calls: Vec<LogLine>,
    next: u64,
    /// Lines to append (the program writes them).
    pub pending_msgs: Vec<String>,
    pub pending_calls: Vec<String>,
}

/// The thread key: the other numbers, sorted, `,`-joined.
pub fn key(me: &str, others: &[String]) -> String {
    let mut v: Vec<String> = others.iter().filter(|n| *n != me && !n.is_empty()).cloned().collect();
    v.sort();
    v.dedup();
    v.join(",")
}

impl Store {
    pub fn load(msgs: &str, calls: &str) -> Store {
        let mut s = Store { next: 1, ..Default::default() };
        for l in msgs.lines() {
            let w: Vec<&str> = l.split_whitespace().collect();
            match w.as_slice() {
                ["msg", id, thread, rest @ ..] if rest.len() == 7 => {
                    let line = format!("msg {} {}", id, rest.join(" "));
                    if let (Ok(id), Some(m)) = (id.parse::<u64>(), MsgLine::parse(&line)) {
                        s.next = s.next.max(id + 1);
                        s.msgs.insert(id, (thread.to_string(), m));
                    }
                }
                ["st", id, st] => {
                    if let Some(m) = id.parse().ok().and_then(|i: u64| s.msgs.get_mut(&i)) {
                        m.1.status = dec(st);
                    }
                }
                ["net", id, nid] => {
                    if let Ok(i) = id.parse() {
                        s.net.insert(nid.to_string(), i);
                    }
                }
                ["del", id] => {
                    if let Ok(i) = id.parse::<u64>() {
                        s.msgs.remove(&i);
                    }
                }
                ["delthread", t] => s.msgs.retain(|_, (k, _)| k != t),
                _ => {}
            }
        }
        for l in calls.lines() {
            if let Some(c) = LogLine::parse(l) {
                s.next = s.next.max(c.id + 1);
                s.calls.push(c);
            }
        }
        s
    }

    fn id(&mut self) -> u64 {
        let i = self.next;
        self.next += 1;
        i
    }

    pub fn add(&mut self, thread: &str, mut m: MsgLine) -> u64 {
        m.id = self.id();
        let line = m.line();
        let rest = line.splitn(3, ' ').nth(2).unwrap_or("").to_string();
        self.pending_msgs.push(format!("msg {} {} {}", m.id, thread, rest));
        let id = m.id;
        self.msgs.insert(id, (thread.into(), m));
        id
    }

    pub fn set_status(&mut self, id: u64, status: &str) {
        if let Some(m) = self.msgs.get_mut(&id) {
            if m.1.status != status {
                m.1.status = status.into();
                self.pending_msgs.push(format!("st {} {}", id, enc(status)));
            }
        }
    }

    pub fn link(&mut self, id: u64, network: &str) {
        self.net.insert(network.into(), id);
        self.pending_msgs.push(format!("net {} {}", id, network));
    }

    pub fn by_network(&self, network: &str) -> Option<u64> {
        self.net.get(network).copied()
    }

    pub fn delete(&mut self, id: u64) -> bool {
        if self.msgs.remove(&id).is_some() {
            self.pending_msgs.push(format!("del {}", id));
            true
        } else {
            false
        }
    }

    pub fn delete_thread(&mut self, t: &str) {
        self.msgs.retain(|_, (k, _)| k != t);
        self.pending_msgs.push(format!("delthread {}", t));
    }

    /// Incoming messages of a thread become read.
    pub fn mark_read(&mut self, t: &str) -> usize {
        let ids: Vec<u64> = self.msgs.iter().filter(|(_, (k, m))| k == t && m.incoming && m.status == "received").map(|(i, _)| *i).collect();
        for i in &ids {
            self.set_status(*i, "read");
        }
        ids.len()
    }

    pub fn threads(&self) -> Vec<ThreadLine> {
        let mut by: BTreeMap<&str, ThreadLine> = BTreeMap::new();
        for (k, m) in self.msgs.values() {
            let t = by.entry(k.as_str()).or_insert_with(|| ThreadLine { key: k.clone(), ..Default::default() });
            if m.incoming && m.status == "received" {
                t.unread += 1;
            }
            if m.time_ms >= t.time_ms {
                t.time_ms = m.time_ms;
                t.preview = if m.text.is_empty() && m.mms { "Picture message".into() } else { m.text.clone() };
            }
        }
        let mut v: Vec<ThreadLine> = by.into_values().collect();
        v.sort_by(|a, b| b.time_ms.cmp(&a.time_ms));
        v
    }

    pub fn messages(&self, t: &str, n: usize) -> Vec<&MsgLine> {
        let v: Vec<&MsgLine> = self.msgs.values().filter(|(k, _)| k == t).map(|(_, m)| m).collect();
        let skip = v.len().saturating_sub(n);
        v.into_iter().skip(skip).collect()
    }

    pub fn unread(&self) -> usize {
        self.msgs.values().filter(|(_, m)| m.incoming && m.status == "received").count()
    }

    pub fn log_call(&mut self, dir: &str, number: &str, start_ms: u64, seconds: u64, reason: &str) {
        let c = LogLine { id: self.id(), dir: dir.into(), number: number.into(), start_ms, seconds, reason: reason.into() };
        self.pending_calls.push(c.line());
        self.calls.push(c);
    }

    pub fn missed(&self) -> usize {
        self.calls.iter().filter(|c| c.dir == "missed").count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_what_it_wrote() {
        let mut s = Store::load("", "");
        let t = key("+12145550142", &["+12145550143".into()]);
        let a = s.add(&t, MsgLine { incoming: false, from: "+12145550142".into(), time_ms: 10, status: "sending".into(), text: "hi bo".into(), ..Default::default() });
        s.set_status(a, "sent");
        s.link(a, "sms:4");
        let b = s.add(&t, MsgLine { incoming: true, from: "+12145550143".into(), time_ms: 20, status: "received".into(), text: "hey!".into(), ..Default::default() });
        s.log_call("missed", "+12145550144", 30, 0, "no answer");
        assert_eq!(s.unread(), 1);
        assert_eq!(s.threads()[0].preview, "hey!");
        let msgs = s.pending_msgs.join("\n");
        let calls = s.pending_calls.join("\n");
        let mut r = Store::load(&msgs, &calls);
        assert_eq!(r.msgs.len(), 2);
        assert_eq!(r.msgs[&a].1.status, "sent");
        assert_eq!(r.by_network("sms:4"), Some(a));
        assert_eq!(r.mark_read(&t), 1);
        assert_eq!(r.unread(), 0);
        assert_eq!(r.missed(), 1);
        assert!(r.delete(b));
        r.delete_thread(&t);
        assert!(r.threads().is_empty());
        assert_eq!(key("+1", &["+3".into(), "+2".into(), "+1".into()]), "+2,+3");
    }
}
