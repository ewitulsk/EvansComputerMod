"""The Chat app's icon: a 32x32 RGBA pixel-art tile with two speech bubbles.

    python rust/wasm-programs/chatapp/assets/make_icon.py

Writes assets/icon.png next to this script (drawn here pixel by pixel; the
app embeds it with include_bytes! and `chatapp --source` writes it into the
package folder).
"""
import os
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
N = 32
T = (0, 0, 0, 0)


def hexc(s, a=255):
    s = s.lstrip('#')
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16), a)


img = Image.new('RGBA', (N, N), T)
p = img.load()


def px(x, y, c):
    if 0 <= x < N and 0 <= y < N:
        p[x, y] = c


def rect(x0, y0, x1, y1, c):
    for y in range(y0, y1 + 1):
        for x in range(x0, x1 + 1):
            px(x, y, c)


def bubble(x0, y0, x1, y1, fill, edge, tail_left):
    """A rounded bubble with a 1px edge and a little tail at the bottom."""
    rect(x0 + 1, y0, x1 - 1, y1, edge)
    rect(x0, y0 + 1, x1, y1 - 1, edge)
    rect(x0 + 1, y0 + 1, x1 - 1, y1 - 1, fill)
    if tail_left:
        for i in range(3):
            px(x0 + 2 + i, y1 + 1 + i, edge)
            px(x0 + 3 + i, y1 + 1 + i, edge)
        px(x0 + 3, y1, fill)
        px(x0 + 4, y1, fill)
        px(x0 + 4, y1 + 1, fill)
    else:
        for i in range(3):
            px(x1 - 2 - i, y1 + 1 + i, edge)
            px(x1 - 3 - i, y1 + 1 + i, edge)
        px(x1 - 3, y1, fill)
        px(x1 - 4, y1, fill)
        px(x1 - 4, y1 + 1, fill)


# Tile: teal, rounded, darker edge, lighter top row.
tile, tile_edge, tile_hi = hexc('#1f8a70'), hexc('#145c4b'), hexc('#36b38f')
rect(2, 1, N - 3, N - 2, tile)
rect(1, 2, N - 2, N - 3, tile)
for x in range(3, N - 3):
    px(x, 0, tile_edge)
    px(x, N - 1, tile_edge)
for y in range(3, N - 3):
    px(0, y, tile_edge)
    px(N - 1, y, tile_edge)
for (x, y) in [(1, 1), (2, 1), (1, 2), (N - 2, 1), (N - 3, 1), (N - 2, 2), (1, N - 2), (2, N - 2), (1, N - 3),
               (N - 2, N - 2), (N - 3, N - 2), (N - 2, N - 3)]:
    px(x, y, tile_edge)
for x in range(3, N - 3):
    px(x, 2, tile_hi)

# A white bubble (someone else) and a yellow one (you), with text lines.
white, white_edge, ink = hexc('#f4f7f6'), hexc('#9fb5ae'), hexc('#4a5a56')
yellow, yellow_edge, ink2 = hexc('#ffd866'), hexc('#c79a1e'), hexc('#7a5a10')
bubble(4, 5, 21, 14, white, white_edge, True)
rect(7, 8, 18, 8, ink)
rect(7, 11, 14, 11, ink)
bubble(11, 17, 27, 25, yellow, yellow_edge, False)
rect(14, 20, 24, 20, ink2)
rect(14, 22, 20, 22, ink2)

img.save(os.path.join(HERE, 'icon.png'))
print('wrote', os.path.join(HERE, 'icon.png'))
