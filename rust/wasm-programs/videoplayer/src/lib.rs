//! The Video Player's logic without the host calls (`main.rs` and the `video`
//! command drive it), so it can be tested on the host:
//!
//! * [`Clock`]: the playback position, which the picture and the sound follow
//!   (wall-clock master: pause, play and seek move it).
//! * [`AudioPump`]: keeps the speaker a little ahead of the clock with the
//!   video's own PCM, and skips sound that fell behind.
//! * [`fit`] and [`decode_size`]: where the picture goes and at which size the
//!   host decodes it.
//! * [`Ui`]: the app's two screens. **Library** lists the video files in
//!   `/home` and `/home/videos`; **Playing** shows the video in a `canvas`
//!   node with a seek bar and the buttons Library, -10s, Play/Pause, +10s and
//!   Loop. The same window works as a phone app and on a Terminal desktop.

use ecm_ui::diff::SceneNode;
use ecm_ui_proto as proto;
use ecm_ui_proto::{Font, NodeKind, Prop};

pub const APP_ID: &str = "dev.ecm.video";

/// File types the player opens (`open_with` in its manifest).
pub const VIDEO_EXTS: [&str; 6] = [".mp4", ".m4v", ".mkv", ".webm", ".mov", ".avi"];

/// Folders the library lists.
pub const LIBRARY_DIRS: [&str; 2] = ["/home", "/home/videos"];

/// The sound the player asks the host for: mono, 16-bit, this rate.
pub const AUDIO_RATE: u32 = 22050;
pub const AUDIO_CHANNELS: u8 = 1;

/// Hit ids.
pub const LIBRARY_HIT: u32 = 40;
pub const PLAY_HIT: u32 = 41;
pub const BACK10_HIT: u32 = 42;
pub const FWD10_HIT: u32 = 43;
pub const LOOP_HIT: u32 = 44;
pub const SEEK_HIT: u32 = 45;
pub const REFRESH_HIT: u32 = 46;
pub const PREV_HIT: u32 = 47;
pub const NEXT_HIT: u32 = 48;
/// Library rows: ROW_HIT + index into the list.
pub const ROW_HIT: u32 = 100;

/// The canvas resource id the picture goes into.
pub const CANVAS: u32 = 1;

/// Most pixels the host decodes per frame; bigger pictures are decoded
/// smaller and scaled up by the canvas (a whole number).
pub const MAX_DECODE_PIXELS: i32 = 400 * 240;

/// Is this a file the player opens?
pub fn is_video(path: &str) -> bool {
    let l = path.to_ascii_lowercase();
    VIDEO_EXTS.iter().any(|e| l.ends_with(e))
}

/// `m:ss`, or `h:mm:ss` from an hour.
pub fn format_time(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A video file in the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub bytes: u64,
}

impl Entry {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

/// `12.3 MB`, `640 KB`.
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// The video files in `dirs` (not their subfolders), sorted by path.
pub fn find_videos(dirs: &[&str]) -> Vec<Entry> {
    let mut out = Vec::new();
    for dir in dirs {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                let is_file = e.file_type().map(|t| t.is_file()).unwrap_or(false);
                if is_file && is_video(&name) {
                    let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push(Entry { path: format!("{}/{}", dir.trim_end_matches('/'), name), bytes });
                }
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup_by(|a, b| a.path == b.path);
    out
}

// ---------------------------------------------------------------- clock

/// The playback position in ms. Time is passed in (`now` in ms on any
/// monotonic clock) so it can be tested.
#[derive(Clone, Debug)]
pub struct Clock {
    anchor_now: i64,
    anchor_pos: i64,
    playing: bool,
}

impl Clock {
    pub fn new(now: i64, pos: i64, playing: bool) -> Clock {
        Clock { anchor_now: now, anchor_pos: pos.max(0), playing }
    }

    pub fn pos(&self, now: i64) -> i64 {
        if self.playing {
            self.anchor_pos + (now - self.anchor_now).max(0)
        } else {
            self.anchor_pos
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn pause(&mut self, now: i64) {
        self.anchor_pos = self.pos(now);
        self.anchor_now = now;
        self.playing = false;
    }

    pub fn play(&mut self, now: i64) {
        self.anchor_pos = self.pos(now);
        self.anchor_now = now;
        self.playing = true;
    }

    pub fn seek(&mut self, now: i64, pos: i64) {
        self.anchor_pos = pos.max(0);
        self.anchor_now = now;
    }
}

// ---------------------------------------------------------------- frame pacing

/// What to do with a decoded frame at `pts` when the clock says `pos`
/// (`frame_ms`: one frame's duration).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FrameTiming {
    /// Not yet: show it in this many ms.
    Wait(i64),
    /// Show it now.
    Show,
    /// So late the next frame is already due: skip it.
    Drop,
}

pub fn frame_timing(pts: i64, pos: i64, frame_ms: i64) -> FrameTiming {
    if pts > pos {
        FrameTiming::Wait(pts - pos)
    } else if pos - pts > frame_ms.max(1) * 2 {
        FrameTiming::Drop
    } else {
        FrameTiming::Show
    }
}

// ---------------------------------------------------------------- audio

/// Where the video's PCM comes from (the host decoder).
pub trait AudioSource {
    /// Fill `buf` with the next PCM; 0 at the end of the sound.
    fn read(&mut self, buf: &mut [u8]) -> usize;
}

/// Where it goes (the speaker).
pub trait AudioSink {
    fn write(&mut self, pcm: &[u8]);
}

/// Keeps the speaker `lead_ms` ahead of the clock. The first byte read after
/// [`reset`](Self::reset)`(start)` is at `start` (the host aligns it), so the
/// sound's position is `start + bytes / bytes_per_ms`.
pub struct AudioPump {
    bytes_per_sec: u64,
    frame_bytes: usize,
    start_ms: i64,
    bytes: u64,
    ended: bool,
    buf: Vec<u8>,
    pub lead_ms: i64,
}

impl AudioPump {
    pub fn new(rate: u32, channels: u8) -> AudioPump {
        let frame_bytes = channels as usize * 2;
        // 40 ms chunks.
        let chunk = (rate as usize / 25).max(1) * frame_bytes;
        AudioPump {
            bytes_per_sec: rate as u64 * frame_bytes as u64,
            frame_bytes,
            start_ms: 0,
            bytes: 0,
            ended: false,
            buf: vec![0; chunk],
            lead_ms: 200,
        }
    }

    /// The sound restarts at `start_ms` (after open or a seek).
    pub fn reset(&mut self, start_ms: i64) {
        self.start_ms = start_ms;
        self.bytes = 0;
        self.ended = false;
    }

    /// Position of the next byte, in ms.
    pub fn pos_ms(&self) -> i64 {
        self.start_ms + (self.bytes * 1000 / self.bytes_per_sec.max(1)) as i64
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    /// Feed the speaker up to `pos + lead_ms`, first throwing away sound
    /// that is already more than a chunk behind `pos`. Returns bytes written.
    pub fn pump(&mut self, pos: i64, src: &mut dyn AudioSource, sink: &mut dyn AudioSink) -> usize {
        let chunk_ms = (self.buf.len() as u64 * 1000 / self.bytes_per_sec.max(1)) as i64;
        while !self.ended && self.pos_ms() + chunk_ms < pos {
            let behind = ((pos - self.pos_ms()) as u64 * self.bytes_per_sec / 1000) as usize;
            let n = behind.min(self.buf.len()) / self.frame_bytes * self.frame_bytes;
            if n == 0 {
                break;
            }
            let got = src.read(&mut self.buf[..n]);
            if got == 0 {
                self.ended = true;
            }
            self.bytes += got as u64;
        }
        let mut written = 0;
        while !self.ended && self.pos_ms() < pos + self.lead_ms {
            let got = src.read(&mut self.buf);
            if got == 0 {
                self.ended = true;
                break;
            }
            sink.write(&self.buf[..got]);
            self.bytes += got as u64;
            written += got;
        }
        written
    }
}

// ---------------------------------------------------------------- layout

/// The largest rect of the video's aspect ratio inside `area` (x, y, w, h), centred.
pub fn fit(area: (i32, i32, i32, i32), vw: i32, vh: i32) -> (i32, i32, i32, i32) {
    let (ax, ay, aw, ah) = area;
    if vw <= 0 || vh <= 0 || aw <= 0 || ah <= 0 {
        return (ax, ay, aw.max(0), ah.max(0));
    }
    let (w, h) = if aw as i64 * vh as i64 <= ah as i64 * vw as i64 {
        (aw, ((aw as i64 * vh as i64) / vw as i64) as i32)
    } else {
        (((ah as i64 * vw as i64) / vh as i64) as i32, ah)
    };
    let (w, h) = (w.max(1), h.max(1));
    (ax + (aw - w) / 2, ay + (ah - h) / 2, w, h)
}

/// The size to decode a `w` x `h` picture at, and the canvas scale that
/// brings it back up: the smallest whole scale that keeps it under
/// [`MAX_DECODE_PIXELS`], with even sides (what scalers like).
pub fn decode_size(w: i32, h: i32) -> (i32, i32, i32) {
    let mut s = 1;
    while (w / s) * (h / s) > MAX_DECODE_PIXELS {
        s += 1;
    }
    let even = |v: i32| (v / s / 2 * 2).max(2);
    (even(w), even(h), s)
}

/// The Playing screen's parts in a `w` x `h` window.
pub struct PlayLayout {
    pub top: (i32, i32, i32, i32),
    pub video_area: (i32, i32, i32, i32),
    pub seek: (i32, i32, i32, i32),
    pub buttons: Vec<(u32, (i32, i32, i32, i32))>,
    pub time_y: i32,
}

pub const TOP_H: i32 = 20;
pub const CONTROLS_H: i32 = 46;

pub fn play_layout(w: i32, h: i32) -> PlayLayout {
    let top = (0, 0, w, TOP_H);
    let video_area = (0, TOP_H, w, (h - TOP_H - CONTROLS_H).max(1));
    let cy = h - CONTROLS_H;
    let seek = (8, cy + 6, (w - 16).max(1), 8);
    let time_y = cy + 16;
    // -10s, Play, +10s centred; Loop at the right.
    let bh = 18;
    let by = h - bh - 4;
    // Narrow windows (a Tier 1 phone is 120 lu wide) get narrower buttons.
    let play_w = 54.min((w - 24) * 2 / 5).max(24);
    let skip_w = 40.min((w - 24 - play_w) / 2).max(16);
    let mid = w / 2;
    let mut buttons = vec![
        (BACK10_HIT, (mid - play_w / 2 - 6 - skip_w, by, skip_w, bh)),
        (PLAY_HIT, (mid - play_w / 2, by, play_w, bh)),
        (FWD10_HIT, (mid + play_w / 2 + 6, by, skip_w, bh)),
    ];
    let loop_w = 40;
    if w - 6 - loop_w > mid + play_w / 2 + 12 + skip_w {
        buttons.push((LOOP_HIT, (w - 6 - loop_w, by, loop_w, bh)));
    }
    PlayLayout { top, video_area, seek, buttons, time_y }
}

// ---------------------------------------------------------------- the screens

pub fn text_w(s: &str, size: u8) -> i32 {
    proto::fonts::face(Font::Ui, size).map(|f| f.text_width(s)).unwrap_or(0)
}

fn ellipsize(s: &str, size: u8, max_w: i32) -> String {
    match proto::fonts::face(Font::Ui, size) {
        Some(f) => ecm_ui::text::ellipsize(&f, s, max_w.max(0)),
        None => s.to_string(),
    }
}

fn rect(key: &str, r: (i32, i32, i32, i32), fill: u32, radius: u16, hit: u32) -> SceneNode {
    let mut p = vec![Prop::X(r.0), Prop::Y(r.1), Prop::W(r.2), Prop::H(r.3), Prop::Fill(fill), Prop::Radius(radius)];
    if hit != 0 {
        p.push(Prop::HitId(hit));
    }
    SceneNode::new(key, NodeKind::Rect, p)
}

fn text(key: &str, x: i32, y: i32, s: &str, size: u8, color: u32, hit: u32) -> SceneNode {
    let mut p = vec![Prop::X(x), Prop::Y(y), Prop::Text(s.into()), Prop::Font(Font::Ui), Prop::Size(size), Prop::Color(color)];
    if hit != 0 {
        p.push(Prop::HitId(hit));
    }
    SceneNode::new(key, NodeKind::Text, p)
}

fn pill(key: &str, r: (i32, i32, i32, i32), label: &str, size: u8, fill: u32, hit: u32, out: &mut Vec<SceneNode>) {
    out.push(rect(key, r, fill, (r.3 / 2) as u16, hit));
    let lh = proto::fonts::face(Font::Ui, size).map_or(size as i32, |f| f.line_height());
    let label = ellipsize(label, size, r.2 - 6);
    out.push(text(&format!("{key}:t"), r.0 + (r.2 - text_w(&label, size)) / 2, r.1 + (r.3 - lh) / 2, &label, size, FG, hit));
}

const BG: u32 = 0x14161cff;
const BAR: u32 = 0x1f222bff;
const BUTTON: u32 = 0x343846ff;
const ACCENT: u32 = 0x4f8cffff;
const TRACK: u32 = 0x3a3e4cff;
const FG: u32 = 0xeef1f6ff;
const MUTED: u32 = 0x8b92a3ff;
const WARN: u32 = 0xff8a65ff;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Screen {
    Library,
    Playing,
}

/// What a press asks `main` to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    /// Open this file and play it.
    Open(String),
    /// Leave the video for the library.
    Library,
    TogglePlay,
    /// Jump by this many ms.
    Skip(i64),
    /// Seek to this fraction of the video (0..=1).
    SeekTo(f64),
    ToggleLoop,
    Refresh,
}

/// What the Playing screen shows.
#[derive(Clone, Debug, Default)]
pub struct Playback {
    pub path: String,
    pub pos_ms: i64,
    pub duration_ms: i64,
    pub playing: bool,
    pub looping: bool,
    pub ended: bool,
    /// The video's own size, for the picture's aspect ratio.
    pub video_w: i32,
    pub video_h: i32,
    /// "sound", "no sound" (the file has none) or "no speaker".
    pub sound: String,
}

pub struct Ui {
    pub screen: Screen,
    pub videos: Vec<Entry>,
    /// First library row shown.
    pub offset: usize,
    pub error: String,
    pub play: Playback,
}

impl Default for Ui {
    fn default() -> Self {
        Ui::new()
    }
}

impl Ui {
    pub fn new() -> Ui {
        Ui { screen: Screen::Library, videos: Vec::new(), offset: 0, error: String::new(), play: Playback::default() }
    }

    /// How many library rows fit in a window `h` high.
    pub fn rows_fit(h: i32) -> usize {
        ((h - 44 - 24) / 22).clamp(1, 50) as usize
    }

    /// A press with hit id `hit` at window x `x` (the seek bar needs it), in a `w` x `h` window.
    pub fn pointer_down(&mut self, hit: u32, x: i32, w: i32, h: i32) -> Action {
        match self.screen {
            Screen::Library => match hit {
                REFRESH_HIT => Action::Refresh,
                PREV_HIT => {
                    self.offset = self.offset.saturating_sub(Ui::rows_fit(h));
                    Action::None
                }
                NEXT_HIT => {
                    let n = Ui::rows_fit(h);
                    if self.offset + n < self.videos.len() {
                        self.offset += n;
                    }
                    Action::None
                }
                h if h >= ROW_HIT => match self.videos.get((h - ROW_HIT) as usize) {
                    Some(e) => Action::Open(e.path.clone()),
                    None => Action::None,
                },
                _ => Action::None,
            },
            Screen::Playing => match hit {
                LIBRARY_HIT => Action::Library,
                PLAY_HIT => Action::TogglePlay,
                BACK10_HIT => Action::Skip(-10_000),
                FWD10_HIT => Action::Skip(10_000),
                LOOP_HIT => Action::ToggleLoop,
                SEEK_HIT => {
                    let s = play_layout(w, h).seek;
                    Action::SeekTo(((x - s.0) as f64 / s.2.max(1) as f64).clamp(0.0, 1.0))
                }
                _ => Action::None,
            },
        }
    }

    /// A key (HID usage, down only): Space / K play-pause, arrows +-10 s,
    /// L loop, Escape / Backspace the library, Enter opens the first video.
    pub fn key(&mut self, usage: u16) -> Action {
        match (self.screen, usage) {
            (Screen::Playing, 0x2C) | (Screen::Playing, 0x0E) => Action::TogglePlay,
            (Screen::Playing, 0x50) => Action::Skip(-10_000),
            (Screen::Playing, 0x4F) => Action::Skip(10_000),
            (Screen::Playing, 0x0F) => Action::ToggleLoop,
            (Screen::Playing, 0x29) | (Screen::Playing, 0x2A) => Action::Library,
            (Screen::Library, 0x28) => match self.videos.get(self.offset) {
                Some(e) => Action::Open(e.path.clone()),
                None => Action::None,
            },
            _ => Action::None,
        }
    }

    /// Mouse wheel over the library.
    pub fn scroll(&mut self, dy: i16, h: i32) {
        if self.screen != Screen::Library {
            return;
        }
        let max = self.videos.len().saturating_sub(Ui::rows_fit(h));
        if dy > 0 {
            self.offset = (self.offset + 1).min(max);
        } else if dy < 0 {
            self.offset = self.offset.saturating_sub(1);
        }
    }

    /// Where the picture goes in a `w` x `h` window (Playing).
    pub fn picture_rect(&self, w: i32, h: i32) -> (i32, i32, i32, i32) {
        fit(play_layout(w, h).video_area, self.play.video_w, self.play.video_h)
    }

    /// The window's nodes. `canvas`: the canvas resource's scale when one
    /// exists (the picture is drawn only then).
    pub fn nodes(&self, w: i32, h: i32, canvas: Option<i32>) -> Vec<SceneNode> {
        let mut n = vec![rect("bg", (0, 0, w, h), BG, 0, 0)];
        match self.screen {
            Screen::Library => self.library_nodes(w, h, &mut n),
            Screen::Playing => self.playing_nodes(w, h, canvas, &mut n),
        }
        if !self.error.is_empty() {
            let line = ellipsize(&self.error, 8, w - 16);
            n.push(text("err", 8, h - 12, &line, 8, WARN, 0));
        }
        n
    }

    fn library_nodes(&self, w: i32, h: i32, n: &mut Vec<SceneNode>) {
        n.push(text("title", 8, 8, "Videos", 12, FG, 0));
        let hint = if self.videos.is_empty() {
            "No videos in /home or /home/videos".to_string()
        } else {
            format!("{} video{}. Tap one to play.", self.videos.len(), if self.videos.len() == 1 { "" } else { "s" })
        };
        n.push(text("hint", 8, 24, &ellipsize(&hint, 8, w - 16), 8, MUTED, 0));
        let rw = text_w("Refresh", 12) + 16;
        pill("refresh", (w - 6 - rw, 6, rw, 18), "Refresh", 12, BUTTON, REFRESH_HIT, n);
        let fit = Ui::rows_fit(h);
        let top = 40;
        for (i, e) in self.videos.iter().enumerate().skip(self.offset).take(fit) {
            let row = i - self.offset;
            let y = top + row as i32 * 22;
            let hit = ROW_HIT + i as u32;
            n.push(rect(&format!("row{row}"), (6, y, w - 12, 18), BUTTON, 3, hit));
            let size = format_size(e.bytes);
            let sw = text_w(&size, 8);
            let name = ellipsize(e.name(), 12, w - 12 - 18 - sw - 8);
            n.push(text(&format!("row{row}:n"), 12, y + 3, &name, 12, FG, hit));
            n.push(text(&format!("row{row}:s"), w - 12 - sw - 6, y + 6, &size, 8, MUTED, hit));
        }
        if self.videos.len() > fit {
            let y = top + fit as i32 * 22 + 2;
            let page = format!("{}-{} of {}", self.offset + 1, (self.offset + fit).min(self.videos.len()), self.videos.len());
            n.push(text("page", 8, y + 5, &page, 8, MUTED, 0));
            let bw = 44;
            if self.offset > 0 {
                pill("prev", (w - 12 - bw * 2, y, bw, 18), "Prev", 8, BUTTON, PREV_HIT, n);
            }
            if self.offset + fit < self.videos.len() {
                pill("next", (w - 6 - bw, y, bw, 18), "Next", 8, BUTTON, NEXT_HIT, n);
            }
        }
    }

    fn playing_nodes(&self, w: i32, h: i32, canvas: Option<i32>, n: &mut Vec<SceneNode>) {
        let l = play_layout(w, h);
        let p = &self.play;
        n.push(rect("top", l.top, BAR, 0, 0));
        let lw = text_w("< Library", 8) + 14;
        pill("library", (4, 2, lw, 16), "< Library", 8, BUTTON, LIBRARY_HIT, n);
        let name = p.path.rsplit('/').next().unwrap_or(&p.path);
        let name = ellipsize(name, 8, w - lw - 16);
        n.push(text("name", lw + 10, 6, &name, 8, FG, 0));
        n.push(rect("screen", l.video_area, 0x000000ff, 0, 0));
        if let Some(scale) = canvas {
            let r = self.picture_rect(w, h);
            n.push(SceneNode::new(
                "picture",
                NodeKind::Canvas,
                vec![Prop::X(r.0), Prop::Y(r.1), Prop::W(r.2), Prop::H(r.3), Prop::Resource(CANVAS), Prop::Scale(scale.clamp(1, 255) as u8)],
            ));
        }
        if p.ended {
            let msg = if p.looping { "Starting again..." } else { "Ended. Play starts it again." };
            let a = l.video_area;
            n.push(text("ended", a.0 + (a.2 - text_w(msg, 8)) / 2, a.1 + a.3 - 14, msg, 8, MUTED, 0));
        }
        // Controls.
        n.push(rect("controls", (0, h - CONTROLS_H, w, CONTROLS_H), BAR, 0, 0));
        let s = l.seek;
        n.push(rect("track", s, TRACK, 4, SEEK_HIT));
        let frac = if p.duration_ms > 0 { (p.pos_ms as f64 / p.duration_ms as f64).clamp(0.0, 1.0) } else { 0.0 };
        let done = ((s.2 as f64) * frac) as i32;
        if done > 0 {
            n.push(rect("played", (s.0, s.1, done, s.3), ACCENT, 4, SEEK_HIT));
        }
        n.push(rect("knob", (s.0 + done - 3, s.1 - 2, 6, s.3 + 4), FG, 3, SEEK_HIT));
        let time = format!("{} / {}", format_time(p.pos_ms), format_time(p.duration_ms));
        n.push(text("time", 8, l.time_y, &time, 8, FG, 0));
        if !p.sound.is_empty() {
            n.push(text("sound", w - 8 - text_w(&p.sound, 8), l.time_y, &p.sound, 8, MUTED, 0));
        }
        for (hit, r) in &l.buttons {
            let (key, label, fill) = match *hit {
                BACK10_HIT => ("back10", "-10s", BUTTON),
                FWD10_HIT => ("fwd10", "+10s", BUTTON),
                PLAY_HIT => ("play", if p.playing { "Pause" } else { "Play" }, ACCENT),
                LOOP_HIT => ("loop", "Loop", if p.looping { ACCENT } else { BUTTON }),
                _ => continue,
            };
            pill(key, *r, label, 8, fill, *hit, n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(n: &[SceneNode]) -> Vec<String> {
        n.iter().filter_map(|s| s.props.iter().find_map(|p| if let Prop::Text(t) = p { Some(t.clone()) } else { None })).collect()
    }

    fn get(s: &SceneNode, f: fn(&Prop) -> Option<i32>) -> Option<i32> {
        s.props.iter().find_map(f)
    }

    fn bounds(s: &SceneNode) -> Option<(i32, i32, i32, i32)> {
        Some((
            get(s, |p| if let Prop::X(v) = p { Some(*v) } else { None })?,
            get(s, |p| if let Prop::Y(v) = p { Some(*v) } else { None })?,
            get(s, |p| if let Prop::W(v) = p { Some(*v) } else { None }).unwrap_or(0),
            get(s, |p| if let Prop::H(v) = p { Some(*v) } else { None }).unwrap_or(0),
        ))
    }

    #[test]
    fn times_sizes_and_types() {
        assert_eq!(format_time(0), "0:00");
        assert_eq!(format_time(61_999), "1:01");
        assert_eq!(format_time(3_725_000), "1:02:05");
        assert_eq!(format_time(-5), "0:00");
        assert_eq!(format_size(1000), "1 KB");
        assert_eq!(format_size(3 << 20), "3.0 MB");
        assert!(is_video("/home/a.MP4") && is_video("x.webm") && is_video("clip.mkv"));
        assert!(!is_video("/home/a.mp3") && !is_video("notes.txt"));
    }

    #[test]
    fn the_clock_pauses_plays_and_seeks() {
        let mut c = Clock::new(1000, 0, true);
        assert_eq!(c.pos(1500), 500);
        c.pause(1500);
        assert_eq!(c.pos(9000), 500, "paused: the position stays");
        c.play(9000);
        assert_eq!(c.pos(9250), 750);
        c.seek(9250, 20_000);
        assert_eq!(c.pos(9300), 20_050);
        c.seek(9300, -10);
        assert_eq!(c.pos(9300), 0, "never before the start");
        let p = Clock::new(0, 400, false);
        assert_eq!(p.pos(10_000), 400);
    }

    #[test]
    fn frames_wait_show_or_drop() {
        assert_eq!(frame_timing(1000, 900, 66), FrameTiming::Wait(100));
        assert_eq!(frame_timing(1000, 1000, 66), FrameTiming::Show);
        assert_eq!(frame_timing(1000, 1100, 66), FrameTiming::Show);
        assert_eq!(frame_timing(1000, 1200, 66), FrameTiming::Drop);
    }

    /// PCM where each 2-byte sample is its own index (so reads can be checked).
    struct Counting {
        next: u64,
        total: u64,
    }

    impl AudioSource for Counting {
        fn read(&mut self, buf: &mut [u8]) -> usize {
            let n = (buf.len() as u64 / 2).min(self.total - self.next) as usize;
            for i in 0..n {
                buf[i * 2..i * 2 + 2].copy_from_slice(&((self.next + i as u64) as u16).to_le_bytes());
            }
            self.next += n as u64;
            n * 2
        }
    }

    #[derive(Default)]
    struct Speaker {
        got: Vec<u8>,
    }

    impl AudioSink for Speaker {
        fn write(&mut self, pcm: &[u8]) {
            self.got.extend_from_slice(pcm);
        }
    }

    #[test]
    fn the_pump_stays_a_little_ahead_and_skips_what_fell_behind() {
        let rate = 1000; // 1 sample per ms, 2 bytes each
        let mut src = Counting { next: 0, total: 5000 };
        let mut spk = Speaker::default();
        let mut p = AudioPump::new(rate, 1);
        p.lead_ms = 200;
        p.pump(0, &mut src, &mut spk);
        assert!(p.pos_ms() >= 200 && p.pos_ms() < 260, "ahead by the lead: {}", p.pos_ms());
        let first = spk.got.len();
        p.pump(0, &mut src, &mut spk);
        assert_eq!(spk.got.len(), first, "nothing more until the clock moves");
        p.pump(100, &mut src, &mut spk);
        assert!(p.pos_ms() >= 300);
        // The clock jumps 2 s ahead (a stall): the old sound is skipped, not played late.
        let before = spk.got.len();
        p.pump(2300, &mut src, &mut spk);
        let played_from = u16::from_le_bytes([spk.got[before], spk.got[before + 1]]) as i64;
        assert!(played_from >= 2260, "skipped to about the clock: sample {played_from}");
        // To the end.
        p.pump(10_000, &mut src, &mut spk);
        assert!(p.ended());
        // After a seek the pump restarts at the target.
        p.reset(3000);
        assert_eq!(p.pos_ms(), 3000);
        assert!(!p.ended());
    }

    #[test]
    fn pictures_fit_and_decode_small_enough() {
        assert_eq!(fit((0, 20, 240, 300), 320, 180), (0, 20 + (300 - 135) / 2, 240, 135));
        assert_eq!(fit((0, 0, 400, 100), 16, 9), ((400 - 177) / 2, 0, 177, 100));
        assert_eq!(decode_size(240, 135), (240, 134, 1));
        let (w, h, s) = decode_size(640, 360);
        assert_eq!(s, 2);
        assert_eq!((w, h), (320, 180));
        let (w, h, s) = decode_size(1280, 720);
        assert!(w * h <= MAX_DECODE_PIXELS && s >= 3 && w % 2 == 0 && h % 2 == 0);
    }

    #[test]
    fn the_library_lists_videos_and_opens_one() {
        let d = std::env::temp_dir().join(format!("videoplayer-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("videos")).unwrap();
        std::fs::write(d.join("b.mp4"), vec![0u8; 2048]).unwrap();
        std::fs::write(d.join("a.txt"), b"no").unwrap();
        std::fs::write(d.join("videos/c.webm"), b"x").unwrap();
        let root = d.to_string_lossy().replace('\\', "/");
        let sub = format!("{root}/videos");
        let v = find_videos(&[&root, &sub]);
        assert_eq!(v.iter().map(|e| e.name()).collect::<Vec<_>>(), vec!["b.mp4", "c.webm"]);
        assert_eq!(v[0].bytes, 2048);

        let mut ui = Ui::new();
        assert!(texts(&ui.nodes(240, 360, None)).contains(&"No videos in /home or /home/videos".to_string()));
        ui.videos = v;
        let t = texts(&ui.nodes(240, 360, None));
        assert!(t.contains(&"2 videos. Tap one to play.".to_string()) && t.contains(&"b.mp4".to_string()), "{t:?}");
        assert_eq!(ui.pointer_down(ROW_HIT + 1, 0, 240, 360), Action::Open(format!("{sub}/c.webm")));
        assert_eq!(ui.pointer_down(REFRESH_HIT, 0, 240, 360), Action::Refresh);
        assert_eq!(ui.key(0x28), Action::Open(format!("{root}/b.mp4")));
    }

    #[test]
    fn a_long_library_pages() {
        let mut ui = Ui::new();
        ui.videos = (0..30).map(|i| Entry { path: format!("/home/v{i:02}.mp4"), bytes: 1 }).collect();
        let fit = Ui::rows_fit(200);
        let t = texts(&ui.nodes(320, 200, None));
        assert!(t.contains(&format!("1-{fit} of 30")), "{t:?}");
        assert!(!t.contains(&"Prev".to_string()) && t.contains(&"Next".to_string()));
        ui.pointer_down(NEXT_HIT, 0, 320, 200);
        assert_eq!(ui.offset, fit);
        assert!(texts(&ui.nodes(320, 200, None)).contains(&format!("v{fit:02}.mp4")));
        ui.pointer_down(PREV_HIT, 0, 320, 200);
        assert_eq!(ui.offset, 0);
        ui.scroll(1, 200);
        assert_eq!(ui.offset, 1);
    }

    #[test]
    fn playing_controls_fit_phone_and_terminal_sizes() {
        let mut ui = Ui::new();
        ui.screen = Screen::Playing;
        ui.play = Playback {
            path: "/home/videos/a-rather-long-video-name-that-will-not-fit.mp4".into(),
            pos_ms: 30_000,
            duration_ms: 60_000,
            playing: true,
            video_w: 320,
            video_h: 180,
            sound: "sound".into(),
            ..Default::default()
        };
        for (w, h) in [(240, 364), (320, 200), (640, 400), (480, 270), (120, 180)] {
            let n = ui.nodes(w, h, Some(1));
            for s in &n {
                if let Some((x, y, bw, bh)) = bounds(s) {
                    if s.key == "knob" {
                        continue; // straddles the track's ends by design
                    }
                    assert!(x >= 0 && y >= 0 && x + bw <= w && y + bh <= h, "{w}x{h}: {} at {x},{y} {bw}x{bh}", s.key);
                }
            }
            let pic = n.iter().find(|s| s.key == "picture").expect("picture");
            let (_, py, _, ph) = bounds(pic).unwrap();
            assert!(py >= TOP_H && py + ph <= h - CONTROLS_H, "{w}x{h}: the picture stays between the bars");
            let t = texts(&n);
            assert!(t.contains(&"Pause".to_string()) && t.contains(&"0:30 / 1:00".to_string()), "{w}x{h}: {t:?}");
        }
        // The seek bar: a press at its middle is half way.
        let s = play_layout(240, 364).seek;
        assert_eq!(ui.pointer_down(SEEK_HIT, s.0 + s.2 / 2, 240, 364), Action::SeekTo(0.5));
        assert_eq!(ui.pointer_down(SEEK_HIT, -50, 240, 364), Action::SeekTo(0.0));
        assert_eq!(ui.pointer_down(PLAY_HIT, 0, 240, 364), Action::TogglePlay);
        assert_eq!(ui.pointer_down(FWD10_HIT, 0, 240, 364), Action::Skip(10_000));
        assert_eq!(ui.pointer_down(LIBRARY_HIT, 0, 240, 364), Action::Library);
        assert_eq!(ui.key(0x2C), Action::TogglePlay);
        assert_eq!(ui.key(0x50), Action::Skip(-10_000));
        // Without a canvas yet there is no picture node.
        assert!(!ui.nodes(240, 364, None).iter().any(|s| s.key == "picture"));
        ui.play.playing = false;
        ui.play.ended = true;
        let t = texts(&ui.nodes(240, 364, Some(1)));
        assert!(t.contains(&"Play".to_string()) && t.contains(&"Ended. Play starts it again.".to_string()));
    }
}
