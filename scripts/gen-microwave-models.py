"""Generate the microwave link's block/item models and procedural textures.

Placeholder art until the Blockbench modelling pass: no vanilla textures, every
pixel is computed here. Outputs (under src/main/resources/assets/evanscomputermod):
  textures/block/dish_surface.png, dish_metal.png, dish_feed.png,
  textures/block/microwave_radio_front.png, _side.png, _top.png
  models/block/dish_<size>_part<P>.json   one slice of the dish per multiblock part
  models/item/dish_<size>.json            the whole dish scaled into one block
  blockstates/dish_<size>.json            facing x part variants
  models/block/microwave_radio.json, models/item/microwave_radio.json, blockstates/microwave_radio.json

The dish is authored facing north (boresight -Z) in a global pixel space of
16n x 16n (n = 1, 2, 3); part P = up*n + column covers x in [16 column, +16],
y in [16 up, +16], matching DishSize/DishBlock (columns run east for a
north-facing dish, i.e. the facing's clockwise side).

Run: py -3 scripts/gen-microwave-models.py
"""
import json
import math
import random
import struct
import zlib
from pathlib import Path

root = Path(__file__).resolve().parent.parent
assets = root / 'src/main/resources/assets/evanscomputermod'
NS = 'evanscomputermod'
SIZES = {'dish_small': 1, 'dish_medium': 2, 'dish_large': 3}
FACING_Y = {'north': 0, 'east': 90, 'south': 180, 'west': 270}


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + '\n')


def write_png(path, w, h, px):
    """px[y][x] = (r, g, b, a)"""
    raw = b''.join(b'\x00' + bytes(c for p in row for c in p) for row in px)
    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    data = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) \
        + chunk(b'IDAT', zlib.compress(raw, 9)) + chunk(b'IEND', b'')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


def clamp(v):
    return max(0, min(255, int(round(v))))


def texture(name, fn, seed):
    rnd = random.Random(seed)
    px = [[None] * 16 for _ in range(16)]
    for y in range(16):
        for x in range(16):
            r, g, b = fn(x, y, rnd)
            px[y][x] = (clamp(r), clamp(g), clamp(b), 255)
    write_png(assets / 'textures/block' / f'{name}.png', 16, 16, px)


def surface(x, y, rnd):
    d = math.hypot(x - 7.5, y - 7.5)
    v = 214 + 6 * math.cos(d * 1.3) + rnd.uniform(-5, 5)
    return v, v + 2, v + 6


def metal(x, y, rnd):
    v = 128 + 10 * math.sin(y * 2.1) + rnd.uniform(-6, 6)
    return v, v + 2, v + 5


def feed(x, y, rnd):
    edge = x in (0, 15) or y in (0, 15)
    v = 92 if edge else 52 + rnd.uniform(-4, 4)
    return v, v, v + 4


def radio_front(x, y, rnd):
    if x in (0, 15) or y in (0, 15):
        return 70, 72, 78
    if y in (3, 4) and x in (3, 5):
        return (60, 220, 90) if x == 3 else (240, 170, 40)   # link / activity LEDs
    if 7 <= y <= 11 and 3 <= x <= 12:
        return 34 + rnd.uniform(-3, 3), 38, 46                # label window
    v = 150 + rnd.uniform(-5, 5)
    return v, v + 3, v + 8


def radio_side(x, y, rnd):
    if x in (0, 15) or y in (0, 15):
        return 70, 72, 78
    if y % 3 == 1 and 3 <= x <= 12:
        return 60, 62, 68                                     # cooling fins
    v = 146 + rnd.uniform(-5, 5)
    return v, v + 3, v + 8


def radio_top(x, y, rnd):
    if (x, y) in ((2, 2), (13, 2), (2, 13), (13, 13)):
        return 200, 200, 205                                  # screws
    if x in (0, 15) or y in (0, 15):
        return 70, 72, 78
    v = 140 + rnd.uniform(-5, 5)
    return v, v + 3, v + 8


# ---------------------------------------------------------------- dish geometry

def dish_boxes(n):
    """(from, to, kind) boxes in global pixels for a north-facing dish of n blocks."""
    S = 16 * n
    R = S / 2 - 0.5
    g = 2 if n == 1 else 4
    z_edge, depth = 2.0, round(S / 10, 1)
    boxes = []
    cells = int(S / g)
    for i in range(cells):
        for j in range(cells):
            cx, cy = (i + 0.5) * g, (j + 0.5) * g
            r = math.hypot(cx - S / 2, cy - S / 2)
            if r > R:
                continue
            zf = round(z_edge + depth * (1 - (r / R) ** 2), 1)
            boxes.append(([i * g, j * g, zf], [(i + 1) * g, (j + 1) * g, zf + 1.5], 'surface'))
    fs = max(2, round(S / 12))
    fz = z_edge - S / 4
    c = S / 2
    boxes.append(([c - fs / 2, c - fs / 2, fz], [c + fs / 2, c + fs / 2, fz + fs], 'feed'))
    boxes.append(([c - 0.5, 1, fz + fs / 2 - 0.5], [c + 0.5, c - fs / 2, fz + fs / 2 + 0.5], 'metal'))   # feed boom
    back = z_edge + depth + 1.5
    boxes.append(([c - 2, 0, back], [c + 2, c, 16], 'metal'))                                           # mount post
    boxes.append(([c - 3, 0, 9], [c + 3, 1, 16], 'metal'))                                              # foot
    return boxes


def faces(frm, to, kind):
    w = {'x': to[0] - frm[0], 'y': to[1] - frm[1], 'z': to[2] - frm[2]}
    def uv(a, b):
        return [0, 0, min(16, round(w[a], 2)), min(16, round(w[b], 2))]
    tex = {'surface': '#surface', 'feed': '#feed', 'metal': '#metal'}[kind]
    side = '#metal' if kind == 'surface' else tex
    return {
        'north': {'uv': uv('x', 'y'), 'texture': tex},
        'south': {'uv': uv('x', 'y'), 'texture': side},
        'east': {'uv': uv('z', 'y'), 'texture': side},
        'west': {'uv': uv('z', 'y'), 'texture': side},
        'up': {'uv': uv('x', 'z'), 'texture': side},
        'down': {'uv': uv('x', 'z'), 'texture': side},
    }


TEXTURES = {
    'surface': f'{NS}:block/dish_surface',
    'metal': f'{NS}:block/dish_metal',
    'feed': f'{NS}:block/dish_feed',
    'particle': f'{NS}:block/dish_metal',
}


def r2(v):
    return round(v, 3)


def part_model(n, part, boxes):
    col, up = part % n, part // n
    x0, y0 = 16 * col, 16 * up
    elements = []
    for frm, to, kind in boxes:
        a = [max(frm[0], x0), max(frm[1], y0), frm[2]]
        b = [min(to[0], x0 + 16), min(to[1], y0 + 16), to[2]]
        if b[0] - a[0] <= 1e-6 or b[1] - a[1] <= 1e-6:
            continue
        f = [r2(a[0] - x0), r2(a[1] - y0), r2(a[2])]
        t = [r2(b[0] - x0), r2(b[1] - y0), r2(b[2])]
        elements.append({'from': f, 'to': t, 'faces': faces(f, t, kind)})
    return {'parent': 'minecraft:block/block', 'ambientocclusion': False, 'textures': TEXTURES, 'elements': elements}


def item_model(n, boxes):
    # Scale the whole dish into one block, centred.
    s = 1.0 / n
    elements = []
    for frm, to, kind in boxes:
        f = [r2(frm[0] * s), r2(frm[1] * s), r2(8 + (frm[2] - 8) * s)]
        t = [r2(to[0] * s), r2(to[1] * s), r2(8 + (to[2] - 8) * s)]
        if min(t[i] - f[i] for i in range(3)) <= 0:
            continue
        elements.append({'from': f, 'to': t, 'faces': faces(f, t, kind)})
    return {'parent': 'minecraft:block/block', 'textures': TEXTURES, 'elements': elements}


def main():
    texture('dish_surface', surface, 1)
    texture('dish_metal', metal, 2)
    texture('dish_feed', feed, 3)
    texture('microwave_radio_front', radio_front, 4)
    texture('microwave_radio_side', radio_side, 5)
    texture('microwave_radio_top', radio_top, 6)

    for name, n in SIZES.items():
        boxes = dish_boxes(n)
        controller = 1 if n == 3 else 0
        for part in range(n * n):
            write_json(assets / 'models/block' / f'{name}_part{part}.json', part_model(n, part, boxes))
        write_json(assets / 'models/item' / f'{name}.json', item_model(n, boxes))
        variants = {}
        for facing, y in FACING_Y.items():
            for part in range(9):
                model = f'{NS}:block/{name}_part{part if part < n * n else controller}'
                v = {'model': model}
                if y:
                    v['y'] = y
                variants[f'facing={facing},part={part}'] = v
        write_json(assets / 'blockstates' / f'{name}.json', {'variants': variants})

    radio = {
        'parent': 'minecraft:block/block',
        'textures': {
            'front': f'{NS}:block/microwave_radio_front',
            'side': f'{NS}:block/microwave_radio_side',
            'top': f'{NS}:block/microwave_radio_top',
            'particle': f'{NS}:block/microwave_radio_side',
        },
        'elements': [
            {'from': [2, 0, 2], 'to': [14, 12, 14], 'faces': {
                'north': {'uv': [2, 4, 14, 16], 'texture': '#front'},
                'south': {'uv': [2, 4, 14, 16], 'texture': '#side'},
                'east': {'uv': [2, 4, 14, 16], 'texture': '#side'},
                'west': {'uv': [2, 4, 14, 16], 'texture': '#side'},
                'up': {'uv': [2, 2, 14, 14], 'texture': '#top'},
                'down': {'uv': [2, 2, 14, 14], 'texture': '#top', 'cullface': 'down'}}},
            {'from': [6, 12, 6], 'to': [10, 15, 10], 'faces': {
                d: {'uv': [6, 6, 10, 9], 'texture': '#top'} for d in ['north', 'south', 'east', 'west', 'up']}},
        ],
    }
    write_json(assets / 'models/block/microwave_radio.json', radio)
    write_json(assets / 'models/item/microwave_radio.json', {'parent': f'{NS}:block/microwave_radio'})
    rv = {}
    for facing, y in FACING_Y.items():
        v = {'model': f'{NS}:block/microwave_radio'}
        if y:
            v['y'] = y
        rv[f'facing={facing}'] = v
    write_json(assets / 'blockstates/microwave_radio.json', {'variants': rv})


if __name__ == '__main__':
    main()
