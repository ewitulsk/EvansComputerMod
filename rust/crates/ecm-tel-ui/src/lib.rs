//! The phone's telephony apps on `ecm-ui` (docs/TELEPHONY.md):
//!
//! * [`dialer`]: **Phone** (`dev.ecm.dialer`, program `dialer`; `dev.ecm.phone` is the
//!   phone shell): Favourites, Recents, Contacts, Keypad, Voicemail; the incoming
//!   call screen; the in-call screen.
//! * [`messages`]: **Messages** (`dev.ecm.messages`, program `messages`): the
//!   conversation list, threads with bubbles and their status, compose with the
//!   part counter, group threads, pictures, voice notes and shared contacts.
//! * [`contacts`]: **Contacts** (`dev.ecm.contacts`, program `contacts`): the list
//!   with search and A-Z, My card, contact pages, edit, add, delete, share, import.
//!
//! None of them touches the network: they ask `telephonyd` over its control socket
//! ([`Tel`]: calls on [`ecm_tel::ctl::PORT`] with the `phone` permission, texts on
//! [`ecm_tel::ctl::SMS_PORT`] with `sms`) and read Contacts' vCard file ([`book`],
//! `contacts` permission) through [`Files`]. The programs give the real ones
//! ([`net`], WASI); tests and the golden recorders give [`fake`]s.

pub mod book;
pub mod contacts;
pub mod dialer;
pub mod messages;
pub mod fake;
#[cfg(target_os = "wasi")]
pub mod net;
pub mod testkit;
pub mod widgets;

pub use ecm_tel::ctl::{CallLine, Event, LogLine, MsgLine, ThreadLine};

/// `telephonyd` as the apps use it.
pub trait Tel {
    /// One request on `port`. Outer `Err`: telephonyd isn't there; inner: its `err` reply.
    fn call(&mut self, port: u16, req: &str) -> Result<Result<Vec<String>, String>, String>;
    /// Events since the last call (the program keeps a `watch` connection open).
    fn events(&mut self) -> Vec<Event>;
}

/// The computer's files as the apps use them (the address book, pictures, voice notes).
pub trait Files {
    fn read(&mut self, path: &str) -> Option<Vec<u8>>;
    fn write(&mut self, path: &str, data: &[u8]) -> Result<(), String>;
    /// File names in a directory ("" entries skipped).
    fn list(&mut self, dir: &str) -> Vec<String>;
}

pub const DIALER_ID: &str = "dev.ecm.dialer";
pub const MESSAGES_ID: &str = "dev.ecm.messages";
pub const CONTACTS_ID: &str = "dev.ecm.contacts";

/// Whether telephonyd answers.
#[derive(Clone, Debug, PartialEq)]
pub enum Daemon {
    Unknown,
    Running,
    /// Not running or unreachable, and why.
    Down(String),
}

/// How often the apps refresh from telephonyd, ms.
pub const POLL_MS: u64 = 1000;

/// A number as people read it: `(214) 555-0142`, else as given.
pub fn pretty(number: &str) -> String {
    ecm_tel::number::E164::parse(number).map(|n| n.display("1")).unwrap_or_else(|| number.to_string())
}

/// `0:42`, `12:05`, `1:02:33`.
pub fn fmt_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// A time of day from the phone's clock (ms since its start): `14:05`, or the age for a list (`2 min`).
pub fn fmt_ago(then_ms: u64, now_ms: u64) -> String {
    let s = now_ms.saturating_sub(then_ms) / 1000;
    if s < 60 {
        "now".into()
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else if s < 86_400 {
        format!("{} h", s / 3600)
    } else {
        format!("{} d", s / 86_400)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats() {
        assert_eq!(super::pretty("+12145550142"), "(214) 555-0142");
        assert_eq!(super::pretty("611"), "611");
        assert_eq!(super::fmt_duration(42), "0:42");
        assert_eq!(super::fmt_duration(3753), "1:02:33");
        assert_eq!(super::fmt_ago(0, 125_000), "2 min");
    }
}
