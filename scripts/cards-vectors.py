"""Reference implementation and test vectors for the card schemes (docs/cards/NFC_MIX.md).

    py -3 scripts/cards-vectors.py          prints the vectors as Markdown tables

Every implementation (Java card applets, the Rust `ecm-cards` crate, the Python and Lua
`cards` modules) must reproduce these exactly. The schemes are toys by design (see
docs/cards/CARDS.md): they must not resemble any real card protocol.
"""
import hashlib
import hmac
import struct

M = 0xFFFFFFFF


def rotl(v, n):
    return ((v << n) | (v >> (32 - n))) & M


def nfc_mix(key: bytes, x: bytes) -> bytes:
    """Sector Lattice `Mix`: 8 rounds of add-rotate-xor over four 32-bit words."""
    assert len(key) == 16 and len(x) == 16
    k = list(struct.unpack("<4I", key))
    v0, v1, v2, v3 = struct.unpack("<4I", x)
    for r in range(8):
        v0 = (v0 + k[r & 3]) & M
        v1 ^= rotl(v0, 7)
        v2 = (v2 + v1) & M
        v3 ^= rotl(v2, 13)
        v0 = (v0 + v3) & M
        v1 = (rotl(v1, 17) + k[(r + 1) & 3]) & M
        v2 ^= rotl(v0, 5)
        v3 = ((v3 ^ r) + k[(r + 2) & 3]) & M
    return struct.pack("<4I", v0, v1, v2, v3)


NFC_FACTORY_KEY = b"ECM-NFC-FACTORY0"   # 16 bytes: every sector of a blank


def bank_tag(key: bytes, nonce: bytes, ctr: int, amount: int) -> bytes:
    """Keyed Ledger Response TAG: HMAC-SHA256(K, "ECM-BANK-TAG" || NONCE || CTR || AMOUNT)[0..16)."""
    assert len(key) == 32 and len(nonce) == 8
    msg = b"ECM-BANK-TAG" + nonce + struct.pack(">Q", ctr) + struct.pack(">Q", amount)
    return hmac.new(key, msg, hashlib.sha256).digest()[:16]


def bank_pin_hash(uid: bytes, pin: str) -> bytes:
    """SHA-256("ECM-BANK-PIN" || UID || PIN ASCII)."""
    assert len(uid) == 8
    return hashlib.sha256(b"ECM-BANK-PIN" + uid + pin.encode("ascii")).digest()


def h(b):
    return b.hex()


if __name__ == "__main__":
    print("## Mix vectors\n")
    print("| key | x | Mix(key, x) |\n|---|---|---|")
    for key, x in [
        (NFC_FACTORY_KEY, bytes(16)),
        (NFC_FACTORY_KEY, bytes.fromhex("000102030405060708090a0b0c0d0e0f")),
        (bytes.fromhex("00112233445566778899aabbccddeeff"), bytes.fromhex("0102030405060708" + "1112131415161718")),
        (bytes(16), bytes(16)),
        (bytes.fromhex("ff" * 16), bytes.fromhex("ff" * 16)),
    ]:
        print(f"| `{h(key)}` | `{h(x)}` | `{h(nfc_mix(key, x))}` |")
    print("\n## Bank TAG vectors\n")
    print("| K | NONCE | CTR | AMOUNT | TAG |\n|---|---|---|---|---|")
    K1 = bytes.fromhex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
    for key, nonce, ctr, amount in [
        (K1, bytes.fromhex("0001020304050607"), 1, 0),
        (K1, bytes.fromhex("0001020304050607"), 2, 1500),
        (bytes(32), bytes(8), 0, 0),
        (bytes.fromhex("ff" * 32), bytes.fromhex("ff" * 8), 2**63 - 1, 2**40),
    ]:
        print(f"| `{h(key)}` | `{h(nonce)}` | {ctr} | {amount} | `{h(bank_tag(key, nonce, ctr, amount))}` |")
    print("\n## PIN hash vectors\n")
    print("| UID | PIN | hash |\n|---|---|---|")
    for uid, pin in [(bytes.fromhex("0102030405060708"), "1234"), (bytes(8), "00000000")]:
        print(f"| `{h(uid)}` | `{pin}` | `{h(bank_pin_hash(uid, pin))}` |")
