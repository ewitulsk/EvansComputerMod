//! Host tests of the card programs written in Python (docs/cards/CARDS.md "Programs and
//! apps"): the Card Designer app shipped in the mod's resources, run against a mock
//! compositor connection (`_ui.use_mock`) and the `_peripheral` stand-in of
//! `cards_tests`; and the example packages in `examples/cards/*` (bankd, pos, doord,
//! skimmer, rx_nfc): every one byte-compiles, and their logic (bankd's ledger: tags,
//! counters, replays, taps; pos's requests; doord's policy; skimmer's copy; rx_nfc's
//! burst detector and probe) runs against the same stand-ins.

use ecm_ui::{samples, MockConnection};
use ecm_ui_proto::Message;
use rustpython_vm::{compiler::Mode, Interpreter, Settings, VirtualMachine};

use super::cards_tests::FAKE_PERIPHERALS;
use super::ui_module::{register_mock, ui_native};

const PERIPHERAL_PY: &str = include_str!("peripheral.py");
const CARDS_PY: &str = include_str!("cards.py");
const DESIGNER_PY: &str =
    include_str!("../../../../src/main/resources/assets/evanscomputermod/ecm_apps/dev.ecm.carddesigner/main.py");
const DESIGNER_TOML: &str =
    include_str!("../../../../src/main/resources/assets/evanscomputermod/ecm_apps/dev.ecm.carddesigner/app.toml");
const EXAMPLES: &[(&str, &str, &str)] = &[
    ("bankd", include_str!("../../../../examples/cards/dev.ecm.bankd/main.py"), include_str!("../../../../examples/cards/dev.ecm.bankd/app.toml")),
    ("pos", include_str!("../../../../examples/cards/dev.ecm.pos/main.py"), include_str!("../../../../examples/cards/dev.ecm.pos/app.toml")),
    ("doord", include_str!("../../../../examples/cards/dev.ecm.doord/main.py"), include_str!("../../../../examples/cards/dev.ecm.doord/app.toml")),
    ("skimmer", include_str!("../../../../examples/cards/dev.ecm.skimmer/main.py"), include_str!("../../../../examples/cards/dev.ecm.skimmer/app.toml")),
    ("rx_nfc", include_str!("../../../../examples/cards/dev.ecm.rxnfc/main.py"), include_str!("../../../../examples/cards/dev.ecm.rxnfc/app.toml")),
];

/// A `shell` stand-in with in-memory files (the apps' Open dialog lists files with it).
const FAKE_SHELL: &str = r#"
_inputs = []
_files = {}
def _norm(path):
    return "/" + str(path).lstrip("/")
def _read_file(path):
    return _files.get(_norm(path))
def _write_file(path, text):
    _files[_norm(path)] = text
    return True
def _file_exists(path):
    return _norm(path) in _files
def _list_files():
    return "\n".join(sorted(_files))
def _input(prompt=""):
    return _inputs.pop(0) if _inputs else ""
S = type(sys)("shell")
S.read_file = _read_file
S.write_file = _write_file
S.file_exists = _file_exists
S.list_files = _list_files
S.input = _input
S.sleep = lambda s: None
sys.modules["shell"] = S
sys.modules["terminal"] = S
"#;

const SETUP: &str = r#"
M = type(sys)('peripheral')
exec(compile(PERIPHERAL_PY, '<peripheral>', 'exec'), M.__dict__)
sys.modules['peripheral'] = M
C = type(sys)('cards')
exec(compile(CARDS_PY, '<cards>', 'exec'), C.__dict__)
sys.modules['cards'] = C
import _ui
U = type(sys)('ui')
exec(compile(_ui._source, '<ui>', 'exec'), U.__dict__)
sys.modules['ui'] = U
import cards, peripheral, ui
# The host test interpreter has no os / random (in-game WASI has): deterministic draws here.
cards.random_hex = lambda n: "5a" * n
def load_app(name, source):
    A = type(sys)(name)
    A.__dict__['_TESTING'] = True
    exec(compile(source, name + '.py', 'exec'), A.__dict__)
    return A
"#;

fn interpreter() -> Interpreter {
    Interpreter::with_init(Settings::default(), |vm| {
        vm.add_native_module("_ui".to_owned(), Box::new(ui_native::make_module));
        vm.add_native_module("_console".to_owned(), Box::new(crate::console_module::make_native_module));
    })
}

fn run(vm: &VirtualMachine, scope: &rustpython_vm::scope::Scope, code: &str) {
    let code_obj = vm.compile(code, Mode::Exec, "<test>".to_owned()).expect("test script compiles");
    if let Err(e) = vm.run_code_obj(code_obj, scope.clone()) {
        let mut s = String::new();
        vm.write_exception(&mut s, &e).unwrap();
        panic!("python raised:\n{s}");
    }
}

/// The app's data folder: a computer path that doesn't exist on the host, so the apps'
/// `open()` fails and their `shell` fallback (the in-memory stand-in) carries the files.
fn tmpdir(name: &str) -> String {
    format!("/var/apps/dev.ecm.test-{name}")
}

/// A scope with the stand-ins installed, `TMP` set and the example sources as globals.
fn scope(vm: &VirtualMachine, tmp: &str) -> rustpython_vm::scope::Scope {
    let scope = vm.new_scope_with_builtins();
    let set = |k: &str, v: &str| scope.globals.set_item(k, vm.ctx.new_str(v).into(), vm).unwrap();
    set("TMP", tmp);
    set("PERIPHERAL_PY", PERIPHERAL_PY);
    set("CARDS_PY", CARDS_PY);
    set("DESIGNER_PY", DESIGNER_PY);
    for (name, src, _) in EXAMPLES {
        set(&format!("{}_PY", name.to_uppercase()), src);
    }
    run(vm, &scope, &format!("{FAKE_PERIPHERALS}{FAKE_SHELL}{SETUP}"));
    scope
}

#[test]
fn manifests_declare_python_entries_and_valid_permissions() {
    for (name, toml) in std::iter::once(("carddesigner", DESIGNER_TOML)).chain(EXAMPLES.iter().map(|(n, _, t)| (*n, *t))) {
        let m = ecm_pkg::manifest::parse(toml).unwrap_or_else(|e| panic!("{name}: app.toml: {}: {}", e.field, e.message));
        assert_eq!(m.entry, "main.py", "{name}");
        assert!(m.id.starts_with("dev.ecm."), "{name}: {}", m.id);
    }
    let d = ecm_pkg::manifest::parse(DESIGNER_TOML).unwrap();
    assert_eq!(d.id, "dev.ecm.carddesigner");
    assert_eq!(d.permissions, vec!["peripherals:card_printer".to_string()]);
}

#[test]
fn every_example_byte_compiles() {
    let interp = interpreter();
    interp.enter(|vm| {
        for (name, src, _) in EXAMPLES {
            vm.compile(src, Mode::Exec, format!("{name}.py")).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        vm.compile(DESIGNER_PY, Mode::Exec, "carddesigner.py".to_owned()).expect("designer compiles");
    });
}

#[test]
fn card_designer_draws_saves_opens_and_prints() {
    let tmp = tmpdir("designer");
    let mock = MockConnection::new();
    mock.state.borrow_mut().idle_timeouts = true;
    mock.push(&[samples::welcome(false)]);
    let handle = register_mock(mock.clone());
    let interp = interpreter();
    interp.enter(|vm| {
        let scope = scope(vm, &tmp);
        run(vm, &scope, &format!("_ui.use_mock({handle})\n"));
        run(
            vm,
            &scope,
            r#"
D = load_app("carddesigner", DESIGNER_PY)
# The chassis masks: the chip pad on the front, the stripe rows on the back.
assert sum(D.face_mask("bank", "front")) == 6 * 5 and sum(D.face_mask("bank", "back")) == 4 * 32
assert sum(D.face_mask("hybrid", "front")) == 30 and sum(D.face_mask("hybrid", "back")) == 128
assert sum(D.face_mask("nfc", "front")) == 0 and sum(D.face_mask("nfc", "back")) == 0
assert sum(D.face_mask("sim", "front")) == 6 * 5 and sum(D.face_mask("sim", "back")) == 0
m = D.face_mask("bank", "front")
assert m[7 * 32 + 3] == 1 and m[11 * 32 + 8] == 1 and m[7 * 32 + 2] == 0 and m[12 * 32 + 3] == 0
d = D.Designer(data_dir=TMP)
assert d.design.kind == "bank" and len(d.design.palette) == 16
assert d.grid_front.width == 32 and d.grid_front.height == 20 and d.grid_front.mask == m
"#,
        );
        // The compositor configures the window: the first draw goes out.
        mock.push(&[Message::Configure(ecm_ui_proto::Configure {
            surface: 1,
            serial: 1,
            w: 640,
            h: 420,
            states: ecm_ui_proto::window_states::ACTIVATED,
        })]);
        run(vm, &scope, "d.step(0)\nd.step(0)\n");
        run(
            vm,
            &scope,
            r#"
# Tools, swatches and the picker.
d.set_tool("fill")
assert d.grid_front.tool == "fill" and d.grid_back.tool == "fill"
d.set_tool("pencil")
d.set_mirror(True)
assert d.grid_front.mirror_h and d.grid_back.mirror_h
d.set_mirror(False)
d.select_index(2)
assert d.grid_front.color == 2 and d.picker.value == D.rgb_int(d.design.palette[2])
d.set_color(0xFF0000)
assert d.design.palette[2] == [255, 0, 0] and d.grid_back.palette[2] == 0xFF0000 and d.swatches.palette[2] == 0xFF0000
# A stroke on the front face reaches the design (the widget's own sync keeps .pixels).
ev = ui.Event("change", [(0, 0, 2), (31, 19, 1)], d.grid_front)
d.grid_front._sync_change(ev)
d._change_handler("front")(ev)
assert d.design.pixel("front", 0, 0) == 2 and d.design.pixel("front", 31, 19) == 1 and d.grid_front.pixel(0, 0) == 2
# The eyedropper's pick becomes the paint colour.
d._on_pick(ui.Event("select", 5, d.grid_front))
assert d.index == 5 and d.grid_back.color == 5
# Fields and stripe.
d.set_field("name", "Ann Example")
d.set_field("label", "")
d.set_track(0, "ANN EXAMPLE")
d.set_track(1, "12345678")
assert d.design.fields == {"name": "Ann Example"} and d.design.stripe == ("ANN EXAMPLE", "12345678", "")
# Save, New, Open.
d.filename.text = "acme"
path = d.save()
assert path == TMP + "/acme.card", path
assert D.card_files(TMP) == ["acme.card"], D.card_files(TMP)
d.ask_kind()
assert d.dialog is not None and len(d.stack.children) == 2
d.new_card("sim")
assert d.dialog is None and d.design.kind == "sim" and d.grid_front.width == 8 and d.grid_front.height == 13
assert not d.tracks[0].enabled and d.design.stripe is None
assert d.tabs.pages == [d.grid_front, d.grid_back]
d.ask_open()
assert d.dialog is not None
assert d.load("acme.card")
assert d.design.kind == "bank" and d.design.fields == {"name": "Ann Example"}
assert d.design.stripe == ("ANN EXAMPLE", "12345678", "") and d.design.pixel("front", 0, 0) == 2
assert d.design.palette[2] == [255, 0, 0] and d.tracks[0].text == "ANN EXAMPLE" and d.fields["name"].text == "Ann Example"
assert d.filename.text == "acme.card"
# Print: the printer's cost, then print, then its verdict as an event.
calls[:] = []
assert d.print_card()
assert [c[1] for c in calls if c[0] == "top"] == ["present", "cost", "print"], calls
assert calls[-1][2][0]["fields"] == {"name": "Ann Example"} and calls[-1][2][0]["stripe"]["t1"] == "ANN EXAMPLE"
assert d.printing == "top" and not d.print_button.enabled and "Printing on top" in d.status_label.text
_events.append(("print_failed", "top", "no_dye:red"))
d.poll_printer()
assert d.printing is None and d.print_button.enabled and "no_dye:red" in d.status_label.text, d.status_label.text
assert d.print_card()
_events.append(("card_inserted", "left", "bank", ["bank"]))
_events.append(("print_done", "top"))
d.poll_printer()
assert d.printing is None and d.status_label.text == "Printed on top.", d.status_label.text
# No printer: Print says to Export, and Export writes the file.
_devices.remove(("top", "card_printer"))
assert not d.print_card() and "Export" in d.status_label.text
d.filename.text = "exported"
d.export()
assert "cardtool print " + TMP + "/exported.card" in d.status_label.text, d.status_label.text
assert D.card_files(TMP) == ["acme.card", "exported.card"]
# The window still serialises after all that (the dialog is closed, the tree is sent).
d.step(0)
"#,
        );
        run(vm, &scope, "_ui.use_mock(None)\n");
    });
    let sent = mock.sent_messages();
    let tree_ops = sent.iter().filter(|m| matches!(m, Message::TreeOps(_))).count();
    assert!(tree_ops >= 2, "only {tree_ops} TreeOps messages");
}

#[test]
fn bankd_issues_cards_and_verifies_counted_signed_payments() {
    let tmp = tmpdir("bankd");
    let interp = interpreter();
    interp.enter(|vm| {
        let scope = scope(vm, &tmp);
        run(
            vm,
            &scope,
            r#"
B = load_app("bankd", BANKD_PY)
L = B.Ledger(TMP + "/accounts.json")
key = "11" * 32
L.open_account("4000123412341234", "Ann", key)
assert L.deposit("4000123412341234", 100) == 100
def pay(ctr, amount, nonce="0102030405060708", tag=None):
    t = tag if tag is not None else cards.tag(key, nonce, ctr, amount)
    return L.authorize({"pan": "4000123412341234", "nonce": nonce, "ctr": ctr, "tag": t, "amount": amount})
r = pay(1, 30)
assert r == {"ok": True, "balance": 70}, r
# Replay: the same signed request again, and any older counter, is refused.
assert pay(1, 30)["reason"].startswith("replay")
assert pay(0, 30)["reason"].startswith("replay")
# A forged tag (no key) is refused; a counter that only moves forward is required.
assert pay(2, 30, tag="00" * 16) == {"ok": False, "reason": "bad tag"}
assert pay(3, 500)["reason"] == "insufficient funds"
assert pay(4, 70) == {"ok": True, "balance": 0}
assert L.authorize({"pan": "9999", "ctr": 1}) == {"ok": False, "reason": "unknown card"}
assert L.authorize({"pan": "4000123412341234", "ctr": "x"})["reason"] == "malformed"
# The ledger persists: a fresh Ledger sees the counter and balance.
L2 = B.Ledger(TMP + "/accounts.json")
assert L2.accounts["4000123412341234"]["ctr"] == 4 and L2.accounts["4000123412341234"]["balance"] == 0
# Taps: a static token read from sector 1; right token pays, anything else doesn't.
L2.open_account("4000000000000002", "Bob", "22" * 32, tap_token="ab" * 16)
L2.deposit("4000000000000002", 10)
assert L2.authorize({"pan": "4000000000000002", "token": "AB" * 16, "amount": 4, "tap": True}) == {"ok": True, "balance": 6}
assert L2.authorize({"pan": "4000000000000002", "token": "cd" * 16, "amount": 4, "tap": True})["reason"] == "bad tap"
assert L2.authorize({"pan": "4000123412341234", "token": "ab" * 16, "amount": 4, "tap": True})["reason"] == "bad tap"
# issue: writes the blank card in the reader, stores the key, enables a tap on request.
_results["bank_blank"] = True
calls[:] = []
L3 = B.Ledger(TMP + "/accounts.json")
assert B.issue(["Cy", "4000111122223333", "--pin", "1234", "--tap"], L3) == 0
names = [c[1] for c in calls if c[0] == "left"]
assert "write_bank" in names and "write_fields" in names and "eject" in names, names
assert names.index("nfc_auth") < names.index("nfc_write") < names.index("nfc_set_key"), names
wb = [c for c in calls if c[1] == "write_bank"][0][2]
assert wb[0] == "acme" and wb[1] == "4000111122223333" and len(wb[2]) == 64 and wb[3] == "1234" and wb[4] is True, wb
acct = L3.accounts["4000111122223333"]
assert acct["key_hex"] == wb[2] and acct["pin"] and len(acct["tap_token"]) == 32 and acct["name"] == "Cy"
assert [c for c in calls if c[1] == "nfc_set_key"][0][2] == [1, L3.terminal_key()]
sector = [c for c in calls if c[1] == "nfc_write"][0][2][1]
assert bytes.fromhex(sector).startswith(b"4000111122223333\x00" + bytes.fromhex(acct["tap_token"]))
# A written chip is not reissued.
_results["bank_blank"] = False
assert B.issue(["Dee", "4000444455556666"], L3) == 1
"#,
        );
    });
}

#[test]
fn pos_doord_skimmer_and_rx_nfc_run_against_the_stand_ins() {
    let tmp = tmpdir("examples");
    let interp = interpreter();
    interp.enter(|vm| {
        let scope = scope(vm, &tmp);
        run(
            vm,
            &scope,
            r#"
# pos: a chip request carries the PAN, a fresh nonce and the chip's (ctr, tag); a tap request
# the PAN and token from sector 1.
P = load_app("pos", POS_PY)
reader = cards.Reader.find()
req = P.chip_request(reader.card(), 30)
assert req["pan"] == "4000123412341234" and req["ctr"] == 5 and req["tag"] == "ab" * 16 and req["amount"] == 30
assert len(req["nonce"]) == 16
nfc = cards.Nfc.find()
_results["read"] = (b"4000123412341234\x00".hex() + "cd" * 16).ljust(128, "0")
tap = P.tap_request(nfc, "0102030405060708", "33" * 16, 7)
assert tap == {"pan": "4000123412341234", "token": "cd" * 16, "amount": 7, "tap": True}, tap
_results["read"] = "11" * 64
try:
    P.tap_request(nfc, "0102030405060708", "33" * 16, 7)
    assert False, "no PAN in sector 1 must be refused"
except ValueError:
    pass
# wait_for_payment takes the chip in the reader first and ejects it.
calls[:] = []
req = P.wait_for_payment(reader, None, 12, None, timeout=2)
assert req["amount"] == 12 and ("left", "eject", []) in calls

# doord: the allow list plus a sector auth, or (deliberately) the UID alone.
Dd = load_app("doord", DOORD_PY)
door = Dd.Door(["0102030405060708"], cards.FACTORY_KEY, sector=1)
assert door.decide("0102030405060708", nfc) == (True, "sector 1 authenticated")
assert door.decide("ffffffffffffffff", nfc) == (False, "not on the list")
bad = Dd.Door(["0102030405060708"], uid_only=True)
ok, why = bad.decide("0102030405060708", nfc)
assert ok and "unverified" in why
latch = Dd.Latch(0)
assert latch.link is None
log = Dd.run(door, nfc, latch, hold=0.5, rounds=1)
assert log == [("0102030405060708", True, "sector 1 authenticated")], log

# skimmer: the tracks of one card become the stripe of another.
Sk = load_app("skimmer", SKIMMER_PY)
calls[:] = []
tracks = Sk.clone(reader.card(), reader.card())
assert tracks == ("ANN", "12345678", "") and ("left", "write_stripe", ["ANN", "12345678", ""]) in calls

# rx_nfc: bursts from block powers, and the probe's frame exchange through raw().
R = load_app("rx_nfc", RX_NFC_PY)
det = R.BurstDetector(threshold_dbm=-80, block_s=0.01)
powers = [-100] * 5 + [-50, -45, -50] + [-100] * 10 + [-60] * 12 + [-100]
bursts = [b for b in (det.feed(p) for p in powers) if b is not None]
assert len(bursts) == 2 and abs(bursts[0][1] - 0.03) < 1e-9 and bursts[0][2] == -45 and abs(bursts[1][1] - 0.12) < 1e-9, bursts
assert "POLL-sized" in R.BurstDetector.describe(bursts[0]) and "session-sized" in R.BurstDetector.describe(bursts[1])
_results["raw"] = "8109" + "0102030405060708" + "02"
calls[:] = []
assert R.probe() == 0
raws = [c[2][0] for c in calls if c[1] == "raw"]
assert raws == ["0100", "02080102030405060708"], raws
_results["raw"] = "8100"
assert R.probe() == 1
"#,
        );
    });
}
