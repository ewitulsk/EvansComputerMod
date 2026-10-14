"""Icons of the Cellular v2 operator apps: 32x32 RGBA pixel art.

    python rust/crates/ecm-cell-ui/assets/make_icons.py

Writes src/main/resources/evanscomputermod/apps/cellsite.png (a mast with a
sector panel and radio waves on blue) and carrier.png (a SIM card with signal
bars on purple); SystemApps installs them with the apps.
"""
import os
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.normpath(os.path.join(HERE, '..', '..', '..', '..', 'src', 'main', 'resources', 'evanscomputermod', 'apps'))
N = 32
T = (0, 0, 0, 0)


def hexc(s, a=255):
    s = s.lstrip('#')
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16), a)


def canvas():
    img = Image.new('RGBA', (N, N), T)
    return img, img.load()


def tile(p, fill, edge, hi):
    def rect(x0, y0, x1, y1, c):
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                p[x, y] = c
    rect(2, 1, N - 3, N - 2, edge)
    rect(1, 2, N - 2, N - 3, edge)
    rect(2, 2, N - 3, N - 3, fill)
    rect(3, 2, N - 4, 2, hi)
    return rect


def cellsite():
    img, p = canvas()
    rect = tile(p, hexc('#2f6fd0'), hexc('#1d4a8f'), hexc('#5a93e8'))
    white, grey, dark = hexc('#f2f5fa'), hexc('#b8c2d3'), hexc('#24334d')
    # the mast: a lattice tower
    for y in range(12, 28):
        half = (y - 12) // 4
        p[15 - half, y] = grey
        p[16 + half, y] = grey
        if y % 4 == 0:
            rect(15 - half, y, 16 + half, y, grey)
    rect(15, 9, 16, 27, white)
    # the sector panel on top
    rect(13, 6, 18, 11, white)
    rect(14, 7, 17, 10, dark)
    # radio waves on both sides
    for r, c in ((4, white), (7, hexc('#d6e4fb')), (10, hexc('#a9c5f3'))):
        for dy in range(-r // 2, r // 2 + 1):
            p[12 - r + abs(dy) // 2, 8 + dy] = c
            p[19 + r - abs(dy) // 2, 8 + dy] = c
    rect(10, 27, 21, 28, dark)
    return img


def carrier():
    img, p = canvas()
    rect = tile(p, hexc('#7a3fc2'), hexc('#4f2685'), hexc('#9d6be0'))
    white, gold, gold_d, dark = hexc('#f4f1fa'), hexc('#e8c25a'), hexc('#b8902e'), hexc('#2e1d4a')
    # a SIM card with its cut corner
    rect(7, 8, 20, 26, white)
    for i in range(4):
        for j in range(4 - i):
            p[17 + i + j, 8 + i] = hexc('#7a3fc2')
    # the chip
    rect(9, 15, 16, 22, gold)
    rect(9, 18, 16, 18, gold_d)
    rect(12, 15, 12, 22, gold_d)
    # signal bars
    for i, h in enumerate((3, 6, 9, 12)):
        rect(21 + i * 2, 24 - h, 21 + i * 2, 24, white if i < 3 else hexc('#d9c8f2'))
    rect(7, 26, 20, 26, dark)
    return img


if __name__ == '__main__':
    os.makedirs(OUT, exist_ok=True)
    for name, fn in (('cellsite', cellsite), ('carrier', carrier)):
        path = os.path.join(OUT, name + '.png')
        fn().save(path)
        print('wrote', path)
