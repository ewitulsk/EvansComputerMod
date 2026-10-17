# Card scheme reference: `Mix`, bank TAG, PIN hash

Normative definitions and test vectors for the toy schemes in [CARDS.md](CARDS.md). The reference
implementation is `scripts/cards-vectors.py` (run it to regenerate the tables). Java (`card/applet`),
Rust (`rust/crates/ecm-cards`), Python (`cards.py`) and Lua (`cards.lua`) each test against these.

These are **not** real-world constructions: `Mix` is a toy ARX permutation with no security analysis, and
the bank scheme is a bare keyed hash with a counter. They are meant to hold inside the game only.

## `Mix(key[16], x[16]) -> [16]` (Sector Lattice)

Words are 32-bit **little-endian**; `rotl` rotates left; all adds are mod 2³².

```
k[0..4] = LE u32 words of key;  v0..v3 = LE u32 words of x
for r in 0..8:
    v0 = v0 + k[r mod 4]
    v1 = v1 xor rotl(v0, 7)
    v2 = v2 + v1
    v3 = v3 xor rotl(v2, 13)
    v0 = v0 + v3
    v1 = rotl(v1, 17) + k[(r+1) mod 4]
    v2 = v2 xor rotl(v0, 5)
    v3 = (v3 xor r) + k[(r+2) mod 4]
out = LE u32 words v0, v1, v2, v3
```

Sector authentication (reader R, card C, sector i with key S_i):

```
R → C: AUTH i, R1 (8 random bytes)
C → R: R2 (8 random bytes), M1 = Mix(S_i, R1 || R2)
R → C: M2 = Mix(S_i, R2 || R1)
C: session for sector i open iff M2 matches
```

Factory key on blank cards, every sector: ASCII `ECM-NFC-FACTORY0` = `45434d2d4e46432d464143544f525930`.

### Mix vectors

| key | x | Mix(key, x) |
|---|---|---|
| `45434d2d4e46432d464143544f525930` | `00000000000000000000000000000000` | `3e2e68d5726a6858461251213df31d59` |
| `45434d2d4e46432d464143544f525930` | `000102030405060708090a0b0c0d0e0f` | `e8938dca0b939828c0b5e18c388b9702` |
| `00112233445566778899aabbccddeeff` | `01020304050607081112131415161718` | `f2668ac938caea225f16cf78a07cfe81` |
| `00000000000000000000000000000000` | `00000000000000000000000000000000` | `5d4fb41a84bab1a9fa2416dcc38b9978` |
| `ffffffffffffffffffffffffffffffff` | `ffffffffffffffffffffffffffffffff` | `355b79459dc55581fc26ab3eb2eb6ce6` |

## Bank TAG (Keyed Ledger Response)

```
K = 32-byte card key; NONCE = 8 bytes from the reader; CTR, AMOUNT = unsigned 64-bit big-endian
TAG = HMAC-SHA256(K, "ECM-BANK-TAG" || NONCE || CTR || AMOUNT)[0..16)
```

The card increments CTR **before** computing TAG and returns `{ctr, tag}`.

| K | NONCE | CTR | AMOUNT | TAG |
|---|---|---|---|---|
| `000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f` | `0001020304050607` | 1 | 0 | `aef6052d8232ad7c4db312596726e55a` |
| `000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f` | `0001020304050607` | 2 | 1500 | `ce2e7e5ad8d1e0b8bd42aa2b8903e5b7` |
| `0000000000000000000000000000000000000000000000000000000000000000` | `0000000000000000` | 0 | 0 | `c1e9e0459b7a23ecb6d54dc7772a25af` |
| `ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff` | `ffffffffffffffff` | 9223372036854775807 | 1099511627776 | `6987930a133eede5b3b6de9ef58f0e91` |

## PIN hash

```
pin_hash = SHA-256("ECM-BANK-PIN" || UID[8] || PIN as ASCII digits)
```

| UID | PIN | hash |
|---|---|---|
| `0102030405060708` | `1234` | `282fbfbe3f85ce96542b0435cc442d28745c372aaacf04b6229e657998169fd2` |
| `0000000000000000` | `00000000` | `72a0521796bf77ccf576d8f5554d61b5a5cd371fad7492f3b64514b7e2a4cb19` |

## NFC air frames (`NF-106`)

Every frame on the near-field band is `cmd(1) len(1) body(len)`; modulation name `NF-106`, 106 kb/s,
channel 13.56 MHz / 848 kHz. Commands (reader → card unless noted):

| cmd | name | body | reply |
|---|---|---|---|
| 0x01 | POLL | — | 0x81 UID(8) kinds(1 bitmask: 1 bank, 2 nfc, 4 sim) from every energised card, in one of 16 slots (UID[7] & 15) |
| 0x02 | SELECT | UID(8) | 0x82 UID(8) |
| 0x03 | AUTH | sector(1) R1(8) | 0x83 R2(8) M1(16), or 0xFF reason |
| 0x04 | AUTH2 | M2(16) | 0x84 (ok) or 0xFF 0x01 (bad) |
| 0x05 | READ | sector(1) | 0x85 data(64) or 0xFF 0x02 (no session) |
| 0x06 | WRITE | sector(1) data(64) | 0x86 or 0xFF |
| 0x07 | SETKEY | sector(1) key(16) | 0x87 or 0xFF |
| 0xFF | ERROR | reason(1): 1 bad auth, 2 no session, 3 bad sector, 4 not selected, 5 malformed | — |

Replies carry the card's UID in frames so a sniffer can attribute them; nothing on the air carries a key.
