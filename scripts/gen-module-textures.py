"""16px textures for module bays, cartridges, items and the link interface block."""
import os
from PIL import Image

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'src', 'main', 'resources',
                   'assets', 'evanscomputermod', 'textures')


def hexc(h, a=255):
    h = h.lstrip('#')
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), a)


def canvas(fill=(0, 0, 0, 0)):
    return Image.new('RGBA', (16, 16), fill)


def rect(im, x0, y0, x1, y1, c):
    for y in range(y0, y1):
        for x in range(x0, x1):
            im.putpixel((x, y), c)


def px(im, x, y, c):
    im.putpixel((x, y), c)


# ---------------------------------------------------------------- bay interior
# Back wall of the recess (west face, projected: u = z, v = 16 - y). Opening is
# z 2..14, y 1..15 -> pixels u 2..13, v 1..14. Two slot backs with a
# connector strip each, divider band at y 7..9 (v 7..8).
back = canvas(hexc('2a2b2a'))
rect(back, 0, 0, 16, 16, hexc('2a2b2a'))
for v0 in (1, 9):  # slot rows (upper slot v 1..6, lower v 9..14)
    rect(back, 2, v0, 14, v0 + 6, hexc('303130'))
    rect(back, 2, v0, 14, v0 + 1, hexc('252625'))          # top shadow
    # connector: dark socket with gold pins, centred
    rect(back, 4, v0 + 2, 12, v0 + 4, hexc('1b1c1b'))
    for u in range(5, 11, 2):
        px(back, u, v0 + 2, hexc('c9a13a'))
        px(back, u, v0 + 3, hexc('8e6f22'))
rect(back, 2, 7, 14, 9, hexc('3a3b3a'))
back.save(OUT + '/block/terminal_bay_back.png')

# Inner walls of the recess (rims' inward faces, divider top/bottom)
inner = canvas(hexc('3b3c3b'))
for y in range(16):
    for x in range(16):
        if (x + y) % 5 == 0:
            px(inner, x, y, hexc('373837'))
rect(inner, 0, 0, 16, 1, hexc('454645'))
inner.save(OUT + '/block/terminal_bay_inner.png')

# Rim / divider faces facing out (projected from the side texture, but with a
# darker bevel so the opening reads as a slot bay)
side = Image.open(OUT + '/block/terminal_side.png').convert('RGBA').resize((16, 16), Image.NEAREST)
rim = side.copy()
# bevel just outside the opening: z 1..15, y 0..16 ring (u 1..14, v 0..15)
for u in range(1, 15):
    px(rim, u, 0, hexc('4a4b4a'))
    px(rim, u, 15, hexc('6c6d6b'))
for v in range(0, 16):
    px(rim, 1, v, hexc('4a4b4a'))
    px(rim, 14, v, hexc('6c6d6b'))
# divider band (v 7..8): brushed plate with two screws
rect(rim, 2, 7, 14, 9, hexc('595a58'))
rect(rim, 2, 7, 14, 8, hexc('656664'))
px(rim, 3, 7, hexc('2f302f'))
px(rim, 12, 7, hexc('2f302f'))
rim.save(OUT + '/block/terminal_bay_rim.png')


# ---------------------------------------------------------------- cartridges
# Layout: face 10x4 at (0,0); top/bottom edge 10x2 at (0,4)/(0,6); side edges
# 2x4 at (10,0)/(12,0); LED 1x1 at (14,0); antenna 1x3 at (15,0).
def cartridge(face_fn, body, light, dark, led, extra=None):
    im = canvas()
    rect(im, 0, 0, 10, 4, body)
    face_fn(im)
    rect(im, 0, 4, 10, 6, light)   # top edge
    rect(im, 0, 6, 10, 8, dark)    # bottom edge
    rect(im, 10, 0, 12, 4, dark)
    rect(im, 12, 0, 14, 4, dark)
    px(im, 14, 0, led)
    if extra:
        extra(im)
    return im


def generic_face(im):
    b, l, d = hexc('6f747a'), hexc('8c9298'), hexc('4a4e53')
    rect(im, 0, 0, 10, 1, l)
    rect(im, 0, 3, 10, 4, d)
    px(im, 0, 1, l); px(im, 0, 2, l); px(im, 9, 1, d); px(im, 9, 2, d)
    rect(im, 2, 1, 7, 3, hexc('d8d4c4'))          # label
    rect(im, 3, 1, 6, 2, hexc('a8a496'))
    px(im, 8, 1, hexc('4ce04c')); px(im, 8, 2, hexc('2a8a2a'))  # status LED


cartridge(generic_face, hexc('6f747a'), hexc('8c9298'), hexc('4a4e53'), hexc('4ce04c')) \
    .save(OUT + '/block/module_generic.png')


def link_face(im):
    brass, hi, lo = hexc('b8893a'), hexc('e0b45e'), hexc('7d5a22')
    rect(im, 0, 0, 10, 4, brass)
    rect(im, 0, 0, 10, 1, hi)
    rect(im, 0, 3, 10, 4, lo)
    px(im, 0, 1, hi); px(im, 0, 2, hi); px(im, 9, 1, lo); px(im, 9, 2, lo)
    # dark window with a red "signal waves" glyph
    rect(im, 1, 1, 7, 3, hexc('2b1d12'))
    px(im, 2, 2, hexc('e8403a'))
    px(im, 3, 1, hexc('b02a24')); px(im, 3, 2, hexc('b02a24'))
    px(im, 5, 1, hexc('7a1c18')); px(im, 5, 2, hexc('7a1c18'))
    px(im, 8, 1, hexc('ff5a4a')); px(im, 8, 2, hexc('a02a20'))  # red LED


def link_extra(im):
    rect(im, 15, 0, 16, 3, hexc('d0302a'))
    px(im, 15, 0, hexc('ff6a5a'))


cartridge(link_face, hexc('b8893a'), hexc('d9aa55'), hexc('7d5a22'), hexc('ff5a4a'), link_extra) \
    .save(OUT + '/block/module_redstone_link.png')


# ---------------------------------------------------------------- items
card = canvas()
pcb, pcb_hi, pcb_lo = hexc('2f7a3a'), hexc('46a052'), hexc('1d4f25')
rect(card, 1, 3, 15, 13, pcb)
rect(card, 1, 3, 15, 4, pcb_hi)
rect(card, 1, 12, 15, 13, pcb_lo)
rect(card, 1, 3, 2, 13, pcb_hi)
rect(card, 14, 3, 15, 13, pcb_lo)
for x in range(3, 14, 2):       # gold edge fingers
    rect(card, x, 13, x + 1, 15, hexc('e0b84a'))
    px(card, x, 14, hexc('a8842a'))
rect(card, 5, 5, 10, 9, hexc('1c1c1e'))   # chip
rect(card, 5, 5, 10, 6, hexc('34343a'))
for x in range(6, 10, 2):
    px(card, x, 4, hexc('b8b8b8')); px(card, x, 9, hexc('b8b8b8'))
for y in (10, 11):               # traces
    rect(card, 3, y, 12, y + 1, hexc('3f9a4c') if y == 10 else pcb)
px(card, 12, 5, hexc('e0b84a')); px(card, 12, 7, hexc('e0b84a'))   # vias
px(card, 3, 6, hexc('d0302a'))   # tiny red SMD
card.save(OUT + '/item/module_expansion_card.png')

mod = canvas()
brass, hi, lo = hexc('b8893a'), hexc('e0b45e'), hexc('7d5a22')
rect(mod, 2, 5, 14, 14, brass)
rect(mod, 2, 5, 14, 6, hi)
rect(mod, 2, 13, 14, 14, lo)
rect(mod, 2, 5, 3, 14, hi)
rect(mod, 13, 5, 14, 14, lo)
rect(mod, 4, 7, 10, 11, hexc('2b1d12'))
px(mod, 5, 9, hexc('e8403a')); px(mod, 6, 8, hexc('b02a24')); px(mod, 6, 9, hexc('b02a24'))
px(mod, 8, 8, hexc('7a1c18')); px(mod, 8, 9, hexc('7a1c18')); px(mod, 8, 10, hexc('7a1c18'))
px(mod, 11, 7, hexc('ff5a4a'))
for x in range(4, 13, 2):        # contacts
    px(mod, x, 12, hexc('5a4012'))
rect(mod, 11, 1, 12, 5, hexc('d0302a'))   # antenna
px(mod, 11, 0, hexc('ff6a5a'))
px(mod, 10, 4, hexc('8a1a16'))
mod.save(OUT + '/item/redstone_link_module.png')

# ---------------------------------------------------------------- interface block top
top = canvas(hexc('b8893a'))
rect(top, 0, 0, 16, 1, hexc('e0b45e')); rect(top, 0, 15, 16, 16, hexc('7d5a22'))
rect(top, 0, 0, 1, 16, hexc('e0b45e')); rect(top, 15, 0, 16, 16, hexc('7d5a22'))
rect(top, 3, 3, 13, 13, hexc('2a2b2a'))
for y in range(4, 12, 2):
    for x in range(4, 12, 2):
        px(top, x, y, hexc('4a4b4a'))
px(top, 5, 5, hexc('4ce04c'))
rect(top, 3, 3, 13, 4, hexc('1f201f'))
top.save(OUT + '/block/redstone_link_interface_top.png')
print('textures ok')
