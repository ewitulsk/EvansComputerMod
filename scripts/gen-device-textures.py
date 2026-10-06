"""16px textures for the Wireless Controller item (and other device items/blocks).

Run from anywhere: python3 scripts/gen-device-textures.py
"""
import os
from PIL import Image

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'src', 'main', 'resources',
                   'assets', 'evanscomputermod', 'textures')


def hexc(h, a=255):
    h = h.lstrip('#')
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), a)


def from_map(rows, palette):
    im = Image.new('RGBA', (16, 16), (0, 0, 0, 0))
    for y, row in enumerate(rows):
        assert len(row) == 16, (y, row)
        for x, ch in enumerate(row):
            if ch != '.':
                im.putpixel((x, y), palette[ch])
    return im


def save(im, *path):
    p = os.path.join(OUT, *path)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    im.save(p)
    print('wrote', os.path.relpath(p))


# ------------------------------------------------------------ wireless controller
CONTROLLER = [
    "................",
    "................",
    "................",
    "................",
    "...oooo..oooo...",
    "..ohhhhoohhhho..",
    ".oBsBBBBBBByBBo.",
    ".osSsBBgBBxBbBo.",
    ".oBsBBBBBBBaBBo.",
    ".oBBBdBBBBsBBBo.",
    ".oBBdddBBsSsBBo.",
    ".oBBBdBooBsBBBo.",
    ".oBBBBo..oBBBBo.",
    "..oBBo....oBBo..",
    "...oo......oo...",
    "................",
]
CONTROLLER_PALETTE = {
    'o': hexc('18191c'), 'B': hexc('3b3f45'), 'h': hexc('55595f'),
    's': hexc('26282b'), 'S': hexc('6b7078'), 'd': hexc('202225'),
    'a': hexc('3fb950'), 'b': hexc('e5534b'), 'x': hexc('4493f8'), 'y': hexc('f0b429'),
    'g': hexc('c9d1d9'),
}

if __name__ == '__main__':
    save(from_map(CONTROLLER, CONTROLLER_PALETTE), 'item', 'wireless_controller.png')
