"""doord: a toy NFC door (docs/cards/CARDS.md "NFC applet").

    python /apps/dev.ecm.doord/main.py --allow <uid> [<uid>...] [--key HEX | --key-file PATH]
                                       [--sector 0] [--channel N] [--uid-only] [--hold 2]

Polls the attached nfc_reader (a block next to the computer, or an NFC module
in a bay). When an allowed card is in the field it opens the door: it prints
"OPEN <uid>" and, when a redstone_link peripheral is attached, raises link
channel N (default 0) to 15 for `hold` seconds. Anything else prints
"DENY <uid> <why>".

Two modes, one of them deliberately bad:

  default   the card must also pass the 3-pass sector authentication with the
            door's key (--key, or --key-file; the factory key with a warning
            when neither is given, so rekey your cards with `cardtool nfc
            setkey`). The UID says *which* card; the auth proves the card
            holds the key, and a sniffed session can't be replayed because
            the reader's challenge is fresh each time.

  --uid-only  trusts the UID alone. The UID is public: every POLL reply
            shouts it to anyone in the field, and a card with the same UID
            -- or an `nfc_reader.raw()` program that answers POLL for it --
            walks in. This mode exists so the difference can be seen.
"""

import sys

import cards
import peripheral


def read_key(args):
    if "--key" in args:
        i = args.index("--key")
        k = args[i + 1]
        del args[i:i + 2]
        return k
    if "--key-file" in args:
        i = args.index("--key-file")
        path = args[i + 1]
        del args[i:i + 2]
        try:
            with open(path) as f:
                return f.read().strip()
        except Exception:
            import shell
            text = shell.read_file(path)
            if text is None:
                raise ValueError("cannot read key file " + path)
            return text.strip()
    return None


def _int_opt(args, name, default):
    if name in args:
        i = args.index(name)
        v = int(args[i + 1])
        del args[i:i + 2]
        return v
    return default


class Door:
    """The policy: decide(uid, nfc) -> (open, why)."""

    def __init__(self, allow, key=None, sector=0, uid_only=False):
        self.allow = set(u.lower() for u in allow)
        self.key = key
        self.sector = int(sector)
        self.uid_only = bool(uid_only)

    def decide(self, uid, nfc):
        uid = str(uid).lower()
        if uid not in self.allow:
            return False, "not on the list"
        if self.uid_only:
            return True, "uid only (unverified!)"
        try:
            nfc.session(uid, self.sector, self.key)
        except peripheral.PeripheralError as e:
            return False, "auth failed: %s" % e
        return True, "sector %d authenticated" % self.sector


class Latch:
    """What opening does: a redstone_link channel, or just the printout."""

    def __init__(self, channel=0):
        self.link = peripheral.find("redstone_link")
        self.channel = int(channel)

    def open(self, on):
        if self.link is None:
            return
        try:
            self.link.set_output(self.channel, 15 if on else 0)
        except peripheral.PeripheralError as e:
            print("doord: redstone link: %s" % e)


def run(door, nfc, latch, hold=2.0, rounds=None):
    """Poll forever (or `rounds` times); returns the log of (uid, opened, why)."""
    log = []
    seen_open = {}
    n = 0
    while rounds is None or n < rounds:
        n += 1
        for card in nfc.poll():
            uid = str(card.get("uid", "")).lower()
            if uid in seen_open:
                continue   # still in the field after opening: don't retrigger
            opened, why = door.decide(uid, nfc)
            log.append((uid, opened, why))
            if opened:
                print("OPEN %s (%s)" % (uid, why))
                latch.open(True)
                seen_open[uid] = hold
            else:
                print("DENY %s %s" % (uid, why))
        # Tick the latch.
        peripheral.pull_event(None, timeout=0.5)
        for uid in list(seen_open):
            seen_open[uid] -= 0.5
            if seen_open[uid] <= 0:
                del seen_open[uid]
                if not seen_open:
                    latch.open(False)
    return log


def main(argv):
    args = list(argv)
    uid_only = "--uid-only" in args
    if uid_only:
        args.remove("--uid-only")
    key = read_key(args)
    sector = _int_opt(args, "--sector", 0)
    channel = _int_opt(args, "--channel", 0)
    hold = _int_opt(args, "--hold", 2)
    if "--allow" not in args or len(args) < 2:
        print(__doc__)
        return 2
    allow = [a for a in args[args.index("--allow") + 1:] if not a.startswith("--")]
    if not uid_only and not key:
        key = cards.FACTORY_KEY
        print("doord: WARNING no --key: using the factory key every blank card has; rekey your cards")
    nfc = cards.Nfc.find()
    if nfc is None:
        print("doord: no nfc_reader attached")
        return 1
    nfc.field(True)
    door = Door(allow, key, sector, uid_only)
    latch = Latch(channel)
    print("doord: %d allowed uid(s), %s, reader %s%s" % (
        len(allow), "UID ONLY (anyone who heard the uid gets in)" if uid_only else "sector %d auth" % sector,
        nfc.name, ", redstone link ch %d" % channel if latch.link else ", no redstone link (print only)"))
    run(door, nfc, latch, hold)
    return 0


if not globals().get("_TESTING"):
    main(list(sys.argv[1:]))
