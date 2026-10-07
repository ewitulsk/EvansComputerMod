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



# ------------------------------------------------------------ speaker (dark oak cabinet)
WOOD = hexc('4a3222')
WOOD_DARK = hexc('33221a')
WOOD_LIGHT = hexc('5a3d29')


def wood(edge=True):
    im = Image.new('RGBA', (16, 16), WOOD)
    for y in range(16):
        for x in range(16):
            # vertical grain
            if (x * 7 + y // 5) % 5 == 0:
                im.putpixel((x, y), WOOD_LIGHT)
            elif (x * 3 + y // 3) % 7 == 0:
                im.putpixel((x, y), WOOD_DARK)
    if edge:
        for i in range(16):
            for (x, y) in ((i, 0), (i, 15), (0, i), (15, i)):
                im.putpixel((x, y), WOOD_DARK)
    return im


def speaker_front(level):
    """Wood frame, black mesh grille, a cone that pushes out as the level rises."""
    im = wood()
    mesh_a, mesh_b = hexc('141414'), hexc('1e1e1e')
    for y in range(1, 15):
        for x in range(1, 15):
            im.putpixel((x, y), mesh_a if (x + y) % 2 else mesh_b)
    cx = cy = 7.5
    surround = [hexc('262626'), hexc('2a2a2a'), hexc('2e2e2e'), hexc('333333')][level]
    cone = [hexc('3a3a3a'), hexc('444444'), hexc('4f4f4f'), hexc('5a5a5a')][level]
    cap = [hexc('4c4c4c'), hexc('626262'), hexc('7c7c7c'), hexc('9a9a9a')][level]
    ring = hexc('6a6a6a') if level >= 2 else None
    for y in range(16):
        for x in range(16):
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            if d <= 1.6:
                im.putpixel((x, y), cap)
            elif d <= 4.6:
                # cone with the mesh still faintly showing through
                im.putpixel((x, y), cone if (x + y) % 2 else tuple(max(0, c - 8) for c in cone[:3]) + (255,))
            elif d <= 6.1:
                im.putpixel((x, y), surround)
            if ring and 4.6 < d <= 5.2:
                im.putpixel((x, y), ring)
    return im


def speaker_back():
    im = wood()
    # a small jack plate
    for y in range(6, 10):
        for x in range(5, 11):
            im.putpixel((x, y), hexc('2b2b2b'))
    im.putpixel((7, 7), hexc('c9a13a'))
    im.putpixel((8, 8), hexc('c9a13a'))
    return im


if __name__ == '__main__':
    save(from_map(CONTROLLER, CONTROLLER_PALETTE), 'item', 'wireless_controller.png')
    for level in range(4):
        save(speaker_front(level), 'block', 'speaker_front_%d.png' % level)
    save(wood(), 'block', 'speaker_side.png')
    save(speaker_back(), 'block', 'speaker_back.png')
