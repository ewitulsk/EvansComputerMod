//! Host tests of the Python console modules (term, events, computer, colors,
//! keys): a real RustPython with `_console` registered and the modules loaded
//! as the bootstrap loads them, over a scripted console (ecm-console's Fake).

use super::*;
use ecm_console::session::fake::Fake;
use rustpython_vm::{compiler::Mode, Interpreter, Settings};

/// Run `code` on a 20 x 6 console whose input and clock are `fake`.
/// Panics with the Python traceback on an exception.
fn run_py(fake: &Fake, code: &str) {
    let term = ecm_console::term::Term::new(20, 6, 1, Some(1));
    install_session(Session::new(term, Box::new(fake.clone())));
    let interp = Interpreter::with_init(Settings::default(), |vm| {
        vm.add_native_module("_console".to_owned(), Box::new(console_native::make_module));
    });
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        let setup = "import sys, _console\nfor n in ('colors', 'keys', 'term', 'events', 'computer', 'fs', 'textutils', 'redstone'):\n    M = type(sys)(n)\n    exec(compile(getattr(_console, '_' + n + '_source'), '<' + n + '>', 'exec'), M.__dict__)\n    sys.modules[n] = M\n";
        let full = format!("{setup}{code}");
        let code_obj = vm.compile(&full, Mode::Exec, "<test>".to_owned()).expect("test script compiles");
        if let Err(e) = vm.run_code_obj(code_obj, scope) {
            let mut s = String::new();
            vm.write_exception(&mut s, &e).unwrap();
            panic!("python raised:\n{s}");
        }
    });
}

#[test]
fn python_term_draws_like_lua_does() {
    let fake = Fake::default();
    run_py(&fake, r#"
import term, colors
term.set_background_color(colors.blue)
term.clear()
term.set_cursor_pos(18, 2)
term.set_text_color(colors.red)
term.write("abcdef")
assert term.get_cursor_pos() == (24, 2)
assert term.get_size() == (20, 6)
term.set_cursor_pos(1, 3)
term.blit("hi", "04", "ff")
assert term.get_text_color() == colors.red
for bad in (lambda: term.set_text_color(0), lambda: term.blit("ab", "0", "ff")):
    try:
        bad()
        raise AssertionError("no error")
    except ValueError:
        pass
term.set_cursor_pos(1, 5)
assert term.write_wrapped("one two three four five\n") == 2
"#);
    let out = fake.output();
    assert!(out.starts_with("\x1b[0;97;44m\x1b[2J\x1b[?25l\x1b[2;18H\x1b[0;31;44mabc\x1b[3;1H\x1b[0;97;40mh\x1b[0;93;40mi"), "{out:?}");
    assert_eq!(out.matches("\x1b[1S").count(), 1, "{out:?}");
}

#[test]
fn python_events_share_one_queue_with_peripherals_and_keys() {
    let fake = Fake::default();
    fake.0.borrow_mut().typed.push_back((20.0, b"a\n".to_vec()));
    fake.0.borrow_mut().periph.push_back((
        120.0,
        vec![Value::Str("redstone_link".into()), Value::Str("left".into()), Value::Int(3)],
    ));
    run_py(&fake, r#"
import events, keys
t = events.start_timer(0.25)
events.queue("mine", "x", 2)
assert events.pull() == ("mine", "x", 2)
assert events.pull("char") == ("char", "a")
assert events.pull("key") == ("key", keys.enter, False)
assert events.pull("redstone_link") == ("redstone_link", "left", 3)
assert events.pull("timer") == ("timer", t)
assert events.pull(None, 0) is None
events.queue("terminate")
assert events.pull("char", 0) == ("terminate",)
events.queue("lost")
events.sleep(0.1)
assert events.pull(None, 0) is None
assert keys.a == 65 and keys.get("end") == 269 and keys.get_name(265) == "up"
"#);
}

#[test]
fn python_clocks_label_and_colours() {
    let fake = Fake::default();
    {
        let mut s = fake.0.borrow_mut();
        s.ticks = Some(18000 + 24000);
        s.unix_ms = 86_400_000 + 3_600_000;
        s.id = Some(7);
    }
    run_py(&fake, r#"
import computer, colors, term
assert computer.time() == 0.0 and computer.day() == 3
assert computer.epoch() == (18000 + 24000 + 6000) * 3600
assert abs(computer.time("utc") - 1.0) < 1e-9 and computer.day("UTC") == 1
assert computer.epoch("utc") == 86400000 + 3600000
assert computer.id() == 7 and computer.label() is None
computer.set_label("base")
assert computer.label() == "base"
try:
    computer.time("mars")
    raise AssertionError("no error")
except ValueError:
    pass
assert colors.combine(colors.white, colors.magenta, colors.lightBlue) == 13
assert colors.subtract(colors.lime, colors.orange, colors.white) == 32
assert colors.test(colors.combine(colors.red, colors.blue), colors.red)
assert colors.pack_rgb(0.7, 0.2, 0.6) == 0xb23399
assert colors.to_blit(colors.red) == "e" and colors.from_blit("e") == 16384 and colors.from_blit("z") is None
r, g, b = term.native_palette_color(colors.red)
assert abs(r - 0.8) < 0.01

class Capture:
    def __init__(self): self.lines, self.x, self.y = [], 1, 1
    def get_size(self): return (10, 3)
    def get_cursor_pos(self): return (self.x, self.y)
    def set_cursor_pos(self, x, y): self.x, self.y = x, y
    def write(self, s): self.lines.append(s); self.x += len(s)
for n in term._NAMES:
    if not hasattr(Capture, n):
        setattr(Capture, n, lambda self, *a: None)
c = Capture()
old = term.redirect(c)
assert term.write_wrapped("abc defghijkl") == 1
assert term.current() is c
term.redirect(old)
assert term.current() is term.native()
assert c.lines == ["abc", " ", "defghijkl"], c.lines
"#);
    assert_eq!(fake.output(), "", "the redirected text never reached the console");
}

#[test]
fn python_fs_textutils_and_redstone() {
    let fake = Fake::default();
    let d = std::env::temp_dir().join(format!("ecm-pyfs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    {
        let mut s = fake.0.borrow_mut();
        s.root = Some(d.clone());
        s.redstone_in[4] = 9;
    }
    run_py(&fake, r#"
import fs, textutils, redstone
fs.make_dir("/data")
h = fs.open("data/log.txt", "w")
h.write_line("one"); h.write("two"); h.close()
h = fs.open("data/log.txt", "r")
assert h.read_line() == "one" and h.read_line(True) == "two" and h.read_line() is None
h.close()
b = fs.open("data/bin", "wb"); b.write(65); b.write(b"BC"); b.close()
b = fs.open("data/bin", "rb"); assert b.read() == 65 and b.read(2) == b"BC"; b.close()
assert fs.list("data") == ["bin", "log.txt"] and fs.get_size("data/log.txt") == 7
assert fs.attributes("data")["is_dir"]
fs.copy("data", "backup")
assert fs.find("backup/*.txt") == ["backup/log.txt"]
try:
    fs.list("nope")
    raise AssertionError("no error")
except OSError as e:
    assert str(e).endswith("/nope: No such file"), str(e)
assert fs.open("nope.txt", "r") == (None, "/nope.txt: No such file")
fs.delete("backup")
assert not fs.exists("backup") and fs.combine("a", "../b") == "b" and fs.get_free_space() == "unlimited"

s = textutils.serialize({"name": "x", "list": [1, 2]})
assert s == '{\n  list = {\n    1,\n    2,\n  },\n  name = "x",\n}', s
assert textutils.unserialize(s) == {"name": "x", "list": [1, 2]}
assert textutils.unserialize("os.exit()") is None
assert textutils.serialize_json({"a": [1, 2], "e": {}}) == '{"a":[1,2],"e":{}}'
assert textutils.unserialize_json('{"a": [1, null]}') == {"a": [1, None]}
assert textutils.format_time(13.5) == "1:30 PM" and textutils.url_encode("a b&c") == "a+b%26c"

redstone.set_output("top", True)
redstone.set_analog_output("back", 7)
assert redstone.get_input("left") and redstone.get_analog_input("left") == 9 and not redstone.get_input("right")
assert redstone.get_output("top") and redstone.get_analog_output("back") == 7
try:
    redstone.get_input("up")
    raise AssertionError("no error")
except ValueError:
    pass
"#);
    assert_eq!(fake.0.borrow().redstone_out, [0, 15, 0, 7, 0, 0]);
    assert!(d.join("data/bin").is_file());
    let _ = std::fs::remove_dir_all(&d);
}
