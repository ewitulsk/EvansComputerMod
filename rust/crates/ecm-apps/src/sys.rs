//! The computer as the apps see it: files (std::fs under a root, `/` on the
//! computer), the clock, Wi-Fi (the `wifi` program's own steps), cellular
//! modems, the display panel and kernel shell sessions. [`HostSys`] is the
//! real computer (wasm32); [`MockSys`] is a scripted one for tests and for
//! recording golden sessions.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// One directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
}

/// What wlan0 is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WifiState {
    /// A Wi-Fi Module is in a bay (wlan0 exists).
    pub present: bool,
    /// Why Wi-Fi can't be used (no module, controller mode, monitor mode), with the fix.
    pub problem: Option<String>,
    pub connected: bool,
    pub ssid: String,
    pub signal_dbm: Option<i32>,
    pub address: Option<String>,
}

/// A network a scan heard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    pub signal_dbm: i32,
    pub channel: u32,
    /// `open`, `wpa2-psk` or `unsupported`.
    pub security: String,
}

impl Network {
    pub fn secured(&self) -> bool {
        self.security != "open"
    }

    /// Signal as 0-4 bars.
    pub fn bars(&self) -> u8 {
        bars(self.signal_dbm)
    }
}

/// dBm to 0-4 bars.
pub fn bars(dbm: i32) -> u8 {
    match dbm {
        d if d >= -55 => 4,
        d if d >= -67 => 3,
        d if d >= -75 => 2,
        d if d >= -85 => 1,
        _ => 0,
    }
}

/// A cellular modem as `celld` (Cellular v2) reports it; state `no daemon` for a modem without celld.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CellState {
    /// The modem's attachment (`module_1`, `left_bay_1`).
    pub name: String,
    pub tech: String,
    pub state: String,
    pub message: String,
    /// The SIM's identity: carrier and IMSI (never anything secret).
    pub sim: String,
    pub carrier: String,
    pub bars: u8,
    pub signal: String,
    /// `cell 2, channel 1`.
    pub cell: String,
    /// `100.65.2.2/24 via 100.65.2.1`.
    pub address: String,
    /// `sent 2.0 KB, received 1.2 KB, 1 handover`.
    pub data: String,
    pub roaming: bool,
    /// Cells heard: (text, serving).
    pub cells: Vec<(String, bool)>,
}

impl CellState {
    /// What Settings shows for celld's `status` and `cells` answers.
    pub fn from_celld(s: &celld::ctl::Status, cells: &[celld::ctl::CellRow]) -> CellState {
        let on = s.on_cell() || s.state == "attaching";
        CellState {
            name: if s.modem.is_empty() { "modem".into() } else { s.modem.clone() },
            tech: s.profile.clone(),
            state: s.state.clone(),
            message: s.message.clone(),
            sim: if s.imsi.is_empty() { "none".into() } else { format!("{} (IMSI {})", s.sim_carrier, s.imsi) },
            carrier: s.carrier.clone(),
            bars: s.bars,
            signal: match (on, s.rssi) {
                (true, Some(r)) => format!(
                    "{r:.1} dBm, SINR {} dB, CQI {}",
                    s.sinr.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into()),
                    s.cqi.map(|c| c.to_string()).unwrap_or_else(|| "-".into())
                ),
                _ => "-".into(),
            },
            cell: match s.cell {
                Some(c) if on => format!("cell {c}, channel {}", s.channel),
                _ => format!("none (channel {})", s.channel),
            },
            address: if s.ip.is_empty() { "none".into() } else { format!("{} via {}", s.ip, s.gateway) },
            data: format!(
                "sent {}, received {}, {} handover{}",
                celld::ctl::human_bytes(s.tx_bytes),
                celld::ctl::human_bytes(s.rx_bytes),
                s.handovers,
                if s.handovers == 1 { "" } else { "s" }
            ),
            roaming: s.roaming,
            cells: cells
                .iter()
                .map(|c| {
                    let note = if c.note.is_empty() { String::new() } else { format!(" ({})", c.note) };
                    (format!("{} cell {}, ch {}, {:.0} dBm{}", c.carrier, c.cell, c.channel, c.rssi, note), c.serving)
                })
                .collect(),
        }
    }
}

/// The installed display panel (`gfx_modes` / `gfx_info`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DisplayState {
    pub tier: u32,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub refresh_hz: u32,
    pub text_cols: u32,
    pub text_rows: u32,
    pub colors: String,
    pub touch: bool,
}

impl DisplayState {
    pub fn tier_name(&self) -> &'static str {
        match self.tier {
            1 => "Phosphor (Tier 1)",
            2 => "Color LCD (Tier 2)",
            3 => "HD (Tier 3)",
            _ => "none",
        }
    }
}

/// A speaker (output) or microphone (input) attached to the computer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SoundDevice {
    /// Attachment name (`top`, `left_bay_1`, `speaker` / `microphone` on an ePhone).
    pub name: String,
    pub input: bool,
    /// Output volume or input gain, 0-100.
    pub volume: i64,
}

/// A Bluetooth device: paired with this computer, or heard advertising nearby.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtDevice {
    /// `AC:DE:48:00:11:22`.
    pub address: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
    /// A connection is being made.
    pub connecting: bool,
    /// In pairing mode (an AirPods case with its lid open).
    pub pairable: bool,
    pub rssi_dbm: Option<i64>,
    pub battery: Option<i64>,
}

/// The computer's Bluetooth radio (an ePhone's `bluetooth` peripheral).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BtState {
    /// There is a Bluetooth radio at all.
    pub present: bool,
    pub powered: bool,
    pub name: String,
    pub address: String,
    /// Where the sound goes: `headphones` or `speaker`.
    pub audio: String,
    /// How the last pair / connect went.
    pub last: String,
    /// Paired devices.
    pub devices: Vec<BtDevice>,
    /// Devices nearby that can be paired (not paired yet).
    pub nearby: Vec<BtDevice>,
}

/// Everything an app needs from the computer.
pub trait Sys {
    /// Where `/` is (the computer's disk; a temp dir in tests).
    fn root(&self) -> PathBuf;
    /// A monotonic clock.
    fn now_ms(&mut self) -> u64;
    /// Wall-clock hours and minutes, if the computer has a clock.
    fn clock(&mut self) -> Option<(u32, u32)>;
    fn wifi(&mut self) -> WifiState;
    /// Scan; `Err` is a rendered problem with its fix.
    fn wifi_scan(&mut self) -> Result<Vec<Network>, String>;
    /// Join and get an address, reporting each step; `Ok` = the final line.
    fn wifi_connect(&mut self, ssid: &str, password: Option<&str>, progress: &mut dyn FnMut(&str)) -> Result<String, String>;
    fn wifi_disconnect(&mut self) -> Result<(), String>;
    fn cells(&mut self) -> Vec<CellState>;
    /// A celld request (`up`, `down`, `roaming on|off`): its answer, or why celld can't be reached.
    fn cell_command(&mut self, cmd: &str) -> Result<String, String>;
    fn display(&mut self) -> Option<DisplayState>;
    /// A kernel shell session (the Terminal app): id, or -1.
    fn session_open(&mut self) -> i32;
    /// Up to `buf.len()` bytes of output: n, 0 for none, -1 closed.
    fn session_read(&mut self, id: i32, buf: &mut [u8]) -> i32;
    fn session_write(&mut self, id: i32, data: &[u8]) -> i32;
    fn session_resize(&mut self, id: i32, cols: u16, rows: u16);
    /// 0 running, 1 exited, -1 invalid.
    fn session_status(&mut self, id: i32) -> i32;
    fn session_close(&mut self, id: i32);
    /// Attached speakers and microphones (Settings > Sound).
    fn sound_devices(&mut self) -> Vec<SoundDevice> {
        Vec::new()
    }
    /// Set a speaker's volume or a microphone's gain (0-100).
    fn sound_set_volume(&mut self, _device: &SoundDevice, _volume: u8) -> Result<(), String> {
        Err("no sound devices".into())
    }
    /// The peak level (dBFS) a microphone heard since the last call (it records while asked); None if it can't be read.
    fn mic_level(&mut self, _name: &str) -> Option<i32> {
        None
    }
    /// Stop reading microphones for [`mic_level`](Self::mic_level).
    fn mic_level_stop(&mut self) {}
    /// The Bluetooth radio, its paired devices and what is nearby (`present: false` without one).
    fn bluetooth(&mut self) -> BtState {
        BtState::default()
    }
    /// `pair`, `connect`, `forget` (with an address), `disconnect`, `power on`, `power off`.
    fn bt_command(&mut self, _cmd: &str, _address: &str) -> Result<(), String> {
        Err("This computer has no Bluetooth radio.".into())
    }
    /// The upload port (files from the player's real computer, abi `[imports.upload]`):
    /// 1 ready, 0 no add-on on the server, 2 not in the player's game, 3 not permitted.
    fn upload_supported(&mut self) -> i32 {
        0
    }
    /// Ask the player for a file to put in `dir`: a ticket, or why not.
    fn upload_begin(&mut self, _dir: &str) -> Result<i32, String> {
        Err("upload isn't available".into())
    }
    fn upload_poll(&mut self, _ticket: i32) -> UploadState {
        UploadState::Failed("upload isn't available".into())
    }
    fn upload_cancel(&mut self, _ticket: i32) {}
}

/// Where an upload is ([`Sys::upload_poll`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadState {
    /// The player is picking a file in their game.
    Picking,
    /// Received, total bytes.
    Receiving(u64, u64),
    /// The new file's path.
    Done(String),
    /// Why there is no file.
    Failed(String),
}

// ------------------------------------------------------------------ files

/// `path` (absolute on the computer) under `root`.
pub fn host_path(root: &Path, path: &str) -> PathBuf {
    let rel = path.trim_start_matches('/');
    if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    }
}

/// Join a directory and a name (`/` + `a` = `/a`).
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The parent of an absolute path (`/` stays `/`).
pub fn parent(path: &str) -> String {
    let p = path.trim_end_matches('/');
    match p.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => p[..i].to_string(),
    }
}

/// The last part of a path.
pub fn file_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// Make `p` absolute against `cwd` and remove `.` / `..` parts.
pub fn normalize(cwd: &str, p: &str) -> String {
    let full = if p.starts_with('/') { p.to_string() } else { join(cwd, p) };
    let mut parts: Vec<&str> = Vec::new();
    for s in full.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

pub fn list_dir(sys: &dyn Sys, path: &str) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(host_path(&sys.root(), path)).map_err(|e| e.to_string())?;
    let mut out: Vec<Entry> = rd
        .filter_map(|e| e.ok())
        .map(|e| {
            let dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            Entry { name: e.file_name().to_string_lossy().into_owned(), dir, size }
        })
        .collect();
    // Folders first, then by name (case-insensitive).
    out.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(out)
}

pub fn read_file(sys: &dyn Sys, path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(host_path(&sys.root(), path)).map_err(|e| e.to_string())
}

pub fn write_file(sys: &dyn Sys, path: &str, data: &[u8]) -> Result<(), String> {
    std::fs::write(host_path(&sys.root(), path), data).map_err(|e| e.to_string())
}

pub fn is_dir(sys: &dyn Sys, path: &str) -> bool {
    host_path(&sys.root(), path).is_dir()
}

pub fn exists(sys: &dyn Sys, path: &str) -> bool {
    host_path(&sys.root(), path).exists()
}

pub fn make_dir(sys: &dyn Sys, path: &str) -> Result<(), String> {
    std::fs::create_dir(host_path(&sys.root(), path)).map_err(|e| e.to_string())
}

pub fn remove(sys: &dyn Sys, path: &str) -> Result<(), String> {
    let p = host_path(&sys.root(), path);
    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(p).map_err(|e| e.to_string())
    }
}

pub fn rename(sys: &dyn Sys, from: &str, to: &str) -> Result<(), String> {
    std::fs::rename(host_path(&sys.root(), from), host_path(&sys.root(), to)).map_err(|e| e.to_string())
}

/// Copy a file or a folder (recursively).
pub fn copy(sys: &dyn Sys, from: &str, to: &str) -> Result<(), String> {
    fn go(a: &Path, b: &Path, depth: u32) -> std::io::Result<()> {
        if depth > 32 {
            return Err(std::io::Error::other("folders nested too deep"));
        }
        if a.is_dir() {
            std::fs::create_dir_all(b)?;
            for e in std::fs::read_dir(a)? {
                let e = e?;
                go(&e.path(), &b.join(e.file_name()), depth + 1)?;
            }
            Ok(())
        } else {
            std::fs::copy(a, b).map(|_| ())
        }
    }
    let (a, b) = (host_path(&sys.root(), from), host_path(&sys.root(), to));
    if b.starts_with(&a) && a.is_dir() {
        return Err("can't copy a folder into itself".into());
    }
    go(&a, &b, 0).map_err(|e| e.to_string())
}

/// A free name like `name (copy)`, `name (copy 2)` in `dir`.
pub fn free_name(sys: &dyn Sys, dir: &str, name: &str) -> String {
    if !exists(sys, &join(dir, name)) {
        return name.to_string();
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 1..1000 {
        let cand = if n == 1 { format!("{stem} (copy){ext}") } else { format!("{stem} (copy {n}){ext}") };
        if !exists(sys, &join(dir, &cand)) {
            return cand;
        }
    }
    format!("{stem} (copy){ext}")
}

/// Human file size.
pub fn human_size(n: u64) -> String {
    match n {
        n if n < 1024 => format!("{n} B"),
        n if n < 1024 * 1024 => format!("{:.1} KB", n as f64 / 1024.0),
        n => format!("{:.1} MB", n as f64 / (1024.0 * 1024.0)),
    }
}

// ------------------------------------------------------------------ the real computer

#[cfg(target_arch = "wasm32")]
pub use host::HostSys;

#[cfg(target_arch = "wasm32")]
mod host {
    use super::*;
    use ecm_host_abi::ipc;
    use ecm_host_abi::peripheral::{self, Value};

    /// The computer this program runs on.
    pub struct HostSys;

    thread_local! {
        /// Microphones Settings > Sound is reading for its level meter (its own capture streams).
        static METERS: RefCell<Vec<(String, std::fs::File)>> = const { RefCell::new(Vec::new()) };
    }

    fn ctl_value(path: &str, key: &str) -> Option<i64> {
        let text = std::fs::read_to_string(path).ok()?;
        text.lines().find_map(|l| {
            let mut it = l.split_whitespace();
            (it.next()? == key).then(|| it.next()?.parse().ok()).flatten()
        })
    }

    fn get<'a>(m: &'a Value, key: &str) -> Option<&'a Value> {
        match m {
            Value::Map(entries) => entries.iter().find(|(k, _)| matches!(k, Value::Str(s) if s == key)).map(|(_, v)| v),
            _ => None,
        }
    }

    fn text(m: &Value, key: &str) -> String {
        match get(m, key) {
            Some(Value::Str(s)) => s.clone(),
            Some(Value::Int(i)) => i.to_string(),
            Some(Value::Float(f)) => {
                if f.fract() == 0.0 && f.abs() < 1e12 {
                    format!("{}", *f as i64)
                } else {
                    format!("{:.1}", f)
                }
            }
            Some(Value::Bool(b)) => b.to_string(),
            _ => String::new(),
        }
    }

    impl Sys for HostSys {
        fn upload_supported(&mut self) -> i32 {
            use ecm_host_abi::upload::Support;
            match ecm_host_abi::upload::supported() {
                Support::Ready => 1,
                Support::NotInstalled => 0,
                Support::NoClient => 2,
                Support::NotPermitted => 3,
            }
        }

        fn upload_begin(&mut self, dir: &str) -> Result<i32, String> {
            ecm_host_abi::upload::begin(dir).map(|t| t.0).map_err(|e| e.message().to_string())
        }

        fn upload_poll(&mut self, ticket: i32) -> UploadState {
            use ecm_host_abi::upload::State;
            match ecm_host_abi::upload::poll(ecm_host_abi::upload::Ticket(ticket)) {
                State::Picking => UploadState::Picking,
                State::Receiving { received, total } => UploadState::Receiving(received, total),
                State::Done(p) => UploadState::Done(p),
                State::Failed(e) => UploadState::Failed(e.message().to_string()),
            }
        }

        fn upload_cancel(&mut self, ticket: i32) {
            ecm_host_abi::upload::cancel(ecm_host_abi::upload::Ticket(ticket));
        }

        fn root(&self) -> PathBuf {
            PathBuf::from("/")
        }

        fn now_ms(&mut self) -> u64 {
            ecm_ui::conn::monotonic_ms()
        }

        fn clock(&mut self) -> Option<(u32, u32)> {
            let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
            Some((((s / 3600) % 24) as u32, ((s / 60) % 60) as u32))
        }

        fn wifi(&mut self) -> WifiState {
            let mut c = wifi::system::Computer;
            if let Err(p) = wifi::system::ready(&mut c) {
                let present = p.what.contains("monitor mode") || p.what.contains("controller mode");
                return WifiState { present, problem: Some(p.render()), ..Default::default() };
            }
            let st = wifi::system::status_text().unwrap_or_default();
            let kv = |k: &str| ecm_host_abi::wifi::kv(&st, k).map(String::from);
            let associated = kv("state").as_deref() == Some("ASSOCIATED");
            let wpa2 = kv("target_security").as_deref() == Some("wpa2");
            let authorized = kv("authorized").as_deref() == Some("1");
            let ssid = kv("ssid")
                .and_then(|h| ecm_host_abi::wifi::unhex(&h))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            WifiState {
                present: true,
                problem: None,
                connected: associated && (!wpa2 || authorized),
                ssid,
                signal_dbm: kv("signal").and_then(|s| s.parse().ok()),
                address: wifi::system::address(),
            }
        }

        fn wifi_scan(&mut self) -> Result<Vec<Network>, String> {
            let found = wifi::system::scan_networks().map_err(|p| p.render())?;
            let mut out: Vec<Network> = Vec::new();
            for e in found {
                let ssid = e.ssid_str();
                if ssid.is_empty() {
                    continue;
                }
                // One row per network: the strongest access point.
                match out.iter_mut().find(|n| n.ssid == ssid) {
                    Some(n) if n.signal_dbm >= e.signal_dbm => {}
                    Some(n) => {
                        n.signal_dbm = e.signal_dbm;
                        n.channel = e.channel;
                    }
                    None => out.push(Network { ssid, signal_dbm: e.signal_dbm, channel: e.channel, security: e.security.clone() }),
                }
            }
            out.sort_by_key(|n| -n.signal_dbm);
            Ok(out)
        }

        fn wifi_connect(&mut self, ssid: &str, password: Option<&str>, progress: &mut dyn FnMut(&str)) -> Result<String, String> {
            let mut warnings = Vec::new();
            let r = wifi::system::connect(ssid, password, progress, &mut |w: &str| warnings.push(w.to_string()));
            for w in warnings {
                progress(&w);
            }
            r.map_err(|p| p.render())
        }

        fn wifi_disconnect(&mut self) -> Result<(), String> {
            wifi::system::disconnect(&mut |_| {}).map_err(|p| p.render())
        }

        fn cells(&mut self) -> Vec<CellState> {
            // Cellular v2: celld (on a phone PhoneUi runs it; on a Terminal the user does).
            if let Ok(st) = celld::host::query("status", 800) {
                let rows = celld::host::query("cells", 800).map(|c| celld::ctl::parse_cells(&c)).unwrap_or_default();
                return vec![CellState::from_celld(&celld::ctl::Status::parse(&st), &rows)];
            }
            let Ok(list) = peripheral::list() else { return Vec::new() };
            let mut out = Vec::new();
            for (name, ty) in &list {
                if ty == "framed_radio" {
                    out.push(CellState {
                        name: name.clone(),
                        tech: "framed".into(),
                        state: "no daemon".into(),
                        message: "celld is not running: type `celld &` in a Terminal".into(),
                        sim: "-".into(),
                        signal: "-".into(),
                        ..Default::default()
                    });
                }
            }
            out
        }

        fn cell_command(&mut self, cmd: &str) -> Result<String, String> {
            celld::host::query(cmd, 1500)
        }

        fn display(&mut self) -> Option<DisplayState> {
            use ecm_host_abi::gfx_child as gfx;
            use ecm_host_abi::video::Target;
            let m = gfx::modes(Target::Terminal)?;
            let info = gfx::info(Target::Terminal);
            let mut colors = Vec::new();
            if m.color_flags & gfx::COLOR_TWO_TONE != 0 {
                colors.push(format!("two-tone #{:06x}", m.tone_rgb));
            }
            if m.color_flags & gfx::COLOR_RGBA != 0 {
                colors.push("RGBA".to_string());
            } else if m.color_flags & gfx::COLOR_RGB565 != 0 {
                colors.push("RGB565".to_string());
            } else if m.color_flags & gfx::COLOR_INDEXED8 != 0 {
                colors.push("256 colours".to_string());
            }
            Some(DisplayState {
                tier: m.tier,
                width: m.max_width,
                height: m.max_height,
                scale: m.scale.max(1),
                refresh_hz: info.map(|i| i.refresh_hz).filter(|r| *r > 0).unwrap_or(m.refresh_hz),
                text_cols: m.text_cols,
                text_rows: m.text_rows,
                colors: colors.join(", "),
                touch: m.touch,
            })
        }

        fn session_open(&mut self) -> i32 {
            ipc::spawn_shell("user")
        }
        fn session_read(&mut self, id: i32, buf: &mut [u8]) -> i32 {
            ipc::session_read(id, buf)
        }
        fn session_write(&mut self, id: i32, data: &[u8]) -> i32 {
            ipc::session_write(id, data)
        }
        fn session_resize(&mut self, id: i32, cols: u16, rows: u16) {
            ipc::session_resize(id, cols, rows);
        }
        fn session_status(&mut self, id: i32) -> i32 {
            ipc::session_status(id)
        }
        fn session_close(&mut self, id: i32) {
            ipc::session_close(id);
        }

        fn sound_devices(&mut self) -> Vec<SoundDevice> {
            let mut out = Vec::new();
            for (name, ty) in peripheral::list().unwrap_or_default() {
                let input = match ty.as_str() {
                    "speaker" => false,
                    "microphone" => true,
                    _ => continue,
                };
                let volume = if input {
                    ctl_value(&format!("/dev/micctl.{name}"), "gain")
                } else {
                    ctl_value(&format!("/dev/audioctl.{name}"), "volume")
                };
                out.push(SoundDevice { name, input, volume: volume.unwrap_or(0) });
            }
            out.sort_by(|a, b| a.input.cmp(&b.input).then_with(|| a.name.cmp(&b.name)));
            out
        }

        fn sound_set_volume(&mut self, d: &SoundDevice, volume: u8) -> Result<(), String> {
            let (path, cmd) = if d.input {
                (format!("/dev/micctl.{}", d.name), format!("gain {}\n", volume.min(100)))
            } else {
                (format!("/dev/audioctl.{}", d.name), format!("volume {}\n", volume.min(100)))
            };
            std::fs::write(&path, cmd).map_err(|e| format!("{path}: {e}"))
        }

        fn mic_level(&mut self, name: &str) -> Option<i32> {
            use std::io::Read;
            METERS.with(|m| {
                let mut m = m.borrow_mut();
                if !m.iter().any(|(n, _)| n == name) {
                    let f = std::fs::File::open(format!("/dev/mic.{name}")).ok()?;
                    m.push((name.to_string(), f));
                }
                let ctl = format!("/dev/micctl.{name}");
                let rate = ctl_value(&ctl, "rate").unwrap_or(48000).max(8000) as usize;
                let frame = (ctl_value(&ctl, "bits").unwrap_or(16) as usize / 8) * ctl_value(&ctl, "channels").unwrap_or(1).max(1) as usize;
                let file = &mut m.iter_mut().find(|(n, _)| n == name)?.1;
                // 50 ms (waits the first time), then whatever this stream has captured since.
                let mut peak = -99;
                let mut want = rate / 20;
                for _ in 0..4 {
                    let mut buf = vec![0u8; want.max(1) * frame];
                    let n = file.read(&mut buf).ok()?;
                    let samples: Vec<i16> = if frame % 2 == 0 {
                        buf[..n].chunks_exact(frame).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
                    } else {
                        buf[..n].chunks_exact(frame).map(|b| ((b[0] as i16) - 128) << 8).collect()
                    };
                    peak = peak.max(ecm_audio::peak_dbfs(&samples));
                    let left = ctl_value(&ctl, "buffered_ms").unwrap_or(0) as usize;
                    if left < 20 {
                        break;
                    }
                    want = (rate * left / 1000).min(rate / 2);
                }
                Some(peak)
            })
        }

        fn mic_level_stop(&mut self) {
            METERS.with(|m| m.borrow_mut().clear());
        }

        fn bluetooth(&mut self) -> BtState {
            let Ok(st) = peripheral::call("bluetooth", "status", &[]) else { return BtState::default() };
            let int = |m: &Value, k: &str| match get(m, k) {
                Some(Value::Int(i)) => Some(*i),
                Some(Value::Float(f)) => Some(*f as i64),
                _ => None,
            };
            let flag = |m: &Value, k: &str| matches!(get(m, k), Some(Value::Bool(true)));
            let mut out = BtState {
                present: true,
                powered: flag(&st, "powered"),
                name: text(&st, "name"),
                address: text(&st, "address"),
                audio: text(&st, "audio"),
                last: text(&st, "last"),
                ..Default::default()
            };
            if let Ok(Value::List(devs)) = peripheral::call("bluetooth", "devices", &[]) {
                for d in &devs {
                    out.devices.push(BtDevice {
                        address: text(d, "address"),
                        name: text(d, "name"),
                        paired: true,
                        connected: flag(d, "connected"),
                        connecting: flag(d, "connecting"),
                        battery: int(d, "battery"),
                        ..Default::default()
                    });
                }
            }
            if let Ok(Value::List(seen)) = peripheral::call("bluetooth", "scan", &[]) {
                for d in &seen {
                    if flag(d, "paired") {
                        continue;
                    }
                    out.nearby.push(BtDevice {
                        address: text(d, "address"),
                        name: text(d, "name"),
                        pairable: flag(d, "pairable"),
                        rssi_dbm: int(d, "rssi_dbm"),
                        battery: int(d, "battery"),
                        ..Default::default()
                    });
                }
            }
            out
        }

        fn bt_command(&mut self, cmd: &str, address: &str) -> Result<(), String> {
            let (method, args) = match cmd {
                "power on" => ("set_powered", vec![Value::Bool(true)]),
                "power off" => ("set_powered", vec![Value::Bool(false)]),
                "disconnect" => ("disconnect", vec![]),
                m => (m, vec![Value::Str(address.to_string())]),
            };
            peripheral::call("bluetooth", method, &args).map(|_| ()).map_err(|e| match e {
                peripheral::Error::Peripheral(m) => m,
                peripheral::Error::Unavailable => "This computer has no Bluetooth radio.".into(),
                peripheral::Error::Malformed(m) => m.to_string(),
            })
        }
    }
}

// ------------------------------------------------------------------ a scripted computer

/// What a [`MockSys`] knows and what was done to it. Clones share it.
#[derive(Default, Debug)]
pub struct MockState {
    pub root: PathBuf,
    pub now: u64,
    pub clock: Option<(u32, u32)>,
    pub wifi: WifiState,
    pub networks: Vec<Network>,
    /// (ssid, password) that joins; anything else fails like a wrong password.
    pub join_ok: Vec<(String, Option<String>)>,
    pub cells: Vec<CellState>,
    pub display: Option<DisplayState>,
    /// Session output waiting to be read.
    pub session_out: Vec<u8>,
    /// Everything written to the session.
    pub session_in: Vec<u8>,
    pub session_size: Option<(u16, u16)>,
    pub session_exited: bool,
    pub log: Vec<String>,
    /// Speakers and microphones.
    pub sound: Vec<SoundDevice>,
    /// What each microphone hears (dBFS) for the level meter.
    pub mic_levels: Vec<(String, i32)>,
    /// The Bluetooth radio.
    pub bluetooth: BtState,
    /// What `upload_supported` answers (1 = ready).
    pub upload_support: i32,
    /// Polls of the running upload answer these in turn (the last one repeats).
    pub upload_script: Vec<UploadState>,
}

#[derive(Clone, Default)]
pub struct MockSys {
    pub state: Rc<RefCell<MockState>>,
}

impl MockSys {
    /// A mock computer whose disk is `root`.
    pub fn new(root: impl Into<PathBuf>) -> MockSys {
        let m = MockSys::default();
        m.state.borrow_mut().root = root.into();
        m
    }
}

impl Sys for MockSys {
    fn upload_supported(&mut self) -> i32 {
        self.state.borrow().upload_support
    }
    fn upload_begin(&mut self, dir: &str) -> Result<i32, String> {
        let mut s = self.state.borrow_mut();
        s.log.push(format!("upload {dir}"));
        if s.upload_support != 1 {
            return Err("upload isn't available".into());
        }
        Ok(7)
    }
    fn upload_poll(&mut self, _ticket: i32) -> UploadState {
        let mut s = self.state.borrow_mut();
        if s.upload_script.len() > 1 {
            s.upload_script.remove(0)
        } else {
            s.upload_script.first().cloned().unwrap_or(UploadState::Picking)
        }
    }
    fn upload_cancel(&mut self, ticket: i32) {
        self.state.borrow_mut().log.push(format!("upload cancel {ticket}"));
    }
    fn root(&self) -> PathBuf {
        self.state.borrow().root.clone()
    }
    fn now_ms(&mut self) -> u64 {
        self.state.borrow().now
    }
    fn clock(&mut self) -> Option<(u32, u32)> {
        self.state.borrow().clock
    }
    fn wifi(&mut self) -> WifiState {
        self.state.borrow().wifi.clone()
    }
    fn wifi_scan(&mut self) -> Result<Vec<Network>, String> {
        let s = self.state.borrow();
        if let Some(p) = &s.wifi.problem {
            return Err(p.clone());
        }
        Ok(s.networks.clone())
    }
    fn wifi_connect(&mut self, ssid: &str, password: Option<&str>, progress: &mut dyn FnMut(&str)) -> Result<String, String> {
        progress(&format!("Looking for '{ssid}'..."));
        let ok = self.state.borrow().join_ok.iter().any(|(s, p)| s == ssid && p.as_deref() == password);
        let mut s = self.state.borrow_mut();
        s.log.push(format!("connect {ssid}"));
        if !ok {
            return Err(format!("wifi: the password for '{ssid}' is wrong (the Access Point rejected it)\n"));
        }
        progress(&format!("Joined '{ssid}'. Getting an address..."));
        s.wifi.connected = true;
        s.wifi.ssid = ssid.to_string();
        s.wifi.signal_dbm = Some(-58);
        s.wifi.address = Some("192.168.60.23/24".into());
        Ok("Connected. Address 192.168.60.23/24, router 192.168.60.1, DNS 1.1.1.1".into())
    }
    fn wifi_disconnect(&mut self) -> Result<(), String> {
        let mut s = self.state.borrow_mut();
        s.wifi.connected = false;
        s.wifi.address = None;
        Ok(())
    }
    fn cells(&mut self) -> Vec<CellState> {
        self.state.borrow().cells.clone()
    }
    fn cell_command(&mut self, cmd: &str) -> Result<String, String> {
        let mut s = self.state.borrow_mut();
        s.log.push(format!("cell {cmd}"));
        let Some(c) = s.cells.first_mut().filter(|c| c.state != "no daemon") else { return Err("celld is not running".into()) };
        match cmd {
            "roaming on" | "roaming off" => {
                c.roaming = cmd == "roaming on";
                Ok(format!("ok {cmd}\n"))
            }
            "down" => {
                c.state = "down".into();
                c.message = "switched off (cell up to attach)".into();
                Ok("ok detaching\n".into())
            }
            "up" => {
                c.state = "searching".into();
                Ok("ok searching\n".into())
            }
            _ => Ok(format!("error unknown command '{cmd}'\n")),
        }
    }
    fn display(&mut self) -> Option<DisplayState> {
        self.state.borrow().display.clone()
    }
    fn session_open(&mut self) -> i32 {
        self.state.borrow_mut().log.push("session open".into());
        1
    }
    fn session_read(&mut self, _id: i32, buf: &mut [u8]) -> i32 {
        let mut s = self.state.borrow_mut();
        if s.session_out.is_empty() {
            return if s.session_exited { -1 } else { 0 };
        }
        let n = buf.len().min(s.session_out.len());
        buf[..n].copy_from_slice(&s.session_out[..n]);
        s.session_out.drain(..n);
        n as i32
    }
    fn session_write(&mut self, _id: i32, data: &[u8]) -> i32 {
        self.state.borrow_mut().session_in.extend_from_slice(data);
        data.len() as i32
    }
    fn session_resize(&mut self, _id: i32, cols: u16, rows: u16) {
        self.state.borrow_mut().session_size = Some((cols, rows));
    }
    fn session_status(&mut self, _id: i32) -> i32 {
        if self.state.borrow().session_exited {
            1
        } else {
            0
        }
    }
    fn session_close(&mut self, _id: i32) {
        self.state.borrow_mut().log.push("session close".into());
    }
    fn sound_devices(&mut self) -> Vec<SoundDevice> {
        self.state.borrow().sound.clone()
    }
    fn sound_set_volume(&mut self, d: &SoundDevice, volume: u8) -> Result<(), String> {
        let mut s = self.state.borrow_mut();
        s.log.push(format!("{} {} {}", if d.input { "gain" } else { "volume" }, d.name, volume));
        match s.sound.iter_mut().find(|x| x.name == d.name && x.input == d.input) {
            Some(x) => {
                x.volume = volume as i64;
                Ok(())
            }
            None => Err(format!("no device {}", d.name)),
        }
    }
    fn mic_level(&mut self, name: &str) -> Option<i32> {
        self.state.borrow().mic_levels.iter().find(|(n, _)| n == name).map(|(_, l)| *l)
    }
    fn bluetooth(&mut self) -> BtState {
        self.state.borrow().bluetooth.clone()
    }
    fn bt_command(&mut self, cmd: &str, address: &str) -> Result<(), String> {
        let mut s = self.state.borrow_mut();
        s.log.push(format!("bt {cmd} {address}").trim_end().to_string());
        if !s.bluetooth.present {
            return Err("This computer has no Bluetooth radio.".into());
        }
        let bt = &mut s.bluetooth;
        match cmd {
            "power on" | "power off" => bt.powered = cmd == "power on",
            "pair" => {
                let i = bt.nearby.iter().position(|d| d.address == address).ok_or("not in range: open its case lid next to the phone")?;
                let mut d = bt.nearby.remove(i);
                if !d.pairable {
                    bt.nearby.insert(i, d);
                    return Err("it isn't in pairing mode: open its case lid".into());
                }
                d.paired = true;
                d.connected = true;
                bt.devices.push(d);
                bt.audio = "headphones".into();
                bt.last = "connected".into();
            }
            "connect" => {
                let d = bt.devices.iter_mut().find(|d| d.address == address).ok_or("not paired")?;
                d.connected = true;
            }
            "disconnect" => {
                for d in bt.devices.iter_mut() {
                    d.connected = false;
                }
                bt.audio = "speaker".into();
            }
            "forget" => bt.devices.retain(|d| d.address != address),
            _ => return Err(format!("unknown command {cmd}")),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/a", "b"), "/a/b");
        assert_eq!(parent("/a/b"), "/a");
        assert_eq!(parent("/a"), "/");
        assert_eq!(parent("/"), "/");
        assert_eq!(file_name("/a/b.txt"), "b.txt");
        assert_eq!(normalize("/a/b", "../c/./d"), "/a/c/d");
        assert_eq!(normalize("/", "/x/.."), "/");
        assert_eq!(bars(-50), 4);
        assert_eq!(bars(-90), 0);
        assert_eq!(human_size(2048), "2.0 KB");
    }
}
