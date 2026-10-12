"""Paint the source textures for the display panel models (P1, docs/gui).

Writes models/textures/panels/*.png. They are imported into the Blockbench
projects (models/terminal_panels.bbmodel, display_panels.bbmodel and the
three tiered screen projects), which embed them; scripts/export-panel-models.py
then writes the game assets from the projects.

Pixel art at 32x32 so a 16-unit face shows 2 texels per unit. Each tier has
its own bezel material and screen tint so hardware reads at a glance:
  Tier 1 Phosphor  - cream 80s plastic, deep green (or amber) glass, scanlines
  Tier 2 Color LCD - graphite bezel, navy LCD, RGB sub-pixel badge
  Tier 3 HD        - thin black glossy bezel, near-black glass with a sheen
  no panel         - empty slot: recessed bay, screw holes, connector pins
"""
import os
import random
from PIL import Image

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'models', 'textures', 'panels')
os.makedirs(OUT, exist_ok=True)
S = 32


def noise(rgb, amt, rnd):
    return tuple(max(0, min(255, c + rnd.randint(-amt, amt))) for c in rgb)


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def panel(name, bezel, bezel_hi, bezel_lo, border, glass, glass_line, glyph=None, extra=None, seed=1):
    rnd = random.Random(seed)
    im = Image.new('RGBA', (S, S))
    px = im.load()
    for y in range(S):
        for x in range(S):
            inside = border <= x < S - border and border <= y < S - border
            if inside:
                c = glass
                if glass_line and (y - border) % 2 == 1:
                    c = glass_line
                # Gentle vignette towards the glass edges.
                d = min(x - border, S - border - 1 - x, y - border, S - border - 1 - y)
                if d < 2:
                    c = lerp(c, (0, 0, 0), 0.35 - d * 0.15)
                px[x, y] = c + (255,)
            else:
                c = bezel
                if y < border and x >= y and x < S - y:
                    c = bezel_hi            # top lit
                elif x < border and y > x and y < S - x:
                    c = lerp(bezel, bezel_hi, 0.5)
                elif y >= S - border:
                    c = bezel_lo            # bottom shadow
                elif x >= S - border:
                    c = lerp(bezel, bezel_lo, 0.5)
                # Inner lip next to the glass.
                if (x in (border - 1, S - border) and border - 1 <= y <= S - border) or \
                        (y in (border - 1, S - border) and border - 1 <= x <= S - border):
                    c = lerp(c, (0, 0, 0), 0.45)
                px[x, y] = noise(c, 4, rnd) + (255,)
    if glyph:
        col, pts = glyph
        for (x, y) in pts:
            px[x, y] = col + (255,)
    if extra:
        extra(px)
    im.save(os.path.join(OUT, name + '.png'))


def prompt_glyph(col, border):
    # "> _" and a short text line, top-left of the glass.
    x0, y0 = border + 2, border + 2
    pts = [(x0, y0), (x0 + 1, y0 + 1), (x0, y0 + 2)]                 # >
    pts += [(x0 + 3 + i, y0 + 2) for i in range(2)]                  # _
    pts += [(x0 + i, y0 + 5) for i in range(9) if i % 4 != 3]        # text
    pts += [(x0 + i, y0 + 7) for i in range(6) if i % 3 != 2]
    return col, pts


# --- Terminal panel overlays (the screen area of the Terminal's face) ---------
CREAM, CREAM_HI, CREAM_LO = (196, 184, 150), (222, 212, 182), (150, 138, 108)
panel('panel_phosphor', CREAM, CREAM_HI, CREAM_LO, 4, (8, 34, 14), (12, 44, 19),
      glyph=prompt_glyph((90, 255, 120), 4), seed=1)
panel('panel_amber', CREAM, CREAM_HI, CREAM_LO, 4, (36, 20, 4), (46, 27, 6),
      glyph=prompt_glyph((255, 176, 0), 4), seed=2)


def rgb_badge(px):
    # Sub-pixel badge bottom-right of the bezel: R G B.
    for i, c in enumerate([(220, 40, 40), (40, 200, 60), (50, 90, 230)]):
        px[S - 9 + i * 2, S - 2] = c + (255,)


GRAPH, GRAPH_HI, GRAPH_LO = (58, 61, 68), (88, 92, 102), (34, 36, 40)
panel('panel_color_lcd', GRAPH, GRAPH_HI, GRAPH_LO, 3, (16, 24, 40), None,
      glyph=prompt_glyph((120, 200, 255), 3), extra=rgb_badge, seed=3)


def sheen(px):
    # Diagonal reflection across the glass and a silver hairline on the bezel.
    for y in range(2, S - 2):
        for x in range(2, S - 2):
            d = x + y
            if 18 <= d <= 21 or 24 <= d <= 25:
                r, g, b, a = px[x, y]
                px[x, y] = lerp((r, g, b), (70, 80, 100), 0.35) + (255,)
    for i in range(1, S - 1):
        px[i, 0] = (150, 154, 162, 255)


BLACK, BLACK_HI, BLACK_LO = (20, 21, 25), (44, 46, 52), (10, 10, 12)
panel('panel_hd', BLACK, BLACK_HI, BLACK_LO, 2, (6, 7, 12), None, extra=sheen, seed=4)


def empty_slot(name):
    rnd = random.Random(9)
    im = Image.new('RGBA', (S, S))
    px = im.load()
    for y in range(S):
        for x in range(S):
            c = (40, 41, 43)
            if x < 2 or y < 2 or x >= S - 2 or y >= S - 2:
                c = (70, 72, 74)                      # slot rim
            elif x < 3 or y < 3:
                c = (22, 22, 24)                      # shadow under the rim
            px[x, y] = noise(c, 3, rnd) + (255,)
    for (cx, cy) in [(5, 5), (S - 6, 5), (5, S - 6), (S - 6, S - 6)]:   # screw holes
        for dx in (-1, 0):
            for dy in (-1, 0):
                px[cx + dx, cy + dy] = (14, 14, 15, 255)
        px[cx, cy] = (110, 112, 116, 255)
    for i in range(10):                                                  # connector pins
        x = 11 + i
        px[x, S - 5] = (196, 160, 60, 255) if i % 2 == 0 else (30, 30, 32, 255)
        px[x, S - 6] = (24, 24, 26, 255)
    im.save(os.path.join(OUT, name + '.png'))


empty_slot('panel_none')


# --- Panel items: PCB back -------------------------------------------------------
def pcb(name):
    rnd = random.Random(11)
    im = Image.new('RGBA', (S, S))
    px = im.load()
    for y in range(S):
        for x in range(S):
            c = (24, 92, 52)
            if (x * 7 + y * 3) % 23 == 0 or (y % 9 == 4 and 4 < x < 28):
                c = (170, 150, 70)                    # traces
            px[x, y] = noise(c, 6, rnd) + (255,)
    for x in range(10, 22):                           # ribbon connector
        for y in range(26, 30):
            px[x, y] = (230, 230, 230, 255) if x % 2 else (40, 40, 40, 255)
    im.save(os.path.join(OUT, name + '.png'))


pcb('panel_back')


# --- Tiered Screens: face (bezel + dark glass, seamless when tiled) and casing ---
def screen_face(name, rim, rim_lo, glass, glass2, seed):
    rnd = random.Random(seed)
    im = Image.new('RGBA', (S, S))
    px = im.load()
    for y in range(S):
        for x in range(S):
            if x < 2 or y < 2 or x >= S - 2 or y >= S - 2:
                c = rim if (x < 2 or y < 2) else rim_lo
                c = noise(c, 4, rnd)
            else:
                c = glass if (y // 2) % 2 == 0 or glass2 is None else glass2
            px[x, y] = c + (255,)
    im.save(os.path.join(OUT, name + '.png'))


def casing(name, base, seed, rivets, seams=True, vents=False):
    rnd = random.Random(seed)
    im = Image.new('RGBA', (S, S))
    px = im.load()
    for y in range(S):
        for x in range(S):
            c = base
            if x in (0, S - 1) or y in (0, S - 1):
                c = lerp(base, (0, 0, 0), 0.3)
            elif seams and y % 8 == 0:
                c = lerp(base, (0, 0, 0), 0.12)   # panel seams
            elif vents and 20 <= y <= 27 and 6 <= x <= 25 and y % 2 == 0:
                c = lerp(base, (0, 0, 0), 0.45)   # vent slots
            px[x, y] = noise(c, 5, rnd) + (255,)
    if rivets:
        for (cx, cy) in [(3, 3), (S - 4, 3), (3, S - 4), (S - 4, S - 4)]:
            px[cx, cy] = lerp(base, (255, 255, 255), 0.4) + (255,)
    im.save(os.path.join(OUT, name + '.png'))


screen_face('phosphor_screen_front', CREAM, CREAM_LO, (8, 30, 12), (11, 38, 16), 21)
casing('phosphor_screen_side', CREAM, 22, True, seams=False, vents=True)
screen_face('hd_screen_front', (30, 31, 36), (16, 16, 19), (5, 6, 10), None, 25)
casing('hd_screen_side', (36, 38, 44), 26, False)
# HD: silver hairline along the top of the bezel and a blue power LED.
_im = Image.open(os.path.join(OUT, 'hd_screen_front.png'))
for _x in range(S):
    _im.putpixel((_x, 0), (150, 154, 162, 255))
_im.putpixel((S - 4, S - 1), (60, 140, 255, 255))
_im.save(os.path.join(OUT, 'hd_screen_front.png'))
_im = Image.open(os.path.join(OUT, 'hd_screen_side.png'))
for _y in range(S):
    _im.putpixel((0, _y), (120, 124, 132, 255))
    _im.putpixel((S - 1, _y), (120, 124, 132, 255))
_im.save(os.path.join(OUT, 'hd_screen_side.png'))
print('textures ->', os.path.abspath(OUT))
