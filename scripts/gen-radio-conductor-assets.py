"""Generate placeholder assets and data for the radio conductor blocks (Phase 4 lanes 4A/4B/4E).

Writes, for every conductor block, procedurally drawn 16x16 textures (no vanilla texture references),
connector6 multipart blockstates (a centre cube plus one arm per connected side at the tier's
thickness), block/item models, loot tables, recipes (c: tags), tags and the rf_conductor data map.
A later Blockbench modelling pass replaces the models and textures; the IDs stay.

Run: py -3 scripts/gen-radio-conductor-assets.py
"""
import json
import random
from pathlib import Path

from PIL import Image

root = Path(__file__).resolve().parent.parent
assets = root / 'src/main/resources/assets/evanscomputermod'
data = root / 'src/main/resources-mc1.21.1/data'
NS = 'evanscomputermod'
DIRS = ['north', 'south', 'west', 'east', 'down', 'up']


def write(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(obj, indent=2, ensure_ascii=False) + '\n', encoding='utf-8')


# ------------------------------------------------------------------ textures

def clamp(c):
    return tuple(max(0, min(255, int(v))) for v in c)


def shade(c, f):
    return clamp((c[0] * f, c[1] * f, c[2] * f) + ((c[3],) if len(c) > 3 else ()))


def noisy(base, seed, amount=0.12, stripes=None, alpha=255):
    rnd = random.Random(seed)
    img = Image.new('RGBA', (16, 16))
    for y in range(16):
        for x in range(16):
            f = 1 + rnd.uniform(-amount, amount)
            if stripes == 'twist' and (x + y) % 4 == 0:
                f *= 0.78
            if stripes == 'braid' and ((x + y) % 4 == 0 or (x - y) % 4 == 0):
                f *= 0.8
            if stripes == 'ridges' and y % 4 in (0, 1):
                f *= 0.82
            if stripes == 'corrugated' and x % 3 == 0:
                f *= 0.75
            if stripes == 'brushed' and rnd.random() < 0.15:
                f *= 1.15
            img.putpixel((x, y), clamp(tuple(v * f for v in base)) + (alpha,))
    return img


COPPER_STAGES = [(205, 112, 76), (170, 120, 95), (96, 158, 120), (76, 170, 145)]


def texture(name, img):
    path = assets / 'textures/block' / f'{name}.png'
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)


def lattice():
    img = Image.new('RGBA', (16, 16), (0, 0, 0, 0))
    steel = (150, 156, 162)
    for i in range(16):
        for (x, y) in [(0, i), (1, i), (15, i), (14, i), (i, 0), (i, 15), (i, i), (15 - i, i)]:
            f = 0.85 if (x + y) % 3 == 0 else 1.0
            img.putpixel((x, y), clamp(tuple(v * f for v in steel)) + (255,))
    return img


def feed_body():
    img = noisy((58, 60, 66), 41, 0.08)
    for x in range(16):
        img.putpixel((x, 7), (210, 160, 60, 255))
        img.putpixel((x, 8), (180, 135, 50, 255))
    return img


def arrestor():
    img = noisy((120, 124, 130), 52, 0.06)
    bolt = [(9, 2), (8, 3), (8, 4), (7, 5), (7, 6), (6, 7), (7, 7), (8, 7), (9, 7), (9, 8), (8, 9), (8, 10), (7, 11), (7, 12), (6, 13)]
    for (x, y) in bolt:
        img.putpixel((x, y), (250, 205, 40, 255))
    return img


def item_icon(name, draw):
    img = Image.new('RGBA', (16, 16), (0, 0, 0, 0))
    draw(img)
    path = assets / 'textures/item' / f'{name}.png'
    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path)


def draw_wrench(img):
    steel, dark = (176, 182, 190, 255), (110, 116, 124, 255)
    for i in range(3, 12):
        img.putpixel((i, 15 - i), steel)
        img.putpixel((i + 1, 15 - i), dark)
    for (x, y) in [(11, 1), (12, 1), (13, 2), (14, 3), (14, 4), (13, 5), (10, 2), (10, 3), (11, 5), (12, 5)]:
        img.putpixel((x, y), steel)
    for (x, y) in [(2, 12), (1, 13), (2, 14), (3, 14), (4, 13), (1, 12)]:
        img.putpixel((x, y), (200, 90, 50, 255))


def draw_analyzer(img):
    body, screen, trace = (60, 64, 72, 255), (20, 40, 30, 255), (90, 230, 120, 255)
    for y in range(2, 15):
        for x in range(3, 13):
            img.putpixel((x, y), body)
    for y in range(3, 9):
        for x in range(4, 12):
            img.putpixel((x, y), screen)
    curve = [8, 7, 6, 5, 4, 4, 5, 6]   # an SWR dip
    for i, y in enumerate(curve):
        img.putpixel((4 + i, y), trace)
    for (x, y) in [(5, 11), (7, 11), (9, 11), (5, 13), (7, 13), (9, 13)]:
        img.putpixel((x, y), (200, 200, 205, 255))
    for y in range(0, 3):
        img.putpixel((11, y), (170, 170, 175, 255))


# ------------------------------------------------------------------ models

def cube(frm, to, tex):
    return {'from': frm, 'to': to, 'faces': {d: {'texture': tex} for d in DIRS}}


def arm_box(direction, t):
    lo, hi = 8 - t / 2, 8 + t / 2
    return {
        'north': ([lo, lo, 0], [hi, hi, lo]),
        'south': ([lo, lo, hi], [hi, hi, 16]),
        'west': ([0, lo, lo], [lo, hi, hi]),
        'east': ([hi, lo, lo], [16, hi, hi]),
        'down': ([lo, 0, lo], [hi, lo, hi]),
        'up': ([lo, hi, lo], [hi, 16, hi]),
    }[direction]


def model(name, elements, tex, cutout=False):
    m = {'parent': 'minecraft:block/block', 'textures': {'t': f'{NS}:block/{tex}', 'particle': f'{NS}:block/{tex}'},
         'elements': elements}
    if cutout:
        m['render_type'] = 'minecraft:cutout'
    write(assets / 'models/block' / f'{name}.json', m)


def connector(name, t, tex, centre_t=None, cutout=False, suffix=''):
    """Centre + six arm models; returns their model ids."""
    ct = centre_t or t
    lo, hi = 8 - ct / 2, 8 + ct / 2
    model(f'{name}_center{suffix}', [cube([lo, lo, lo], [hi, hi, hi], '#t')], tex, cutout)
    for d in DIRS:
        frm, to = arm_box(d, t)
        model(f'{name}_{d}{suffix}', [cube(frm, to, '#t')], tex, cutout)
    # Inventory model: centre with east/west arms.
    els = [cube([lo, lo, lo], [hi, hi, hi], '#t')] + [cube(*arm_box(d, t), '#t') for d in ('east', 'west')]
    model(f'{name}_inventory{suffix}', els, tex, cutout)


def mid(name):
    return f'{NS}:block/{name}'


def connector_states(name, extra_when=None, suffix=''):
    parts = [{'apply': {'model': mid(f'{name}_center{suffix}')}}]
    for d in DIRS:
        parts.append({'when': {d: 'true'}, 'apply': {'model': mid(f'{name}_{d}{suffix}')}})
    if extra_when:
        for p in parts:
            p['when'] = {**extra_when, **p.get('when', {})}
    return parts


def item_model(name, parent):
    write(assets / 'models/item' / f'{name}.json', {'parent': parent})


# Tier table: name -> (arm px, centre px, texture kind)
COPPER_TIERS = {'copper_wire': (2, 2, 'twist'), 'antenna_wire': (3, 3, 'twist'), 'heavy_cable': (5, 5, 'braid')}


def gen_assets():
    # Copper tiers with four oxidation stages.
    for name, (t, ct, stripes) in COPPER_TIERS.items():
        parts = []
        for s, colour in enumerate(COPPER_STAGES):
            tex = f'{name}_{s}'
            texture(tex, noisy(colour, sum(map(ord, tex)), 0.1, stripes))
            connector(name, t, tex, ct, suffix=f'_{s}')
            parts += connector_states(name, {'oxidation': str(s)}, suffix=f'_{s}')
        write(assets / 'blockstates' / f'{name}.json', {'multipart': parts})
        item_model(name, mid(f'{name}_inventory_0'))

    texture('antenna_rod', noisy((178, 184, 192), 7, 0.06, 'brushed'))
    connector('antenna_rod', 7, 'antenna_rod')
    write(assets / 'blockstates/antenna_rod.json', {'multipart': connector_states('antenna_rod')})
    item_model('antenna_rod', mid('antenna_rod_inventory'))

    texture('lattice_mast', lattice())
    connector('lattice_mast', 15, 'lattice_mast', cutout=True)
    write(assets / 'blockstates/lattice_mast.json', {'multipart': connector_states('lattice_mast')})
    item_model('lattice_mast', mid('lattice_mast_inventory'))

    texture('insulator', noisy((232, 226, 210), 11, 0.05, 'ridges'))
    connector('insulator', 3, 'insulator', centre_t=6)
    write(assets / 'blockstates/insulator.json', {'multipart': connector_states('insulator')})
    item_model('insulator', mid('insulator_inventory'))

    texture('coax_cable', noisy((34, 34, 38), 21, 0.08, 'twist'))
    connector('coax_cable', 4, 'coax_cable')
    write(assets / 'blockstates/coax_cable.json', {'multipart': connector_states('coax_cable')})
    item_model('coax_cable', mid('coax_cable_inventory'))

    texture('hardline', noisy((196, 120, 80), 31, 0.06, 'corrugated'))
    connector('hardline', 6, 'hardline')
    write(assets / 'blockstates/hardline.json', {'multipart': connector_states('hardline')})
    item_model('hardline', mid('hardline_inventory'))

    texture('lightning_arrestor', arrestor())
    texture('lightning_arrestor_arm', noisy((34, 34, 38), 22, 0.08, 'twist'))
    connector('lightning_arrestor', 4, 'lightning_arrestor_arm', centre_t=7)
    model('lightning_arrestor_center', [cube([4.5, 4.5, 4.5], [11.5, 11.5, 11.5], '#t')], 'lightning_arrestor')
    write(assets / 'blockstates/lightning_arrestor.json', {'multipart': connector_states('lightning_arrestor')})
    model('lightning_arrestor_inventory', [cube([4.5, 4.5, 4.5], [11.5, 11.5, 11.5], '#t')], 'lightning_arrestor')
    item_model('lightning_arrestor', mid('lightning_arrestor_inventory'))

    # Feed point: body + lugs along its axis (always shown) + coax arms on the other connected sides.
    texture('feed_point', feed_body())
    texture('feed_point_lug', noisy((210, 160, 60), 43, 0.08))
    texture('feed_point_coax', noisy((34, 34, 38), 44, 0.08, 'twist'))
    model('feed_point_center', [cube([4, 4, 4], [12, 12, 12], '#t')], 'feed_point')
    lugs = {'x': ([0, 6.5, 6.5], [16, 9.5, 9.5]), 'y': ([6.5, 0, 6.5], [9.5, 16, 9.5]), 'z': ([6.5, 6.5, 0], [9.5, 9.5, 16])}
    for axis, (frm, to) in lugs.items():
        model(f'feed_point_lug_{axis}', [cube(frm, to, '#t')], 'feed_point_lug')
    for d in DIRS:
        model(f'feed_point_{d}', [cube(*arm_box(d, 4), '#t')], 'feed_point_coax')
    parts = [{'apply': {'model': mid('feed_point_center')}}]
    for axis in 'xyz':
        parts.append({'when': {'axis': axis}, 'apply': {'model': mid(f'feed_point_lug_{axis}')}})
    axis_of = {'north': 'z', 'south': 'z', 'west': 'x', 'east': 'x', 'down': 'y', 'up': 'y'}
    for d in DIRS:
        others = '|'.join(a for a in 'xyz' if a != axis_of[d])
        parts.append({'when': {d: 'true', 'axis': others}, 'apply': {'model': mid(f'feed_point_{d}')}})
    write(assets / 'blockstates/feed_point.json', {'multipart': parts})
    write(assets / 'models/block/feed_point_inventory.json', {
        'parent': 'minecraft:block/block',
        'textures': {'b': f'{NS}:block/feed_point', 'l': f'{NS}:block/feed_point_lug', 'particle': f'{NS}:block/feed_point'},
        'elements': [cube([4, 4, 4], [12, 12, 12], '#b'), cube(*lugs['x'], '#l')]})
    item_model('feed_point', mid('feed_point_inventory'))

    item_icon('rf_wrench', draw_wrench)
    item_icon('antenna_analyzer', draw_analyzer)
    for name in ('rf_wrench', 'antenna_analyzer'):
        write(assets / 'models/item' / f'{name}.json', {'parent': 'minecraft:item/handheld' if name == 'rf_wrench' else 'minecraft:item/generated',
                                                       'textures': {'layer0': f'{NS}:item/{name}'}})

    # Blockbench-authored models (models/*.bbmodel) replace these placeholders: re-export them last.
    import runpy
    runpy.run_path(str(Path(__file__).with_name('export-radio-models.py')), run_name='__main__')


# ------------------------------------------------------------------ data

BLOCKS = ['copper_wire', 'antenna_wire', 'heavy_cable', 'antenna_rod', 'lattice_mast', 'insulator', 'feed_point',
          'coax_cable', 'hardline', 'lightning_arrestor']


def tag(t):
    return {'tag': t}


def item(i):
    return {'item': i}


def shaped(name, pattern, key, count=1, category='redstone'):
    write(data / NS / 'recipe' / f'{name}.json', {
        'type': 'minecraft:crafting_shaped', 'category': category, 'pattern': pattern, 'key': key,
        'result': {'id': f'{NS}:{name}', 'count': count}})


def gen_data():
    for b in BLOCKS:
        write(data / NS / 'loot_table/blocks' / f'{b}.json', {
            'type': 'minecraft:block',
            'pools': [{'rolls': 1, 'bonus_rolls': 0, 'entries': [{'type': 'minecraft:item', 'name': f'{NS}:{b}'}],
                       'conditions': [{'condition': 'minecraft:survives_explosion'}]}]})

    cu, fe = tag('c:ingots/copper'), tag('c:ingots/iron')
    shaped('copper_wire', ['CCC'], {'C': cu}, 16)
    shaped('antenna_wire', ['WWW', 'WNW', 'WWW'], {'W': item(f'{NS}:copper_wire'), 'N': tag('c:nuggets/iron')}, 8)
    shaped('heavy_cable', ['CCC', 'NNN'], {'C': cu, 'N': tag('c:nuggets/iron')}, 4)
    shaped('antenna_rod', ['I', 'I', 'I'], {'I': fe}, 4)
    shaped('lattice_mast', ['I I', ' I ', 'I I'], {'I': fe}, 4)
    shaped('insulator', [' N ', 'TTT'], {'N': tag('c:nuggets/iron'), 'T': item('minecraft:white_terracotta')}, 4)
    shaped('feed_point', ['WIW', ' X '], {'W': item(f'{NS}:copper_wire'), 'I': item(f'{NS}:insulator'),
                                         'X': item(f'{NS}:coax_cable')}, 1)
    shaped('coax_cable', ['KCK'], {'K': tag('minecraft:wool'), 'C': cu}, 6)
    shaped('hardline', ['CCC', 'C C', 'CCC'], {'C': cu}, 4)
    shaped('lightning_arrestor', [' R ', 'XIX'], {'R': item('minecraft:lightning_rod'), 'X': item(f'{NS}:coax_cable'),
                                                  'I': fe}, 1)
    shaped('rf_wrench', ['I I', ' C ', ' I '], {'I': fe, 'C': cu}, 1, 'equipment')
    shaped('antenna_analyzer', ['GQG', 'CRC', 'III'], {'G': tag('c:glass_panes'), 'Q': tag('c:gems/quartz'), 'C': cu,
                                                      'R': tag('c:dusts/redstone'), 'I': fe}, 1, 'equipment')

    def opt(t):
        return {'id': t, 'required': False}

    write(data / NS / 'tags/block/rf_conductors.json', {'replace': False, 'values': [
        'minecraft:lightning_rod', 'minecraft:iron_bars', 'minecraft:chain', 'minecraft:iron_block', 'minecraft:copper_block',
        'minecraft:gold_block', 'minecraft:iron_trapdoor', 'minecraft:iron_door', 'minecraft:cut_copper',
        'minecraft:waxed_copper_block', opt('#c:storage_blocks/iron'), opt('#c:storage_blocks/copper'),
        opt('#c:storage_blocks/gold'), opt('#c:storage_blocks/steel'), opt('#c:storage_blocks/aluminum')]})
    write(data / NS / 'tags/block/rf_insulators.json', {'replace': False, 'values': [f'{NS}:insulator', f'{NS}:feed_point']})
    write(data / NS / 'tags/block/rf_good_ground.json', {'replace': False, 'values': [
        'minecraft:mud', 'minecraft:clay', 'minecraft:iron_block', 'minecraft:copper_block', 'minecraft:waxed_copper_block',
        'minecraft:iron_trapdoor', opt('#c:storage_blocks/iron'), opt('#c:storage_blocks/copper'),
        opt('#c:storage_blocks/steel')]})
    write(data / NS / 'tags/block/rf_coax_ports.json', {'replace': False, 'values': []})
    write(data / NS / 'tags/item/rf_wrenches.json', {'replace': False, 'values': [f'{NS}:rf_wrench']})
    write(data / 'minecraft/tags/block/climbable.json', {'replace': False, 'values': [f'{NS}:lattice_mast']})
    pick = data / 'minecraft/tags/block/mineable/pickaxe.json'
    cur = json.loads(pick.read_text(encoding='utf-8'))
    for b in BLOCKS:
        if f'{NS}:{b}' not in cur['values']:
            cur['values'].append(f'{NS}:{b}')
    write(pick, cur)

    # Data map: same numbers as RfDefaults (the Java fallback).
    def cond(name, r_mm, rho, amps, corona_kv, ox=False, **extra):
        e = {'name': name, 'radius_mm': r_mm, 'resistivity': rho, 'current_rating_a': amps, 'corona_kv': corona_kv}
        if ox:
            e['oxidizes'] = True
        e.update(extra)
        return e

    CU, AL, FE = 1.68e-8, 2.65e-8, 9.7e-8
    write(data / NS / 'data_maps/block/rf_conductor.json', {'values': {
        f'{NS}:copper_wire': cond('copper wire', 1.0, CU, 0.85, 1.5, True),
        f'{NS}:antenna_wire': cond('antenna wire', 1.6, CU, 1.65, 2.5, True),
        f'{NS}:heavy_cable': cond('heavy cable', 5.0, CU, 5.2, 5.0, True),
        f'{NS}:antenna_rod': cond('antenna rod', 12.5, AL, 11.7, 10.0),
        f'{NS}:lattice_mast': cond('lattice mast', 200, FE, 26, 40.0),
        f'{NS}:feed_point': cond('feed point', 3.0, CU, 30, 3.0, voltage_rating_kv=3.0),
        f'{NS}:insulator': {'name': 'insulator', 'voltage_rating_kv': 4.0},
        f'{NS}:coax_cable': {'name': 'coax', 'coax_loss_10mhz_db': 0.49, 'coax_loss_1ghz_db': 6.6, 'max_power_w': 600},
        f'{NS}:hardline': {'name': 'hardline', 'coax_loss_10mhz_db': 0.02, 'coax_loss_1ghz_db': 0.23, 'max_power_w': 20000},
        f'{NS}:lightning_arrestor': {'name': 'lightning arrestor', 'coax_loss_10mhz_db': 0.05, 'coax_loss_1ghz_db': 1.0,
                                     'max_power_w': 5000},
        'minecraft:lightning_rod': cond('lightning rod', 4.0, CU, 8, 4.0),
        'minecraft:iron_bars': cond('iron bars', 30, FE, 20, 8.0),
        'minecraft:chain': cond('chain', 10, FE, 10, 5.0),
    }})


if __name__ == '__main__':
    gen_assets()
    gen_data()
    print('radio conductor assets and data written')
