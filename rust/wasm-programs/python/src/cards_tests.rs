//! Host tests of the Python `cards` module: a real RustPython interpreter with
//! `_ui` registered (for the `.card` JSON), peripheral.py and cards.py loaded
//! as the bootstrap does, and a Python stand-in for the host's `_peripheral`
//! (a Card Reader/Writer, an NFC Reader and a Card Printer that record calls).
//! The scheme functions are checked against the vectors in docs/cards/NFC_MIX.md.

use rustpython_vm::{compiler::Mode, Interpreter, Settings};

const PERIPHERAL_PY: &str = include_str!("peripheral.py");
const CARDS_PY: &str = include_str!("cards.py");

/// A `_peripheral` with one of each card device; `calls` records every call.
pub(crate) const FAKE_PERIPHERALS: &str = r#"
import sys
calls = []
state = {"field": False}
_devices = [("left", "card_reader"), ("right", "nfc_reader"), ("top", "card_printer")]
_methods = {
    "card_reader": ["present", "eject", "status", "identity", "read_stripe", "write_stripe", "write_fields",
                    "write_bank", "verify_pin", "sign", "bank_blank", "nfc_auth", "nfc_read", "nfc_write",
                    "nfc_set_key", "write", "is_blank", "respond"],
    "nfc_reader": ["set_field", "field", "set_power", "poll", "select", "auth", "read", "write", "set_key", "raw", "rssi"],
    "card_printer": ["present", "status", "print", "cost", "eject"],
}
_results = {
    "present": True, "eject": True,
    "status": {"present": True, "released": False, "auto_eject": True, "state": "card", "applets": ["bank", "nfc"]},
    "identity": {"kind": "hybrid", "uid": "0102030405060708", "applets": ["bank", "nfc"], "print": {"name": "Ann"},
                 "bank": {"issuer": "acme", "pan": "4000123412341234", "locked": False, "ctr": 4, "blank": False},
                 "nfc": {"sectors": [{"written": True}] + [{"written": False}] * 7}},
    "read_stripe": {"t1": "ANN", "t2": "12345678", "t3": "", "corrupted": False},
    "write_stripe": True, "write_fields": True,
    "write_bank": {"ejected": True}, "verify_pin": True, "sign": {"ctr": 5, "tag": "ab" * 16}, "bank_blank": False,
    "nfc_auth": True, "nfc_read": "00" * 64, "nfc_write": True, "nfc_set_key": True,
    "write": {"imsi": "310150123456789", "carrier": "cell-red", "ejected": False},
    "is_blank": True, "respond": {"res": "11" * 16, "ks": "22" * 32},
    "set_field": True, "set_power": True,
    "poll": [{"uid": "0102030405060708", "kinds": ["nfc"]}], "select": True, "auth": True,
    "read": "11" * 64, "set_key": True, "raw": "8100", "rssi": -40,
    "print": {"dyes": {"white": 1, "black": 1}}, "cost": {"dyes": {"white": 1, "black": 1}},
}
_events = [("redstone", "left"), ("card_inserted", "left", "bank", ["bank"]), ("card_removed", "left")]

def _type_of(name):
    for n, t in _devices:
        if n == name:
            return t

def _list():
    return list(_devices)

def _methods_of(name):
    t = _type_of(name)
    if t is None:
        return (False, "no peripheral '%s'" % name)
    return (True, (t, list(_methods[t])))

def _call(name, method, args):
    calls.append((name, method, list(args)))
    t = _type_of(name)
    if t is None:
        return (False, "no peripheral '%s'" % name)
    if method not in _methods[t]:
        return (False, "no method %s" % method)
    if method == "field":
        return (True, state["field"])
    if method == "set_field":
        state["field"] = args[0]
        return (True, True)
    if method == "write" and t == "nfc_reader":
        return (True, True)
    return (True, _results[method])

def _wait_event(filter, ms):
    if _events:
        return _events.pop(0)
    return None

_m = type(sys)("_peripheral")
_m.list = _list
_m.methods = _methods_of
_m.call = _call
_m.wait_event = _wait_event
sys.modules["_peripheral"] = _m
"#;

/// Run `code` with `peripheral` and `cards` importable against the stand-ins;
/// `vars` are string globals. Panics with the Python traceback on an exception.
fn run_py(code: &str, vars: &[(&str, String)]) {
    let interp = Interpreter::with_init(Settings::default(), |vm| {
        vm.add_native_module("_ui".to_owned(), Box::new(crate::ui_module::ui_native::make_module));
    });
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        for (k, v) in vars {
            scope.globals.set_item(*k, vm.ctx.new_str(v.as_str()).into(), vm).unwrap();
        }
        scope.globals.set_item("PERIPHERAL_PY", vm.ctx.new_str(PERIPHERAL_PY).into(), vm).unwrap();
        scope.globals.set_item("CARDS_PY", vm.ctx.new_str(CARDS_PY).into(), vm).unwrap();
        let setup = "\n\
            M = type(sys)('peripheral')\n\
            exec(compile(PERIPHERAL_PY, '<peripheral>', 'exec'), M.__dict__)\n\
            sys.modules['peripheral'] = M\n\
            C = type(sys)('cards')\n\
            exec(compile(CARDS_PY, '<cards>', 'exec'), C.__dict__)\n\
            sys.modules['cards'] = C\n\
            import cards, peripheral\n";
        let full = format!("{FAKE_PERIPHERALS}{setup}{code}");
        let code_obj = vm.compile(&full, Mode::Exec, "<test>".to_owned()).expect("test script compiles");
        if let Err(e) = vm.run_code_obj(code_obj, scope) {
            let mut s = String::new();
            vm.write_exception(&mut s, &e).unwrap();
            panic!("python raised:\n{s}");
        }
    });
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ecm-pycards-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// docs/cards/NFC_MIX.md: every Mix, TAG and PIN-hash vector, through the
/// pure-Python SHA-256 / HMAC the in-game interpreter relies on.
#[test]
fn cards_scheme_vectors() {
    run_py(
        r#"
F = cards.FACTORY_KEY
assert F == "45434d2d4e46432d464143544f525930" and bytes.fromhex(F) == b"ECM-NFC-FACTORY0"
for key, x, want in [
    (F, "00" * 16, "3e2e68d5726a6858461251213df31d59"),
    (F, "000102030405060708090a0b0c0d0e0f", "e8938dca0b939828c0b5e18c388b9702"),
    ("00112233445566778899aabbccddeeff", "01020304050607081112131415161718", "f2668ac938caea225f16cf78a07cfe81"),
    ("00" * 16, "00" * 16, "5d4fb41a84bab1a9fa2416dcc38b9978"),
    ("ff" * 16, "ff" * 16, "355b79459dc55581fc26ab3eb2eb6ce6"),
]:
    assert cards.mix(key, x) == want, (key, x, cards.mix(key, x))
assert cards.auth_m1(F, "01" * 8, "02" * 8) == cards.mix(F, "01" * 8 + "02" * 8)
assert cards.auth_m2(F, "01" * 8, "02" * 8) == cards.mix(F, "02" * 8 + "01" * 8)

# SHA-256 / HMAC themselves (FIPS 180-4 / RFC 4231 known answers).
assert cards.sha256("") == "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
assert cards.sha256(b"abc".hex()) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
assert cards.sha256(("a" * 1000).encode().hex()) == "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
assert cards.hmac_sha256("0b" * 20, b"Hi There".hex()) == "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
assert cards.hmac_sha256("aa" * 131, b"Test Using Larger Than Block-Size Key - Hash Key First".hex()) == "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"

K1 = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
for k, nonce, ctr, amount, want in [
    (K1, "0001020304050607", 1, 0, "aef6052d8232ad7c4db312596726e55a"),
    (K1, "0001020304050607", 2, 1500, "ce2e7e5ad8d1e0b8bd42aa2b8903e5b7"),
    ("00" * 32, "00" * 8, 0, 0, "c1e9e0459b7a23ecb6d54dc7772a25af"),
    ("ff" * 32, "ff" * 8, 9223372036854775807, 1099511627776, "6987930a133eede5b3b6de9ef58f0e91"),
]:
    assert cards.tag(k, nonce, ctr, amount) == want, (ctr, amount, cards.tag(k, nonce, ctr, amount))
    assert cards.verify_tag(k, nonce, ctr, amount, want)
    assert cards.verify_tag(k, nonce, ctr, amount, want.upper())
    assert not cards.verify_tag(k, nonce, ctr + 1, amount, want)
    assert not cards.verify_tag(k, nonce, ctr, amount, want[:-2])
assert cards.tag(K1, "0001020304050607", "0000000000000002", "00000000000005dc") == "ce2e7e5ad8d1e0b8bd42aa2b8903e5b7"
for uid, pin, want in [
    ("0102030405060708", "1234", "282fbfbe3f85ce96542b0435cc442d28745c372aaacf04b6229e657998169fd2"),
    ("00" * 8, "00000000", "72a0521796bf77ccf576d8f5554d61b5a5cd371fad7492f3b64514b7e2a4cb19"),
]:
    assert cards.pin_hash(uid, pin) == want
# bad lengths are ValueErrors
for bad in [lambda: cards.mix(F, "00"), lambda: cards.tag("00", "00" * 8, 1, 0), lambda: cards.pin_hash("zz", "1")]:
    try:
        bad()
        assert False, "no error"
    except ValueError:
        pass
# air frames
assert cards.frame_encode(cards.CMD_POLL) == "0100"
assert cards.frame_encode(cards.CMD_ERROR, "01") == "ff0101"
assert cards.frame_decode("8309" + "01" * 8 + "ff") == (0x83, "01" * 8 + "ff")
assert cards.frame_name(0x83) == "AUTH" and cards.frame_name(cards.CMD_ERROR) == "ERROR"
assert cards.error_reason(cards.ERR_NO_SESSION) == "no session"
try:
    cards.frame_decode("0502ff")
    assert False
except ValueError:
    pass
try:
    assert len(cards.random_hex(8)) == 16
except RuntimeError:
    pass   # the host test interpreter has no os / random
"#,
        &[],
    );
}

/// A design round-trips through the `.card` file; the font and the packing
/// match ecm-cards (high nibble first, 3x5 glyphs advancing 4 texels).
#[test]
fn cards_design_file_and_font() {
    let d = tmpdir("design");
    let path = d.join("acme.card").to_string_lossy().replace('\\', "/");
    run_py(
        r##"
D = cards.Design
d = D.new("hybrid")
assert (d.width, d.height, len(d.front)) == (32, 20, 320)
d.palette = [[255, 255, 255], [0, 0, 0], [200, 30, 30]]
d.fill("front", 2)
assert d.front[0] == 0x22
d.set_pixel("front", 0, 0, 0xA)
d.set_pixel("front", 1, 0, 0xB)
assert d.front[0] == 0xAB and d.pixel("front", 1, 0) == 0xB
d.set_pixel("front", 32, 0, 1)   # outside: ignored
assert d.pixel("front", 32, 0) == 0
d.fill("front", 2)
end = d.text("front", 0, 0, "A1-. ", 1)
assert end == 20
rows = ["".join('#' if d.pixel("front", x, y) == 1 else '.' for x in range(3)) for y in range(5)]
assert rows == ['.#.', '#.#', '###', '#.#', '#.#'], rows
assert d.pixel("front", 5, 0) == 1 and d.pixel("front", 4, 1) == 1 and d.pixel("front", 6, 4) == 1
assert (d.pixel("front", 8, 2), d.pixel("front", 8, 0)) == (1, 2)
assert (d.pixel("front", 13, 4), d.pixel("front", 13, 3)) == (1, 2)
assert all(d.pixel("front", x, y) == 2 for x in range(16, 19) for y in range(5))
d.text("back", 0, 0, "a", 1)
assert d.pixel("back", 1, 0) == 1
for c in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ -.":
    assert c in cards.FONT
d.set_pixel("back", 5, 7, 1)
d.fields["name"] = "Ann Example"
d.fields["number"] = "12345678"
d.stripe = ("ANN", "12345678", "")
assert d.color(0, 0, 0) == 1 and d.color(1, 2, 3) == 3 and len(d.palette) == 4
m = d.to_map()
assert sorted(m) == ["back", "fields", "front", "palette", "stripe"]
assert m["front"] == d.front.hex() and len(m["front"]) == 640 and m["stripe"] == {"t1": "ANN", "t2": "12345678", "t3": ""}
assert "kind" not in m
text = d.to_json()
assert text.startswith('{"kind":"hybrid","palette":[[255,255,255],[0,0,0],[200,30,30],[1,2,3]],"front":"'), text
back = D.from_json(text)
assert back.kind == "hybrid" and back.palette == d.palette and bytes(back.front) == bytes(d.front) and bytes(back.back) == bytes(d.back)
assert back.fields == d.fields and back.stripe == d.stripe
d.save(PATH)
again = D.load(PATH)
assert again.to_json() == text
mini = D.from_json('{"kind": "sim", "palette": [[1, 2, 3]]}')
assert (mini.width, mini.height, len(mini.front), mini.stripe) == (8, 13, 52, None)
assert bytes(mini.front) == bytes(52)
for bad in ['{"kind": "ticket"}', '{"kind": "sim", "palette": [[1, 2]]}', '{"kind": "sim", "palette": [], "front": "*"}', '[]']:
    try:
        D.from_json(bad)
        assert False, bad
    except ValueError:
        pass
assert repr(d) == "<card design hybrid 32x20, 4 colours, 2 fields, stripe>"
"##,
        &[("PATH", path)],
    );
    let saved = std::fs::read_to_string(d.join("acme.card")).unwrap();
    assert!(saved.contains("\"stripe\":{\"t1\":\"ANN\""), "{saved}");
    // The file is the ecm-cards format too.
    let design = ecm_cards::design::Design::from_json_string(&saved).unwrap();
    assert_eq!(design.kind, "hybrid");
    assert_eq!(design.pixel(ecm_cards::design::Face::Front, 1, 0), 1, "the 'A' drawn in Python");
    assert_eq!(design.fields["name"], "Ann Example");
    let _ = std::fs::remove_dir_all(&d);
}

/// Reader / Nfc / Printer against the stand-in: the calls reach the right
/// peripheral with the arguments the CARDS.md tables give.
#[test]
fn cards_readers_and_printer_call_the_peripherals() {
    run_py(
        r#"
reader = cards.Reader.find()
assert reader is not None and reader.name == "left" and repr(reader) == "<card reader 'left'>"
assert cards.Reader.find("left").name == "left" and cards.Reader.find("nope") is None
assert [r.name for r in cards.Reader.find_all()] == ["left"]
try:
    cards.Reader.find("right")
    assert False
except peripheral.PeripheralError as e:
    assert "not a card_reader" in str(e)
assert reader.status()["state"] == "card"
card = reader.card()
assert card is not None and card.identity()["kind"] == "hybrid" and card.kind() == "hybrid"
assert card.stripe.read() == ("ANN", "12345678", "") and not card.stripe.corrupted()
card.stripe.write("X", "Y")
assert calls[-1] == ("left", "write_stripe", ["X", "Y", ""])
card.bank.write("acme", "4000123412341234", "ab" * 32, "1234", True)
assert calls[-1] == ("left", "write_bank", ["acme", "4000123412341234", "ab" * 32, "1234", True])
try:
    card.bank.write("acme", "4000123412341234", "ab" * 16)
    assert False
except ValueError:
    pass
assert card.bank.verify_pin("1234") is True and calls[-1] == ("left", "verify_pin", ["1234"])
assert card.bank.sign("00" * 8, 1500) == (5, "ab" * 16) and calls[-1] == ("left", "sign", ["00" * 8, 1500])
assert card.bank.sign("00" * 8) == (5, "ab" * 16) and calls[-1][2] == ["00" * 8, 0]
assert card.bank.blank() is False
assert card.nfc.auth(1, cards.FACTORY_KEY) is True and calls[-1] == ("left", "nfc_auth", [1, cards.FACTORY_KEY])
assert card.nfc.read(1) == "00" * 64
card.nfc.write(1, "48656c6c6f")
assert calls[-1] == ("left", "nfc_write", [1, "48656c6c6f" + "00" * 59])
card.nfc.set_key(1, "11" * 16)
assert calls[-1] == ("left", "nfc_set_key", [1, "11" * 16])
assert card.sim.is_blank() is True
assert card.sim.write("310150123456789", "cell-red", "AB" * 16)["imsi"] == "310150123456789"
assert calls[-1] == ("left", "write", ["310150123456789", "cell-red", "ab" * 16])
assert card.sim.respond("00" * 16)["res"] == "11" * 16
card.write_fields({"name": "Ann"})
assert calls[-1] == ("left", "write_fields", [{"name": "Ann"}])
assert reader.eject() is True
assert reader.wait(5) == ("card_inserted", "bank", ["bank"])
assert reader.wait(5) == ("card_removed",)
assert reader.wait(1) is None
try:
    peripheral.call("left", "nope")
    assert False
except peripheral.PeripheralError:
    pass

nfc = cards.Nfc.find()
assert nfc.name == "right" and repr(nfc) == "<nfc reader 'right'>"
assert nfc.field() is False
seen = nfc.poll()
assert seen == [{"uid": "0102030405060708", "kinds": ["nfc"]}]
assert ("right", "set_field", [True]) in calls and nfc.field() is True
s = nfc.session("0102030405060708", 1, cards.FACTORY_KEY)
assert calls[-2] == ("right", "select", ["0102030405060708"]) and calls[-1] == ("right", "auth", [1, cards.FACTORY_KEY])
assert s.read() == "11" * 64 and calls[-1] == ("right", "read", [1])
s.write("ff")
assert calls[-1] == ("right", "write", [1, "ff" + "00" * 63])
s.set_key("22" * 16)
assert calls[-1] == ("right", "set_key", [1, "22" * 16])
assert nfc.raw("0100") == "8100" and nfc.rssi() == -40
nfc.power(6)
assert calls[-1] == ("right", "set_power", [6])

printer = cards.Printer.find()
assert printer.name == "top" and printer.present() and printer.status()["state"] == "card"
d = cards.Design.new("bank")
assert printer.cost(d) == {"dyes": {"white": 1, "black": 1}}
assert calls[-1][0:2] == ("top", "cost") and calls[-1][2][0]["front"] == "00" * 320
printer.print(d.to_map())
assert calls[-1][0:2] == ("top", "print") and sorted(calls[-1][2][0]) == ["back", "fields", "front", "palette"]
assert printer.eject() is True
"#,
        &[],
    );
}
