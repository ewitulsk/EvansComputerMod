//! The playback engine: HTTP `Range` reads of a song in 64 KB pieces, the
//! decoder, and the speaker, as a state machine driven by [`Engine::tick`]
//! (bytes in from [`Net`], samples out to [`Speaker`], a clock). No I/O of
//! its own, so every rule below is tested on the host against a real musicd.
//!
//! - Playback starts after [`START_MS`] of decoded audio, and the download
//!   stays [`AHEAD_MS`] ahead of what plays.
//! - The speaker is the clock: the engine keeps at most [`LATENCY_MS`] queued
//!   there, so its writes are paced by what the speaker plays, and the
//!   position is what was written minus what is still queued.
//! - When the decoded audio runs out the state is Buffering; playback goes on
//!   from the same sample once [`START_MS`] is back (nothing is skipped).
//! - A dropped connection is reconnected and the next request starts at the
//!   byte where the last one stopped.
//! - Seeking uses the MP3's Xing/VBRI table (read from the first piece) or a
//!   proportional offset, and the Data saver file's block offsets.
//! - While the current song is downloaded far enough, the start of the next
//!   song in the queue is fetched ([`Engine::prefetch`]), so it starts at once.
//! - Data saver that isn't ready yet (503) plays Original instead.

use crate::httpc::{self, Reader};
use crate::net::{Net, Speaker};
use ecm_music::mp3::{self, Mp3Info, Timing};
use ecm_music::transcode::{Mp3Decoder, SaverDecoder, SaverHeader};
use std::collections::VecDeque;

/// The speaker rate: Data saver's own rate, and MP3s are resampled to it.
pub const RATE: u32 = 22050;
/// Decoded audio needed before playback starts (or resumes after running dry).
pub const START_MS: u64 = 3000;
/// How far ahead the download goes.
pub const AHEAD_MS: u64 = 30_000;
/// Bytes per `Range` read.
pub const CHUNK: u64 = 64 * 1024;
/// Audio kept queued at the speaker.
pub const LATENCY_MS: i64 = 400;
/// Decoded audio fetched ahead for the next song.
pub const PREFETCH_MS: u64 = 3000;
/// Wait before reconnecting after a dropped connection or a refusal.
pub const RETRY_MS: u64 = 1000;
/// Downloaded bytes decoded per turn (an MP3 second or two): the speaker is fed between slices.
pub const DECODE_STEP: usize = 8 * 1024;
/// A song counts as played after this long (or half of it).
pub const PLAYED_MS: u64 = 30_000;

/// Stream quality.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Quality {
    Original,
    Saver,
}

impl Quality {
    pub fn param(self) -> &'static str {
        match self {
            Quality::Original => "original",
            Quality::Saver => "saver",
        }
    }
}

/// A song in the queue, with the backend it comes from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub art: bool,
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Buffering,
    Playing,
    Paused,
    Ended,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The song began to sound.
    Started(String),
    /// The song played to its end.
    Ended(String),
    /// The song passed 30 s or half its length (POST /played).
    Played(String),
    /// The backend refused the token or proof: new auth headers are needed.
    NeedAuth { host: String, port: u16 },
    /// Data saver isn't ready: playing Original.
    Fallback(String),
    Failed(String),
}

enum Dec {
    Mp3(Mp3Decoder),
    Saver(SaverDecoder),
}

struct Request {
    reader: Reader,
}

struct Stream {
    track: Track,
    quality: Quality,
    auth: String,
    conn: Option<i32>,
    req: Option<Request>,
    next_byte: u64,
    total: Option<u64>,
    eof: bool,
    finished: bool,
    dec: Dec,
    /// Downloaded bytes not decoded yet: decoded a slice per turn, so the speaker is
    /// fed between slices even when the computer is slow (a capped background app).
    raw: VecDeque<u8>,
    pcm: VecDeque<i16>,
    /// The first bytes of an MP3 (its Xing header, the ID3 tag's size).
    first: Vec<u8>,
    info: Option<Mp3Info>,
    probed: bool,
    saver: Option<SaverHeader>,
    pending_seek: Option<u64>,
    retry_at: u64,
    pub failures: u32,
    /// Song time of the first sample the decoder gives after the last seek.
    base_ms: u64,
    failed: Option<String>,
}

fn new_dec(q: Quality) -> Dec {
    match q {
        Quality::Original => Dec::Mp3(Mp3Decoder::new(RATE)),
        Quality::Saver => Dec::Saver(SaverDecoder::new()),
    }
}

impl Stream {
    fn new(track: Track, quality: Quality, auth: String) -> Stream {
        Stream {
            track,
            quality,
            auth,
            conn: None,
            req: None,
            next_byte: 0,
            total: None,
            eof: false,
            finished: false,
            dec: new_dec(quality),
            raw: VecDeque::new(),
            pcm: VecDeque::new(),
            first: Vec::new(),
            info: None,
            probed: false,
            saver: None,
            pending_seek: None,
            retry_at: 0,
            failures: 0,
            base_ms: 0,
            failed: None,
        }
    }

    fn pcm_ms(&self) -> u64 {
        self.pcm.len() as u64 * 1000 / RATE as u64
    }

    fn drop_conn(&mut self, net: &mut dyn Net) {
        if let Some(h) = self.conn.take() {
            net.close(h);
        }
        self.req = None;
    }

    /// Download and decode until `limit_ms` is decoded ahead.
    fn pump(&mut self, net: &mut dyn Net, now: u64, limit_ms: u64, events: &mut Vec<Event>) {
        if self.failed.is_some() {
            return;
        }
        if self.req.is_none() {
            if self.eof || !self.raw.is_empty() || self.pcm_ms() >= limit_ms || now < self.retry_at {
                return;
            }
            if self.conn.is_none() {
                match net.connect(&self.track.host, self.track.port) {
                    Ok(h) => self.conn = Some(h),
                    Err(_) => {
                        self.failures += 1;
                        self.retry_at = now + RETRY_MS;
                        return;
                    }
                }
            }
            let end = self.total.map_or(self.next_byte + CHUNK - 1, |t| (self.next_byte + CHUNK - 1).min(t.saturating_sub(1)));
            let path = format!("/stream/{}?q={}", self.track.id, self.quality.param());
            let headers = format!("{}Range: bytes={}-{}\r\n", self.auth, self.next_byte, end);
            let host = format!("{}:{}", self.track.host, self.track.port);
            let h = self.conn.unwrap();
            if net.send(h, &httpc::request("GET", &path, &host, &headers, b"")).is_err() {
                self.drop_conn(net);
                self.failures += 1;
                self.retry_at = now + RETRY_MS;
                return;
            }
            self.req = Some(Request { reader: Reader::new() });
        }
        let h = self.conn.unwrap();
        loop {
            let data = match net.recv(h) {
                Ok(d) if d.is_empty() => break,
                Ok(d) => d,
                Err(_) => {
                    // Dropped: the next request starts where this one stopped.
                    self.drop_conn(net);
                    self.failures += 1;
                    self.retry_at = now + RETRY_MS / 4;
                    return;
                }
            };
            let req = self.req.as_mut().unwrap();
            let body = match req.reader.push(&data) {
                Ok(b) => b,
                Err(e) => {
                    self.drop_conn(net);
                    self.failures += 1;
                    self.retry_at = now + RETRY_MS;
                    let _ = e;
                    return;
                }
            };
            let status = req.reader.head.as_ref().map_or(0, |hd| hd.status);
            if status == 206 || status == 200 {
                if self.total.is_none() {
                    self.total = req.reader.head.as_ref().and_then(|hd| hd.total());
                }
                if !body.is_empty() {
                    self.failures = 0;
                    self.feed(&body);
                }
            }
            let req = self.req.as_ref().unwrap();
            if req.reader.done() {
                let head = req.reader.head.clone().unwrap();
                self.req = None;
                self.done(net, &head, now, events);
                return;
            }
            if self.conn.is_none() {
                return;
            }
        }
    }

    /// A response finished.
    fn done(&mut self, net: &mut dyn Net, head: &httpc::Head, now: u64, events: &mut Vec<Event>) {
        match head.status {
            200 | 206 => {
                if self.total.is_some_and(|t| self.next_byte >= t) || head.status == 200 {
                    self.end_of_data();
                }
            }
            416 => {
                if self.total.is_none() {
                    self.total = head.total();
                }
                self.end_of_data();
            }
            503 if self.quality == Quality::Saver => {
                // Not made yet: Original, from the same moment.
                events.push(Event::Fallback(self.track.id.clone()));
                let at = self.pending_seek.unwrap_or(self.base_ms);
                self.quality = Quality::Original;
                self.restart(at);
            }
            401 => {
                events.push(Event::NeedAuth { host: self.track.host.clone(), port: self.track.port });
                self.retry_at = now + RETRY_MS;
            }
            503 | 500 | 502 => self.retry_at = now + RETRY_MS * 2,
            s => {
                let why = format!("the server answered {s}");
                events.push(Event::Failed(why.clone()));
                self.failed = Some(why);
                self.drop_conn(net);
            }
        }
        if let Some(ms) = self.pending_seek {
            self.try_seek(net, ms);
        }
    }

    fn restart(&mut self, at_ms: u64) {
        self.dec = new_dec(self.quality);
        self.next_byte = 0;
        self.total = None;
        self.eof = false;
        self.finished = false;
        self.pcm.clear();
        self.raw.clear();
        self.first.clear();
        self.info = None;
        self.probed = false;
        self.saver = None;
        self.pending_seek = (at_ms > 0).then_some(at_ms);
        self.base_ms = 0;
    }

    fn end_of_data(&mut self) {
        self.eof = true;
    }

    /// Bytes downloaded: kept for decoding a slice at a time ([`Stream::decode_some`]);
    /// the MP3's first piece is probed for its seek table, the Data saver header read.
    fn feed(&mut self, body: &[u8]) {
        self.next_byte += body.len() as u64;
        match self.dec {
            Dec::Mp3(_) => {
                if !self.probed && self.first.len() < CHUNK as usize {
                    self.first.extend_from_slice(body);
                }
                if !self.probed && (self.first.len() >= 16 * 1024 || self.total.is_some_and(|t| self.next_byte >= t)) {
                    self.probed = true;
                    self.info = mp3::probe(&self.first);
                }
            }
            Dec::Saver(_) => {
                if self.saver.is_none() && self.first.len() < 64 {
                    self.first.extend_from_slice(body);
                    self.saver = SaverHeader::parse(&self.first);
                }
            }
        }
        self.raw.extend(body.iter().copied());
    }

    /// Decode up to `max` downloaded bytes; at the end of the data, the decoder's tail.
    fn decode_some(&mut self, max: usize) {
        let n = max.min(self.raw.len());
        if n > 0 {
            let chunk: Vec<u8> = self.raw.drain(..n).collect();
            let mut out = Vec::new();
            match &mut self.dec {
                Dec::Mp3(d) => d.push(&chunk, &mut out),
                Dec::Saver(d) => {
                    if d.push(&chunk, &mut out).is_err() {
                        self.failed = Some("the Data saver stream is broken".into());
                    }
                }
            }
            // Audio from before a seek that waits for the header is not wanted.
            if self.pending_seek.is_none() {
                self.pcm.extend(out);
            }
        }
        if self.eof && self.raw.is_empty() && !self.finished && self.pending_seek.is_none() {
            if let Dec::Mp3(d) = &mut self.dec {
                let mut out = Vec::new();
                d.finish(&mut out);
                self.pcm.extend(out);
            }
            self.finished = true;
        }
    }
    /// Seek now if the header (Data saver) or the first piece (MP3) is known; else later.
    fn try_seek(&mut self, net: &mut dyn Net, ms: u64) -> bool {
        let target = match self.quality {
            Quality::Saver => {
                let Some(h) = self.saver else {
                    self.pending_seek = Some(ms);
                    return false;
                };
                let b = h.block_for_ms(ms);
                self.dec = Dec::Saver(SaverDecoder::at_block(h, b));
                self.base_ms = h.ms_for_block(b);
                h.block_offset(b)
            }
            Quality::Original => {
                let (Some(total), true) = (self.total, self.probed) else {
                    self.pending_seek = Some(ms);
                    return false;
                };
                let off = match &self.info {
                    Some(i) if i.timing != Timing::Counted => i.offset_for_ms(ms),
                    Some(i) => {
                        let start = i.audio_start;
                        let d = self.track.duration_ms.max(1);
                        start + (total.saturating_sub(start)) * ms.min(d) / d
                    }
                    None => total * ms.min(self.track.duration_ms.max(1)) / self.track.duration_ms.max(1),
                };
                if let Dec::Mp3(d) = &mut self.dec {
                    d.seek();
                }
                self.base_ms = ms;
                off.min(total)
            }
        };
        // The answer in flight (if any) is for the old place: drop the connection.
        if self.req.is_some() {
            self.drop_conn(net);
        }
        self.pending_seek = None;
        self.pcm.clear();
        self.raw.clear();
        self.next_byte = target;
        self.eof = self.total.is_some_and(|t| target >= t);
        self.finished = self.eof;
        true
    }
}

pub struct Engine {
    cur: Option<Stream>,
    next: Option<Stream>,
    pub state: State,
    want_play: bool,
    written: u64,
    /// The last second written (taken back from the speaker on pause).
    recent: VecDeque<i16>,
    started: bool,
    played: bool,
    speaker_open: bool,
    /// The last buffered_ms read from the speaker.
    queued_ms: i64,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

impl Engine {
    pub fn new() -> Engine {
        Engine {
            cur: None,
            next: None,
            state: State::Idle,
            want_play: false,
            written: 0,
            recent: VecDeque::new(),
            started: false,
            played: false,
            speaker_open: false,
            queued_ms: 0,
        }
    }

    pub fn track(&self) -> Option<&Track> {
        self.cur.as_ref().map(|s| &s.track)
    }

    pub fn quality(&self) -> Option<Quality> {
        self.cur.as_ref().map(|s| s.quality)
    }

    /// Decoded audio waiting to play.
    pub fn ahead_ms(&self) -> u64 {
        self.cur.as_ref().map_or(0, |s| s.pcm_ms())
    }

    /// The next song's prefetched audio.
    pub fn prefetched_ms(&self) -> u64 {
        self.next.as_ref().map_or(0, |s| s.pcm_ms())
    }

    pub fn prefetched(&self) -> Option<&Track> {
        self.next.as_ref().map(|s| &s.track)
    }

    /// Bytes downloaded of the current song (tests).
    pub fn downloaded(&self) -> u64 {
        self.cur.as_ref().map_or(0, |s| s.next_byte)
    }

    /// Times the connection dropped or failed in a row.
    pub fn failures(&self) -> u32 {
        self.cur.as_ref().map_or(0, |s| s.failures)
    }

    pub fn playing(&self) -> bool {
        self.want_play
    }

    /// The song position heard now.
    pub fn position_ms(&self) -> u64 {
        let played = (self.written * 1000 / RATE as u64) as i64 - self.queued_ms.max(0);
        let Some(s) = self.cur.as_ref() else { return 0 };
        (s.base_ms + played.max(0) as u64).min(s.track.duration_ms)
    }

    /// Play `track` from `start_ms` (paused if `play` is false). The prefetched
    /// start is used when it is this song in this quality.
    pub fn load(&mut self, net: &mut dyn Net, spk: &mut dyn Speaker, track: Track, quality: Quality, auth: &str, start_ms: u64, play: bool) {
        if let Some(mut old) = self.cur.take() {
            old.drop_conn(net);
        }
        let reuse = self.next.as_ref().is_some_and(|n| n.track.id == track.id && n.track.host == track.host && n.quality == quality) && start_ms == 0;
        let mut s = if reuse {
            self.next.take().unwrap()
        } else {
            if let Some(mut n) = self.next.take() {
                n.drop_conn(net);
            }
            Stream::new(track, quality, auth.to_string())
        };
        s.auth = auth.to_string();
        if start_ms > 0 && !s.try_seek(net, start_ms) {
            s.pending_seek = Some(start_ms);
            s.base_ms = start_ms;
        }
        self.cur = Some(s);
        if self.speaker_open {
            spk.flush();
        }
        self.written = 0;
        self.recent.clear();
        self.queued_ms = 0;
        self.started = false;
        self.played = false;
        self.want_play = play;
        self.state = if play { State::Buffering } else { State::Paused };
    }

    /// Fetch the start of the song after this one.
    pub fn prefetch(&mut self, net: &mut dyn Net, track: Track, quality: Quality, auth: &str) {
        if self.next.as_ref().is_some_and(|n| n.track.id == track.id && n.track.host == track.host && n.quality == quality) {
            return;
        }
        if let Some(mut n) = self.next.take() {
            n.drop_conn(net);
        }
        self.next = Some(Stream::new(track, quality, auth.to_string()));
    }

    /// New auth headers for streams from `host:port`.
    pub fn set_auth(&mut self, host: &str, port: u16, auth: &str) {
        for s in self.cur.iter_mut().chain(self.next.iter_mut()) {
            if s.track.host == host && s.track.port == port {
                s.auth = auth.to_string();
            }
        }
    }

    pub fn play(&mut self) {
        if self.cur.is_none() {
            return;
        }
        if self.state == State::Ended {
            return;
        }
        self.want_play = true;
        if self.state == State::Paused || self.state == State::Idle {
            self.state = State::Buffering;
        }
    }

    /// Pause, taking back what the speaker hadn't played yet (nothing is lost).
    pub fn pause(&mut self, spk: &mut dyn Speaker) {
        if !self.want_play {
            return;
        }
        self.want_play = false;
        if self.speaker_open {
            let q = spk.buffered_ms().max(0) as u64;
            let n = ((q * RATE as u64 / 1000) as usize).min(self.recent.len()).min(self.written as usize);
            if let Some(s) = self.cur.as_mut() {
                let back: Vec<i16> = self.recent.drain(self.recent.len() - n..).collect();
                for v in back.into_iter().rev() {
                    s.pcm.push_front(v);
                }
            }
            self.written -= n as u64;
            spk.flush();
            self.queued_ms = 0;
        }
        if matches!(self.state, State::Playing | State::Buffering) {
            self.state = State::Paused;
        }
    }

    pub fn toggle(&mut self, spk: &mut dyn Speaker) {
        if self.want_play {
            self.pause(spk);
        } else {
            self.play();
        }
    }

    pub fn seek(&mut self, net: &mut dyn Net, spk: &mut dyn Speaker, ms: u64) {
        let Some(s) = self.cur.as_mut() else { return };
        let ms = ms.min(s.track.duration_ms.saturating_sub(500));
        s.finished = false;
        if !s.try_seek(net, ms) {
            s.base_ms = ms;
            s.pcm.clear();
        }
        if self.speaker_open {
            spk.flush();
        }
        self.written = 0;
        self.recent.clear();
        self.queued_ms = 0;
        if self.state == State::Ended {
            self.state = State::Paused;
        }
        if self.want_play {
            self.state = State::Buffering;
        }
    }

    pub fn stop(&mut self, net: &mut dyn Net, spk: &mut dyn Speaker) {
        for mut s in self.cur.take().into_iter().chain(self.next.take()) {
            s.drop_conn(net);
        }
        if self.speaker_open {
            spk.flush();
        }
        self.want_play = false;
        self.state = State::Idle;
        self.written = 0;
        self.queued_ms = 0;
    }

    /// One turn: download, decode, feed the speaker. Returns what happened.
    pub fn tick(&mut self, net: &mut dyn Net, spk: &mut dyn Speaker, now: u64) -> Vec<Event> {
        let mut ev = Vec::new();
        let Some(cur) = self.cur.as_mut() else { return ev };
        cur.pump(net, now, AHEAD_MS, &mut ev);
        cur.decode_some(DECODE_STEP);
        if let Some(why) = &cur.failed {
            if !matches!(self.state, State::Failed(_)) {
                self.state = State::Failed(why.clone());
            }
            return ev;
        }
        // The start of the next song, once this one is far enough ahead.
        let cur_ready = cur.finished || cur.pcm_ms() >= AHEAD_MS;
        if cur_ready {
            if let Some(n) = self.next.as_mut() {
                n.pump(net, now, PREFETCH_MS, &mut ev);
                n.decode_some(DECODE_STEP);
                if n.pcm_ms() >= PREFETCH_MS || n.eof {
                    // One piece is enough: free the connection.
                    if n.req.is_none() {
                        n.drop_conn(net);
                    }
                }
            }
        }
        let cur = self.cur.as_mut().unwrap();
        if self.state == State::Buffering && self.want_play && (cur.pcm_ms() >= START_MS || cur.finished) {
            self.state = State::Playing;
        }
        if self.state == State::Playing {
            if !self.speaker_open {
                self.speaker_open = spk.open(RATE).is_ok();
                if !self.speaker_open {
                    self.state = State::Failed("no speaker".into());
                    ev.push(Event::Failed("no speaker".into()));
                    return ev;
                }
            }
            let queued = spk.buffered_ms();
            let room = (LATENCY_MS - queued).max(0) as u64;
            let n = ((room * RATE as u64 / 1000) as usize).min(cur.pcm.len());
            if n > 0 {
                let chunk: Vec<i16> = cur.pcm.drain(..n).collect();
                spk.write(&chunk);
                self.written += n as u64;
                self.recent.extend(chunk.iter().copied());
                let keep = RATE as usize;
                if self.recent.len() > keep {
                    let cut = self.recent.len() - keep;
                    self.recent.drain(..cut);
                }
                if !self.started {
                    self.started = true;
                    ev.push(Event::Started(cur.track.id.clone()));
                }
            }
            self.queued_ms = spk.buffered_ms();
            if cur.pcm.is_empty() {
                if cur.finished {
                    if self.queued_ms <= 0 {
                        self.state = State::Ended;
                        self.want_play = false;
                        ev.push(Event::Ended(cur.track.id.clone()));
                    }
                } else {
                    self.state = State::Buffering;
                }
            }
        } else if self.speaker_open {
            self.queued_ms = spk.buffered_ms();
        }
        let pos = self.position_ms();
        let d = self.cur.as_ref().map_or(0, |s| s.track.duration_ms);
        if !self.played && self.started && pos >= PLAYED_MS.min(d / 2).max(1) {
            self.played = true;
            ev.push(Event::Played(self.cur.as_ref().unwrap().track.id.clone()));
        }
        ev
    }

    /// Close the speaker (the app is closing).
    pub fn close(&mut self, net: &mut dyn Net, spk: &mut dyn Speaker) {
        self.stop(net, spk);
        if self.speaker_open {
            spk.close();
            self.speaker_open = false;
        }
    }
}
