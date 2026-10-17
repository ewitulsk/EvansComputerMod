"""Cards: bank chips, magnetic stripes, NFC sectors, SIMs, the readers that
talk to them and the printer that paints them (docs/cards/CARDS.md).

    import cards

    reader = cards.Reader.find()            # the first Card Reader/Writer (block or bay module)
    card = reader.card()                    # the card in its slot, or None
    print(card.identity())                  # {'kind': 'bank', 'uid': '…', 'applets': [...], ...}

    # magnetic stripe: plain text, anyone can read or clone it
    t1, t2, t3 = card.stripe.read()
    card.stripe.write("ANN EXAMPLE", "12345678", "")

    # bank chip: keyed, counted; the key never leaves the card
    key = cards.random_hex(32)
    card.bank.write("acme", "4000123412341234", key, pin="1234", pin_required=True)
    card.bank.verify_pin("1234")            # opens a PIN session (until the card leaves)
    ctr, tag = card.bank.sign(nonce_hex, amount=1500)
    cards.verify_tag(key, nonce_hex, ctr, amount, tag)   # what the issuer checks

    # NFC sectors by contact (the same applet the NFC reader reaches over the air)
    card.nfc.auth(1, cards.FACTORY_KEY)
    card.nfc.write(1, "48656c6c6f")         # padded to 64 bytes
    card.nfc.set_key(1, new_key_hex)
    reader.eject()

    nfc = cards.Nfc.find()                  # an NFC Reader (block or module)
    nfc.field(True)
    for seen in nfc.poll():                 # [{'uid': '…', 'kinds': [...]}, ...]
        s = nfc.session(seen["uid"], 1, cards.FACTORY_KEY)
        print(s.read())                     # 64 bytes, hex
        s.write(data_hex); s.set_key(key_hex)

    printer = cards.Printer.find()
    d = cards.Design.new("bank")            # or cards.Design.load("acme.card")
    d.palette = [[255, 255, 255], [0, 0, 0], [200, 30, 30]]
    d.fill("front", 2)
    d.text("front", 1, 1, "ACME BANK", 1)   # 3x5 font: digits, A-Z, space, '-', '.'
    d.set_pixel("back", 5, 7, 1)
    d.fields["name"] = "Ann Example"
    d.stripe = ("ANN EXAMPLE", "12345678", "")
    print(printer.cost(d))                  # the dyes it takes
    printer.print(d)
    d.save("acme.card")

Faces are palette-indexed bitmaps: 32x20 texels for bank / nfc / hybrid
cards, 8x13 (portrait) for a SIM, at most 16 palette colours, index 0 = the first. The
`.card` file is JSON ({kind, palette, front, back, fields, stripe}; faces are
base64 of 4-bit indices, row-major, two texels per byte, high nibble first),
so a bank can stamp a customer's name on its template and print.

Bytes are hex strings everywhere. The pure functions mix(key_hex, x_hex),
verify_tag(key_hex, nonce_hex, ctr, amount, tag_hex), tag(...) and
pin_hash(uid_hex, pin) are the reference implementations of the toy schemes in
docs/cards/NFC_MIX.md, for programs that play the issuer or the reader side.
Peripheral errors raise peripheral.PeripheralError.
"""

import peripheral

# ---------------------------------------------------------------- bytes and hashes

_M32 = 0xFFFFFFFF


def _bytes(hex_text, n=None, what="value"):
    """Hex text -> bytes, checking the length when `n` is given."""
    if isinstance(hex_text, (bytes, bytearray)):
        b = bytes(hex_text)
    else:
        try:
            b = bytes.fromhex(hex_text)
        except (ValueError, TypeError):
            raise ValueError("%s is not hex: %r" % (what, hex_text))
    if n is not None and len(b) != n:
        raise ValueError("%s is %d bytes (%d hex digits), got %d" % (what, n, 2 * n, len(b)))
    return b


def _hex(b):
    return bytes(b).hex()


def random_hex(n):
    """`n` random bytes as hex (from the computer's random source)."""
    try:
        import os
        return _hex(os.urandom(n))
    except Exception:
        pass
    try:
        import random
        return _hex(bytes(random.getrandbits(8) for _ in range(n)))
    except Exception:
        raise RuntimeError("no random source on this computer")


def _rotl(v, n):
    return ((v << n) | (v >> (32 - n))) & _M32


def _rotr(v, n):
    return ((v >> n) | (v << (32 - n))) & _M32


_SHA_K = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1,
    0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
    0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]
_SHA_H = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]


def _sha256(data):
    """SHA-256 of `data` (bytes) -> 32 bytes. Pure Python: the in-game
    interpreter has no hashlib."""
    data = bytes(data)
    msg = data + b"\x80" + b"\x00" * ((55 - len(data)) % 64) + (len(data) * 8).to_bytes(8, "big")
    h = list(_SHA_H)
    for start in range(0, len(msg), 64):
        w = [int.from_bytes(msg[i:i + 4], "big") for i in range(start, start + 64, 4)]
        for i in range(16, 64):
            s0 = _rotr(w[i - 15], 7) ^ _rotr(w[i - 15], 18) ^ (w[i - 15] >> 3)
            s1 = _rotr(w[i - 2], 17) ^ _rotr(w[i - 2], 19) ^ (w[i - 2] >> 10)
            w.append((w[i - 16] + s0 + w[i - 7] + s1) & _M32)
        a, b, c, d, e, f, g, hh = h
        for i in range(64):
            t1 = (hh + (_rotr(e, 6) ^ _rotr(e, 11) ^ _rotr(e, 25)) + ((e & f) ^ (~e & g)) + _SHA_K[i] + w[i]) & _M32
            t2 = ((_rotr(a, 2) ^ _rotr(a, 13) ^ _rotr(a, 22)) + ((a & b) ^ (a & c) ^ (b & c))) & _M32
            hh, g, f, e, d, c, b, a = g, f, e, (d + t1) & _M32, c, b, a, (t1 + t2) & _M32
        h = [(x + y) & _M32 for x, y in zip(h, [a, b, c, d, e, f, g, hh])]
    return b"".join(x.to_bytes(4, "big") for x in h)


def _hmac_sha256(key, msg):
    key = bytes(key)
    if len(key) > 64:
        key = _sha256(key)
    key = key + b"\x00" * (64 - len(key))
    ipad = bytes(k ^ 0x36 for k in key)
    opad = bytes(k ^ 0x5C for k in key)
    return _sha256(opad + _sha256(ipad + bytes(msg)))


def sha256(hex_text):
    """SHA-256 of hex bytes, as hex (for programs that need a hash)."""
    return _hex(_sha256(_bytes(hex_text)))


def hmac_sha256(key_hex, msg_hex):
    """HMAC-SHA256 of hex bytes under a hex key, as hex."""
    return _hex(_hmac_sha256(_bytes(key_hex), _bytes(msg_hex)))


# ---------------------------------------------------------------- the schemes (NFC_MIX.md)

FACTORY_KEY = "45434d2d4e46432d464143544f525930"   # ASCII "ECM-NFC-FACTORY0": every sector of a blank
SECTORS = 8
SECTOR_BYTES = 64


def _mix(key, x):
    k = [int.from_bytes(key[4 * i:4 * i + 4], "little") for i in range(4)]
    v0, v1, v2, v3 = [int.from_bytes(x[4 * i:4 * i + 4], "little") for i in range(4)]
    for r in range(8):
        v0 = (v0 + k[r & 3]) & _M32
        v1 ^= _rotl(v0, 7)
        v2 = (v2 + v1) & _M32
        v3 ^= _rotl(v2, 13)
        v0 = (v0 + v3) & _M32
        v1 = (_rotl(v1, 17) + k[(r + 1) & 3]) & _M32
        v2 ^= _rotl(v0, 5)
        v3 = ((v3 ^ r) + k[(r + 2) & 3]) & _M32
    return b"".join(v.to_bytes(4, "little") for v in (v0, v1, v2, v3))


def mix(key_hex, x_hex):
    """Sector Lattice Mix(key[16], x[16]) -> 16 bytes, all hex."""
    return _hex(_mix(_bytes(key_hex, 16, "key"), _bytes(x_hex, 16, "x")))


def auth_m1(key_hex, r1_hex, r2_hex):
    """The card's answer M1 = Mix(S_i, R1 || R2)."""
    return mix(key_hex, _hex(_bytes(r1_hex, 8, "R1") + _bytes(r2_hex, 8, "R2")))


def auth_m2(key_hex, r1_hex, r2_hex):
    """The reader's answer M2 = Mix(S_i, R2 || R1)."""
    return mix(key_hex, _hex(_bytes(r2_hex, 8, "R2") + _bytes(r1_hex, 8, "R1")))


def _u64(v, what):
    """A counter or amount: a non-negative int, or 16 hex digits (as Lua passes a full u64)."""
    if isinstance(v, str):
        return _bytes(v, 8, what)
    return int(v).to_bytes(8, "big")


def tag(key_hex, nonce_hex, ctr, amount=0):
    """Bank TAG = HMAC-SHA256(K, "ECM-BANK-TAG" || NONCE || CTR || AMOUNT)[0..16), hex."""
    msg = b"ECM-BANK-TAG" + _bytes(nonce_hex, 8, "nonce") + _u64(ctr, "ctr") + _u64(amount, "amount")
    return _hex(_hmac_sha256(_bytes(key_hex, 32, "key"), msg)[:16])


def verify_tag(key_hex, nonce_hex, ctr, amount, tag_hex):
    """Whether a card's tag for (nonce, ctr, amount) is right under the issuer's copy of K."""
    want = tag(key_hex, nonce_hex, ctr, amount)
    got = str(tag_hex).lower()
    if len(got) != len(want):
        return False
    d = 0
    for a, b in zip(want, got):
        d |= ord(a) ^ ord(b)
    return d == 0


def pin_hash(uid_hex, pin):
    """SHA-256("ECM-BANK-PIN" || UID[8] || PIN ASCII), hex."""
    return _hex(_sha256(b"ECM-BANK-PIN" + _bytes(uid_hex, 8, "uid") + str(pin).encode()))


# ---------------------------------------------------------------- NFC air frames

CMD_POLL, CMD_SELECT, CMD_AUTH, CMD_AUTH2, CMD_READ, CMD_WRITE, CMD_SETKEY, CMD_ERROR = 1, 2, 3, 4, 5, 6, 7, 0xFF
REPLY_BIT = 0x80
ERR_BAD_AUTH, ERR_NO_SESSION, ERR_BAD_SECTOR, ERR_NOT_SELECTED, ERR_MALFORMED = 1, 2, 3, 4, 5
_CMD_NAMES = {1: "POLL", 2: "SELECT", 3: "AUTH", 4: "AUTH2", 5: "READ", 6: "WRITE", 7: "SETKEY", 0xFF: "ERROR"}
_REASONS = {1: "bad auth", 2: "no session", 3: "bad sector", 4: "not selected", 5: "malformed"}


def frame_encode(cmd, body_hex=""):
    """An NF-106 air frame cmd(1) len(1) body, as hex."""
    body = _bytes(body_hex, None, "body")
    if len(body) > 255:
        raise ValueError("a frame body is at most 255 bytes")
    return _hex(bytes([cmd & 0xFF, len(body)]) + body)


def frame_decode(frame_hex):
    """(cmd, body_hex) of an air frame; ValueError when malformed."""
    b = _bytes(frame_hex, None, "frame")
    if len(b) < 2:
        raise ValueError("frame shorter than 2 bytes")
    if len(b) != 2 + b[1]:
        raise ValueError("frame length does not match its len byte")
    return b[0], _hex(b[2:])


def frame_name(cmd):
    """POLL, SELECT, ... for a request or reply cmd byte."""
    if cmd == CMD_ERROR:
        return "ERROR"
    return _CMD_NAMES.get(cmd & ~REPLY_BIT, "?")


def error_reason(reason):
    return _REASONS.get(reason, "?")


# ---------------------------------------------------------------- designs

KINDS = ("bank", "nfc", "hybrid", "sim")

# The 3x5 font: five rows per glyph, each a 3-bit value with the left texel in
# bit 2 (shared verbatim with ecm-cards and cards.lua).
FONT = {
    "0": (7, 5, 5, 5, 7), "1": (2, 6, 2, 2, 7), "2": (7, 1, 7, 4, 7), "3": (7, 1, 7, 1, 7),
    "4": (5, 5, 7, 1, 1), "5": (7, 4, 7, 1, 7), "6": (7, 4, 7, 5, 7), "7": (7, 1, 1, 1, 1),
    "8": (7, 5, 7, 5, 7), "9": (7, 5, 7, 1, 7),
    "A": (2, 5, 7, 5, 5), "B": (6, 5, 6, 5, 6), "C": (7, 4, 4, 4, 7), "D": (6, 5, 5, 5, 6),
    "E": (7, 4, 7, 4, 7), "F": (7, 4, 7, 4, 4), "G": (7, 4, 5, 5, 7), "H": (5, 5, 7, 5, 5),
    "I": (7, 2, 2, 2, 7), "J": (1, 1, 1, 5, 7), "K": (5, 5, 6, 5, 5), "L": (4, 4, 4, 4, 7),
    "M": (5, 7, 7, 5, 5), "N": (6, 5, 5, 5, 5), "O": (7, 5, 5, 5, 7), "P": (7, 5, 7, 4, 4),
    "Q": (7, 5, 5, 7, 1), "R": (7, 5, 6, 5, 5), "S": (7, 4, 7, 1, 7), "T": (7, 2, 2, 2, 2),
    "U": (5, 5, 5, 5, 7), "V": (5, 5, 5, 5, 2), "W": (5, 5, 7, 7, 5), "X": (5, 5, 2, 5, 5),
    "Y": (5, 5, 2, 2, 2), "Z": (7, 1, 2, 4, 7),
    " ": (0, 0, 0, 0, 0), "-": (0, 0, 7, 0, 0), ".": (0, 0, 0, 0, 2),
}
GLYPH_ADVANCE = 4


def face_size(kind):
    """(width, height) in texels of a card kind's faces."""
    if kind in ("bank", "nfc", "hybrid"):
        return (32, 20)
    if kind == "sim":
        return (8, 13)
    raise ValueError("unknown card kind '%s' (bank, nfc, hybrid, sim)" % kind)


_B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"


def _b64encode(data):
    data = bytes(data)
    out = []
    for i in range(0, len(data), 3):
        chunk = data[i:i + 3]
        n = len(chunk)
        v = (chunk[0] << 16) | ((chunk[1] if n > 1 else 0) << 8) | (chunk[2] if n > 2 else 0)
        out.append(_B64[(v >> 18) & 63] + _B64[(v >> 12) & 63] + (_B64[(v >> 6) & 63] if n > 1 else "=") + (_B64[v & 63] if n > 2 else "="))
    return "".join(out)


def _b64decode(text):
    out = bytearray()
    acc = bits = 0
    for c in str(text):
        if c in "=\n\r ":
            continue
        i = _B64.find(c)
        if i < 0:
            i = {"-": 62, "_": 63}.get(c, -1)
            if i < 0:
                raise ValueError("not base64")
        acc = (acc << 6) | i
        bits += 6
        if bits >= 8:
            bits -= 8
            out.append((acc >> bits) & 0xFF)
            acc &= (1 << bits) - 1
    return bytes(out)


def _read_text(path):
    try:
        with open(path) as f:
            return f.read()
    except Exception:
        try:
            import shell
            text = shell.read_file(path)
        except Exception:
            text = None
        if text is None:
            raise
        return text


def _write_text(path, text):
    try:
        with open(path, "w") as f:
            f.write(text)
    except Exception:
        try:
            import shell
            ok = shell.write_file(path, text)
        except Exception:
            ok = False
        if not ok:
            raise


class Design:
    """A card design: `kind`, `palette` ([[r, g, b], ...], at most 16),
    `front` / `back` (bytearrays of packed 4-bit indices), `fields` (dict of
    strings, at most 8) and `stripe` ((t1, t2, t3) or None)."""

    def __init__(self, kind, palette=None, front=None, back=None, fields=None, stripe=None):
        self.kind = kind
        w, h = face_size(kind)
        n = (w * h + 1) // 2
        self.width, self.height = w, h
        self.palette = [list(c) for c in (palette or [[255, 255, 255]])]
        self.front = bytearray(front) if front is not None else bytearray(n)
        self.back = bytearray(back) if back is not None else bytearray(n)
        self.front += bytes(n - len(self.front))
        self.back += bytes(n - len(self.back))
        self.fields = dict(fields or {})
        self.stripe = tuple(stripe) if stripe else None

    @staticmethod
    def new(kind):
        """A blank design of `kind` (every texel index 0, a one-entry white palette)."""
        return Design(kind)

    def _face(self, face):
        if face in ("front", 0):
            return self.front
        if face in ("back", 1):
            return self.back
        raise ValueError("face is 'front' or 'back'")

    def pixel(self, face, x, y):
        """The palette index at (x, y) (0 outside the face)."""
        if not (0 <= x < self.width and 0 <= y < self.height):
            return 0
        i = y * self.width + x
        b = self._face(face)[i // 2]
        return b >> 4 if i % 2 == 0 else b & 15

    def set_pixel(self, face, x, y, idx):
        """Set the palette index at (x, y); outside the face nothing happens."""
        if not (0 <= x < self.width and 0 <= y < self.height):
            return
        i = y * self.width + x
        data = self._face(face)
        idx = int(idx) & 15
        if i % 2 == 0:
            data[i // 2] = (data[i // 2] & 0x0F) | (idx << 4)
        else:
            data[i // 2] = (data[i // 2] & 0xF0) | idx

    def fill(self, face, idx):
        data = self._face(face)
        v = (int(idx) & 15) * 0x11
        for i in range(len(data)):
            data[i] = v

    def text(self, face, x, y, s, idx=1):
        """Draw `s` with the 3x5 font, top-left at (x, y), in palette index
        `idx` (only set pixels are painted). Returns the x after the text."""
        cx = x
        for c in str(s):
            rows = FONT.get(c.upper())
            if rows:
                for dy in range(5):
                    for dx in range(3):
                        if rows[dy] & (4 >> dx):
                            self.set_pixel(face, cx + dx, y + dy, idx)
            cx += GLYPH_ADVANCE
        return cx

    def color(self, r, g, b):
        """The palette index of a colour, adding it when there is room; else the nearest."""
        rgb = [int(r), int(g), int(b)]
        for i, c in enumerate(self.palette):
            if list(c) == rgb:
                return i
        if len(self.palette) < 16:
            self.palette.append(rgb)
            return len(self.palette) - 1
        return min(range(len(self.palette)), key=lambda i: sum((a - b) ** 2 for a, b in zip(self.palette[i], rgb)))

    def to_map(self):
        """The design as the card_printer peripheral takes it:
        {palette, front: hex, back: hex, fields, stripe?}."""
        m = {
            "palette": [[int(v) for v in c] for c in self.palette],
            "front": _hex(self.front),
            "back": _hex(self.back),
            "fields": dict(self.fields),
        }
        if self.stripe:
            t = list(self.stripe) + ["", "", ""]
            m["stripe"] = {"t1": str(t[0]), "t2": str(t[1]), "t3": str(t[2])}
        return m

    def to_json(self):
        """The `.card` file text."""
        import _ui
        m = {
            "kind": self.kind,
            "palette": [[int(v) for v in c] for c in self.palette],
            "front": _b64encode(self.front),
            "back": _b64encode(self.back),
            "fields": dict(self.fields),
        }
        if self.stripe:
            t = list(self.stripe) + ["", "", ""]
            m["stripe"] = {"t1": str(t[0]), "t2": str(t[1]), "t3": str(t[2])}
        return _ui.json_dumps(m)

    @staticmethod
    def from_json(text):
        import _ui
        j = _ui.json_loads(text)
        if not isinstance(j, dict) or "kind" not in j:
            raise ValueError("a .card file is a JSON object with a kind")
        kind = j["kind"]
        w, h = face_size(kind)
        n = (w * h + 1) // 2
        palette = []
        for c in j.get("palette") or []:
            if len(c) != 3 or any(not (0 <= int(v) <= 255) for v in c):
                raise ValueError("a palette entry is [r, g, b]")
            palette.append([int(v) for v in c])

        def face(key):
            v = j.get(key)
            if v is None:
                return bytes(n)
            b = _b64decode(v)
            if len(b) > n:
                raise ValueError("\"%s\" holds more than %dx%d texels" % (key, w, h))
            return b + bytes(n - len(b))

        stripe = None
        s = j.get("stripe")
        if s:
            stripe = (str(s.get("t1", "")), str(s.get("t2", "")), str(s.get("t3", "")))
        return Design(kind, palette or None, face("front"), face("back"), j.get("fields") or {}, stripe)

    @staticmethod
    def load(path):
        """Read a `.card` file."""
        return Design.from_json(_read_text(path))

    def save(self, path):
        _write_text(path, self.to_json())

    def __repr__(self):
        return "<card design %s %dx%d, %d colours, %d fields%s>" % (
            self.kind, self.width, self.height, len(self.palette), len(self.fields), ", stripe" if self.stripe else "")


# ---------------------------------------------------------------- readers

def _design_map(design):
    return design.to_map() if isinstance(design, Design) else dict(design)


class Stripe:
    """The magnetic stripe of the card in a reader: plain text, no secret."""

    def __init__(self, p):
        self._p = p

    def read(self):
        """(t1, t2, t3), or None without a stripe. `corrupted()` tells if a field scrambled it."""
        r = self._p.call("read_stripe")
        if r is None:
            return None
        return (r.get("t1", ""), r.get("t2", ""), r.get("t3", ""))

    def corrupted(self):
        r = self._p.call("read_stripe")
        return bool(r and r.get("corrupted"))

    def write(self, t1, t2="", t3=""):
        """Write the tracks (64 / 32 / 96 printable characters) and clear `corrupted`."""
        return self._p.call("write_stripe", str(t1), str(t2), str(t3))


class Bank:
    """The bank chip of the card in a reader (contact only)."""

    def __init__(self, p):
        self._p = p

    def write(self, issuer, pan, key_hex, pin="", pin_required=False):
        """Write a blank chip once: issuer [a-z0-9_-]{1,24}, PAN 8-19 digits,
        32-byte key (hex, keep it: the card never gives it back), PIN 4-8 digits."""
        _bytes(key_hex, 32, "key")
        return self._p.call("write_bank", str(issuer), str(pan), str(key_hex).lower(), str(pin), bool(pin_required))

    def verify_pin(self, pin):
        """Open a PIN session (until the card leaves); False on a wrong PIN
        (three failures lock the chip)."""
        r = self._p.call("verify_pin", str(pin))
        return bool(r.get("ok")) if isinstance(r, dict) else bool(r)

    def sign(self, nonce_hex, amount=0):
        """(ctr, tag_hex) for an 8-byte nonce; the chip increments ctr first."""
        _bytes(nonce_hex, 8, "nonce")
        r = self._p.call("sign", str(nonce_hex).lower(), int(amount))
        return (int(r["ctr"]), str(r["tag"]))

    def blank(self):
        return bool(self._p.call("bank_blank"))


class NfcApplet:
    """The NFC sectors of the card in a reader, by contact."""

    def __init__(self, p):
        self._p = p

    def auth(self, sector, key_hex):
        _bytes(key_hex, 16, "sector key")
        return self._p.call("nfc_auth", int(sector), str(key_hex).lower())

    def read(self, sector):
        """64 bytes of an authenticated sector, hex."""
        return self._p.call("nfc_read", int(sector))

    def write(self, sector, data_hex):
        """Write up to 64 bytes (hex, zero-padded)."""
        return self._p.call("nfc_write", int(sector), _pad_sector(data_hex))

    def set_key(self, sector, key_hex):
        _bytes(key_hex, 16, "sector key")
        return self._p.call("nfc_set_key", int(sector), str(key_hex).lower())


class Sim:
    """The SIM applet of the card in a reader (what `cardtool sim` uses)."""

    def __init__(self, p):
        self._p = p

    def write(self, imsi, carrier, key_hex):
        """Write a blank SIM once (16-byte key, hex); returns the reader's result."""
        _bytes(key_hex, 16, "key")
        return self._p.call("write", str(imsi), str(carrier), str(key_hex).lower())

    def respond(self, challenge_hex):
        """{res, ks} for a 16-byte challenge."""
        _bytes(challenge_hex, 16, "challenge")
        return self._p.call("respond", str(challenge_hex).lower())

    def is_blank(self):
        return bool(self._p.call("is_blank"))


def _pad_sector(data_hex):
    b = _bytes(data_hex, None, "sector data")
    if len(b) > SECTOR_BYTES:
        raise ValueError("a sector holds %d bytes" % SECTOR_BYTES)
    return _hex(b + bytes(SECTOR_BYTES - len(b)))


class Card:
    """The card in a reader's slot: `stripe`, `bank`, `nfc` and `sim` are its applets."""

    def __init__(self, reader):
        self.reader = reader
        p = reader.peripheral
        self.stripe = Stripe(p)
        self.bank = Bank(p)
        self.nfc = NfcApplet(p)
        self.sim = Sim(p)

    def identity(self):
        """{kind, uid?, applets, print, bank?: {issuer, pan, locked, ctr, blank},
        nfc?: {sectors}, sim?: {imsi, carrier, blank}}"""
        return self.reader.peripheral.call("identity")

    def kind(self):
        id = self.identity()
        return id.get("kind") or id.get("type")

    def write_fields(self, fields):
        """Set the public printed fields (name, number, issuer, label, ...)."""
        return self.reader.peripheral.call("write_fields", dict(fields))

    def __repr__(self):
        return "<card in %s>" % self.reader.peripheral.name


class Reader:
    """A Card Reader/Writer (block) or Card Reader Module (bay): peripheral type card_reader."""

    TYPE = "card_reader"

    def __init__(self, p):
        self.peripheral = p
        self.name = p.name

    @staticmethod
    def find(name=None):
        """The reader attached as `name`, or the first one; None without."""
        return _find(Reader, name)

    @staticmethod
    def find_all():
        return [Reader(p) for p in peripheral.find_all(Reader.TYPE)]

    def present(self):
        return bool(self.peripheral.call("present"))

    def status(self):
        """{present, released, auto_eject, state, applets}"""
        return self.peripheral.call("status")

    def card(self):
        """The Card in the slot, or None."""
        return Card(self) if self.present() else None

    def eject(self):
        """Release the card (Auto Eject pushes it into an adjacent inventory).
        True when it was ejected, False when it waits to be taken."""
        return bool(self.peripheral.call("eject"))

    def wait(self, timeout=30):
        """The next card_inserted / card_removed event of this reader as
        (event, kind, applets) / (event,), or None after `timeout` seconds."""
        waited = 0.0
        while timeout is None or waited < timeout:
            ev = peripheral.pull_event(None, timeout=0.5)
            waited += 0.5
            if ev and len(ev) > 1 and ev[1] == self.name and str(ev[0]).startswith("card_"):
                return tuple(ev[0:1]) + tuple(ev[2:])
        return None

    def __repr__(self):
        return "<card reader '%s'>" % self.name


class Session:
    """An authenticated sector on a card in an NFC reader's field."""

    def __init__(self, nfc, uid, sector):
        self.nfc = nfc
        self.uid = uid
        self.sector = sector

    def read(self):
        """64 bytes, hex."""
        return self.nfc.read(self.sector)

    def write(self, data_hex):
        return self.nfc.write(self.sector, data_hex)

    def set_key(self, key_hex):
        return self.nfc.set_key(self.sector, key_hex)


class Nfc:
    """An NFC Reader (block) or NFC module (bay): peripheral type nfc_reader."""

    TYPE = "nfc_reader"

    def __init__(self, p):
        self.peripheral = p
        self.name = p.name

    @staticmethod
    def find(name=None):
        return _find(Nfc, name)

    @staticmethod
    def find_all():
        return [Nfc(p) for p in peripheral.find_all(Nfc.TYPE)]

    def field(self, on=None):
        """Energise the field (`field(True)`), or ask whether it is on."""
        if on is None:
            return bool(self.peripheral.call("field"))
        return self.peripheral.call("set_field", bool(on))

    def power(self, dbm):
        return self.peripheral.call("set_power", int(dbm))

    def poll(self):
        """[{uid, kinds}, ...] heard in the field (the field is switched on if needed)."""
        if not self.field():
            self.field(True)
        return self.peripheral.call("poll") or []

    def select(self, uid):
        return self.peripheral.call("select", str(uid).lower())

    def auth(self, sector, key_hex):
        _bytes(key_hex, 16, "sector key")
        return self.peripheral.call("auth", int(sector), str(key_hex).lower())

    def read(self, sector):
        return self.peripheral.call("read", int(sector))

    def write(self, sector, data_hex):
        return self.peripheral.call("write", int(sector), _pad_sector(data_hex))

    def set_key(self, sector, key_hex):
        _bytes(key_hex, 16, "sector key")
        return self.peripheral.call("set_key", int(sector), str(key_hex).lower())

    def session(self, uid, sector, key_hex):
        """select(uid) + auth(sector, key): a Session with read / write / set_key."""
        self.select(uid)
        r = self.auth(sector, key_hex)
        if r is False:
            raise peripheral.PeripheralError("auth failed for sector %d of %s" % (int(sector), uid))
        return Session(self, str(uid).lower(), int(sector))

    def raw(self, frame_hex):
        """Send an arbitrary air frame (hex) and return the reply (hex)."""
        return self.peripheral.call("raw", _hex(_bytes(frame_hex, None, "frame")))

    def rssi(self):
        return self.peripheral.call("rssi")

    def __repr__(self):
        return "<nfc reader '%s'>" % self.name


class Printer:
    """A Card Printer: peripheral type card_printer. It only paints."""

    TYPE = "card_printer"

    def __init__(self, p):
        self.peripheral = p
        self.name = p.name

    @staticmethod
    def find(name=None):
        return _find(Printer, name)

    @staticmethod
    def find_all():
        return [Printer(p) for p in peripheral.find_all(Printer.TYPE)]

    def present(self):
        return bool(self.peripheral.call("present"))

    def status(self):
        """{present, kind, printing, progress, dyes: {white: 3, ...}}"""
        return self.peripheral.call("status")

    def cost(self, design):
        """The dyes a design takes (one per palette colour used), without printing."""
        return self.peripheral.call("cost", _design_map(design))

    def print(self, design):
        """Print a Design (or its to_map()) on the card in the printer; the
        card is released when done (print_done / print_failed events)."""
        return self.peripheral.call("print", _design_map(design))

    def eject(self):
        return bool(self.peripheral.call("eject"))

    def __repr__(self):
        return "<card printer '%s'>" % self.name


def _find(cls, name):
    if name is None:
        p = peripheral.find(cls.TYPE)
    else:
        p = peripheral.wrap(name)
        if p is not None and p.type != cls.TYPE:
            raise peripheral.PeripheralError("'%s' is a %s, not a %s" % (name, p.type, cls.TYPE))
    return None if p is None else cls(p)
