#!/usr/bin/env python3
"""Generate the empty GameTest structures (all air) used by the mod's GameTests.

Writes src/main/resources/data/evanscomputermod/structure/<name>.nbt for each
entry of STRUCTURES (gametest_empty: ecm_network; gametest_switch: ecm_switch).
Tests build their setups (terminals, cables) in code on top of it, so no
hand-made .nbt files need maintaining. Re-run if the size must change.
"""
import gzip
import struct
from pathlib import Path

STRUCTURES = {             # name: (x, y, z)
    "gametest_empty": (12, 6, 6),
    "gametest_switch": (20, 8, 12),   # must fit every SwitchScenarios layout (checked at registration)
}
DATA_VERSION = 4786        # Minecraft 26.1 (from vanilla data/minecraft/structure/empty.nbt)
OUT_DIR = Path(__file__).resolve().parent.parent / "src/main/resources/data/evanscomputermod/structure"

TAG_END, TAG_INT, TAG_STRING, TAG_LIST, TAG_COMPOUND = 0, 3, 8, 9, 10


def name(n):
    b = n.encode()
    return struct.pack(">H", len(b)) + b


def tag_int(n, v):
    return bytes([TAG_INT]) + name(n) + struct.pack(">i", v)


def tag_string(n, v):
    return bytes([TAG_STRING]) + name(n) + name(v)


def tag_list(n, elem_type, payloads):
    return bytes([TAG_LIST]) + name(n) + bytes([elem_type]) + struct.pack(">i", len(payloads)) + b"".join(payloads)


def compound_payload(*tags):
    return b"".join(tags) + bytes([TAG_END])


def int_payload(v):
    return struct.pack(">i", v)


def write(struct_name, SIZE):
    blocks = []
    for x in range(SIZE[0]):
        for y in range(SIZE[1]):
            for z in range(SIZE[2]):
                pos = tag_list("pos", TAG_INT, [int_payload(x), int_payload(y), int_payload(z)])
                blocks.append(compound_payload(pos, tag_int("state", 0)))

    root = bytes([TAG_COMPOUND]) + name("") + compound_payload(
        tag_list("size", TAG_INT, [int_payload(v) for v in SIZE]),
        tag_list("entities", TAG_END, []),
        tag_list("blocks", TAG_COMPOUND, blocks),
        tag_list("palette", TAG_COMPOUND, [compound_payload(tag_string("Name", "minecraft:air"))]),
        tag_int("DataVersion", DATA_VERSION),
    )
    OUT = OUT_DIR / f"{struct_name}.nbt"
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(gzip.compress(root))
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes, {len(blocks)} blocks)")


for n, size in STRUCTURES.items():
    write(n, size)
