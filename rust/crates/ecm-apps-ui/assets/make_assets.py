"""Pixel-art icons for the app platform's programs and the sample repository.

    python rust/crates/ecm-apps-ui/assets/make_assets.py

Writes 32x32 RGBA PNGs drawn here pixel by pixel (no textures from the game):
  assets/store.png, assets/picker.png, assets/chooser.png, assets/app.png
  fixtures/repo/<app>/icon.png for the three sample apps
and the Notes screenshot, cropped from the compositor's own golden frame
(src/test/resources/ui-golden/notes_typed.png).
"""
import os
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.dirname(HERE)
REPO = os.path.normpath(os.path.join(CRATE, '..', '..', '..'))
FIX = os.path.join(CRATE, 'fixtures', 'repo')

T = (0, 0, 0, 0)


def hexc(s, a=255):
    s = s.lstrip('#')
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16), a)


class Canvas:
    def __init__(self, n=32):
        self.n = n
        self.img = Image.new('RGBA', (n, n), T)
        self.p = self.img.load()

    def px(self, x, y, c):
        if 0 <= x < self.n and 0 <= y < self.n:
            self.p[x, y] = c

    def rect(self, x0, y0, x1, y1, c):
        for y in range(y0, y1 + 1):
            for x in range(x0, x1 + 1):
                self.px(x, y, c)

    def frame(self, x0, y0, x1, y1, c):
        for x in range(x0, x1 + 1):
            self.px(x, y0, c)
            self.px(x, y1, c)
        for y in range(y0, y1 + 1):
            self.px(x0, y, c)
            self.px(x1, y, c)

    def tile(self, c, edge):
        """A rounded square tile with a 1px darker edge and a light top row."""
        n = self.n
        self.rect(2, 1, n - 3, n - 2, c)
        self.rect(1, 2, n - 2, n - 3, c)
        for x in range(3, n - 3):
            self.px(x, 0, edge)
            self.px(x, n - 1, edge)
        for y in range(3, n - 3):
            self.px(0, y, edge)
            self.px(n - 1, y, edge)
        for (x, y) in [(1, 1), (2, 1), (1, 2), (n - 2, 1), (n - 3, 1), (n - 2, 2),
                       (1, n - 2), (2, n - 2), (1, n - 3), (n - 2, n - 2), (n - 3, n - 2), (n - 2, n - 3)]:
            self.px(x, y, edge)
        hi = tuple(min(255, v + 40) for v in c[:3]) + (255,)
        for x in range(3, n - 3):
            self.px(x, 1, hi)

    def text(self, x, y, glyphs, c):
        """Tiny 3x5 digits/letters: glyphs is a list of 5-row bit strings."""
        for g in glyphs:
            for r, row in enumerate(g):
                for i, b in enumerate(row):
                    if b == '#':
                        self.px(x + i, y + r, c)
            x += len(g[0]) + 1

    def save(self, path):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        self.img.save(path)


WHITE = hexc('f4f6fa')
INK = hexc('1e2228')


def store():
    c = Canvas()
    c.tile(hexc('3d8bfd'), hexc('1f5fc4'))
    # A shopping bag: handle, body, and a down arrow.
    bag, bag_edge = hexc('f4f6fa'), hexc('c9d2e0')
    c.frame(12, 6, 19, 11, bag)
    c.rect(13, 7, 18, 7, T if False else hexc('3d8bfd'))
    c.rect(8, 11, 23, 25, bag)
    c.frame(8, 11, 23, 25, bag_edge)
    c.rect(15, 14, 16, 20, hexc('3d8bfd'))
    for i in range(4):
        c.rect(12 + i, 18 + i, 19 - i, 18 + i, hexc('3d8bfd'))
    return c


def picker():
    c = Canvas()
    c.tile(hexc('d29922'), hexc('9a6b0d'))
    # A folder with a document peeking out.
    c.rect(6, 9, 14, 11, hexc('fff1c2'))
    c.rect(6, 11, 25, 24, hexc('ffe08a'))
    c.frame(6, 11, 25, 24, hexc('b07d16'))
    c.rect(11, 6, 21, 15, WHITE)
    c.frame(11, 6, 21, 15, hexc('c9d2e0'))
    for y in (8, 10, 12):
        c.rect(13, y, 19, y, hexc('8c95a3'))
    c.rect(6, 15, 25, 24, hexc('f5c84c'))
    c.frame(6, 15, 25, 24, hexc('b07d16'))
    return c


def chooser():
    c = Canvas()
    c.tile(hexc('8957e5'), hexc('5a32a8'))
    # A file on the left, arrows forking to two app squares on the right.
    c.rect(5, 11, 11, 20, WHITE)
    c.frame(5, 11, 11, 20, hexc('c9d2e0'))
    c.rect(7, 14, 9, 14, hexc('8c95a3'))
    c.rect(7, 16, 9, 16, hexc('8c95a3'))
    c.rect(12, 15, 15, 16, WHITE)
    for i in range(5):
        c.px(16 + i, 15 - i, WHITE)
        c.px(16 + i, 16 + i, WHITE)
    c.rect(21, 6, 27, 12, hexc('3fb950'))
    c.rect(21, 19, 27, 25, hexc('f78166'))
    return c


def generic():
    c = Canvas()
    c.tile(hexc('454c58'), hexc('2a2f37'))
    c.rect(9, 9, 22, 22, hexc('8c95a3'))
    c.rect(11, 11, 20, 20, hexc('353b45'))
    return c


def notes():
    c = Canvas()
    c.tile(hexc('f5c84c'), hexc('b07d16'))
    c.rect(8, 5, 23, 26, WHITE)
    c.frame(8, 5, 23, 26, hexc('c9d2e0'))
    for y in range(9, 25, 3):
        c.rect(10, y, 21, y, hexc('6faeff'))
    c.rect(8, 5, 23, 6, hexc('e5534b'))
    return c


DIG = {
    '+': ['...', '.#.', '###', '.#.', '...'],
    '1': ['.#.', '##.', '.#.', '.#.', '###'],
}


def counter():
    c = Canvas()
    c.tile(hexc('3fb950'), hexc('23863a'))
    c.rect(6, 9, 25, 22, INK)
    c.frame(6, 9, 25, 22, hexc('2a2f37'))
    # "+1" in big pixels (each glyph pixel is 2x2).
    x = 9
    for g in (DIG['+'], DIG['1']):
        for r, row in enumerate(g):
            for i, b in enumerate(row):
                if b == '#':
                    c.rect(x + 2 * i, 11 + 2 * r, x + 2 * i + 1, 12 + 2 * r, hexc('7ee787'))
        x += 8
    return c


def netcheck():
    c = Canvas()
    c.tile(hexc('1f6feb'), hexc('0d419d'))
    # A globe: circle with meridian and parallels.
    cx, cy, r = 15.5, 15.5, 10
    for y in range(32):
        for x in range(32):
            d = ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5
            if d <= r:
                c.px(x, y, hexc('79c0ff'))
            if r - 1 <= d <= r:
                c.px(x, y, WHITE)
    for y in range(6, 26):
        c.px(15, y, WHITE)
        c.px(16, y, WHITE)
    for y in (11, 20):
        for x in range(8, 24):
            if ((x - cx) ** 2 + (y - cy) ** 2) ** 0.5 <= r:
                c.px(x, y, WHITE)
    # A check mark badge.
    c.rect(20, 20, 28, 28, hexc('3fb950'))
    for (x, y) in [(22, 24), (23, 25), (24, 26), (25, 25), (26, 24), (27, 23)]:
        c.px(x, y, WHITE)
    return c


def main():
    store().save(os.path.join(HERE, 'store.png'))
    picker().save(os.path.join(HERE, 'picker.png'))
    chooser().save(os.path.join(HERE, 'chooser.png'))
    generic().save(os.path.join(HERE, 'app.png'))
    notes().save(os.path.join(FIX, 'dev.ecm.notes', 'icon.png'))
    counter().save(os.path.join(FIX, 'dev.visual.counter', 'icon.png'))
    netcheck().save(os.path.join(FIX, 'dev.ecm.netcheck', 'icon.png'))
    golden = os.path.join(REPO, 'src', 'test', 'resources', 'ui-golden', 'notes_typed.png')
    shot = Image.open(golden).convert('RGBA').crop((4, 22, 316, 118))
    os.makedirs(os.path.join(FIX, 'dev.ecm.notes', 'screenshots'), exist_ok=True)
    shot.save(os.path.join(FIX, 'dev.ecm.notes', 'screenshots', '1.png'))
    print('icons and screenshot written')


if __name__ == '__main__':
    main()
