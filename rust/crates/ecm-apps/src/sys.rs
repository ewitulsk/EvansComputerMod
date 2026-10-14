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
