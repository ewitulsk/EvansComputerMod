#!/usr/bin/env python3
# NOTE: the Blockbench model (models/wifi_module.bbmodel, scripts/export-radio-models.py) supersedes this texture.
"""Draw the Wi-Fi module item texture (16x16) procedurally.

A dark-green radio card with a gold bay connector at the bottom, a shielded RF
can, and a black whip antenna with three signal arcs. Run from the repo root:

    python scripts/gen-wifi-module-texture.py
"""
from pathlib import Path

from PIL import Image

OUT = Path("src/main/resources/assets/evanscomputermod/textures/item/wifi_module.png")

PCB = (34, 96, 58, 255)
PCB_DARK = (22, 66, 40, 255)
PCB_EDGE = (16, 44, 28, 255)
GOLD = (222, 178, 64, 255)
GOLD_DARK = (160, 120, 36, 255)
CAN = (176, 182, 190, 255)
CAN_DARK = (120, 126, 134, 255)
CAN_LIGHT = (214, 220, 226, 255)
ANT = (28, 28, 32, 255)
ANT_TIP = (70, 70, 78, 255)
ARC = (120, 210, 255, 255)
ARC_DIM = (70, 150, 210, 255)
LED = (90, 255, 120, 255)
CLEAR = (0, 0, 0, 0)


def main() -> None:
    im = Image.new("RGBA", (16, 16), CLEAR)
    px = im.load()
    # Card body: x 2..13, y 7..15
    for y in range(7, 16):
        for x in range(2, 14):
            edge = x in (2, 13) or y == 7
            px[x, y] = PCB_EDGE if edge else (PCB if (x + y) % 5 else PCB_DARK)
    # Gold bay connector fingers along the bottom.
    for x in range(3, 13):
        px[x, 15] = GOLD if x % 2 else GOLD_DARK
        px[x, 14] = GOLD_DARK if x % 2 else PCB_EDGE
    # RF shield can.
    for y in range(9, 13):
        for x in range(6, 12):
            px[x, y] = CAN_LIGHT if y == 9 or x == 6 else (CAN_DARK if y == 12 or x == 11 else CAN)
    # Status LED.
    px[4, 9] = LED
    # Whip antenna from the card's top-left up to y=1.
    for y in range(1, 8):
        px[4, y] = ANT
    px[4, 1] = ANT_TIP
    # Signal arcs to the right of the antenna tip.
    arcs = [
        [(6, 2), (7, 3), (6, 4)],
        [(8, 1), (9, 2), (9, 3), (9, 4), (8, 5)],
        [(11, 0), (12, 1), (12, 2), (12, 3), (12, 4), (11, 5)],
    ]
    for i, arc in enumerate(arcs):
        for (x, y) in arc:
            px[x, y] = ARC if i < 2 else ARC_DIM
    OUT.parent.mkdir(parents=True, exist_ok=True)
    im.save(OUT)
    print(f"wrote {OUT}")


if __name__ == "__main__":
    main()
