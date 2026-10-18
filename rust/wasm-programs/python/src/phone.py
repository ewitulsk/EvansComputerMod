"""The phone's number, calls and texts, through telephonyd (man phone; docs/TELEPHONY.md).

    import phone

    print(phone.number())                    # '+12145550142', or None without a SIM
    print(phone.status())                    # {'registration': 'registered', 'carrier': ..., 'bars': '4', ...}

    mid = phone.sms_send("+12145550143", "On my way")
    print(phone.sms_status(mid))             # 'sending' / 'sent' / 'delivered' / 'failed'
    for m in phone.sms_inbox():              # [{'id', 'from', 'thread', 'time', 'text', 'kind', 'files', ...}]
        print(m["from"], m["text"])
    phone.mms_send(["+12145550143", "+12145550144"], "Look", ["/home/Pictures/view.png"])   # a group

    call = phone.dial("611")
    call.send_tones("1")                     # keypad tones
    call.play("/home/hello.wav")             # the file goes to the far end
    path = call.record(5)                    # the far end for 5 s, as a WAV
    call.hangup()

    phone.on_sms(lambda sender, text: print(sender, text))
    phone.on_mms(lambda sender, group, text, files: print(sender, files))
    def answer(call):                        # incoming calls
        call.answer(); call.play("/home/greeting.wav"); call.hangup()
    phone.on_call(answer)
    phone.run()                              # handle events until stopped (phone.poll() handles what's waiting)

    phone.contacts()                         # [{'name', 'numbers': [{'label', 'number'}], 'emails', 'note', ...}]
    phone.add_contact("Alex", "+12145550143")

A script needs the permissions an app would: `phone` for calls (and
everything), `sms` for texts only, `contacts` for the address book; texts and
calls a non-system app starts are confirmed by the player. Refusals,
telephonyd not running and its `err` replies raise PhoneError. Lua's phone
module has the same functions.
"""

import net

PORT = 7720
SMS_PORT = 7726
CONTACTS = "/home/.contacts/contacts.vcf"

# The transport (tests replace it).
_net = net


class PhoneError(Exception):
    """telephonyd refused or isn't running, or a permission is missing."""


def _enc(s):
    s = str(s)
    if s == "":
        return "-"
    if s == "-":
        return "%2D"
    out = []
    for b in s.encode("utf-8"):
        c = chr(b)
        if b <= 32 or b >= 127 or c in "%=,":
            out.append("%%%02X" % b)
        else:
            out.append(c)
    return "".join(out)


def _dec(s):
    if s == "-":
        return ""
    raw = bytearray()
    i = 0
    while i < len(s):
        if s[i] == "%" and i + 3 <= len(s):
            try:
                raw.append(int(s[i + 1:i + 3], 16))
                i += 3
                continue
            except ValueError:
                pass
        raw.extend(s[i].encode("utf-8"))
        i += 1
    return raw.decode("utf-8", "replace")


class _Conn:
    """One connection to a control port: request line in, `ok`/`err` + body + `.` out."""

    def __init__(self, port):
        try:
            self.fd = _net.tcp_connect("127.0.0.1", port)
        except Exception as e:
            raise PhoneError("telephonyd is not running (or this script may not use port %d): %s" % (port, e)) from None
        self.buf = b""

    def line(self, wait=True):
        while b"\n" not in self.buf:
            if not wait and not _net.tcp_ready(self.fd, 0):
                return None
            chunk = _net.tcp_recv(self.fd, 4096)
            if not chunk:
                raise PhoneError("telephonyd closed the connection")
            self.buf += chunk
        l, self.buf = self.buf.split(b"\n", 1)
        return l.decode("utf-8", "replace").rstrip("\r")

    def request(self, req):
        _net.tcp_send(self.fd, (req + "\n").encode("utf-8"))
        head = self.line()
        if head.startswith("err"):
            raise PhoneError(head[4:] or "refused")
        body = []
        while True:
            l = self.line()
            if l == ".":
                return body
            body.append(l[1:] if l.startswith("..") else l)

    def close(self):
        try:
            _net.tcp_close(self.fd)
        except Exception:
            pass


_conns = {}


def _call(req, port=PORT):
    for attempt in (0, 1):
        c = _conns.get(port)
        if c is None:
            c = _conns[port] = _Conn(port)
        try:
            return c.request(req)
        except PhoneError:
            raise
        except Exception:
            c.close()
            del _conns[port]
            if attempt:
                raise PhoneError("the connection to telephonyd failed") from None


def _kv(body):
    out = {}
    for l in body:
        k, _, v = l.partition(" ")
        out[k] = v
    return out


def _first_id(body):
    return int(body[0].split()[1])


# ------------------------------------------------------------- status

def status():
    """registration, msisdn, domain, carrier, bars, volte, hd, voicemail, unread, missed."""
    return _kv(_call("status", SMS_PORT))


def number():
    """This phone's number (E.164), or None (no SIM, no number, not registered yet)."""
    n = status().get("msisdn", "-")
    return None if n in ("", "-") else n


# ------------------------------------------------------------- texts

def _msg(l, thread):
    w = l.split(" ")
    # msg <id> <in|out> <from> <time> <status> <sms|mms> <text> <files>
    files = [_dec(f) for f in w[8].split(",") if f and f != "-"] if len(w) > 8 else []
    return {"id": int(w[1]), "incoming": w[2] == "in", "from": _dec(w[3]), "time": int(w[4]), "status": _dec(w[5]),
            "kind": w[6], "text": _dec(w[7]) if len(w) > 7 else "", "files": files,
            "thread": thread}


def threads():
    """[{'key', 'members', 'unread', 'time', 'preview'}] newest first."""
    out = []
    for l in _call("threads", SMS_PORT):
        w = l.split(" ")
        key = w[1]
        out.append({"key": key, "members": key.split(","), "unread": int(w[2]), "time": int(w[3]),
                    "preview": _dec(w[4]) if len(w) > 4 else ""})
    return out


def messages(thread, n=50):
    """The last n messages of a thread (its key: a number, or the group's numbers joined by commas)."""
    return [_msg(l, thread) for l in _call("messages %s %d" % (thread, n), SMS_PORT) if l.startswith("msg ")]


def sms_send(to, text):
    """Send a text; returns its id (sms_status follows it)."""
    return _first_id(_call("sms %s %s" % (_enc(to), _enc(text)), SMS_PORT))


def mms_send(to, text="", files=()):
    """A picture, voice note or group text: to is a number or a list (several = a group). Returns its id."""
    if isinstance(to, str):
        to = [to]
    return _first_id(_call("mms %s %s %s" % (",".join(_enc(t) for t in to), _enc(text or ""),
                                              ",".join(_enc(f) for f in files) if files else "-"), SMS_PORT))


def sms_status(mid):
    """'sending', 'sent', 'delivered' or 'failed' (None: no such message)."""
    for t in threads():
        for m in messages(t["key"], 200):
            if m["id"] == mid:
                return m["status"]
    return None


def sms_inbox(n=50):
    """Received texts and MMS, newest first."""
    out = []
    for t in threads():
        out.extend(m for m in messages(t["key"], n) if m["incoming"])
    out.sort(key=lambda m: m["time"], reverse=True)
    return out[:n]


def mark_read(thread):
    _call("read %s" % thread, SMS_PORT)


# ------------------------------------------------------------- calls

class Call:
    """A call: its id and the other end's number (hidden callers: 'anonymous')."""

    def __init__(self, cid, number="", incoming=False):
        self.id = cid
        self.number = number
        self.incoming = incoming
        self._tones = ""

    def __repr__(self):
        return "Call(%d, %r, %s)" % (self.id, self.number, self.state())

    def info(self):
        """{'state', 'number', 'hidden', 'outgoing', 'seconds', 'muted', 'hd', 'codec', 'loss', 'merged'}, or None once it ended."""
        for l in _call("calls"):
            w = l.split(" ")
            if int(w[1]) == self.id:
                return {"state": w[2], "number": _dec(w[3]), "hidden": w[4] == "1", "outgoing": w[5] == "1",
                        "seconds": int(w[6]), "muted": w[7] == "1", "hd": w[8] == "1", "codec": w[9],
                        "loss": float(w[10]), "merged": w[11] == "1"}
        return None

    def state(self):
        """dialing, ringing (theirs), incoming, active, held, ended."""
        i = self.info()
        return i["state"] if i else "ended"

    def wait(self, states=("active", "ended"), timeout=30):
        """Wait until the call is in one of states (or timeout seconds); returns the state."""
        import time
        end = time.time() + timeout
        while True:
            s = self.state()
            if s in states or time.time() >= end:
                return s
            poll(0.2)

    def answer(self):
        _call("answer %d" % self.id)

    def reject(self):
        _call("decline %d" % self.id)

    def hangup(self):
        _call("hangup %d" % self.id)

    def hold(self, on=True):
        _call("hold %d %s" % (self.id, "on" if on else "off"))

    def mute(self, on=True):
        _call("mute %d %s" % (self.id, "on" if on else "off"))

    def send_tones(self, digits):
        """Keypad tones (0-9 * # A-D) to the far end, e.g. to work an IVR."""
        _call("dtmf %d %s" % (self.id, digits))

    def tones(self):
        """Keypad tones the far end pressed since the last call (from events: poll() or run() collects them)."""
        poll()
        t, self._tones = self._tones, ""
        return t

    def play(self, wav):
        """Send a WAV file to the far end instead of the microphone."""
        _call("play %d %s" % (self.id, _enc(wav)))

    def record(self, seconds, path=None):
        """Record the far end for seconds into a WAV file; returns its path."""
        path = path or "/home/Recordings/call-%d.wav" % self.id
        _call("record %d %d %s" % (self.id, int(seconds), _enc(path)))
        return path


_calls = {}


def _call_obj(cid, number="", incoming=False):
    c = _calls.get(cid)
    if c is None:
        c = _calls[cid] = Call(cid, number, incoming)
    return c


def dial(number, hide=False):
    """Place a call (hide: withhold your number). Returns a Call."""
    body = _call("dial %s%s" % (_enc(number), " hide" if hide else ""))
    return _call_obj(_first_id(body), number)


def calls():
    """The calls in progress, as Call objects."""
    out = []
    for l in _call("calls"):
        w = l.split(" ")
        out.append(_call_obj(int(w[1]), _dec(w[3]), w[5] == "0"))
    return out


def speaker(on=True):
    _call("speaker %s" % ("on" if on else "off"))


def call_log(n=50):
    """[{'id', 'dir' (in/out/missed), 'number', 'start', 'seconds', 'reason'}] newest first."""
    out = []
    for l in _call("log %d" % n):
        w = l.split(" ")
        out.append({"id": int(w[1]), "dir": w[2], "number": _dec(w[3]), "start": int(w[4]), "seconds": int(w[5]),
                    "reason": _dec(w[6]) if len(w) > 6 else ""})
    return out


# ------------------------------------------------------------- events

_handlers = {"sms": [], "mms": [], "call": []}
_watch = None


def _open_watch():
    global _watch
    port = PORT if _handlers["call"] else SMS_PORT
    if _watch is not None:
        if _watch[0] == port:
            return
        _watch[1].close()
    c = _Conn(port)
    _net.tcp_send(c.fd, b"watch\n")
    if not c.line().startswith("ok"):
        raise PhoneError("telephonyd refused watch")
    try:
        # Texts and calls wake a sleeping phone for this script.
        _net.set_wake(c.fd, True)
    except Exception:
        pass
    _watch = (port, c)


def on_sms(fn):
    """fn(sender, text) for each new text (handled in run() / poll())."""
    _handlers["sms"].append(fn)
    _open_watch()


def on_mms(fn):
    """fn(sender, group, text, files) for each new MMS; group is the other members (empty for a 1:1);
    files are the saved parts under /home/Messages/."""
    _handlers["mms"].append(fn)
    _open_watch()


def on_call(fn):
    """fn(call) for each incoming call (call.answer(), call.reject(), call.tones())."""
    _handlers["call"].append(fn)
    _open_watch()


def _event(line):
    w = line.split(" ")
    name = w[0]
    f = {}
    for kv in w[1:]:
        k, _, v = kv.partition("=")
        f[k] = _dec(v)
    return name, f


def _dispatch(name, f):
    if name == "sms":
        for fn in _handlers["sms"]:
            fn(f.get("from", ""), f.get("text", ""))
    elif name == "mms":
        me = number() if _handlers["mms"] else None
        group = [m for m in f.get("thread", "").split(",") if m and m != f.get("from") and m != me]
        files = [x for x in f.get("files", "").split(",") if x]
        for fn in _handlers["mms"]:
            fn(f.get("from", ""), group if len(group) > 0 and "," in f.get("thread", "") else [], f.get("text", ""), files)
    elif name == "incoming":
        c = _call_obj(int(f.get("call", "0")), f.get("from", ""), True)
        for fn in _handlers["call"]:
            fn(c)
    elif name == "dtmf":
        c = _calls.get(int(f.get("call", "0")))
        if c is not None:
            c._tones += f.get("digit", "")


def poll(timeout=0):
    """Handle the events waiting (waiting up to timeout seconds for the first). Returns how many."""
    if _watch is None:
        if timeout:
            import time
            time.sleep(timeout)
        return 0
    c = _watch[1]
    n = 0
    wait_ms = int(timeout * 1000)
    while True:
        if b"\n" not in c.buf and not _net.tcp_ready(c.fd, wait_ms):
            return n
        wait_ms = 0
        l = c.line()
        if l:
            _dispatch(*_event(l))
            n += 1


def run():
    """Handle events until the script is stopped."""
    if _watch is None:
        raise PhoneError("nothing to wait for: register on_sms, on_mms or on_call first")
    while True:
        poll(1)


# ------------------------------------------------------------- contacts

def _vunescape(s):
    return s.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")


def _vescape(s):
    return s.replace("\\", "\\\\").replace("\n", "\\n").replace(",", "\\,").replace(";", "\\;")


def _read_book():
    try:
        import shell
        return shell.read_file(CONTACTS) or ""
    except ImportError:
        pass
    try:
        with open(CONTACTS, "r") as f:
            return f.read()
    except OSError:
        return ""


def _write_book(text):
    try:
        import shell
    except ImportError:
        shell = None
    if shell is not None:
        # shell.write_file makes the folder too
        ok = shell.write_file(CONTACTS, text)
    else:
        try:
            with open(CONTACTS, "w") as f:
                f.write(text)
            ok = True
        except OSError:
            ok = False
    if not ok:
        raise PhoneError("can't write the address book (does this script have the contacts permission?)")


def contacts():
    """The address book: [{'name', 'numbers': [{'label', 'number'}], 'emails', 'note', 'favourite'}]."""
    out = []
    cur = None
    lines = []
    for raw in _read_book().replace("\r", "").split("\n"):
        if raw[:1] in (" ", "\t") and lines:
            lines[-1] += raw[1:]
        elif raw.strip():
            lines.append(raw)
    for l in lines:
        key, _, value = l.partition(":")
        name = key.split(";")[0].upper()
        if name == "BEGIN":
            cur = {"name": "", "numbers": [], "emails": [], "note": "", "favourite": False}
        elif cur is None:
            continue
        elif name == "FN":
            cur["name"] = _vunescape(value)
        elif name == "TEL":
            label = ""
            for p in key.split(";")[1:]:
                if p.upper().startswith("TYPE="):
                    label = p[5:].split(",")[0].lower()
            cur["numbers"].append({"label": label, "number": _vunescape(value)})
        elif name == "EMAIL":
            cur["emails"].append(_vunescape(value))
        elif name == "NOTE":
            cur["note"] = _vunescape(value)
        elif name == "X-ECM-FAVOURITE":
            cur["favourite"] = value.strip() == "1"
        elif name == "END":
            out.append(cur)
            cur = None
    out.sort(key=lambda c: c["name"].lower())
    return out


def add_contact(name, number, label="cell"):
    """Add a contact to the address book (Contacts shows it)."""
    words = name.split()
    family = words.pop() if len(words) > 1 else ""
    card = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:%s\r\nN:%s;%s;;;\r\nTEL;TYPE=%s:%s\r\nEND:VCARD\r\n" % (
        _vescape(name), _vescape(family), _vescape(" ".join(words)), label.upper(), _vescape(number))
    text = _read_book()
    if text and not text.endswith("\n"):
        text += "\r\n"
    _write_book(text + card)
