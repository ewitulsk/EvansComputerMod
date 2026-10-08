"""Generate placeholder assets and data for the radio power/hazard blocks (lanes 4D/5C).

Amplifiers (100 W / 1 kW / 10 kW), the antenna tuner, the RF meter and melted scrap: procedurally
drawn 16x16 textures (no vanilla texture references), facing blockstates, block/item models, loot
tables, recipes (c: tags), the rf_burn damage type, and the amplifier/tuner/SDR entries in
#evanscomputermod:rf_coax_ports and #minecraft:mineable/pickaxe. A later Blockbench pass replaces
the models and textures; the IDs stay.

Run: py -3 scripts/gen-radio-amp-assets.py
"""
import json
import random
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parent.parent
assets = root / 'src/main/resources/assets/evanscomputermod'
data = root / 'src/main/resources-mc1.21.1/data'
NS = 'evanscomputermod'
DIRS = ['north', 'south', 'west', 'east', 'up', 'down']


def write(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


def clamp(c):
    return tuple(max(0, min(255, int(v))) for v in c)


def noisy(base, seed, amount=0.08):
    rnd = random.Random(seed)
    img = Image.new('RGBA', (16, 16))
    for y in range(16):
        for x in range(16):
            f = 1 + rnd.uniform(-amount, amount)
            img.putpixel((x, y), clamp(tuple(v * f for v in base)) + (255,))
    return img


def save(img, kind, name):
    path = assets / 'textures' / kind / f'{name}.png'
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)


def frame(img, color):
    for i in range(16):
        for (x, y) in [(i, 0), (i, 15), (0, i), (15, i)]:
            img.putpixel((x, y), color)


# ------------------------------------------------------------------ textures

AMPS = {
    # id: (case colour, accent, label LEDs, model height)
    'amplifier_100w': ((70, 74, 82), (200, 120, 60), 1, 8),
    'amplifier_1kw': ((52, 58, 70), (220, 170, 60), 2, 12),
    'amplifier_10kw': ((40, 44, 52), (230, 70, 60), 3, 16),
}


def amp_front(base, accent, leds, seed):
    img = noisy(base, seed)
    frame(img, clamp(tuple(v * 0.7 for v in base)) + (255,))
    # meter window
    for y in range(3, 8):
        for x in range(3, 10):
            img.putpixel((x, y), (225, 220, 190, 255))
    for i, y in enumerate([7, 6, 5, 5, 4, 4, 4]):
        img.putpixel((3 + i, y), (40, 40, 40, 255))
    # LEDs
    for i in range(leds):
        img.putpixel((12, 3 + 2 * i), (90, 230, 90, 255))
    img.putpixel((12, 9), (230, 60, 50, 255))
    # coax jacks
    for cx in (4, 11):
        for (x, y) in [(cx, 11), (cx + 1, 11), (cx, 12), (cx + 1, 12)]:
            img.putpixel((x, y), (205, 205, 210, 255))
    for x in range(2, 14):
        img.putpixel((x, 14), accent + (255,))
    return img


def amp_side(base, seed):
    img = noisy(base, seed)
    frame(img, clamp(tuple(v * 0.7 for v in base)) + (255,))
    for x in range(2, 14):
        for y in range(3, 13):
            if x % 3 == 0:
                img.putpixel((x, y), clamp(tuple(v * 0.55 for v in base)) + (255,))
    return img


def amp_top(base, seed):
    img = noisy(base, seed)
    for x in range(16):
        for y in range(16):
            if y % 2 == 0:
                img.putpixel((x, y), clamp(tuple(v * 1.35 for v in base)) + (255,))
    return img


def tuner_front():
    base = (88, 72, 56)
    img = noisy(base, 71)
    frame(img, (50, 40, 32, 255))
    for (cx, cy) in [(4, 6), (11, 6)]:
        for y in range(cy - 2, cy + 3):
            for x in range(cx - 2, cx + 3):
                if (x - cx) ** 2 + (y - cy) ** 2 <= 5:
                    img.putpixel((x, y), (30, 30, 34, 255))
        img.putpixel((cx, cy - 2), (240, 240, 240, 255))
    for x in range(3, 13):
        img.putpixel((x, 11), (230, 210, 150, 255))
    img.putpixel((8, 11), (220, 50, 40, 255))
    return img


def tuner_side():
    img = noisy((88, 72, 56), 72)
    frame(img, (50, 40, 32, 255))
    return img


def tuner_top():
    img = noisy((96, 80, 62), 73)
    for x in range(4, 12):
        img.putpixel((x, 7), (184, 115, 51, 255))
        img.putpixel((x, 8), (150, 90, 40, 255))
    return img


def rf_meter():
    img = Image.new('RGBA', (16, 16), (0, 0, 0, 0))
    body, face = (190, 160, 40, 255), (235, 230, 200, 255)
    for y in range(4, 15):
        for x in range(3, 13):
            img.putpixel((x, y), body)
    for y in range(5, 10):
        for x in range(4, 12):
            img.putpixel((x, y), face)
    for i, (x, y) in enumerate([(5, 9), (6, 8), (7, 7), (8, 6), (9, 6)]):
        img.putpixel((x, y), (200, 40, 30, 255))
    for x in (5, 8, 10):
        img.putpixel((x, 12), (40, 40, 40, 255))
    for y in range(0, 4):
        img.putpixel((10, y), (170, 170, 175, 255))
    img.putpixel((10, 0), (220, 220, 225, 255))
    return img


def scrap():
    img = Image.new('RGBA', (16, 16), (0, 0, 0, 0))
    rnd = random.Random(99)
    for y in range(5, 13):
        for x in range(3, 14):
            d = ((x - 8) / 5.5) ** 2 + ((y - 9) / 3.6) ** 2
            if d <= 1 and rnd.random() > 0.08:
                c = (120, 100, 90) if rnd.random() > 0.3 else (184, 110, 60)
                if d < 0.3:
                    c = (210, 140, 80)
                img.putpixel((x, y), clamp(tuple(v * rnd.uniform(0.8, 1.1) for v in c)) + (255,))
    return img


# ------------------------------------------------------------------ models

def box(frm, to, textures):
    return {'from': frm, 'to': to, 'faces': {d: {'texture': textures.get(d, '#side'), 'cullface': None} for d in DIRS}}


def clean(el):
    for f in el['faces'].values():
        f.pop('cullface', None)
    return el


def amp_model(name, height):
    inset = {8: 2, 12: 1, 16: 0}[height]
    lo, hi = inset, 16 - inset
    elements = [clean(box([lo, 0, lo], [hi, height - 1, hi], {'north': '#front', 'up': '#top', 'down': '#side'}))]
    # heatsink fins on top
    for x in range(lo + 1, hi - 1, 3):
        elements.append(clean(box([x, height - 1, lo + 1], [x + 1, height, hi - 1], {'up': '#top'})))
    return {'parent': 'minecraft:block/block', 'textures': {
        'front': f'{NS}:block/{name}_front', 'side': f'{NS}:block/{name}_side', 'top': f'{NS}:block/{name}_top',
        'particle': f'{NS}:block/{name}_side'}, 'elements': elements}


def tuner_model():
    name = 'antenna_tuner'
    elements = [clean(box([2, 0, 3], [14, 8, 13], {'north': '#front', 'up': '#top'}))]
    for x in (4, 10):
        elements.append(clean(box([x, 5, 2], [x + 2, 7, 3], {'north': '#knob', 'up': '#knob', 'down': '#knob', 'west': '#knob', 'east': '#knob'})))
    return {'parent': 'minecraft:block/block', 'textures': {
        'front': f'{NS}:block/{name}_front', 'side': f'{NS}:block/{name}_side', 'top': f'{NS}:block/{name}_top',
        'knob': f'{NS}:block/{name}_side', 'particle': f'{NS}:block/{name}_side'}, 'elements': elements}


def facing_blockstate(name):
    rot = {'north': 0, 'east': 90, 'south': 180, 'west': 270}
    return {'variants': {f'facing={f}': ({'model': f'{NS}:block/{name}'} | ({'y': r} if r else {})) for f, r in rot.items()}}


def loot(name):
    return {'type': 'minecraft:block', 'pools': [{'rolls': 1, 'entries': [{'type': 'minecraft:item', 'name': f'{NS}:{name}'}],
                                                  'conditions': [{'condition': 'minecraft:survives_explosion'}]}]}


def shaped(result, pattern, key, count=1):
    return {'type': 'minecraft:crafting_shaped', 'category': 'redstone', 'pattern': pattern,
            'key': {k: ({'item': v[1:]} if v.startswith('!') else {'tag': v}) for k, v in key.items()},
            'result': {'id': f'{NS}:{result}', 'count': count}}


RECIPES = {
    'amplifier_100w': shaped('amplifier_100w', ['IQI', 'CRC', 'III'],
                             {'I': 'c:ingots/iron', 'Q': 'c:gems/quartz', 'C': 'c:ingots/copper', 'R': 'c:dusts/redstone'}),
    'amplifier_1kw': shaped('amplifier_1kw', ['GQG', 'AAA', 'CRC'],
                            {'G': 'c:ingots/gold', 'Q': 'c:gems/quartz', 'A': f'!{NS}:amplifier_100w', 'C': 'c:storage_blocks/copper',
                             'R': 'c:storage_blocks/redstone'}),
    'amplifier_10kw': shaped('amplifier_10kw', ['DGD', 'AAA', 'CRC'],
                             {'D': 'c:gems/diamond', 'G': 'c:storage_blocks/gold', 'A': f'!{NS}:amplifier_1kw',
                              'C': 'c:storage_blocks/copper', 'R': 'c:storage_blocks/redstone'}),
    'antenna_tuner': shaped('antenna_tuner', ['IQI', 'CRC', 'III'],
                            {'I': 'c:ingots/iron', 'Q': 'c:gems/quartz', 'C': f'!{NS}:coax_cable', 'R': 'c:dusts/redstone'}),
    'rf_meter': shaped('rf_meter', [' C ', 'GRG', 'GIG'],
                       {'C': 'c:ingots/copper', 'G': 'c:ingots/gold', 'R': 'c:dusts/redstone', 'I': 'c:ingots/iron'}),
}


def merge_tag(path, values):
    path.parent.mkdir(parents=True, exist_ok=True)
    tag = json.loads(path.read_text(encoding='utf-8')) if path.exists() else {'replace': False, 'values': []}
    for v in values:
        if v not in tag['values']:
            tag['values'].append(v)
    write(path, tag)


def main():
    for i, (name, (base, accent, leds, height)) in enumerate(AMPS.items()):
        save(amp_front(base, accent, leds, 10 + i), 'block', f'{name}_front')
        save(amp_side(base, 20 + i), 'block', f'{name}_side')
        save(amp_top(base, 30 + i), 'block', f'{name}_top')
        write(assets / 'models/block' / f'{name}.json', amp_model(name, height))
    save(tuner_front(), 'block', 'antenna_tuner_front')
    save(tuner_side(), 'block', 'antenna_tuner_side')
    save(tuner_top(), 'block', 'antenna_tuner_top')
    write(assets / 'models/block/antenna_tuner.json', tuner_model())
    blocks = list(AMPS) + ['antenna_tuner']
    for name in blocks:
        write(assets / 'blockstates' / f'{name}.json', facing_blockstate(name))
        write(assets / 'models/item' / f'{name}.json', {'parent': f'{NS}:block/{name}'})
        write(data / NS / 'loot_table/blocks' / f'{name}.json', loot(name))
    save(rf_meter(), 'item', 'rf_meter')
    save(scrap(), 'item', 'melted_scrap')
    for name in ('rf_meter', 'melted_scrap'):
        write(assets / 'models/item' / f'{name}.json', {'parent': 'minecraft:item/generated', 'textures': {'layer0': f'{NS}:item/{name}'}})
    for name, r in RECIPES.items():
        write(data / NS / 'recipe' / f'{name}.json', r)
    write(data / NS / 'recipe/melted_scrap_smelting.json', {
        'type': 'minecraft:smelting', 'category': 'misc', 'ingredient': {'item': f'{NS}:melted_scrap'},
        'result': {'id': 'minecraft:iron_nugget', 'count': 1}, 'experience': 0.1, 'cookingtime': 200})
    write(data / NS / 'damage_type/rf_burn.json', {'message_id': f'{NS}.rf_burn', 'exhaustion': 0.1,
                                                     'scaling': 'when_caused_by_living_non_player'})
    merge_tag(data / NS / 'tags/block/rf_coax_ports.json',
              [f'{NS}:{n}' for n in blocks] + [f'{NS}:sdr_basic', f'{NS}:sdr_standard', f'{NS}:sdr_advanced'])
    merge_tag(data / 'minecraft/tags/block/mineable/pickaxe.json', [f'{NS}:{n}' for n in blocks])


if __name__ == '__main__':
    main()

# Blockbench-authored models (models/*.bbmodel) replace these placeholders: re-export them last.
if __name__ == '__main__':
    import runpy as _runpy
    from pathlib import Path as _Path
    _runpy.run_path(str(_Path(__file__).with_name('export-radio-models.py')), run_name='__main__')
