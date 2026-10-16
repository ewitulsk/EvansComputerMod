//! `videoplayer` — the Video Player app (`dev.ecm.video`), a system app on
//! every computer: the Terminal desktop and the ePhone. One window: the video
//! in a `canvas` node (the host decodes it with FFmpeg, `video_decode_frame`,
//! straight into RGB565 and the frames go in with `ui_put_pixels`) with a seek
//! bar and buttons under it; its sound goes to the computer's speaker. The
//! screens are in `lib.rs`.
//!
//!   videoplayer [file]       without a file: the library (videos in /home, /home/videos)
//!
//! Files opens videos with it ("open with", `.mp4 .m4v .mkv .webm .mov .avi`).
//!
//! Controls: tap or click the buttons and the seek bar; keys Space (or K)
//! play/pause, Left/Right 10 s, L loop, Esc/Backspace the library; in hand on
//! an ePhone, hotkey slots 1 = play/pause, 2 = back 10 s, 3 = forward 10 s.
//! Leaving it (home, another app, the phone to the hotbar) pauses it; it saves
//! the file and position (`SaveState`) and opens there again, paused, after a
//! cold start (`Restore`).

use ecm_host_abi::video::{self, DecodeStep, PixelFormat};
use ecm_ui::diff::TreeState;
use ecm_ui::{ConnError, Connection, Json};
use ecm_ui_proto as proto;
use ecm_ui_proto::Message;
use std::time::{Duration, Instant};
use videoplayer::{
    decode_size, find_videos, fit, frame_timing, play_layout, Action, AudioPump, AudioSink, AudioSource, Clock, FrameTiming,
    Playback, Screen, Ui, APP_ID, AUDIO_CHANNELS, AUDIO_RATE, CANVAS, LIBRARY_DIRS,
};

const SURFACE: u32 = 1;

const HOTKEYS: [(u8, &str); 3] = [(1, "Play/Pause"), (2, "Back 10s"), (3, "Forward 10s")];

struct Client {
    conn: Box<dyn Connection>,
    out: Vec<u8>,
    pending: std::collections::VecDeque<Message>,
}

impl Client {
    fn send(&mut self, m: Message) {
        m.encode(&mut self.out);
    }

    fn flush(&mut self) -> Result<(), ConnError> {
        if self.out.is_empty() {
            return Ok(());
        }
        let b = std::mem::take(&mut self.out);
        self.conn.send(&b)
    }

    fn recv(&mut self, timeout_ms: i32) -> Result<Vec<Message>, ConnError> {
        self.flush()?;
        if self.pending.is_empty() {
            let b = self.conn.recv(timeout_ms)?;
            self.pending.extend(Message::decode_all(&b).map_err(|_| ConnError::Protocol)?);
        }
        Ok(self.pending.drain(..).collect())
    }
}

struct HostAudio(video::Handle);

impl AudioSource for HostAudio {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        video::read_audio(self.0, buf).unwrap_or(0)
    }
}

struct Speaker {
    dev: ecm_audio::Device,
    failed: bool,
}

impl AudioSink for Speaker {
    fn write(&mut self, pcm: &[u8]) {
        if !self.failed && self.dev.write_bytes(pcm).is_err() {
            self.failed = true;
        }
    }
}

fn open_speaker() -> Option<Speaker> {
    let mut d = ecm_audio::Device::open(None).ok()?;
    d.set_format(AUDIO_RATE, 16, AUDIO_CHANNELS).ok()?;
    d.set_latency_ms(400).ok()?;
    Some(Speaker { dev: d, failed: false })
}

/// An open video.
struct Player {
    handle: video::Handle,
    path: String,
    info: video::VideoInfo,
    /// Decode size and the canvas scale.
    size: (i32, i32, i32),
    buf: Vec<u8>,
    /// The pts of the frame in `buf` that isn't shown yet.
    pending: Option<i64>,
    /// Show the next frame even while paused (after open or a seek).
    show_next: bool,
    eof: bool,
    last_shown: i64,
    frame_ms: i64,
    clock: Clock,
    audio: Option<(HostAudio, AudioPump)>,
    speaker: Option<Speaker>,
    sound: String,
}

impl Player {
    fn duration(&self) -> i64 {
        self.info.duration_ms as i64
    }

    fn seek(&mut self, now: i64, to: i64) {
        let d = self.duration();
        let to = if d > 0 { to.clamp(0, (d - 1).max(0)) } else { to.max(0) };
        let _ = video::seek(self.handle, to);
        self.pending = None;
        self.eof = false;
        self.show_next = true;
        self.last_shown = to;
        self.clock.seek(now, to);
        if let Some((_, pump)) = self.audio.as_mut() {
            pump.reset(to);
        }
        if let Some(s) = self.speaker.as_mut() {
            let _ = s.dev.flush_queue();
        }
    }

    fn pause(&mut self, now: i64) {
        self.clock.pause(now);
        if let Some(s) = self.speaker.as_mut() {
            let _ = s.dev.flush_queue();
        }
    }

    /// Where playback is, for the controls.
    fn pos(&self, now: i64) -> i64 {
        let p = self.clock.pos(now);
        let d = self.duration();
        if d > 0 {
            p.min(d)
        } else {
            p
        }
    }

    /// The end: decoded everything and the clock passed the last frame.
    fn at_end(&self, now: i64) -> bool {
        self.eof && self.pending.is_none() && self.clock.pos(now) >= self.last_shown + self.frame_ms
    }

    /// Close the decoder; the speaker is kept for the next video.
    fn close(mut self) -> Option<Speaker> {
        let _ = video::close(self.handle);
        if let Some(s) = self.speaker.as_mut() {
            let _ = s.dev.flush_queue();
        }
        self.speaker.take()
    }
}

/// Open `path` for a picture area of `area` (w, h) at `start` ms.
fn open_player(path: &str, area: (i32, i32), start: i64, playing: bool, now: i64, speaker: Option<Speaker>) -> Result<Player, String> {
    // A first, tiny open for the video's own size (the decode size depends on it).
    let probe = video::open(path, 16, 16, PixelFormat::Rgb565).map_err(|_| format!("Can't open {path}"))?;
    let info = video::get_info(probe).map_err(|_| format!("Can't read {path}"));
    let _ = video::close(probe);
    let info = info?;
    let size = picture_size(area, &info);
    let handle = video::open(path, size.0, size.1, PixelFormat::Rgb565).map_err(|_| format!("Can't open {path}"))?;
    let info = video::get_info(handle).unwrap_or(info);
    let fps = info.fps();
    let frame_ms = if fps > 0.5 { (1000.0 / fps).round() as i64 } else { 33 };
    let mut speaker = speaker;
    let (audio, sound) = match video::audio_source(handle) {
        None => (None, "no sound"),
        Some(_) => {
            if speaker.is_none() {
                speaker = open_speaker();
            }
            match speaker {
                None => (None, "no speaker"),
                Some(_) if video::audio_open(handle, AUDIO_RATE, AUDIO_CHANNELS).is_ok() => {
                    (Some((HostAudio(handle), AudioPump::new(AUDIO_RATE, AUDIO_CHANNELS))), "sound")
                }
                Some(_) => (None, "no sound"),
            }
        }
    };
    let mut p = Player {
        handle,
        path: path.to_string(),
        info,
        size,
        buf: vec![0; (size.0 * size.1 * 2) as usize],
        pending: None,
        show_next: true,
        eof: false,
        last_shown: 0,
        frame_ms,
        clock: Clock::new(now, 0, playing),
        audio,
        speaker,
        sound: sound.to_string(),
    };
    if start > 0 {
        p.seek(now, start);
    }
    println!(
        "{APP_ID}: {} ({}x{}, {:.1} fps, {} ms) decoded at {}x{} x{}, {}",
        path, info.width, info.height, fps, info.duration_ms, size.0, size.1, size.2, p.sound
    );
    Ok(p)
}

/// The decode size and canvas scale for a video in a picture area of `area` (w, h).
fn picture_size(area: (i32, i32), info: &video::VideoInfo) -> (i32, i32, i32) {
    let fitted = fit((0, 0, area.0.max(2), area.1.max(2)), info.width as i32, info.height as i32);
    decode_size(fitted.2, fitted.3)
}

/// The video area's size in a window of `size`.
fn picture_area(size: (i32, i32)) -> (i32, i32) {
    let l = play_layout(size.0, size.1);
    (l.video_area.2, l.video_area.3)
}

/// Paths from `OpenFile` or arguments may come without the leading slash.
fn abs(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn main() {
    let conn = match ecm_ui::conn::default_connection(proto::host::ROLE_APP) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("videoplayer: {e} (start the GUI, or use `video play` on the console)");
            std::process::exit(1);
        }
    };
    let mut c = Client { conn, out: Vec::new(), pending: Default::default() };
    c.send(Message::Hello(proto::Hello { version: proto::VERSION, role: proto::Role::App, capabilities: vec![] }));
    let mut restore: Option<Vec<u8>> = None;
    let mut welcomed = false;
    let mut wants_restore = false;
    let t0 = Instant::now();
    while (!welcomed || (wants_restore && restore.is_none())) && t0.elapsed() < Duration::from_secs(5) {
        for m in c.recv(5000).unwrap_or_default() {
            match m {
                Message::Welcome(w) => {
                    welcomed = true;
                    wants_restore = w.restore_follows;
                }
                Message::Restore(r) => restore = Some(r.data),
                other => c.pending.push_back(other),
            }
        }
    }
    if !welcomed {
        eprintln!("videoplayer: no answer from the compositor");
        std::process::exit(1);
    }
    let start = Instant::now();
    let now_ms = || start.elapsed().as_millis() as i64;

    // What to open first: an argument, else what was open before a cold start (paused).
    let mut open_req: Option<(String, i64, bool)> = std::env::args().nth(1).map(|p| (abs(&p), 0, true));
    if open_req.is_none() {
        let restored = restore.and_then(|b| String::from_utf8(b).ok()).and_then(|s| Json::parse(&s).ok());
        if let Some(j) = &restored {
            if let Some(p) = j.get("path").and_then(|v| v.as_str()) {
                let pos = match j.get("pos") {
                    Some(Json::Int(v)) => *v,
                    _ => 0,
                };
                open_req = Some((p.to_string(), pos, false));
            }
        }
    }

    c.send(Message::SurfaceCreate(proto::SurfaceCreate {
        surface: SURFACE,
        kind: proto::SurfaceKind::Tree,
        role: proto::SurfaceRole::Window,
        layer: proto::Layer::None,
        parent: 0,
        x: 0,
        y: 0,
        w: 0,
        h: 0,
        format: proto::PixelFormat::None,
        scale: 1,
        title: "Video Player".into(),
    }));
    for (slot, label) in HOTKEYS {
        c.send(Message::HotkeyClaim(proto::HotkeyClaim { slot, label: label.into() }));
    }

    let mut ui = Ui::new();
    ui.videos = find_videos(&LIBRARY_DIRS);
    let mut tree = TreeState::new();
    let mut size = (0, 0);
    let mut serial: Option<u32> = None;
    let mut dirty = true;
    let mut player: Option<Player> = None;
    let mut spare_speaker: Option<Speaker> = None;
    let mut canvas: Option<(i32, i32, i32)> = None;
    let mut looping = false;
    let mut ended = false;
    let mut shown_second = -1i64;
    let mut quit = false;

    while !quit {
        let now = now_ms();
        // Open what was asked for once the window has its size.
        if size.0 > 0 {
            if let Some((path, pos, playing)) = open_req.take() {
                if let Some(p) = player.take() {
                    spare_speaker = p.close();
                }
                match open_player(&path, picture_area(size), pos, playing, now, spare_speaker.take()) {
                    Ok(p) => {
                        ui.play = Playback {
                            path: p.path.clone(),
                            duration_ms: p.duration(),
                            playing,
                            looping,
                            video_w: p.info.width as i32,
                            video_h: p.info.height as i32,
                            sound: p.sound.clone(),
                            pos_ms: pos,
                            ..Default::default()
                        };
                        ui.screen = Screen::Playing;
                        ui.error.clear();
                        ended = false;
                        player = Some(p);
                    }
                    Err(e) => {
                        ui.error = e;
                        ui.screen = Screen::Library;
                    }
                }
                dirty = true;
            }
        }
        // The canvas follows the decode size (opened, reopened after a resize).
        if let Some(p) = player.as_mut() {
            if canvas != Some(p.size) {
                if canvas.is_some() {
                    c.send(Message::ResourceDestroy(proto::ResourceDestroy { id: CANVAS }));
                }
                c.send(Message::ResourceCreate(proto::ResourceCreate {
                    id: CANVAS,
                    kind: proto::ResourceKind::Canvas,
                    w: p.size.0,
                    h: p.size.1,
                    format: proto::PixelFormat::Rgb565,
                }));
                canvas = Some(p.size);
                p.show_next = true;
                dirty = true;
            }
        }

        // Draw the window when something changed.
        if size.0 > 0 && (dirty || serial.is_some()) {
            if let Some(p) = &player {
                ui.play.pos_ms = p.pos(now);
                ui.play.playing = p.clock.playing();
                ui.play.looping = looping;
                ui.play.ended = ended;
            }
            let scale = canvas.filter(|_| ui.screen == Screen::Playing).map(|c| c.2);
            let nodes = ui.nodes(size.0, size.1, scale);
            let ops = tree.update(&nodes);
            if !ops.is_empty() {
                c.send(Message::TreeOps(proto::TreeOps { surface: SURFACE, ops }));
            }
            if let Some(s) = serial.take() {
                c.send(Message::AckConfigure(proto::AckConfigure { surface: SURFACE, serial: s }));
            }
            c.send(Message::Commit(proto::Commit { surface: SURFACE }));
            dirty = false;
        }

        // Wait for input until the next frame is due (20 ms at most while playing, for the sound).
        let wait = match &player {
            Some(p) if p.show_next => 0,
            Some(p) if p.clock.playing() => p.pending.map(|pts| (pts - p.clock.pos(now)).clamp(0, 20)).unwrap_or(0) as i32,
            _ => 500,
        };
        let msgs = match c.recv(wait) {
            Ok(m) => m,
            Err(_) => break,
        };
        let now = now_ms();
        for m in msgs {
            let action = match m {
                Message::Configure(cf) if cf.surface == SURFACE => {
                    let resized = size != (cf.w, cf.h);
                    size = (cf.w, cf.h);
                    serial = Some(cf.serial);
                    if resized && open_req.is_none() {
                        if let Some(p) = &player {
                            if picture_size(picture_area(size), &p.info) != p.size {
                                open_req = Some((p.path.clone(), p.pos(now), p.clock.playing()));
                            }
                        }
                    }
                    dirty = true;
                    Action::None
                }
                Message::Pointer(pt) => match pt.kind {
                    proto::PointerKind::Down => ui.pointer_down(pt.hit_id, pt.x, size.0, size.1),
                    proto::PointerKind::Scroll => {
                        ui.scroll(pt.scroll_y, size.1);
                        dirty = true;
                        Action::None
                    }
                    _ => Action::None,
                },
                Message::Key(k) if k.down => ui.key(k.key),
                Message::Hotkey(hk) if hk.down => match hk.slot {
                    1 => Action::TogglePlay,
                    2 => Action::Skip(-10_000),
                    3 => Action::Skip(10_000),
                    _ => Action::None,
                },
                Message::OpenFile(f) => Action::Open(abs(&f.path)),
                Message::WillSuspend(_) => {
                    // Leaving the screen pauses, as on a phone; the place is saved for a cold start.
                    let state = match player.as_mut() {
                        Some(p) => {
                            p.pause(now);
                            Json::Obj(vec![("path".into(), Json::Str(p.path.clone())), ("pos".into(), Json::Int(p.pos(now)))])
                        }
                        None => Json::Obj(vec![]),
                    };
                    c.send(Message::SaveState(proto::SaveState { data: state.to_string().into_bytes() }));
                    let _ = c.flush();
                    dirty = true;
                    Action::None
                }
                Message::CloseRequested(_) => {
                    quit = true;
                    Action::None
                }
                Message::Error(e) => {
                    eprintln!("videoplayer: {}", e.message);
                    quit = true;
                    Action::None
                }
                _ => Action::None,
            };
            match action {
                Action::None => continue,
                Action::Open(path) => open_req = Some((path, 0, true)),
                Action::Refresh => {
                    ui.videos = find_videos(&LIBRARY_DIRS);
                    ui.offset = 0;
                }
                Action::Library => {
                    if let Some(p) = player.take() {
                        spare_speaker = p.close();
                    }
                    if canvas.take().is_some() {
                        c.send(Message::ResourceDestroy(proto::ResourceDestroy { id: CANVAS }));
                    }
                    ui.screen = Screen::Library;
                    ui.videos = find_videos(&LIBRARY_DIRS);
                }
                Action::TogglePlay => {
                    if let Some(p) = player.as_mut() {
                        if p.clock.playing() {
                            p.pause(now);
                        } else {
                            if ended {
                                p.seek(now, 0);
                                ended = false;
                            }
                            p.clock.play(now);
                        }
                    }
                }
                Action::Skip(d) => {
                    if let Some(p) = player.as_mut() {
                        let to = p.pos(now) + d;
                        p.seek(now, to);
                        ended = false;
                    }
                }
                Action::SeekTo(f) => {
                    if let Some(p) = player.as_mut() {
                        let to = (p.duration() as f64 * f) as i64;
                        p.seek(now, to);
                        ended = false;
                    }
                }
                Action::ToggleLoop => looping = !looping,
            }
            dirty = true;
        }

        // Play: the sound ahead of the clock, then the frame that is due.
        let Some(p) = player.as_mut() else { continue };
        let now = now_ms();
        let pos = p.clock.pos(now);
        if p.clock.playing() {
            if let (Some((src, pump)), Some(spk)) = (p.audio.as_mut(), p.speaker.as_mut()) {
                pump.pump(pos, src, spk);
            }
        }
        let mut drops = 0;
        loop {
            if p.pending.is_none() && !p.eof {
                match video::decode_frame(p.handle, &mut p.buf) {
                    DecodeStep::Frame(pts) => p.pending = Some(pts),
                    DecodeStep::Eof => p.eof = true,
                    DecodeStep::Error => {
                        ui.error = "The video can't be decoded any further.".into();
                        p.eof = true;
                        dirty = true;
                    }
                }
            }
            let Some(pts) = p.pending else { break };
            let timing = if p.show_next {
                FrameTiming::Show
            } else if p.clock.playing() {
                frame_timing(pts, pos, p.frame_ms)
            } else {
                FrameTiming::Wait(0)
            };
            match timing {
                // Late: skip a few frames to catch up, then show one anyway.
                FrameTiming::Drop if drops < 8 => {
                    drops += 1;
                    p.pending = None;
                }
                FrameTiming::Show | FrameTiming::Drop => {
                    if c.flush().is_err() || c.conn.put_pixels(CANVAS, &p.buf, &[]).is_err() {
                        quit = true;
                        break;
                    }
                    c.send(Message::Commit(proto::Commit { surface: SURFACE }));
                    p.last_shown = pts;
                    p.pending = None;
                    p.show_next = false;
                    break;
                }
                FrameTiming::Wait(_) => break,
            }
        }
        if p.clock.playing() && p.at_end(now) {
            if looping {
                p.seek(now, 0);
            } else {
                p.pause(now);
                ended = true;
            }
            dirty = true;
        }
        // The controls move once a second while playing.
        let second = p.pos(now) / 1000;
        if second != shown_second {
            shown_second = second;
            dirty = true;
        }
    }
    if let Some(p) = player.take() {
        p.close();
    }
    let _ = c.flush();
}
