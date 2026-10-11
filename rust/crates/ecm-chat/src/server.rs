//! The chat room (`chatd`'s logic) as a state machine.
//!
//! `chatd` owns the sockets; it calls [`Room::connect`], [`Room::receive`] and
//! [`Room::disconnect`] and performs the returned [`Out`] actions in order. Clients are
//! identified by any number the caller chooses (the socket fd).

use crate::proto::{self, Event, Request};
use std::collections::{BTreeMap, VecDeque};

/// Most clients one room accepts (the kernel allows 32 sockets per program).
pub const MAX_CLIENTS: usize = 28;
/// Messages replayed to a new joiner.
pub const HISTORY: usize = 20;

/// Something `chatd` must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    /// Send this line (a newline is appended) to the client.
    Send(u32, String),
    /// Close the client's socket (after sending what came before).
    Close(u32),
    /// Print to `chatd`'s own console.
    Log(String),
}

#[derive(Debug, Default)]
struct Client {
    nick: Option<String>,
    peer: String,
    buf: Vec<u8>,
}

/// One chat room.
#[derive(Debug)]
pub struct Room {
    name: String,
    clients: BTreeMap<u32, Client>,
    history: VecDeque<(String, String, String)>,
}

impl Room {
    pub fn new(name: &str) -> Room {
        Room { name: name.to_string(), clients: BTreeMap::new(), history: VecDeque::new() }
    }

    pub fn len(&self) -> usize {
        self.clients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.clients.len() >= MAX_CLIENTS
    }

    /// Nicknames of the joined clients, sorted.
    pub fn nicks(&self) -> Vec<String> {
        let mut n: Vec<String> = self.clients.values().filter_map(|c| c.nick.clone()).collect();
        n.sort();
        n
    }

    /// A client connected from `peer` ("a.b.c.d:port"). It joins with its first NICK.
    pub fn connect(&mut self, id: u32, peer: &str) -> Vec<Out> {
        if self.is_full() {
            return vec![
                Out::Send(id, Event::Err(format!("server full ({} users)", MAX_CLIENTS)).line()),
                Out::Close(id),
                Out::Log(format!("refused {}: room full", peer)),
            ];
        }
        self.clients.insert(id, Client { nick: None, peer: peer.to_string(), buf: Vec::new() });
        vec![Out::Log(format!("connection from {}", peer))]
    }

    /// Bytes arrived from a client.
    pub fn receive(&mut self, id: u32, data: &[u8], now: u64) -> Vec<Out> {
        let lines = match self.clients.get_mut(&id) {
            Some(c) => proto::take_lines(&mut c.buf, data),
            None => return Vec::new(),
        };
        let mut out = Vec::new();
        for line in lines {
            if line.trim().is_empty() {
                continue;
            }
            out.extend(self.request(id, &line, now));
            if !self.clients.contains_key(&id) {
                break;
            }
        }
        out
    }

    /// The client's connection ended (EOF or error).
    pub fn disconnect(&mut self, id: u32, now: u64) -> Vec<Out> {
        self.leave(id, "connection closed", now)
    }

    fn leave(&mut self, id: u32, reason: &str, now: u64) -> Vec<Out> {
        let Some(c) = self.clients.remove(&id) else { return Vec::new() };
        let mut out = Vec::new();
        if let Some(nick) = c.nick {
            out.extend(self.broadcast(&Event::Leave { time: proto::hms(now), nick: nick.clone(), reason: reason.into() }));
            out.push(Out::Log(format!("{} left ({})", nick, reason)));
        } else {
            out.push(Out::Log(format!("{} disconnected", c.peer)));
        }
        out
    }

    fn broadcast(&self, e: &Event) -> Vec<Out> {
        let line = e.line();
        self.clients
            .iter()
            .filter(|(_, c)| c.nick.is_some())
            .map(|(id, _)| Out::Send(*id, line.clone()))
            .collect()
    }

    fn taken(&self, nick: &str, except: u32) -> bool {
        self.clients
            .iter()
            .any(|(id, c)| *id != except && c.nick.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(nick)))
    }

    /// A free variant of `wanted`: `wanted`, `wanted2`, `wanted3`, ...
    fn free_nick(&self, wanted: &str, id: u32) -> String {
        if !self.taken(wanted, id) {
            return wanted.to_string();
        }
        for n in 2.. {
            let suffix = n.to_string();
            let base: String = wanted.chars().take(proto::MAX_NICK - suffix.len()).collect();
            let candidate = format!("{}{}", base, suffix);
            if !self.taken(&candidate, id) {
                return candidate;
            }
        }
        unreachable!()
    }

    fn join(&mut self, id: u32, wanted: &str, now: u64) -> Vec<Out> {
        let nick = self.free_nick(wanted, id);
        let peer = match self.clients.get_mut(&id) {
            Some(c) => {
                c.nick = Some(nick.clone());
                c.peer.clone()
            }
            None => return Vec::new(),
        };
        let mut out = vec![Out::Send(id, Event::Welcome { nick: nick.clone(), room: self.name.clone() }.line())];
        for (time, who, text) in &self.history {
            out.push(Out::Send(id, Event::History { time: time.clone(), nick: who.clone(), text: text.clone() }.line()));
        }
        out.extend(self.broadcast(&Event::Join { time: proto::hms(now), nick: nick.clone() }));
        out.push(Out::Log(format!("{} joined from {}", nick, peer)));
        out
    }

    fn request(&mut self, id: u32, line: &str, now: u64) -> Vec<Out> {
        let req = match proto::parse_request(line) {
            Ok(r) => r,
            Err(e) => return vec![Out::Send(id, Event::Err(e).line())],
        };
        let joined = self.clients.get(&id).and_then(|c| c.nick.clone());
        match (req, joined) {
            (Request::Quit, _) => {
                // Close after the goodbye broadcast reached everyone else.
                let mut out = self.leave(id, "quit", now);
                out.push(Out::Close(id));
                out
            }
            (Request::Nick(wanted), None) => self.join(id, &wanted, now),
            (Request::Nick(wanted), Some(old)) => {
                if wanted == old {
                    return Vec::new();
                }
                if self.taken(&wanted, id) {
                    return vec![Out::Send(id, Event::Err(format!("nickname {} is taken", wanted)).line())];
                }
                if let Some(c) = self.clients.get_mut(&id) {
                    c.nick = Some(wanted.clone());
                }
                let mut out = self.broadcast(&Event::Nick { time: proto::hms(now), old: old.clone(), new: wanted.clone() });
                out.push(Out::Log(format!("{} is now {}", old, wanted)));
                out
            }
            (Request::Msg(text), joined) => {
                let mut out = Vec::new();
                let nick = match joined {
                    Some(n) => n,
                    None => {
                        out.extend(self.join(id, &format!("guest{}", id), now));
                        self.clients[&id].nick.clone().unwrap_or_default()
                    }
                };
                let time = proto::hms(now);
                self.history.push_back((time.clone(), nick.clone(), text.clone()));
                while self.history.len() > HISTORY {
                    self.history.pop_front();
                }
                out.extend(self.broadcast(&Event::Msg { time, nick: nick.clone(), text: text.clone() }));
                out.push(Out::Log(format!("<{}> {}", nick, text)));
                out
            }
            (Request::Who, _) => vec![Out::Send(id, Event::Who(self.nicks()).line())],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sent(out: &[Out], to: u32) -> Vec<String> {
        out.iter()
            .filter_map(|o| match o {
                Out::Send(id, l) if *id == to => Some(l.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn join_broadcast_history_and_leave() {
        let mut room = Room::new("Village chat");
        room.connect(5, "100.65.1.20:50000");
        let out = room.receive(5, b"NICK alice\n", 3600);
        assert_eq!(sent(&out, 5), vec!["WELCOME alice Village chat", "JOIN 01:00:00 alice"]);
        let out = room.receive(5, b"MSG first!\n", 3601);
        assert_eq!(sent(&out, 5), vec!["MSG 01:00:01 alice first!"]);

        room.connect(6, "100.66.1.30:50001");
        let out = room.receive(6, b"NICK bob\n", 3602);
        // Bob gets the welcome, the history, then his own join; alice sees the join.
        assert_eq!(
            sent(&out, 6),
            vec!["WELCOME bob Village chat", "HIST 01:00:01 alice first!", "JOIN 01:00:02 bob"]
        );
        assert_eq!(sent(&out, 5), vec!["JOIN 01:00:02 bob"]);

        let out = room.receive(6, b"MSG hi alice\n", 3603);
        assert_eq!(sent(&out, 5), vec!["MSG 01:00:03 bob hi alice"]);
        assert_eq!(sent(&out, 6), vec!["MSG 01:00:03 bob hi alice"]);

        let out = room.receive(5, b"WHO\n", 3604);
        assert_eq!(sent(&out, 5), vec!["WHO alice bob"]);

        let out = room.receive(6, b"QUIT\n", 3605);
        assert_eq!(sent(&out, 5), vec!["LEAVE 01:00:05 bob quit"]);
        assert_eq!(out.last(), Some(&Out::Close(6)), "close after the goodbye");
        assert_eq!(room.nicks(), vec!["alice"]);

        // Dropped connection: the others hear about it; the gone client gets nothing.
        let out = room.disconnect(5, 3606);
        assert!(sent(&out, 5).is_empty());
        assert!(room.is_empty());
    }

    #[test]
    fn duplicate_nicks_and_renames() {
        let mut room = Room::new("r");
        room.connect(1, "a");
        room.connect(2, "b");
        room.receive(1, b"NICK pc\n", 0);
        let out = room.receive(2, b"NICK PC\n", 0);
        assert_eq!(sent(&out, 2)[0], "WELCOME PC2 r", "case-insensitive collision gets a suffix");
        let out = room.receive(2, b"NICK pc\n", 1);
        assert_eq!(sent(&out, 2), vec!["ERR nickname pc is taken"]);
        let out = room.receive(2, b"NICK carol\n", 2);
        assert_eq!(sent(&out, 1), vec!["NICK 00:00:02 PC2 carol"]);
        assert_eq!(room.nicks(), vec!["carol", "pc"]);
        let out = room.receive(2, b"NICK no spaces\n", 3);
        assert!(sent(&out, 2)[0].starts_with("ERR invalid nickname"));
    }

    #[test]
    fn message_before_nick_joins_as_guest_and_partial_lines_wait() {
        let mut room = Room::new("r");
        room.connect(9, "x");
        assert!(room.receive(9, b"MSG hel", 0).is_empty(), "no newline yet");
        let out = room.receive(9, b"lo\n", 0);
        let lines = sent(&out, 9);
        assert_eq!(lines[0], "WELCOME guest9 r");
        assert_eq!(lines.last().unwrap(), "MSG 00:00:00 guest9 hello");
    }

    #[test]
    fn history_is_bounded_and_garbage_is_refused() {
        let mut room = Room::new("r");
        room.connect(1, "a");
        room.receive(1, b"NICK a\n", 0);
        for i in 0..30 {
            room.receive(1, format!("MSG m{}\n", i).as_bytes(), 0);
        }
        room.connect(2, "b");
        let out = room.receive(2, b"NICK b\n", 0);
        let hist: Vec<_> = sent(&out, 2).into_iter().filter(|l| l.starts_with("HIST")).collect();
        assert_eq!(hist.len(), HISTORY);
        assert!(hist[0].ends_with(" a m10"));
        let out = room.receive(2, b"HELLO\n", 0);
        assert_eq!(sent(&out, 2), vec!["ERR unknown command HELLO"]);
        // Unknown ids and empty lines are harmless.
        assert!(room.receive(77, b"MSG x\n", 0).is_empty());
        assert!(room.receive(2, b"\r\n\n", 0).is_empty());
    }

    #[test]
    fn full_room_refuses() {
        let mut room = Room::new("r");
        for id in 0..MAX_CLIENTS as u32 {
            room.connect(id, "p");
        }
        let out = room.connect(100, "late");
        assert_eq!(out[1], Out::Close(100));
        assert!(sent(&out, 100)[0].starts_with("ERR server full"));
        assert_eq!(room.len(), MAX_CLIENTS);
    }
}
