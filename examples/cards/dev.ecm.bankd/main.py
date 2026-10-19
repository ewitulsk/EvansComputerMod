"""bankd: a toy card issuer (docs/cards/CARDS.md "Bank chip").

The lesson: a chip card is *keyed and counted*. The card holds a secret key K
that never leaves it; for every payment it increments its counter and signs
(nonce, counter, amount) with K. The issuer keeps its own copy of K and
accepts a payment only when the tag verifies AND the counter is newer than
the last one it saw. A thief who records a payment can't replay it (old
counter) or forge one (no K); a stripe or a sniffed NFC session gives them
nothing a chip payment needs. Compare: a stripe, which is plain text.

    python /apps/dev.ecm.bankd/main.py issue <name> <pan> [--pin 1234] [--issuer acme] [--tap]
    python /apps/dev.ecm.bankd/main.py serve <port>
    python /apps/dev.ecm.bankd/main.py deposit <pan> <amount>
    python /apps/dev.ecm.bankd/main.py accounts

(or `pkg run dev.ecm.bankd serve 7000` once installed from the App Store).

issue   writes a blank bank card in the attached Card Reader/Writer (block or
        bay module): a fresh random 32-byte key, the PAN, an optional PIN
        (then the card demands verify_pin before signing), and the printed
        fields. The key goes into the subscriber file, nowhere else.
        --tap also stores the PAN and a tap token in NFC sector 1 of a hybrid
        card, keyed with the issuer's terminal key (printed once; give it to
        your point-of-sale program). A tap is a convenience, not a signature:
        anyone who sniffs that sector can replay it, which is the point.
serve   answers TCP requests, one JSON object per connection:
          {"pan", "nonce", "ctr", "tag", "amount"}          a chip payment
          {"pan", "token", "amount", "tap": true}           an NFC tap
        and replies {"ok": true, "balance": n} or {"ok": false, "reason": "..."}.

Subscriber file: $ECM_APP_DATA/accounts.json (/var/bankd/accounts.json when
run outside the app manager): {pan: {key_hex, balance, name, ctr, pin, tap_token?}}.
"""

import sys

import cards
import peripheral

DEFAULT_ISSUER = "acme"


def _json():
    try:
        import _ui
        return _ui.json_dumps, _ui.json_loads
    except Exception:
        import json
        return json.dumps, json.loads


json_dumps, json_loads = _json()


def data_dir():
    try:
        import os
        d = os.environ.get("ECM_APP_DATA")
        if d:
            return d
    except Exception:
        pass
    return "/var/bankd"


def accounts_path():
    return data_dir() + "/accounts.json"


def read_text(path):
    try:
        with open(path) as f:
            return f.read()
    except Exception:
        try:
            import shell
            return shell.read_file(path)
        except Exception:
            return None


def write_text(path, text):
    try:
        with open(path, "w") as f:
            f.write(text)
        return True
    except Exception:
        try:
            import shell
            return bool(shell.write_file(path, text))
        except Exception:
            return False


class Ledger:
    """The subscriber file: PAN -> {key_hex, balance, name, ctr, pin, tap_token?}, plus the
    issuer's terminal key for taps."""

    def __init__(self, path=None):
        self.path = path or accounts_path()
        self.data = {"accounts": {}, "terminal_key": None}
        text = read_text(self.path)
        if text:
            j = json_loads(text)
            if isinstance(j, dict) and "accounts" in j:
                self.data = j
            elif isinstance(j, dict):
                self.data = {"accounts": j, "terminal_key": None}

    @property
    def accounts(self):
        return self.data["accounts"]

    def save(self):
        if not write_text(self.path, json_dumps(self.data)):
            raise RuntimeError("cannot write " + self.path)

    def terminal_key(self):
        k = self.data.get("terminal_key")
        if not k:
            k = cards.random_hex(16)
            self.data["terminal_key"] = k
            self.save()
        return k

    def open_account(self, pan, name, key_hex, pin="", tap_token=None):
        self.accounts[pan] = {"key_hex": key_hex, "balance": 0, "name": name, "ctr": 0, "pin": bool(pin)}
        if tap_token:
            self.accounts[pan]["tap_token"] = tap_token
        self.save()

    def deposit(self, pan, amount):
        a = self.accounts.get(pan)
        if a is None:
            raise KeyError("no account " + pan)
        a["balance"] = int(a["balance"]) + int(amount)
        self.save()
        return a["balance"]

    def authorize(self, req):
        """Decide one request dict; returns the reply dict. Pure apart from the file write."""
        pan = str(req.get("pan", ""))
        a = self.accounts.get(pan)
        if a is None:
            return {"ok": False, "reason": "unknown card"}
        try:
            amount = int(req.get("amount", 0))
        except (TypeError, ValueError):
            return {"ok": False, "reason": "bad amount"}
        if amount < 0:
            return {"ok": False, "reason": "bad amount"}
        if req.get("tap"):
            token = str(req.get("token", "")).lower()
            if not a.get("tap_token") or token != a["tap_token"]:
                return {"ok": False, "reason": "bad tap"}
        else:
            try:
                ctr = int(req.get("ctr"))
                nonce = str(req.get("nonce", ""))
                tag = str(req.get("tag", ""))
            except (TypeError, ValueError):
                return {"ok": False, "reason": "malformed"}
            # Replay protection: the chip's counter only ever moves forward.
            if ctr <= int(a.get("ctr", 0)):
                return {"ok": False, "reason": "replay (ctr %d <= %d)" % (ctr, a.get("ctr", 0))}
            try:
                if not cards.verify_tag(a["key_hex"], nonce, ctr, amount, tag):
                    return {"ok": False, "reason": "bad tag"}
            except ValueError as e:
                return {"ok": False, "reason": "malformed: %s" % e}
            a["ctr"] = ctr
        if amount > int(a["balance"]):
            self.save()   # the counter moved even though the payment failed
            return {"ok": False, "reason": "insufficient funds", "balance": a["balance"]}
        a["balance"] = int(a["balance"]) - amount
        self.save()
        return {"ok": True, "balance": a["balance"]}


def _opt(args, name, default=None):
    if name in args:
        i = args.index(name)
        if i + 1 < len(args):
            v = args[i + 1]
            del args[i:i + 2]
            return v
        del args[i]
        return True
    return default


def issue(args, ledger=None):
    args = list(args)
    pin = _opt(args, "--pin", "")
    issuer = _opt(args, "--issuer", DEFAULT_ISSUER)
    tap = "--tap" in args
    if tap:
        args.remove("--tap")
    if len(args) < 2:
        print("usage: bankd issue <name> <pan> [--pin 1234] [--issuer acme] [--tap]")
        return 2
    name, pan = args[0], args[1]
    ledger = ledger or Ledger()
    reader = cards.Reader.find()
    if reader is None:
        print("bankd: no card_reader attached (a Card Reader/Writer block or a reader module)")
        return 1
    if not reader.present():
        print("insert a blank bank card into %s..." % reader.name)
        ev = reader.wait(60)
        if not ev or ev[0] != "card_inserted":
            print("bankd: no card")
            return 1
    card = reader.card()
    if not card.bank.blank():
        print("bankd: the card's bank chip is already written (reissue on a blank)")
        return 1
    key = cards.random_hex(32)
    card.bank.write(issuer, pan, key, pin=pin, pin_required=bool(pin))
    card.write_fields({"name": name, "number": pan, "issuer": issuer})
    if cards.face_size(card.kind()) and tap:
        token = cards.random_hex(16)
        tkey = ledger.terminal_key()
        body = pan.encode().hex() + "00" + token
        # The factory key is the blank's; move sector 1 to the terminal key so only a POS that
        # knows it can read the tap data (anyone with an SDR in the field still sees it).
        card.nfc.auth(1, cards.FACTORY_KEY)
        card.nfc.write(1, body)
        card.nfc.set_key(1, tkey)
        ledger.open_account(pan, name, key, pin, tap_token=token)
        print("tap enabled: sector 1 holds the PAN and a tap token under the terminal key %s" % tkey)
        print("  (give that key to your point-of-sale program: %s)" % tkey)
    else:
        ledger.open_account(pan, name, key, pin)
    reader.eject()
    print("issued %s to %s (issuer %s%s); the key stays in %s" % (pan, name, issuer, ", PIN" if pin else "", ledger.path))
    return 0


def serve(args):
    if not args:
        print("usage: bankd serve <port>")
        return 2
    import net
    port = int(args[0])
    ledger = Ledger()
    listener = net.tcp_listen(port)
    print("bankd: serving %d accounts on port %d (%s)" % (len(ledger.accounts), port, ledger.path))
    while True:
        try:
            conn = net.tcp_accept(listener)
        except RuntimeError:
            continue   # accept timed out: keep waiting
        try:
            data = b""
            while b"\n" not in data and len(data) < 4096:
                chunk = net.tcp_recv(conn, 1024)
                if not chunk:
                    break
                data += chunk
            text = data.decode().strip()
            try:
                req = json_loads(text) if text else {}
                if not isinstance(req, dict):
                    raise ValueError("not an object")
            except Exception:
                reply = {"ok": False, "reason": "bad request"}
            else:
                reply = ledger.authorize(req)
            print("%s %s %s -> %s" % (req.get("pan", "?") if isinstance(req, dict) else "?",
                                     "tap" if isinstance(req, dict) and req.get("tap") else "chip",
                                     req.get("amount", "?") if isinstance(req, dict) else "?",
                                     "approved" if reply.get("ok") else "declined: " + str(reply.get("reason"))))
            net.tcp_send(conn, (json_dumps(reply) + "\n").encode())
        except RuntimeError as e:
            print("bankd: %s" % e)
        finally:
            net.tcp_close(conn)


def deposit(args):
    if len(args) < 2:
        print("usage: bankd deposit <pan> <amount>")
        return 2
    ledger = Ledger()
    try:
        bal = ledger.deposit(args[0], int(args[1]))
    except KeyError as e:
        print("bankd: %s" % e)
        return 1
    print("%s: balance %d" % (args[0], bal))
    return 0


def accounts(args):
    ledger = Ledger()
    if not ledger.accounts:
        print("no accounts in %s" % ledger.path)
    for pan, a in sorted(ledger.accounts.items()):
        print("%-19s %-20s balance %6d  ctr %d%s%s" % (pan, a.get("name", ""), a.get("balance", 0), a.get("ctr", 0),
                                                     "  PIN" if a.get("pin") else "", "  tap" if a.get("tap_token") else ""))
    return 0


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    cmd, rest = argv[0], list(argv[1:])
    if cmd == "issue":
        return issue(rest)
    if cmd == "serve":
        return serve(rest)
    if cmd == "deposit":
        return deposit(rest)
    if cmd == "accounts":
        return accounts(rest)
    print(__doc__)
    return 2


if not globals().get("_TESTING"):
    main(list(sys.argv[1:]))
