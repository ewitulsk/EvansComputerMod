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
DATA_VERSION_1211 = 3955   # Minecraft 1.21.1
ROOT = Path(__file__).resolve().parent.parent
OUT_DIR = ROOT / "src/main/resources/data/evanscomputermod/structure"
# 1.21.1 looks a test's structure up under its @GameTestHolder namespace, so
# each 1.21.1 test namespace gets its own copy (resources overlay, 1.21.1 only).
STRUCTURES_1211 = {"ecm_switch": ["gametest_switch"], "ecm_sync": ["gametest_switch"], "ecm_periph": ["gametest_empty"]}
# 1.21.1-only structures (features that only exist on 1.21.1): namespace -> {name: size}.
ONLY_1211 = {"ecm_sensor": {"gametest_sensor": (24, 8, 20)},   # lidar ranges, and the lidar_room scenario
             "ecm_storage": {"gametest_storage": (16, 6, 12)}}

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


def write(struct_name, SIZE, out_dir=OUT_DIR, data_version=DATA_VERSION):
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
        tag_int("DataVersion", data_version),
    )
    OUT = out_dir / f"{struct_name}.nbt"
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_bytes(gzip.compress(root))
    print(f"wrote {OUT} ({OUT.stat().st_size} bytes, {len(blocks)} blocks)")


for n, size in STRUCTURES.items():
    write(n, size)
for ns, names in STRUCTURES_1211.items():
    for n in names:
        write(n, STRUCTURES[n], ROOT / f"src/main/resources-mc1.21.1/data/{ns}/structure", DATA_VERSION_1211)
for ns, structs in ONLY_1211.items():
    for n, size in structs.items():
        write(n, size, ROOT / f"src/main/resources-mc1.21.1/data/{ns}/structure", DATA_VERSION_1211)
