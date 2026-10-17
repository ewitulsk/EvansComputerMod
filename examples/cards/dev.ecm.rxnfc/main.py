"""rx_nfc: listen to the near-field band (docs/cards/CARDS.md "Near-field radio").

    python /apps/dev.ecm.rxnfc/main.py [--sdr NAME] [--seconds S] [--threshold DBM]
    python /apps/dev.ecm.rxnfc/main.py --probe [--uid UID] [--sector N --key HEX]

The lesson: NFC is radio. A reader's field and a card's backscattered
replies are emissions on 13.56 MHz like any other, and an SDR in range hears
them: the mod's SDR path renders packet-radio emissions as bursts at the
right power and airtime (it does not demodulate NF-106 frames into bytes --
`IqSynthesizer`: "correct on a waterfall, not decodable"), so the sniffer
mode shows *when* a reader talks and *how loud* each exchange is: a POLL
every few hundred ms while a field is on, then the longer AUTH / READ
bursts of a session. From 2 blocks away that is enough to know a door was
opened and when. The probe mode shows the bytes themselves, from the
reader's side, with the nfc_reader's `raw` method: it sends POLL and SELECT
(and AUTH + READ when you give a sector key) as raw NF-106 frames and
decodes the replies with `cards.frame_decode`, so you see what a session
puts on the air: UIDs in the clear, the challenge/response pair, and after a
successful auth the sector's 64 data bytes. Nothing on the air carries a
key, but a replayed POLL reply or a copied sector is all a UID-only door or a
static tap token needs (`doord --uid-only`, `pos --nfc-key`).

Sniffer mode needs an SDR block (any tier tunes 13.56 MHz) and the `radio`
permission; probe mode needs an nfc_reader (block or module).
"""

import sys

import cards

NEAR_FIELD_HZ = 13.56e6


def _opt(args, name, default=None):
    if name in args:
        i = args.index(name)
        v = args[i + 1]
        del args[i:i + 2]
        return v
    return default


class BurstDetector:
    """Turns a stream of sample-block powers into bursts: (start_s, length_s, peak_dbm)."""

    def __init__(self, threshold_dbm=-80.0, block_s=0.01):
        self.threshold = float(threshold_dbm)
        self.block_s = float(block_s)
        self.t = 0.0
        self.start = None
        self.peak = None
        self.bursts = []

    def feed(self, power_dbm):
        t = self.t
        self.t += self.block_s
        if power_dbm >= self.threshold:
            if self.start is None:
                self.start, self.peak = t, power_dbm
            elif power_dbm > self.peak:
                self.peak = power_dbm
            return None
        if self.start is not None:
            b = (self.start, t - self.start, self.peak)
            self.bursts.append(b)
            self.start = None
            return b
        return None

    @staticmethod
    def describe(burst):
        start, length, peak = burst
        ms = length * 1000
        kind = "POLL-sized" if ms < 40 else ("session-sized" if ms < 200 else "long")
        return "t=%7.2fs burst %6.1f ms peak %6.1f dB  (%s)" % (start, ms, peak, kind)


def sniff(sdr_name="", seconds=None, threshold=-80.0):
    import radio
    sdr = radio.open(sdr_name)
    sdr.tune(NEAR_FIELD_HZ, rate=48000)
    try:
        sdr.gain(30)
    except Exception:
        pass
    block = 480   # 10 ms at 48 kS/s
    det = BurstDetector(threshold, block / 48000.0)
    print("rx_nfc: listening on 13.56 MHz with %s (threshold %.0f dB)%s" % (
        sdr.data_path, threshold, "" if seconds is None else " for %s s" % seconds))
    try:
        while seconds is None or det.t < seconds:
            s = sdr.read(block)
            if len(s) == 0:
                continue
            b = det.feed(s.power_db())
            if b is not None:
                print(det.describe(b))
    finally:
        sdr.close()
    print("%d bursts" % len(det.bursts))
    return det.bursts


def show(direction, frame_hex):
    try:
        cmd, body = cards.frame_decode(frame_hex)
    except ValueError as e:
        print("  %s ?? %s (%s)" % (direction, frame_hex, e))
        return None, ""
    name = cards.frame_name(cmd)
    if cmd == cards.CMD_ERROR:
        name = "ERROR %s" % cards.error_reason(int(body[:2] or "0", 16))
    print("  %s %-6s %s" % (direction, name, body))
    return cmd, body


def probe(uid=None, sector=None, key=None):
    """The reader's side of a session, frame by frame, through nfc_reader.raw()."""
    nfc = cards.Nfc.find()
    if nfc is None:
        print("rx_nfc: no nfc_reader attached (probe mode needs one)")
        return 1
    nfc.field(True)
    print("on the air from %s:" % nfc.name)
    poll = cards.frame_encode(cards.CMD_POLL)
    print("  >> POLL   (%s)" % poll)
    reply = nfc.raw(poll)
    if not reply:
        print("  << (silence: no energised card in the field)")
        return 1
    cmd, body = show("<<", reply)
    heard = body[:16] if cmd == (cards.CMD_POLL | cards.REPLY_BIT) else None
    uid = (uid or heard or "").lower()
    if not uid:
        return 1
    sel = cards.frame_encode(cards.CMD_SELECT, uid)
    print("  >> SELECT %s" % uid)
    show("<<", nfc.raw(sel))
    if sector is not None and key:
        # The 3-pass auth, spelled out: R1 from us, R2 + M1 from the card, M2 from us.
        r1 = cards.random_hex(8)
        print("  >> AUTH   sector %d R1=%s" % (sector, r1))
        cmd, body = show("<<", nfc.raw(cards.frame_encode(cards.CMD_AUTH, "%02x" % sector + r1)))
        if cmd != (cards.CMD_AUTH | cards.REPLY_BIT):
            return 1
        r2, m1 = body[:16], body[16:48]
        if m1 != cards.auth_m1(key, r1, r2):
            print("  !! M1 does not match our key: the sector has another key")
            return 1
        m2 = cards.auth_m2(key, r1, r2)
        print("  >> AUTH2  M2=%s" % m2)
        cmd, _ = show("<<", nfc.raw(cards.frame_encode(cards.CMD_AUTH2, m2)))
        if cmd != (cards.CMD_AUTH2 | cards.REPLY_BIT):
            return 1
        print("  >> READ   sector %d" % sector)
        show("<<", nfc.raw(cards.frame_encode(cards.CMD_READ, "%02x" % sector)))
        print("(those 64 bytes crossed the air in the clear; so did the UID)")
    return 0


def main(argv):
    args = list(argv)
    if "--probe" in args:
        args.remove("--probe")
        uid = _opt(args, "--uid")
        sector = _opt(args, "--sector")
        key = _opt(args, "--key")
        return probe(uid, None if sector is None else int(sector), key)
    sdr = _opt(args, "--sdr", "")
    seconds = _opt(args, "--seconds")
    threshold = float(_opt(args, "--threshold", -80))
    if args and args[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    sniff(sdr, None if seconds is None else float(seconds), threshold)
    return 0


if not globals().get("_TESTING"):
    main(list(sys.argv[1:]))
