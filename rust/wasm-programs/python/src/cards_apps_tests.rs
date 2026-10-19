//! Host tests of the card example written in Python (docs/cards/CARDS.md "Programs and
//! apps"): the example package in `examples/cards/*` (bankd) byte-compiles, and its
//! ledger logic (tags, counters, replays, taps) runs against the `_peripheral` stand-in
//! of `cards_tests`.

use rustpython_vm::{compiler::Mode, Interpreter, Settings, VirtualMachine};

use super::cards_tests::FAKE_PERIPHERALS;
use super::ui_module::ui_native;

const PERIPHERAL_PY: &str = include_str!("peripheral.py");
const CARDS_PY: &str = include_str!("cards.py");
const EXAMPLES: &[(&str, &str, &str)] = &[
    ("bankd", include_str!("../../../../examples/cards/dev.ecm.bankd/main.py"), include_str!("../../../../examples/cards/dev.ecm.bankd/app.toml")),
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
    for (name, src, _) in EXAMPLES {
        set(&format!("{}_PY", name.to_uppercase()), src);
    }
    run(vm, &scope, &format!("{FAKE_PERIPHERALS}{FAKE_SHELL}{SETUP}"));
    scope
}

#[test]
fn manifests_declare_python_entries_and_valid_permissions() {
    for (name, _, toml) in EXAMPLES {
        let m = ecm_pkg::manifest::parse(toml).unwrap_or_else(|e| panic!("{name}: app.toml: {}: {}", e.field, e.message));
        assert_eq!(m.entry, "main.py", "{name}");
        assert!(m.id.starts_with("dev.ecm."), "{name}: {}", m.id);
    }
}

#[test]
fn every_example_byte_compiles() {
    let interp = interpreter();
    interp.enter(|vm| {
        for (name, src, _) in EXAMPLES {
            vm.compile(src, Mode::Exec, format!("{name}.py")).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    });
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
