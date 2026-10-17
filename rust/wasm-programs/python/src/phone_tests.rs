//! Host tests of the Python `phone` module (phone.py) on a real RustPython against
//! a scripted telephonyd: a stand-in `net` whose connections answer with
//! telephonyd's control protocol (ecm_tel::ctl). The same cases as Lua's
//! phone_tests, so both interpreters send the same requests.

use rustpython_vm::{compiler::Mode, Interpreter, Settings};

const PHONE_PY: &str = include_str!("phone.py");

const FAKE_TELEPHONYD: &str = r#"
import sys
sent = []
_events = []
_replies = {
    "status": "ok\nregistration registered\nmsisdn +12145550142\ncarrier cell-red\nbars 4\n.\n",
    "threads": "ok\nthread +12145550143 1 9000 See%20you\nthread +12145550143,+12145550144 0 8000 Hi\n.\n",
    "messages +12145550143 200": "ok\nmsg 7 out +12145550142 5000 delivered sms On%20my%20way -\nmsg 8 in +12145550143 9000 received sms See%20you -\n.\n",
    "messages +12145550143 50": "ok\nmsg 7 out +12145550142 5000 delivered sms On%20my%20way -\nmsg 8 in +12145550143 9000 received sms See%20you -\n.\n",
    "messages +12145550143,+12145550144 200": "ok\nmsg 9 in +12145550144 8000 received mms Hi /home/Messages/9/a.png\n.\n",
    "messages +12145550143,+12145550144 50": "ok\nmsg 9 in +12145550144 8000 received mms Hi /home/Messages/9/a.png\n.\n",
    "sms +12145550143 On%20my%20way": "ok\nsms 7\n.\n",
    "dial 611": "ok\ncall 3\n.\n",
    "calls": "ok\ncall 3 active 611 0 1 42 0 1 ADPCM-WB 0.0 0\n.\n",
    "dial %2D": "err not a number\n",
}

class FakeNet:
    def __init__(self):
        self.conns = {}
        self.next = 1
    def tcp_connect(self, host, port):
        assert host == "127.0.0.1"
        fd = self.next
        self.next += 1
        self.conns[fd] = {"port": port, "out": b"", "watch": False}
        return fd
    def tcp_send(self, fd, data):
        c = self.conns[fd]
        for l in data.decode().split("\n")[:-1]:
            sent.append("%d %s" % (c["port"], l))
            if l == "watch":
                c["watch"] = True
                c["out"] += b"ok\n"
            else:
                c["out"] += _replies.get(l, "ok\n.\n").encode()
        return len(data)
    def tcp_ready(self, fd, timeout=0):
        c = self.conns[fd]
        if c["watch"] and _events:
            c["out"] += (_events.pop(0) + "\n").encode()
        return c["out"] != b""
    def tcp_recv(self, fd, n):
        c = self.conns[fd]
        if c["watch"] and not c["out"] and _events:
            c["out"] = (_events.pop(0) + "\n").encode()
        s, c["out"] = c["out"][:n], c["out"][n:]
        return s
    def tcp_close(self, fd):
        self.conns.pop(fd, None)
    def set_wake(self, fd, on=True):
        self.conns[fd]["wake"] = on

fake = FakeNet()
sys.modules["net"] = fake
P = type(sys)("phone")
exec(compile(PHONE_PY, "<phone>", "exec"), P.__dict__)
sys.modules["phone"] = P
import phone
phone._net = fake
def push(line):
    _events.append(line)
"#;

fn run_py(code: &str, vars: &[(&str, String)]) {
    let interp = Interpreter::with_init(Settings::default(), |_| {});
    interp.enter(|vm| {
        let scope = vm.new_scope_with_builtins();
        for (k, v) in vars {
            scope.globals.set_item(*k, vm.ctx.new_str(v.as_str()).into(), vm).unwrap();
        }
        scope.globals.set_item("PHONE_PY", vm.ctx.new_str(PHONE_PY).into(), vm).unwrap();
        let full = format!("{FAKE_TELEPHONYD}{code}");
        let code_obj = vm.compile(&full, Mode::Exec, "<test>".to_owned()).expect("test script compiles");
        if let Err(e) = vm.run_code_obj(code_obj, scope) {
            let mut s = String::new();
            vm.write_exception(&mut s, &e).unwrap();
            panic!("python raised:\n{s}");
        }
    });
}

#[test]
fn texts_calls_and_events_through_telephonyd() {
    run_py(
        r#"
assert phone.number() == "+12145550142" and phone.status()["bars"] == "4"
assert phone.sms_send("+12145550143", "On my way") == 7
assert phone.sms_status(7) == "delivered"
inbox = ["%s:%s:%d" % (m["from"], m["text"], len(m["files"])) for m in phone.sms_inbox()]
assert inbox == ["+12145550143:See you:0", "+12145550144:Hi:1"], inbox
c = phone.dial("611")
c.send_tones("1#")
i = c.info()
assert (c.id, c.state(), i["seconds"], i["hd"], i["codec"]) == (3, "active", 42, True, "ADPCM-WB"), i
try:
    phone.dial("-")
    raise AssertionError("no error")
except phone.PhoneError as e:
    assert str(e) == "not a number", str(e)
got = []
phone.on_sms(lambda f, t: got.append("sms %s %s" % (f, t)))
phone.on_mms(lambda f, g, t, files: got.append("mms %s %s %s %s" % (f, ",".join(g), t, files[0])))
def ring(call):
    got.append("call %d %s" % (call.id, call.number))
    call.answer()
phone.on_call(ring)
push("sms id=10 from=+12145550143 thread=+12145550143 text=Hello%20there")
push("mms id=11 from=+12145550144 thread=+12145550143,+12145550144 text=Look files=/home/Messages/11/a.png")
push("incoming call=5 from=+12145550145 hidden=0 waiting=0")
push("dtmf call=5 digit=4")
n = phone.poll(0)
got.append("%d %d" % (n, phone.calls()[0].id))
assert got == ["sms +12145550143 Hello there", "mms +12145550144 +12145550143 Look /home/Messages/11/a.png", "call 5 +12145550145", "4 3"], got
s = "|".join(sent)
assert "7726 sms +12145550143 On%20my%20way" in s, s
assert "7720 dial 611" in s and "7720 dtmf 3 1#" in s and "7720 answer 5" in s, s
assert "7726 watch" in s and "7720 watch" in s, s
"#,
        &[],
    );
}

#[test]
fn the_address_book_is_the_contacts_vcard_file() {
    let d = std::env::temp_dir().join(format!("ecm-py-phone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join(".contacts")).unwrap();
    let path = d.join(".contacts/contacts.vcf").to_string_lossy().replace('\\', "/");
    run_py(
        r#"
phone.CONTACTS = BOOK
phone.add_contact("Bo Jones", "+12145550143")
phone.add_contact("Alex, the Builder", "+12145550144", "work")
got = ["%s=%s:%s" % (c["name"], c["numbers"][0]["label"], c["numbers"][0]["number"]) for c in phone.contacts()]
assert got == ["Alex, the Builder=work:+12145550144", "Bo Jones=cell:+12145550143"], got
"#,
        &[("BOOK", path.clone())],
    );
    let cards = ecm_tel::vcard::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[1].name, "Alex, the Builder");
    let _ = std::fs::remove_dir_all(&d);
}
