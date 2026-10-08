"""Access Point: procedurally drawn 16px texture atlas (idle + active LED) and the
block / item models and blockstate. No vanilla textures are referenced. A
Blockbench modelling pass will replace the model later; rerun this script to
regenerate the placeholder assets.

Atlas layout (u, v in texture pixels):
  0..12 x 0..10   body top: off-white plastic, vent slots, Wi-Fi logo
  12..16 x 0..10  body underside: darker plastic, rubber feet
  0..12 x 10..14  body front / back: seam line, port row
  12..16 x 10..14 body sides
  12..13 x 14..15 status LED (green when active, dark when idle)
  13..14 x 14..15 power LED (amber)
  14..15 x 0..9   antenna (black rubber, ribbed)
  15..16 x 0..2   antenna tip / hinge (dark grey)
  0..4 x 14..16   antenna hinge block
"""
import json
import os
from PIL import Image

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'src', 'main', 'resources', 'assets', 'evanscomputermod')


def hexc(h, a=255):
    h = h.lstrip('#')
    return (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16), a)


def rect(im, x0, y0, x1, y1, c):
    for y in range(y0, y1):
        for x in range(x0, x1):
            im.putpixel((x, y), c)


def noise(x, y, k):
    """Deterministic speckle so flat plastic isn't perfectly flat."""
    h = (x * 73856093) ^ (y * 19349663) ^ (k * 83492791)
    h = (h ^ (h >> 13)) * 1274126177
    return ((h >> 8) & 0xFF) / 255.0


def shade(c, f):
    return tuple(max(0, min(255, int(v * f))) for v in c[:3]) + (c[3],)


WHITE, WHITE_LO, WHITE_HI = hexc('dfe3e6'), hexc('b9bfc4'), hexc('f4f6f7')
DARK, DARK_LO = hexc('5b6168'), hexc('3c4046')
RUBBER, RUBBER_HI = hexc('202326'), hexc('34383d')
BLUE = hexc('3d8bd9')
PORT = hexc('2a2e33')


def atlas(active):
    im = Image.new('RGBA', (16, 16), (0, 0, 0, 0))
    # top
    for y in range(0, 10):
        for x in range(0, 12):
            im.putpixel((x, y), shade(WHITE, 0.96 + 0.06 * noise(x, y, 1)))
    rect(im, 0, 0, 12, 1, WHITE_HI)
    rect(im, 0, 9, 12, 10, WHITE_LO)
    for x in (2, 4, 6, 8):                      # vent slots near the back
        rect(im, x, 1, x + 1, 4, WHITE_LO)
    # Wi-Fi logo (three arcs + dot), centred lower half
    for (x, y) in [(5, 8), (6, 8), (4, 7), (7, 7), (3, 6), (8, 6), (5, 6), (6, 6), (4, 5), (7, 5)]:
        im.putpixel((x, y), BLUE)
    # underside
    for y in range(0, 10):
        for x in range(12, 16):
            im.putpixel((x, y), shade(DARK, 0.95 + 0.08 * noise(x, y, 2)))
    for (x, y) in [(12, 0), (15, 0), (12, 9), (15, 9)]:
        im.putpixel((x, y), RUBBER)
    # front / back strip
    for y in range(10, 14):
        for x in range(0, 12):
            im.putpixel((x, y), shade(WHITE, 0.92 + 0.06 * noise(x, y, 3)))
    rect(im, 0, 10, 12, 11, WHITE_HI)
    rect(im, 0, 13, 12, 14, WHITE_LO)            # seam to the base
    for x in (6, 8, 10):                         # ethernet ports
        rect(im, x, 11, x + 1, 13, PORT)
    # sides
    for y in range(10, 14):
        for x in range(12, 16):
            im.putpixel((x, y), shade(WHITE, 0.88 + 0.06 * noise(x, y, 4)))
    rect(im, 12, 13, 16, 14, WHITE_LO)
    # LEDs
    im.putpixel((12, 14), hexc('39d353') if active else hexc('1f3a26'))
    im.putpixel((13, 14), hexc('f0a020') if active else hexc('5a4520'))
    # antenna
    for y in range(0, 9):
        im.putpixel((14, y), RUBBER_HI if y % 3 == 0 else RUBBER)
    rect(im, 15, 0, 16, 2, DARK_LO)
    # hinge
    rect(im, 0, 14, 4, 16, DARK)
    rect(im, 0, 14, 4, 15, shade(DARK, 1.15))
    return im


def face(uv, cull=None):
    f = {"uv": uv, "texture": "#0"}
    if cull:
        f["cullface"] = cull
    return f


def model(texture):
    body = {
        "name": "body", "from": [2, 0, 3], "to": [14, 4, 13],
        "faces": {
            "north": face([0, 10, 12, 14]), "south": face([0, 10, 12, 14]),
            "east": face([12, 10, 16, 14]), "west": face([12, 10, 16, 14]),
            "up": face([0, 0, 12, 10]), "down": face([12, 0, 16, 10], "down"),
        },
    }
    led = {
        "name": "status_led", "from": [3, 2, 2.75], "to": [4, 3, 3],
        "faces": {"north": face([12, 14, 13, 15])},
    }
    pwr = {
        "name": "power_led", "from": [4.5, 2, 2.75], "to": [5.5, 3, 3],
        "faces": {"north": face([13, 14, 14, 15])},
    }
    els = [body, led, pwr]
    for name, x in (("antenna_left", 3), ("antenna_right", 12)):
        els.append({
            "name": name + "_hinge", "from": [x - 0.5, 4, 11], "to": [x + 1.5, 5, 13],
            "faces": {d: face([0, 14, 4, 16]) for d in ("north", "south", "east", "west", "up")},
        })
        els.append({
            "name": name, "from": [x, 5, 11.5], "to": [x + 1, 14, 12.5],
            "faces": {
                **{d: face([14, 0, 15, 9]) for d in ("north", "south", "east", "west")},
                "up": face([15, 0, 16, 1]),
            },
        })
    return {
        "parent": "minecraft:block/block",
        "ambientocclusion": False,
        "textures": {"0": "evanscomputermod:block/" + texture, "particle": "evanscomputermod:block/" + texture},
        "elements": els,
        "display": {
            "gui": {"rotation": [30, 225, 0], "translation": [0, 2, 0], "scale": [0.8, 0.8, 0.8]},
            "ground": {"translation": [0, 3, 0], "scale": [0.5, 0.5, 0.5]},
            "fixed": {"scale": [0.75, 0.75, 0.75]},
            "thirdperson_righthand": {"rotation": [75, 45, 0], "translation": [0, 2.5, 0], "scale": [0.5, 0.5, 0.5]},
            "firstperson_righthand": {"rotation": [0, 45, 0], "translation": [0, 2, 0], "scale": [0.5, 0.5, 0.5]},
        },
    }


def write_json(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, 'w', newline='\n') as f:
        json.dump(obj, f, indent=2)
        f.write('\n')


def main():
    tex = os.path.join(ROOT, 'textures', 'block')
    os.makedirs(tex, exist_ok=True)
    atlas(False).save(os.path.join(tex, 'access_point.png'))
    atlas(True).save(os.path.join(tex, 'access_point_active.png'))
    write_json(os.path.join(ROOT, 'models', 'block', 'access_point.json'), model('access_point'))
    write_json(os.path.join(ROOT, 'models', 'block', 'access_point_active.json'), model('access_point_active'))
    write_json(os.path.join(ROOT, 'models', 'item', 'access_point.json'), {"parent": "evanscomputermod:block/access_point_active"})
    rot = {"north": 0, "east": 90, "south": 180, "west": 270}
    variants = {}
    for facing, y in rot.items():
        for active in (False, True):
            v = {"model": "evanscomputermod:block/access_point" + ("_active" if active else "")}
            if y:
                v["y"] = y
            variants["active=%s,facing=%s" % (str(active).lower(), facing)] = v
    write_json(os.path.join(ROOT, 'blockstates', 'access_point.json'), {"variants": variants})


if __name__ == '__main__':
    main()
