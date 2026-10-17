//! Host tests of the Lua modules: a real Lua 5.2 with [`install`] run exactly
//! as the `lua` program runs it, against mock UI connections, sample files and
//! stand-ins for the host-ABI modules (`_peripheral`, `shell`), plus the rule
//! that Python and Lua offer the same modules (AGENTS.md).

use super::*;
use ecm_dsp::iq::SampleFormat;
use ecm_dsp::modem::fm::{FmMod, FmParams};
use ecm_ui::conn::MockConnection;
use ecm_ui::samples::{self, click};
use ecm_ui_proto::Message;
use std::path::PathBuf;

fn lua() -> Lua {
    let lua = Lua::new();
    install(&lua).expect("install");
    lua
}

/// Run `code`; panics with Lua's message and traceback on an error.
fn run(lua: &Lua, code: &str) {
    if let Err(e) = lua.load(code).set_name("=test").exec() {
        panic!("lua raised:\n{e}\nin:\n{code}");
    }
}

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ecm-lua-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn p(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn set(lua: &Lua, name: &str, value: impl mlua::IntoLua) {
    lua.globals().set(name, value).unwrap();
}

fn f32_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

// ---------------------------------------------------------------- ui

/// The §13 Notes app in Lua, line for line the same app as samples::NOTES_PY.
const NOTES_LUA: &str = r#"
local ui = require("ui")

local app = ui.App("dev.evan.notes")
local notes = app:restore().notes or ui.array()
local entry = ui.TextField{placeholder = "New note"}
local function labels()
  local t = {}
  for i, n in ipairs(notes) do t[i] = ui.Label(n) end
  return t
end
local rows = ui.List{items = labels()}

local function add()
  if entry.text ~= "" then
    notes[#notes + 1] = entry.text
    entry.text = ""
    rows.items = labels()
  end
end

app:window("Notes", ui.Column{gap = 8, padding = 12, children = {
  ui.Row{gap = 8, children = {entry,
    ui.Button("Add", {on_click = add})}},
  rows,
}})
app:hotkey(1, "Add note", add)
app:on_suspend(function() return {notes = notes} end)
app:run()
"#;

fn first_difference(a: &[u8], b: &[u8]) -> String {
    let ma = Message::decode_all(a).expect("rust stream decodes");
    let mb = Message::decode_all(b).expect("lua stream decodes");
    for (i, (x, y)) in ma.iter().zip(mb.iter()).enumerate() {
        if x != y {
            return format!("message {i} differs:\n  rust: {}\n  lua:  {}", x.to_json(), y.to_json());
        }
    }
    format!("rust sent {} messages, lua {}", ma.len(), mb.len())
}

/// The same app in Rust and in Lua plays the same scripted session and sends
/// the same bytes (as the Python ui test checks for Python).
#[test]
fn lua_and_rust_notes_apps_send_identical_bytes() {
    let dry = MockConnection::new();
    for chunk in samples::notes_session(0, 0).into_iter().take(2) {
        dry.push_bytes(chunk);
    }
    let mut app = samples::notes(Box::new(dry));
    app.step(0);
    app.step(0);
    let w = app.core.window(1).unwrap();
    let (field, add) = (w.hit_id_of("entry").unwrap(), w.hit_id_of("/0column/0row/1button").unwrap());
    let session = samples::notes_session(field, add);

    let conn = MockConnection::new();
    for chunk in session.clone() {
        conn.push_bytes(chunk);
    }
    samples::notes(Box::new(conn.clone())).run();
    let rust = conn.sent();

    let lm = MockConnection::new();
    for chunk in session {
        lm.push_bytes(chunk);
    }
    let handle = ui_native::register_mock(lm.clone());
    let l = lua();
    set(&l, "MOCK", handle);
    run(&l, "require('_ui').use_mock(MOCK)");
    run(&l, NOTES_LUA);
    run(&l, "require('_ui').use_mock(nil)");
    let lua_bytes = lm.sent();

    assert!(rust == lua_bytes, "{}", first_difference(&rust, &lua_bytes));
    let msgs = Message::decode_all(&rust).unwrap();
    let tree_ops = msgs.iter().filter(|m| matches!(m, Message::TreeOps(_))).count();
    assert!(tree_ops >= 6, "only {tree_ops} TreeOps messages");
    let saved = msgs.iter().find_map(|m| if let Message::SaveState(s) = m { Some(s.data.clone()) } else { None }).unwrap();
    assert_eq!(String::from_utf8(saved).unwrap(), "{\"notes\":[\"first\",\"milk\",\"eggs \u{e9}\"]}");
}

fn hit(app: i64, id: &str) -> u32 {
    ui_native::with_core(app, |c| c.window(1).unwrap().hit_id_of(id)).flatten().unwrap_or_else(|| panic!("no hit id for {id}"))
}

#[test]
fn lua_widgets_follow_input_and_push_changes() {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let handle = ui_native::register_mock(mock.clone());
    let l = lua();
    set(&l, "MOCK", handle);
    run(
        &l,
        r#"
_ui = require("_ui")
_ui.use_mock(MOCK)
log = {}
app = ui.App("dev.test.widgets")
name = ui.TextField{id = "name", placeholder = "Name", on_submit = function(e) log[#log + 1] = "submit:" .. e.value end}
agree = ui.Checkbox("Agree", {id = "agree", on_change = function(e) log[#log + 1] = "agree:" .. tostring(e.value) end})
vol = ui.Slider(0, 10, {id = "vol", width = 110})
tabs = ui.Tabs({"One", "Two"}, {ui.Label("page one"), ui.Label("page two")}, {id = "tabs"})
pick = ui.List({"a", "b", "c"}, {id = "pick"})
win = app:window("Widgets", ui.Column{gap = 4, children = {name, agree, vol, tabs, pick}})
APP = app._h
"#,
    );
    let app_h: i64 = l.globals().get("APP").unwrap();
    let step = |msgs: Vec<Message>| {
        mock.push(&msgs);
        run(&l, "app:step(0)\napp:step(0)");
    };
    step(vec![Message::Configure(ecm_ui_proto::Configure { surface: 1, serial: 1, w: 400, h: 300, states: ecm_ui_proto::window_states::ACTIVATED })]);
    step(click(hit(app_h, "agree"), 0, 0));
    step(click(hit(app_h, "name"), 0, 0));
    step(vec![Message::Text(ecm_ui_proto::Text { surface: 1, text: "Ann".into() })]);
    step(vec![Message::Key(ecm_ui_proto::Key { surface: 1, key: 0x28, mods: 0, down: true, repeat: false })]);
    step(vec![Message::Key(ecm_ui_proto::Key { surface: 1, key: 0x2B, mods: 0, down: true, repeat: false })]);
    step(click(hit(app_h, "tabs") + 1, 0, 0));
    step(click(hit(app_h, "pick") + 2, 0, 0));
    run(
        &l,
        r#"
assert(agree.checked == true, tostring(agree.checked))
assert(name.text == "Ann", name.text)
assert(tabs.selected == 1, tostring(tabs.selected))
assert(pick.selected == 2, tostring(pick.selected))
assert(#log == 2 and log[1] == "agree:true" and log[2] == "submit:Ann", table.concat(log, ","))
-- Changing values from Lua reaches the screen.
name.text = ""
agree.checked = false
vol.value = 7
app:step(0)
"#,
    );
    let state = ui_native::with_core(app_h, |c| {
        let w = c.window(1).unwrap();
        (w.state("name").unwrap().text.clone(), w.state("agree").unwrap().checked, w.state("vol").unwrap().value)
    })
    .unwrap();
    assert_eq!(state, (String::new(), false, 7));
    assert!(mock.sent_messages().iter().any(|m| matches!(m, Message::TreeOps(_))));
    run(
        &l,
        r#"
local text = _ui.json_dumps({a = {1, 2.5, true}, b = 'x"y', c = ui.array(), d = {}})
assert(text == '{"a":[1,2.5,true],"b":"x\\"y","c":[],"d":{}}', text)
local v = _ui.json_loads('{"n": [1, "two"], "e": []}')
assert(v.n[1] == 1 and v.n[2] == "two" and _ui.json_dumps(v.e) == "[]")
assert(next(app:restore()) == nil)
-- an unknown option is an error
assert(not pcall(ui.Label, "x", {colour = 1}))
_ui.use_mock(nil)
"#,
    );
}

#[test]
fn lua_loads_ui_json_layouts_and_reports_console_mode() {
    let dir = tmpdir("uijson");
    let path = dir.join("notes.ui.json");
    std::fs::write(
        &path,
        r#"{ "window": "Notes",
  "root": { "column": { "gap": 8, "padding": 12, "children": [
    { "row": { "gap": 8, "children": [
      { "textfield": { "id": "entry", "placeholder": "New note" } },
      { "button": { "id": "add", "text": "Add", "on_click": "add_note" } } ] } },
    { "list": { "id": "rows" } } ] } } }"#,
    )
    .unwrap();
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let handle = ui_native::register_mock(mock.clone());
    let l = lua();
    set(&l, "MOCK", handle);
    set(&l, "PATH", p(&path));
    run(
        &l,
        r#"
require("_ui").use_mock(MOCK)
hits = {}
app, win = ui.load(PATH, {add_note = function(e) hits[#hits + 1] = e.kind end})
APP = app._h
assert(app:layout():get("add", "text") == "Add")
"#,
    );
    let app_h: i64 = l.globals().get("APP").unwrap();
    mock.push(&[Message::Configure(ecm_ui_proto::Configure { surface: 1, serial: 1, w: 400, h: 300, states: 0 })]);
    run(&l, "app:step(0)\napp:step(0)");
    mock.push(&click(hit(app_h, "add"), 0, 0));
    run(
        &l,
        r#"
app:step(0)
assert(#hits == 1 and hits[1] == "click", #hits)
app:layout():set("add", {text = 2})
assert(app:layout():value("add") == "2")
require("_ui").use_mock(nil)
-- No compositor: a clear error.
local ok, e = pcall(ui.App, "dev.test.console")
assert(not ok and tostring(e):find("console mode"), tostring(e))
"#,
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- radio

fn goertzel(x: &[f32], f: f64, fs: f64) -> f64 {
    let w = std::f64::consts::TAU * f / fs;
    let (mut re, mut im) = (0.0, 0.0);
    for (n, &v) in x.iter().enumerate() {
        re += v as f64 * (w * n as f64).cos();
        im -= v as f64 * (w * n as f64).sin();
    }
    (re * re + im * im) / (x.len() as f64).powi(2)
}

/// A synthetic 1 kHz NBFM recording read through radio.open (the SDR path),
/// demodulated by sdr .. fm_demod .. lowpass .. wav_sink: the WAV holds the tone.
#[test]
fn lua_fm_demod_of_synthetic_iq_gives_the_tone() {
    let d = tmpdir("fm");
    let fs = 48_000.0f32;
    let audio: Vec<f32> = (0..48_000).map(|n| 0.8 * (std::f32::consts::TAU * 1000.0 * n as f32 / fs).sin()).collect();
    let iq = FmMod::new(FmParams { fs, deviation: 5_000.0, tau: 0.0 }).modulate_vec(&audio);
    let mut bytes = Vec::new();
    SampleFormat::Cs16.encode(&iq, &mut bytes);
    std::fs::write(d.join("sdr0"), &bytes).unwrap();
    std::fs::write(d.join("sdrctl0"), "tier sdr_standard\nfreq 146520000\nrate 48000\nformat cs16\ntimestamp 0\n").unwrap();
    let out = d.join("out.wav");
    let l = lua();
    set(&l, "DATA", p(&d.join("sdr0")));
    set(&l, "CTL", p(&d.join("sdrctl0")));
    set(&l, "OUT", p(&out));
    run(
        &l,
        r#"
local sdr = radio.open{data_path = DATA, ctl_path = CTL}
assert(sdr.rate == 48000 and sdr.format == "cs16" and sdr.freq == 146520000)
local fg = radio.Flowgraph(sdr .. radio.fm_demod(5e3) .. radio.lowpass(3e3) .. radio.wav_sink(OUT))
assert(fg:describe() == "fm_demod(5000 Hz) >> lowpass(3000 Hz)", fg:describe())
fg:run{samples = 48000}
assert(fg.consumed == 48000, fg.consumed)
"#,
    );
    let wav = std::fs::read(&out).unwrap();
    let a = ecm_radio::audio::read_audio(&wav, 0).unwrap();
    assert_eq!((a.rate, a.samples.len()), (48_000, 48_000));
    let x = &a.samples[4800..];
    let (p1, p2) = (goertzel(x, 1000.0, 48e3), goertzel(x, 2000.0, 48e3));
    assert!(p1 > 1000.0 * p2, "1 kHz {p1:e}, 2 kHz {p2:e}");
    let amp = (p1 * 4.0).sqrt();
    assert!((amp - 0.8).abs() < 0.05, "amplitude {amp}");
    let _ = std::fs::remove_dir_all(&d);
}

/// AX.25 frames -> afsk1200 -> NBFM -> .cf32 + SigMF, then back: both frames intact.
#[test]
fn lua_afsk1200_round_trip_through_the_radio_api() {
    let d = tmpdir("afsk");
    let l = lua();
    set(&l, "IQ", p(&d.join("packet.cf32")));
    run(
        &l,
        r#"
local msgs = {radio.ax25("APRS", "N0CALL-1", "hello from ecm"), radio.ax25("CQ", "N0CALL-1", string.rep("second frame ", 4))}
local tx = radio.Flowgraph(radio.frames_source(msgs, {rate = 48000}) .. radio.afsk1200() .. radio.fm_mod(3e3)
  .. radio.iq_sink(IQ, {freq = 144.39e6, description = "afsk test"}))
tx:run()
local src = radio.file_source(IQ)
assert(src.rate == 48000 and src.freq == 144390000, src.rate .. " " .. src.freq)
local got = {}
radio.Flowgraph(src .. radio.fm_demod(3e3) .. radio.afsk1200()
  .. radio.frames{callback = function(f) got[#got + 1] = f end, echo = false}):run()
assert(#got == 2, #got)
local p = radio.parse_ax25(got[1])
assert(p.src == "N0CALL-1" and p.dest == "APRS" and p.text == "hello from ecm", p.text)
assert(radio.parse_ax25(got[2]).text == string.rep("second frame ", 4))
"#,
    );
    let meta = std::fs::read_to_string(d.join("packet.sigmf-meta")).unwrap();
    let m = ecm_radio::sigmf::Meta::parse(&meta).unwrap();
    assert_eq!((m.format, m.sample_rate, m.frequency), (SampleFormat::Cf32, 48_000.0, 144.39e6));
    assert_eq!(m.description, "afsk test");
    let _ = std::fs::remove_dir_all(&d);
}

/// Composition rules, sources, conversions and SDR control.
#[test]
fn lua_flowgraph_composition_and_sdr_control() {
    let d = tmpdir("comp");
    std::fs::write(d.join("ctl"), "rate 48000\nformat cf32\nfreq 100000000\ntimestamp 77\n").unwrap();
    std::fs::write(d.join("data"), b"").unwrap();
    let l = lua();
    set(&l, "DATA", p(&d.join("data")));
    set(&l, "CTL", p(&d.join("ctl")));
    run(
        &l,
        r#"
local function read(path) local f = assert(io.open(path, "rb")) local s = f:read("*a") f:close() return s end
local a = radio.tone(1000, {rate = 8000, seconds = 1.0}) .. radio.gain(0.5)
local b = a .. (radio.real() .. radio.lowpass(2000))
assert(#b.items == 4)
local fg = radio.Flowgraph(b)
assert(fg:describe() == "gain(0.5) >> real >> lowpass(2000 Hz)", fg:describe())
local s = fg:run():samples()
assert(s.kind == "real" and s:count() == 8000 and s.rate == 8000, tostring(s))
assert(#s == 8000)
local r = radio.Flowgraph(radio.pipeline(radio.tone(0, 48000, 0.5, 0.5), radio.resample(8000))):run():samples()
assert(r.rate == 8000 and math.abs(r:count() - 4000) <= 2)
for _, bad in ipairs({radio.tone(1000) .. radio.am_mod(), radio.tone(1000, {kind = "real"}) .. radio.fm_demod()}) do
  local ok, e = pcall(radio.Flowgraph, bad)
  assert(not ok and tostring(e):find("takes a"), tostring(e))
end
local ok, e = pcall(radio.Flowgraph, radio.lowpass(1000) .. radio.collect())
assert(not ok and tostring(e):find("RadioError"), tostring(e))
ok, e = pcall(radio.Flowgraph, radio.tone(100) .. radio.afsk1200() .. radio.frames())
assert(not ok and tostring(e):find("afsk"), tostring(e))
local l = radio.Samples.from_list({{1, 2}, {0, -0.5}}, 1000):to_list()
assert(l[1][1] == 1 and l[1][2] == 2 and l[2][1] == 0 and l[2][2] == -0.5)
assert(math.abs(radio.tone(0, {amplitude = 1.0, seconds = 0.01}):read(10):power_db()) < 1e-3)
assert(radio.parse_freq("144.39M") == 144390000)
local dp, cp = require("_radio").resolve_sdr("sdr_1")
assert(dp == "/dev/sdr1" and cp == "/dev/sdrctl1")
local sdr = radio.open{data_path = DATA, ctl_path = CTL}
assert(sdr.format == "cf32" and sdr:timestamp() == 77)
sdr:tune("146.52M", 24000)
assert(read(CTL):sub(1, 11) == "rate 24000\n", read(CTL))
assert(sdr.rate == 24000 and sdr.freq == 146520000)
sdr:tx(true, 20)
assert(read(CTL):sub(1, 9) == "tx on 20\n")
local fg2 = radio.Flowgraph(radio.tone(500, {rate = 48000, seconds = 0.1}) .. sdr)
assert(fg2:describe() == "resample(24000)", fg2:describe())
fg2:run()
assert(#read(DATA) == 2400 * 8, #read(DATA))
local hits = radio.find_signals(radio.tone(5000, {rate = 48000, amplitude = 0.5}):read(8192), {center = 1e6})
assert(#hits == 1 and math.abs(hits[1][1] - 1005000) < 100)
"#,
    );
    let _ = std::fs::remove_dir_all(&d);
}

// ---------------------------------------------------------------- peripherals

/// A stand-in for the host's `_peripheral` (the real one is the host ABI):
/// a speaker, a Wired Sensor Module with one lidar, two controllers.
const FAKE_PERIPHERALS: &str = r#"
calls = {}
local devices = {
  {"left", "speaker"}, {"left_bay_1", "wired_sensors"},
  {"controller_2", "xbox_controller"}, {"controller_1", "xbox_controller"},
}
local methods = {
  speaker = {"status", "play"},
  wired_sensors = {"names", "list", "mount", "configure", "get_config", "start", "stop", "get_seq", "scan", "get_scan", "get_points"},
  xbox_controller = {"get_state", "is_down", "get_axis", "get_buttons", "get_raw"},
}
local function type_of(name)
  for _, d in ipairs(devices) do if d[1] == name then return d[2] end end
end
local events = {
  {"redstone", "left"},
  {"controller_axis", "controller_1", "lx", -1},
  {"controller_button", "controller_1", "a", true},
}
package.loaded._peripheral = {
  list = function() return devices end,
  methods = function(name)
    local t = type_of(name)
    if not t then return false, "no peripheral '" .. name .. "'" end
    return true, t, methods[t]
  end,
  call = function(name, method, ...)
    calls[#calls + 1] = {name, method, ...}
    local t = type_of(name)
    if not t then return false, "no peripheral '" .. name .. "'" end
    if method == "status" then return true, "ok " .. name end
    if method == "names" then return true, {"lidar_1"} end
    if method == "list" then return true, {{name = "lidar_1", type = "lidar"}} end
    if method == "configure" then return true, select(2, ...) end
    if method == "scan" then return true, 1 end
    if method == "get_seq" then return true, 1 end
    if method == "get_scan" then
      return true, {seq = 1, start_tick = 0, end_tick = 2, rows = 1, columns = 3, hits = 2,
        config = {az_min = -10, az_max = 10, el_min = 0, el_max = 0}, ranges = RANGES}
    end
    if method == "get_points" then return true, {points = POINTS} end
    if method == "is_down" then return true, (...) == "a" end
    if method == "get_axis" then return true, 0.5 end
    return false, "no method " .. method
  end,
  wait_event = function(filter, ms)
    last_wait = {filter, ms}
    local e = table.remove(events, 1)
    if e then return (table.unpack or unpack)(e) end
  end,
}
"#;

#[test]
fn lua_peripheral_sensors_and_controller_modules() {
    let l = lua();
    set(&l, "RANGES", l.create_string(f32_bytes(&[3.0, f32::INFINITY, 1.5])).unwrap());
    set(&l, "POINTS", l.create_string(f32_bytes(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0])).unwrap());
    run(&l, FAKE_PERIPHERALS);
    run(
        &l,
        r#"
-- peripheral
local names = peripheral.names()
assert(#names == 4 and names[1] == "left")
assert(peripheral.attached()[2].type == "wired_sensors" and peripheral.attached()[2][1] == "left_bay_1")
assert(peripheral.is_present("left") and not peripheral.is_present("right"))
assert(peripheral.get_type("left") == "speaker" and peripheral.get_type("right") == nil)
assert(peripheral.get_methods("left")[1] == "status" and peripheral.get_methods("right") == nil)
local spk = peripheral.wrap("left")
assert(spk.type == "speaker" and spk.status() == "ok left" and spk:status() == "ok left")
assert(spk.call("status") == "ok left" and spk.methods()[2] == "play")
assert(peripheral.wrap("right") == nil)
local ok, e = pcall(peripheral.call, "right", "status")
assert(not ok and tostring(e):find("no peripheral"), tostring(e))
assert(peripheral.find("speaker").name == "left" and #peripheral.find_all("xbox_controller") == 2)
local ev, att = peripheral.pull_event("redstone", 2.5)
assert(ev == "redstone" and att == "left" and last_wait[1] == "redstone" and last_wait[2] == 2500)
-- controller (skips other events, sorted by player number)
local pads = controller.all()
assert(#pads == 2 and pads[1].name == "controller_1" and pads[1].player == 1)
assert(controller.find(2).name == "controller_2" and controller.find(3) == nil)
assert(pads[1]:is_down("a") and not pads[1]:is_down("b") and pads[1]:axis("lx") == 0.5)
local kind, name, axis, value = controller.pull_event()
assert(kind == "axis" and name == "controller_1" and axis == "lx" and value == -1)
local k2, _, button, down = controller.pull_event()
assert(k2 == "button" and button == "a" and down == true)
-- sensors
local hub = sensors.find()
assert(hub and hub:names()[1] == "lidar_1" and #hub:lidars() == 1)
assert(not pcall(hub.lidar, hub, "nope"))
local lidar = hub:lidar("lidar_1")
local cfg = lidar:configure{rows = 3}
assert(cfg.rows == 3)
local scan = lidar:scan()
assert(scan.seq == 1 and scan:range_at(0, 0) == 3 and scan:range_at(0, 1) == sensors.INF)
assert(scan:azimuth(0) == -10 and scan:azimuth(2) == 10 and scan:elevation(0) == 0)
local r, row, col = scan:nearest()
assert(r == 1.5 and row == 0 and col == 2)
local pts = lidar:points()
assert(#pts == 2 and pts[2][1] == 4 and pts[2][3] == 6)
"#,
    );
}

#[test]
fn lua_audio_writes_pcm_and_reads_status() {
    let d = tmpdir("audio");
    let l = lua();
    set(&l, "DIR", p(&d));
    run(
        &l,
        r#"
-- /dev/audio* lives in DIR for this test
local real_open = io.open
io.open = function(path, mode) return real_open((path:gsub("^/dev/", DIR .. "/")), mode) end
assert(math.abs(audio.note_freq("A4") - 440) < 1e-9)
assert(math.abs(audio.note_freq("C#5") - 554.365) < 1e-3)
assert(math.abs(audio.note_freq("Eb3") - 155.563) < 1e-3)
local spk = audio.open()
spk:tone(440, 0.01)
spk:notes("A4 -", 0.01)
spk:close()
-- a side's devices are /dev/audio.left and /dev/audioctl.left
local f = real_open(DIR .. "/audioctl.left", "wb") f:write("rate 8000\nbuffered_ms 40\nunderruns 0\n") f:close()
local side = audio.open("left")
local s = side:status()
side:close()
assert(s.rate == 8000 and s.buffered_ms == 40 and s.underruns == 0)
"#,
    );
    // 0.01 s of A4, then "A4" and a rest, each 0.009 s of tone + 0.001 s of silence, at 8 kHz 8-bit.
    let pcm = std::fs::read(d.join("audio")).unwrap();
    assert_eq!(pcm.len(), 80 + 2 * (72 + 8));
    assert_eq!(pcm[0], 178, "a square wave at volume 0.5 starts at 128 + 50");
    assert!(pcm.iter().all(|&b| b == 128 || b == 78 || b == 178), "{pcm:?}");
    let ctl = std::fs::read_to_string(d.join("audioctl")).unwrap();
    assert_eq!(ctl, "channels 1\n", "each control command is its own write");
    let _ = std::fs::remove_dir_all(&d);
}

// ---------------------------------------------------------------- cards

/// A `_peripheral` with one of each card device (a Card Reader/Writer, an NFC
/// Reader, a Card Printer); `calls` records every call.
const FAKE_CARDS: &str = r#"
calls = {}
state = { field = false }
local devices = { {"left", "card_reader"}, {"right", "nfc_reader"}, {"top", "card_printer"} }
local methods = {
  card_reader = {"present", "eject", "status", "identity", "read_stripe", "write_stripe", "write_fields",
    "write_bank", "verify_pin", "sign", "bank_blank", "nfc_auth", "nfc_read", "nfc_write", "nfc_set_key",
    "write", "is_blank", "respond"},
  nfc_reader = {"set_field", "field", "set_power", "poll", "select", "auth", "read", "write", "set_key", "raw", "rssi"},
  card_printer = {"present", "status", "print", "cost", "eject"},
}
local results = {
  present = true, eject = true,
  status = { present = true, released = false, auto_eject = true, state = "card", applets = {"bank", "nfc"} },
  identity = { kind = "hybrid", uid = "0102030405060708", applets = {"bank", "nfc"}, print = { name = "Ann" },
    bank = { issuer = "acme", pan = "4000123412341234", locked = false, ctr = 4, blank = false } },
  read_stripe = { t1 = "ANN", t2 = "12345678", t3 = "", corrupted = false },
  write_stripe = true, write_fields = true,
  write_bank = { ejected = true }, verify_pin = true, sign = { ctr = 5, tag = string.rep("ab", 16) }, bank_blank = false,
  nfc_auth = true, nfc_read = string.rep("00", 64), nfc_write = true, nfc_set_key = true,
  write = { imsi = "310150123456789", carrier = "cell-red", ejected = false },
  is_blank = true, respond = { res = string.rep("11", 16), ks = string.rep("22", 32) },
  set_field = true, set_power = true,
  poll = { { uid = "0102030405060708", kinds = {"nfc"} } }, select = true, auth = true,
  read = string.rep("11", 64), set_key = true, raw = "8100", rssi = -40,
  print = { dyes = { white = 1, black = 1 } }, cost = { dyes = { white = 1, black = 1 } },
}
local events = { {"redstone", "left"}, {"card_inserted", "left", "bank", {"bank"}}, {"card_removed", "left"} }
local function type_of(name)
  for _, d in ipairs(devices) do if d[1] == name then return d[2] end end
end
package.loaded._peripheral = {
  list = function() return devices end,
  methods = function(name)
    local t = type_of(name)
    if not t then return false, "no peripheral '" .. name .. "'" end
    return true, t, methods[t]
  end,
  call = function(name, method, ...)
    calls[#calls + 1] = {name, method, ...}
    local t = type_of(name)
    if not t then return false, "no peripheral '" .. name .. "'" end
    local known = false
    for _, m in ipairs(methods[t]) do if m == method then known = true end end
    if not known then return false, "no method " .. method end
    if method == "field" then return true, state.field end
    if method == "set_field" then state.field = (...) return true, true end
    if method == "write" and t == "nfc_reader" then return true, true end
    return true, results[method]
  end,
  wait_event = function(filter, ms)
    local e = table.remove(events, 1)
    if e then return table.unpack(e) end
  end,
}
"#;

/// docs/cards/NFC_MIX.md: every Mix, TAG and PIN-hash vector, through the
/// pure-Lua SHA-256 / HMAC the in-game interpreter relies on.
#[test]
fn lua_cards_scheme_vectors() {
    let l = lua();
    run(&l, FAKE_CARDS);
    run(
        &l,
        r#"
local F = cards.FACTORY_KEY
assert(F == "45434d2d4e46432d464143544f525930" and cards.unhex(F) == "ECM-NFC-FACTORY0")
for _, v in ipairs({
  {F, string.rep("00", 16), "3e2e68d5726a6858461251213df31d59"},
  {F, "000102030405060708090a0b0c0d0e0f", "e8938dca0b939828c0b5e18c388b9702"},
  {"00112233445566778899aabbccddeeff", "01020304050607081112131415161718", "f2668ac938caea225f16cf78a07cfe81"},
  {string.rep("00", 16), string.rep("00", 16), "5d4fb41a84bab1a9fa2416dcc38b9978"},
  {string.rep("ff", 16), string.rep("ff", 16), "355b79459dc55581fc26ab3eb2eb6ce6"},
}) do
  assert(cards.mix(v[1], v[2]) == v[3], v[1] .. " " .. v[2] .. " -> " .. cards.mix(v[1], v[2]))
end
assert(cards.auth_m1(F, string.rep("01", 8), string.rep("02", 8)) == cards.mix(F, string.rep("01", 8) .. string.rep("02", 8)))
assert(cards.auth_m2(F, string.rep("01", 8), string.rep("02", 8)) == cards.mix(F, string.rep("02", 8) .. string.rep("01", 8)))
-- SHA-256 / HMAC themselves (FIPS 180-4 / RFC 4231 known answers).
assert(cards.sha256("") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
assert(cards.sha256(cards.hex("abc")) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
assert(cards.sha256(cards.hex(string.rep("a", 1000))) == "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3")
assert(cards.hmac_sha256(string.rep("0b", 20), cards.hex("Hi There")) == "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")
assert(cards.hmac_sha256(string.rep("aa", 131), cards.hex("Test Using Larger Than Block-Size Key - Hash Key First")) == "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54")
local K1 = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
for _, v in ipairs({
  {K1, "0001020304050607", 1, 0, "aef6052d8232ad7c4db312596726e55a"},
  {K1, "0001020304050607", 2, 1500, "ce2e7e5ad8d1e0b8bd42aa2b8903e5b7"},
  {string.rep("00", 32), string.rep("00", 8), 0, 0, "c1e9e0459b7a23ecb6d54dc7772a25af"},
  -- 2^63-1 is beyond a Lua 5.2 number: a counter or amount can also be 16 hex digits (u64 big-endian).
  {string.rep("ff", 32), string.rep("ff", 8), "7fffffffffffffff", 2^40, "6987930a133eede5b3b6de9ef58f0e91"},
}) do
  local k, nonce, ctr, amount, want = table.unpack(v)
  assert(cards.tag(k, nonce, ctr, amount) == want, cards.tag(k, nonce, ctr, amount))
  assert(cards.verify_tag(k, nonce, ctr, amount, want) and cards.verify_tag(k, nonce, ctr, amount, want:upper()))
  assert(not cards.verify_tag(k, nonce, ctr, amount + 1, want) and not cards.verify_tag(k, nonce, ctr, amount, want:sub(1, -3)))
end
assert(cards.tag(K1, "0001020304050607", "0000000000000002", "00000000000005dc") == "ce2e7e5ad8d1e0b8bd42aa2b8903e5b7")
for _, v in ipairs({
  {"0102030405060708", "1234", "282fbfbe3f85ce96542b0435cc442d28745c372aaacf04b6229e657998169fd2"},
  {string.rep("00", 8), "00000000", "72a0521796bf77ccf576d8f5554d61b5a5cd371fad7492f3b64514b7e2a4cb19"},
}) do
  assert(cards.pin_hash(v[1], v[2]) == v[3], cards.pin_hash(v[1], v[2]))
end
assert(not pcall(cards.mix, F, "00") and not pcall(cards.tag, "00", string.rep("00", 8), 1, 0) and not pcall(cards.pin_hash, "zz", "1"))
-- air frames
assert(cards.frame_encode(cards.CMD_POLL) == "0100" and cards.frame_encode(cards.CMD_ERROR, "01") == "ff0101")
local cmd, body = cards.frame_decode("8309" .. string.rep("01", 8) .. "ff")
assert(cmd == 0x83 and body == string.rep("01", 8) .. "ff")
assert(cards.frame_name(0x83) == "AUTH" and cards.frame_name(cards.CMD_ERROR) == "ERROR")
assert(cards.error_reason(cards.ERR_NO_SESSION) == "no session" and not pcall(cards.frame_decode, "0502ff"))
assert(#cards.random_hex(8) == 16)
"#,
    );
}

/// A design round-trips through the `.card` file, the font and packing match
/// ecm-cards and cards.py, and Reader / Nfc / Printer reach the right peripheral.
#[test]
fn lua_cards_design_and_devices() {
    let d = tmpdir("cards");
    let l = lua();
    set(&l, "PATH", p(&d.join("acme.card")));
    run(&l, FAKE_CARDS);
    run(
        &l,
        r##"
local D = cards.Design
local d = D.new("hybrid")
assert(d.width == 32 and d.height == 20 and #d.front == 320)
d.palette = { {255, 255, 255}, {0, 0, 0}, {200, 30, 30} }
d:fill("front", 2)
assert(d.front:byte(1) == 0x22)
d:set_pixel("front", 0, 0, 0xA)
d:set_pixel("front", 1, 0, 0xB)
assert(d.front:byte(1) == 0xAB and d:pixel("front", 1, 0) == 0xB)
d:set_pixel("front", 32, 0, 1)
assert(d:pixel("front", 32, 0) == 0)
d:fill("front", 2)
assert(d:text("front", 0, 0, "A1-. ", 1) == 20)
local rows = {}
for y = 0, 4 do
  local r = ""
  for x = 0, 2 do r = r .. (d:pixel("front", x, y) == 1 and "#" or ".") end
  rows[#rows + 1] = r
end
assert(table.concat(rows, "/") == ".#./#.#/###/#.#/#.#", table.concat(rows, "/"))
assert(d:pixel("front", 5, 0) == 1 and d:pixel("front", 4, 1) == 1 and d:pixel("front", 6, 4) == 1)
assert(d:pixel("front", 8, 2) == 1 and d:pixel("front", 8, 0) == 2)
assert(d:pixel("front", 13, 4) == 1 and d:pixel("front", 13, 3) == 2)
for x = 16, 18 do for y = 0, 4 do assert(d:pixel("front", x, y) == 2) end end
d:text("back", 0, 0, "a", 1)
assert(d:pixel("back", 1, 0) == 1)
for c in ("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ -."):gmatch(".") do assert(cards.FONT[c], c) end
d:set_pixel("back", 5, 7, 1)
d.fields.name = "Ann Example"
d.fields.number = "12345678"
d.stripe = { "ANN", "12345678", "" }
assert(d:color(0, 0, 0) == 1 and d:color(1, 2, 3) == 3 and #d.palette == 4)
local m = d:to_map()
assert(m.kind == nil and m.front == cards.hex(d.front) and #m.front == 640 and m.stripe.t2 == "12345678" and m.fields.name == "Ann Example")
local text = d:to_json()
assert(text:sub(1, 80) == '{"kind":"hybrid","palette":[[255,255,255],[0,0,0],[200,30,30],[1,2,3]],"front":"', text:sub(1, 80))
local back = D.from_json(text)
assert(back.kind == "hybrid" and #back.palette == 4 and back.front == d.front and back.back == d.back)
assert(back.fields.name == "Ann Example" and back.stripe[1] == "ANN" and back.stripe[3] == "")
d:save(PATH)
assert(D.load(PATH):to_json() == text)
local mini = D.from_json('{"kind": "sim", "palette": [[1, 2, 3]]}')
assert(mini.width == 8 and mini.height == 13 and #mini.front == 52 and mini.stripe == nil and mini.front == string.rep("\0", 52))
for _, bad in ipairs({'{"kind": "ticket"}', '{"kind": "sim", "palette": [[1, 2]]}', '{"kind": "sim", "palette": [], "front": "*"}', '[]'}) do
  assert(not pcall(D.from_json, bad), bad)
end
assert(tostring(d) == "<card design hybrid 32x20, 4 colours, 2 fields, stripe>", tostring(d))

-- devices
local reader = cards.Reader.find()
assert(reader and reader.name == "left" and tostring(reader) == "<card reader 'left'>")
assert(cards.Reader.find("left").name == "left" and cards.Reader.find("nope") == nil and #cards.Reader.find_all() == 1)
local ok, e = pcall(cards.Reader.find, "right")
assert(not ok and tostring(e):find("not a card_reader"), tostring(e))
assert(reader:status().state == "card")
local card = reader:card()
assert(card and card:identity().kind == "hybrid" and card:kind() == "hybrid")
local t1, t2, t3 = card.stripe:read()
assert(t1 == "ANN" and t2 == "12345678" and t3 == "" and not card.stripe:corrupted())
card.stripe:write("X", "Y")
local last = function() return calls[#calls] end
assert(last()[1] == "left" and last()[2] == "write_stripe" and last()[3] == "X" and last()[4] == "Y" and last()[5] == "")
card.bank:write("acme", "4000123412341234", string.rep("AB", 32), "1234", true)
assert(last()[2] == "write_bank" and last()[5] == string.rep("ab", 32) and last()[6] == "1234" and last()[7] == true)
assert(not pcall(card.bank.write, card.bank, "acme", "4000123412341234", string.rep("ab", 16)))
assert(card.bank:verify_pin("1234") == true and last()[2] == "verify_pin")
local ctr, tag = card.bank:sign(string.rep("00", 8), 1500)
assert(ctr == 5 and tag == string.rep("ab", 16) and last()[2] == "sign" and last()[4] == 1500)
card.bank:sign(string.rep("00", 8))
assert(last()[4] == 0 and card.bank:blank() == false)
assert(card.nfc:auth(1, cards.FACTORY_KEY) == true and last()[2] == "nfc_auth" and last()[3] == 1 and last()[4] == cards.FACTORY_KEY)
assert(card.nfc:read(1) == string.rep("00", 64))
card.nfc:write(1, "48656c6c6f")
assert(last()[2] == "nfc_write" and last()[4] == "48656c6c6f" .. string.rep("00", 59))
card.nfc:set_key(1, string.rep("11", 16))
assert(last()[2] == "nfc_set_key" and last()[4] == string.rep("11", 16))
assert(card.sim:is_blank() == true and card.sim:write("310150123456789", "cell-red", string.rep("AB", 16)).imsi == "310150123456789")
assert(last()[2] == "write" and last()[5] == string.rep("ab", 16))
assert(card.sim:respond(string.rep("00", 16)).res == string.rep("11", 16))
card:write_fields({ name = "Ann" })
assert(last()[2] == "write_fields" and last()[3].name == "Ann")
assert(reader:eject() == true)
local ev, kind, applets = reader:wait(5)
assert(ev == "card_inserted" and kind == "bank" and applets[1] == "bank")
assert(reader:wait(5) == "card_removed" and reader:wait(1) == nil)

local nfc = cards.Nfc.find()
assert(nfc.name == "right" and tostring(nfc) == "<nfc reader 'right'>" and nfc:field() == false)
local seen = nfc:poll()
assert(#seen == 1 and seen[1].uid == "0102030405060708" and seen[1].kinds[1] == "nfc" and nfc:field() == true)
local s = nfc:session("0102030405060708", 1, cards.FACTORY_KEY)
assert(calls[#calls - 1][2] == "select" and last()[2] == "auth" and last()[3] == 1)
assert(s:read() == string.rep("11", 64) and last()[2] == "read")
s:write("ff")
assert(last()[2] == "write" and last()[4] == "ff" .. string.rep("00", 63))
s:set_key(string.rep("22", 16))
assert(last()[2] == "set_key" and last()[4] == string.rep("22", 16))
assert(nfc:raw("0100") == "8100" and nfc:rssi() == -40)
nfc:power(6)
assert(last()[2] == "set_power" and last()[3] == 6)

local printer = cards.Printer.find()
assert(printer.name == "top" and printer:present() and printer:status().state == "card")
local bank = D.new("bank")
assert(printer:cost(bank).dyes.white == 1 and last()[2] == "cost" and last()[3].front == string.rep("00", 320))
printer:print(bank:to_map())
assert(last()[2] == "print" and last()[3].palette[1][1] == 255 and last()[3].kind == nil)
assert(printer:eject() == true)
"##,
    );
    let saved = std::fs::read_to_string(d.join("acme.card")).unwrap();
    let design = ecm_cards::design::Design::from_json_string(&saved).unwrap();
    assert_eq!(design.kind, "hybrid");
    assert_eq!(design.pixel(ecm_cards::design::Face::Front, 1, 0), 1, "the 'A' drawn in Lua");
    assert_eq!(design.fields["name"], "Ann Example");
    let _ = std::fs::remove_dir_all(&d);
}

// ---------------------------------------------------------------- loader

#[test]
fn require_finds_the_computers_files_and_modules_are_globals() {
    let d = tmpdir("require");
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::create_dir_all(d.join("apps/demo")).unwrap();
    std::fs::write(d.join("lib/greet.lua"), "return { hello = function(n) return 'hi ' .. n end }").unwrap();
    std::fs::create_dir_all(d.join("lib/pkg")).unwrap();
    std::fs::write(d.join("lib/pkg/init.lua"), "return 'pkg'").unwrap();
    std::fs::write(d.join("apps/demo/helper.lua"), "return 42").unwrap();
    std::fs::write(d.join("bad.lua"), "return (").unwrap();
    let l = lua();
    set(&l, "DIR", p(&d));
    run(
        &l,
        r#"
-- The computer's files through `shell` (paths from its root).
package.loaded.shell = { read_file = function(path)
  local f = io.open(DIR .. "/" .. path:gsub("^/", ""), "rb")
  if not f then return nil end
  local s = f:read("*a") f:close() return s
end }
assert(require("lib.greet").hello("ann") == "hi ann")
assert(require("lib.pkg") == "pkg")
arg = { [0] = "apps/demo/main.lua" }
assert(require("helper") == 42)
local ok, e = pcall(require, "missing.mod")
assert(not ok and tostring(e):find("no file 'missing/mod.lua' on this computer"), tostring(e))
ok, e = pcall(require, "bad")
assert(not ok and tostring(e):find("error loading module 'bad'"), tostring(e))
-- Modules are globals, loaded on first use; unknown globals stay nil.
assert(type(radio.tone) == "function" and rawget(_G, "radio") == radio)
assert(type(ui.App) == "function" and nothing_here == nil)
assert(unpack == table.unpack and loadstring == load)
"#,
    );
    let _ = std::fs::remove_dir_all(&d);
}

// ---------------------------------------------------------------- parity

/// Public top-level names of a Python module source: `def f`, `class C`, `NAME =`.
fn python_names(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let word = |rest: &str| rest.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or("").to_string();
        let name = if let Some(r) = line.strip_prefix("def ") {
            word(r)
        } else if let Some(r) = line.strip_prefix("class ") {
            word(r)
        } else if line.contains(" = ") && word(line).chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            word(line)
        } else {
            continue;
        };
        if !name.is_empty() && !name.starts_with('_') {
            out.push(name);
        }
    }
    out
}

/// `#[pyfunction] fn name` / `#[pyfunction(name = "x")]` / `#[pyattr] const NAME` inside
/// the `mod <module> {` block of a Rust source.
fn pymodule_names(src: &str, module: &str) -> Vec<String> {
    let start = src.find(&format!("mod {module} {{")).unwrap_or_else(|| panic!("no mod {module}"));
    let body = &src[start..];
    let end = body[1..].find("\nmod ").map(|i| i + 1).unwrap_or(body.len());
    let body = &body[..end];
    let mut out = Vec::new();
    let mut pending: Option<Option<String>> = None;
    for line in body.lines().map(str::trim) {
        if line.starts_with("#[pyfunction") || line.starts_with("#[pyattr") {
            pending = Some(line.split("name = \"").nth(1).and_then(|r| r.split('"').next()).map(String::from));
            continue;
        }
        if let Some(rename) = pending.take() {
            let decl = line.trim_start_matches("pub ").trim_start_matches("const ").trim_start_matches("fn ");
            let name = rename.unwrap_or_else(|| decl.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap().to_string());
            if !name.starts_with('_') {
                out.push(name);
            }
        }
    }
    out
}

/// `m.set("name"` in a Rust source, plus the quoted side names of the shell module.
fn lua_native_names(src: &str) -> Vec<String> {
    let mut out: Vec<String> = src.split("m.set(\"").skip(1).map(|r| r.split('"').next().unwrap().to_string()).collect();
    if let Some(line) = src.lines().find(|l| l.contains("[\"DOWN\", \"UP\"")) {
        out.extend(line.split('"').skip(1).step_by(2).filter(|s| s.chars().all(|c| c.is_ascii_uppercase())).map(String::from));
    }
    out
}

/// Python names reached differently in Lua (each with the reason).
const NOT_IN_LUA: &[(&str, &str)] = &[
    // peripheral: Lua errors are values; wrap() returns a plain table.
    ("peripheral.PeripheralError", "a peripheral error is a Lua error (pcall)"),
    ("peripheral.Peripheral", "peripheral.wrap returns the wrapped table"),
    // sensors / controller / audio: objects come from find()/open().
    ("sensors.Scan", "made by Lidar:scan()/latest()"),
    ("sensors.Lidar", "made by hub:lidar()"),
    ("sensors.Hub", "made by sensors.find()"),
    ("controller.Controller", "made by controller.find()/all()"),
    ("audio.Speaker", "made by audio.open()"),
    // cards: the applet objects hang off reader:card(); a session off nfc:session().
    ("cards.Card", "made by Reader:card()"),
    ("cards.Stripe", "card.stripe of Reader:card()"),
    ("cards.Bank", "card.bank of Reader:card()"),
    ("cards.NfcApplet", "card.nfc of Reader:card()"),
    ("cards.Sim", "card.sim of Reader:card()"),
    ("cards.Session", "made by Nfc:session()"),
    // radio: classes behind the constructor functions.
    ("radio.RadioError", "errors are Lua errors starting with RadioError:"),
    ("radio.Chain", "internal to Flowgraph"),
    ("radio.ToneSource", "made by radio.tone()"),
    ("radio.FileSource", "made by radio.file_source()"),
    ("radio.WavSource", "made by radio.wav_source()"),
    ("radio.FramesSource", "made by radio.frames_source()"),
    ("radio.SpeakerSink", "made by radio.speaker()"),
    ("radio.WavSink", "made by radio.wav_sink()"),
    ("radio.IqSink", "made by radio.iq_sink()"),
    ("radio.FramesSink", "made by radio.frames()"),
    ("radio.Framed", "made by radio.framed()"),
    // ui: handlers get a plain event table.
    ("ui.Event", "handlers receive {kind, value, widget}"),
];

/// The repository rule (AGENTS.md): every module the Python interpreter offers
/// has a Lua counterpart with the same public names.
#[test]
fn lua_offers_every_python_module_and_name() {
    const PY: &str = "../python/src/";
    let read = |f: &str| std::fs::read_to_string(format!("{}/{PY}{f}", env!("CARGO_MANIFEST_DIR"))).unwrap_or_else(|e| panic!("{f}: {e}"));
    let lua_src = |f: &str| std::fs::read_to_string(format!("{}/src/{f}", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let mut missing = Vec::new();

    // Python's module table (main.rs + the bootstrap) must all exist in Lua.
    let py_main = read("main.rs");
    let boot = read("python_bootstrap.py");
    let mut py_modules: Vec<String> = py_main
        .split("add_native_module(")
        .skip(1)
        .filter_map(|r| r.split('"').nth(1).map(String::from))
        .filter(|n| !n.starts_with('_'))
        .collect();
    py_modules.extend(boot.split("sys.modules['").skip(1).map(|r| r.split('\'').next().unwrap().to_string()));
    let lua_main = lua_src("main.rs");
    for m in &py_modules {
        let in_lua = MODULES.iter().any(|(n, _)| n == m) || lua_main.contains(&format!("(\"{m}\", &"));
        if !in_lua {
            missing.push(format!("module {m}"));
        }
    }

    // Lua-source modules: load each against the stand-ins and compare names.
    let l = lua();
    set(&l, "RANGES", "");
    set(&l, "POINTS", "");
    run(&l, FAKE_PERIPHERALS);
    for (name, file) in [("peripheral", "peripheral.py"), ("sensors", "sensors.py"), ("controller", "controller.py"),
        ("audio", "audio.py"), ("cards", "cards.py"), ("radio", "radio.py"), ("ui", "ui.py")]
    {
        let module: Table = l.load(format!("return require('{name}')")).eval().unwrap();
        for n in python_names(&read(file)) {
            let key = format!("{name}.{n}");
            if NOT_IN_LUA.iter().any(|(k, _)| *k == key) {
                continue;
            }
            if module.get::<mlua::Value>(n.as_str()).unwrap().is_nil() {
                missing.push(key);
            }
        }
    }

    // Native modules: Python's #[pyfunction]s vs the Lua bindings.
    let shell_lua = lua_native_names(&lua_main);
    for n in pymodule_names(&py_main, "shell_module") {
        if !shell_lua.contains(&n) {
            missing.push(format!("shell.{n}"));
        }
    }
    let net_lua = lua_native_names(&lua_src("net.rs"));
    for n in pymodule_names(&py_main, "net_module") {
        if !net_lua.contains(&n) {
            missing.push(format!("net.{n}"));
        }
    }

    assert!(
        missing.is_empty(),
        "the Lua interpreter is missing what Python has (AGENTS.md: Python and Lua offer the same modules): {missing:?}"
    );
}

// ---------------------------------------------------------------- scenario scripts

const SCENARIOS: &str = "../../../src/main/resources-mc1.21.1/evanscomputermod/scenarios/";

fn scenario_file(name: &str) -> String {
    std::fs::read_to_string(format!("{}/{SCENARIOS}{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// lua_modules.lua (the lua_modules scenario) against stand-ins for the
/// computer: its files, net, the speaker's /dev/audio.
#[test]
fn the_lua_modules_scenario_script_runs() {
    let d = tmpdir("modules-script");
    std::fs::create_dir_all(d.join("lib")).unwrap();
    std::fs::write(d.join("lib/tones.lua"), scenario_file("lua_tones.lua")).unwrap();
    std::fs::write(d.join("audioctl"), "").unwrap();
    let l = lua();
    set(&l, "DIR", p(&d));
    set(&l, "RANGES", "");
    set(&l, "POINTS", "");
    run(&l, FAKE_PERIPHERALS);
    run(
        &l,
        r##"
package.loaded._peripheral.list = function() return {{"left", "speaker"}} end
local real_open = io.open
io.open = function(path, mode)
  if path == "/dev/audioctl" and mode == "rb" then
    local f = real_open(DIR .. "/status", "wb") f:write("rate 8000\n") f:close()
    return real_open(DIR .. "/status", "rb")
  end
  return real_open((path:gsub("^/dev/", DIR .. "/")), mode)
end
local fake_shell = { read_file = function(path)
  local f = real_open(DIR .. "/" .. path:gsub("^/", ""), "rb")
  if not f then return nil end
  local s = f:read("*a") f:close() return s
end }
package.loaded.shell = fake_shell
shell, terminal = fake_shell, fake_shell
net = { interfaces = function() return {{name = "lo"}, {name = "eth0"}} end, dns_get = function() return "0.0.0.0" end }
out = {}
print = function(...) local t = {} for i = 1, select("#", ...) do t[i] = tostring(select(i, ...)) end out[#out + 1] = table.concat(t, "\t") end
"##,
    );
    run(&l, &scenario_file("lua_modules.lua"));
    let out: Vec<String> = l.load("return out").eval::<Table>().unwrap().sequence_values::<String>().map(Result::unwrap).collect();
    assert_eq!(
        out,
        [
            "REQUIRE tones", "MISSING false", "TERMINAL true", "NET 2 0.0.0.0", "PERIPHERAL left=speaker",
            "SENSORS nil CONTROLLER nil", "AUDIO 8000", "RADIO N0LUA>CQ:tones", "UI false console mode",
        ]
    );
    assert!(std::fs::metadata(d.join("audio")).unwrap().len() > 3000, "the notes were written to /dev/audio");
    let _ = std::fs::remove_dir_all(&d);
}

/// lua_counter.lua (the lua_ui scenario): two clicks on Add one make Count: 2.
#[test]
fn the_lua_counter_app_counts_clicks() {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let handle = ui_native::register_mock(mock.clone());
    let l = lua();
    set(&l, "MOCK", handle);
    // Everything but the event loop: the test steps the app itself.
    let body = scenario_file("lua_counter.lua").replace("local app = ", "app = ").replace("app:run()", "");
    run(&l, "require('_ui').use_mock(MOCK)");
    run(&l, &body);
    run(&l, "APP = app._h");
    let app_h: i64 = l.globals().get("APP").unwrap();
    mock.push(&[Message::Configure(ecm_ui_proto::Configure { surface: 1, serial: 1, w: 320, h: 200, states: 0 })]);
    run(&l, "app:step(0)\napp:step(0)");
    for _ in 0..2 {
        mock.push(&click(hit(app_h, "add"), 0, 0));
        run(&l, "app:step(0)");
    }
    run(&l, "app:step(0)\nrequire('_ui').use_mock(nil)");
    let shown = mock.sent_messages().iter().any(|m| match m {
        Message::TreeOps(t) => t.ops.iter().any(|op| match op {
            ecm_ui_proto::TreeOp::Set { props, .. } | ecm_ui_proto::TreeOp::Create { props, .. } => {
                props.iter().any(|p| matches!(p, ecm_ui_proto::Prop::Text(s) if s == "Count: 2"))
            }
            _ => false,
        }),
        _ => false,
    });
    assert!(shown, "no TreeOps set the label to Count: 2");
}

/// The card widgets through `ui`: a PixelGrid's .pixels follows the player's
/// painting (and the eyedropper sets .color), pushes from Lua reach the
/// screen, a locked texel stays, and a ColorPicker's .value follows drags and
/// typed hex while pushes from Lua restyle it (the Python test's twin).
#[test]
fn lua_pixel_grid_and_color_picker_follow_input_and_push_changes() {
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let handle = ui_native::register_mock(mock.clone());
    let l = lua();
    set(&l, "MOCK", handle);
    run(
        &l,
        r##"
_ui = require("_ui")
_ui.use_mock(MOCK)
log = {}
local mask = {}
for i = 1, 32 do mask[i] = 0 end
mask[6] = 1
grid = ui.PixelGrid(8, 4, nil, {0x000000, 0xffffff, 0xff0000, 0x3d8bfd}, {zoom = 10, color = 3, mask = mask, id = "grid",
  on_change = function(e) log[#log + 1] = {"change", e.value} end,
  on_select = function(e) log[#log + 1] = {"select", e.value} end})
pick = ui.ColorPicker(0xff0000, {id = "pick", width = 120, on_change = function(e) log[#log + 1] = {"color", e.value} end})
app = ui.App("dev.test.cards")
win = app:window("Cards", ui.Column{gap = 0, children = {grid, pick}})
assert(grid.width == 8 and grid.height == 4 and #grid.pixels == 32)
APP = app._h
"##,
    );
    let app_h: i64 = l.globals().get("APP").unwrap();
    let step = |msgs: Vec<Message>| {
        mock.push(&msgs);
        run(&l, "app:step(0)\napp:step(0)");
    };
    step(vec![Message::Configure(ecm_ui_proto::Configure { surface: 1, serial: 1, w: 200, h: 300, states: ecm_ui_proto::window_states::ACTIVATED })]);
    // Texel (2, 1) is at (25, 15); the locked texel (5, 0) at (55, 5).
    step(click(hit(app_h, "grid"), 25, 15));
    step(click(hit(app_h, "grid"), 55, 5));
    run(
        &l,
        r##"
assert(grid:pixel(2, 1) == 3)
assert(grid:pixel(5, 0) == 0)
assert(#log == 1 and log[1][1] == "change" and #log[1][2] == 1 and log[1][2][1][1] == 2 and log[1][2][1][2] == 1 and log[1][2][1][3] == 3)
grid.tool = "eyedropper"
app:step(0)
"##,
    );
    step(click(hit(app_h, "grid"), 25, 15));
    run(
        &l,
        r##"
assert(log[#log][1] == "select" and log[#log][2] == 3, "eyedropper: " .. tostring(log[#log][1]) .. " " .. tostring(log[#log][2]))
assert(grid.color == 3)
grid.tool = "fill"
grid.color = 1
grid:set_pixel(0, 0, 2)
app:step(0)
"##,
    );
    let pushed = ui_native::with_core(app_h, |c| c.window(1).unwrap().pixels("grid")).unwrap();
    assert_eq!(pushed[0], 2, "set_pixel reached the toolkit");
    assert_eq!(pushed[10], 3);
    step(click(hit(app_h, "grid"), 15, 5));
    run(
        &l,
        r##"
assert(grid:pixel(1, 0) == 1 and grid:pixel(7, 3) == 1 and grid:pixel(0, 0) == 2 and grid:pixel(5, 0) == 0, "fill: " .. table.concat(grid.pixels, ","))
assert(log[#log][1] == "change" and #log[#log][2] == 32 - 3, #log[#log][2])
"##,
    );
    let (hue, _, field) = ui_native::with_core(app_h, |c| c.window(1).unwrap().picker_hit_ids("pick")).flatten().unwrap();
    let initial = ui_native::with_core(app_h, |c| c.window(1).unwrap().state("pick").unwrap().value).unwrap();
    assert_eq!(initial, 0xff0000, "the constructor's value reached the toolkit");
    step(click(hue, 120 - 8, 40 + 58));
    run(
        &l,
        r##"
assert(log[#log][1] == "color" and log[#log][2] == pick.value, "picker: " .. tostring(log[#log][1]) .. " " .. tostring(log[#log][2]) .. " " .. tostring(pick.value))
assert(pick.value ~= 0xff0000 and pick.value < 0x10000, pick.value)
pick.value = 0x123456
app:step(0)
assert(pick.hex == "#123456", pick.hex)
"##,
    );
    let v = ui_native::with_core(app_h, |c| c.window(1).unwrap().state("pick").unwrap().value).unwrap();
    assert_eq!(v, 0x123456);
    step(click(field, 60, 40 + 130));
    for _ in 0..7 {
        step(vec![Message::Key(ecm_ui_proto::Key { surface: 1, key: 0x2A, mods: 0, down: true, repeat: false })]);
    }
    step(vec![Message::Text(ecm_ui_proto::Text { surface: 1, text: "#00ff00".into() })]);
    run(&l, "assert(pick.value == 0x00ff00, pick.value)\nassert(log[#log][1] == 'color' and log[#log][2] == 0x00ff00)\n_ui.use_mock(nil)");
    assert!(mock.sent_messages().iter().any(|m| matches!(m, Message::TreeOps(_))));
}
