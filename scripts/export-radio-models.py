"""Export the radio feature's Blockbench projects (models/<name>.bbmodel) to game assets.

Each project is authored through the Blockbench MCP with its texture embedded.
Kinds:
  facing       one model, blockstate variants for the 4 horizontal facings (model faces north)
  facing_lit   like facing; cubes named lit_* only appear in <name>_lit, used when lit=true
  facing_active  like facing_lit but for an `active` property (<name>_active)
  connector6   cubes named center*/north*/south*/west*/east*/down*/up* -> multipart on 6 sides
  simple       one model, one variant
  item         an item model (no blockstate)
  parts        part models only (<name>_center, _north.., _lug_x.., _inventory); the block's own
               blockstate (cut masks etc.) is kept as is
  parts_ox     like parts, once per copper oxidation stage 0-3 (<name>_<part>_<stage>), with the
               authored texture recoloured towards exposed / weathered / oxidized copper
Run: py -3 scripts/export-radio-models.py [name ...]
"""
import base64
import json
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
assets = root / 'src/main/resources/assets/evanscomputermod'

# name: (kind, texture folder)
PROJECTS = {
    'burner_generator': ('facing_lit', 'block'),
    'sdr_basic': ('facing', 'block'),
    'sdr_standard': ('facing', 'block'),
    'sdr_advanced': ('facing', 'block'),
    'handheld_radio': ('item', 'item'),
    'controller_receiver_module': ('item', 'item'),
    'access_point': ('facing_active', 'block'),
    'copper_wire': ('parts_ox', 'block'),
    'antenna_wire': ('parts_ox', 'block'),
    'heavy_cable': ('parts_ox', 'block'),
    'antenna_rod': ('parts', 'block'),
    'lattice_mast': ('parts', 'block'),
    'insulator': ('parts', 'block'),
    'feed_point': ('parts', 'block'),
    'coax_cable': ('parts', 'block'),
    'hardline': ('parts', 'block'),
    'lightning_arrestor': ('parts', 'block'),
}

DIRECTIONS = ['north', 'south', 'west', 'east', 'down', 'up']
PART_PREFIXES = ['center', *DIRECTIONS, 'lug_x', 'lug_y', 'lug_z']
# Copper patina targets for oxidation stages 1-3 (stage 0 is the authored texture) and blend amounts.
OXIDATION = [None, ((150, 110, 80), 0.45), ((96, 160, 120), 0.6), ((84, 168, 140), 0.85)]
Y_ROT = {'north': 0, 'east': 90, 'south': 180, 'west': 270}


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + '\n')


def convert(name, folder):
    project = json.loads((root / 'models' / f'{name}.bbmodel').read_text())
    textures = project['textures']
    res = project['resolution']
    by_uuid = {str(t['uuid']): str(t.get('id', i)) for i, t in enumerate(textures)}
    out = {'parent': 'minecraft:block/block', 'credit': 'Made with Blockbench', 'textures': {}, 'elements': []}
    for i, t in enumerate(textures):
        key = str(t.get('id', i))
        filename = t['name'].removesuffix('.png')
        out['textures'][key] = f'evanscomputermod:{folder}/{filename}'
        (assets / 'textures' / folder).mkdir(parents=True, exist_ok=True)
        (assets / 'textures' / folder / f'{filename}.png').write_bytes(base64.b64decode(t['source'].split(',')[1]))
    out['textures']['particle'] = next(iter(out['textures'].values()))
    for e in project['elements']:
        if e.get('visibility') is False:
            continue
        cube = {k: e[k] for k in ['name', 'from', 'to']}
        if e.get('rotation') and any(e['rotation']):
            axis = 'xyz'[[abs(a) > 0 for a in e['rotation']].index(True)]
            cube['rotation'] = {'angle': e['rotation']['xyz'.index(axis)], 'axis': axis, 'origin': e.get('origin', [8, 8, 8])}
        cube['faces'] = {}
        for direction, face in e['faces'].items():
            tex = face.get('texture')
            if tex is None:
                continue
            key = by_uuid.get(str(tex), str(tex))
            uv = face['uv']
            f = {'uv': [round(v * 16 / res['width' if j % 2 == 0 else 'height'], 4) for j, v in enumerate(uv)], 'texture': '#' + key}
            if face.get('rotation'):
                f['rotation'] = face['rotation']
            cube['faces'][direction] = f
        out['elements'].append(cube)
    return out


def has_alpha(name, folder):
    from PIL import Image
    img = Image.open(assets / 'textures' / folder / f'{name}.png').convert('RGBA')
    return any(a < 255 for a in img.getchannel('A').getdata())


def recolour(name, folder, stage):
    from PIL import Image
    target, amount = OXIDATION[stage]
    img = Image.open(assets / 'textures' / folder / f'{name}.png').convert('RGBA')
    px = img.load()
    for y in range(img.height):
        for x in range(img.width):
            r, g, b, a = px[x, y]
            lum = (r * 0.3 + g * 0.59 + b * 0.11) / 160
            tr, tg, tb = (min(255, int(c * lum)) for c in target)
            px[x, y] = (int(r + (tr - r) * amount), int(g + (tg - g) * amount), int(b + (tb - b) * amount), a)
    img.save(assets / 'textures' / folder / f'{name}_{stage}.png')


def write_parts(name, out, suffix, texture_ref):
    models = assets / 'models' / 'block'
    tex = {'0': texture_ref, 'particle': texture_ref}
    for part in PART_PREFIXES:
        els = [e for e in out['elements'] if e['name'].startswith(part)]
        if els or part in ('center', *DIRECTIONS):
            write(models / f'{name}_{part}{suffix}.json', {**out, 'textures': tex, 'elements': els})
    inv = [e for e in out['elements'] if e['name'].startswith(('center', 'north', 'south', 'lug_z'))]
    write(models / f'{name}_inventory{suffix}.json', {**out, 'textures': tex, 'elements': inv})


def model_ref(folder, model):
    return f'evanscomputermod:{folder}/{model}'


def export(name):
    kind, folder = PROJECTS[name]
    out = convert(name, folder)
    models = assets / 'models' / ('item' if kind == 'item' else 'block')
    if kind in ('parts', 'parts_ox'):
        if has_alpha(name, folder):
            out['render_type'] = 'minecraft:cutout'
        # Elements reference texture "#0"; point every face at it.
        for e in out['elements']:
            for f in e['faces'].values():
                f['texture'] = '#0'
        if kind == 'parts':
            write_parts(name, out, '', f'evanscomputermod:{folder}/{name}')
        else:
            src = assets / 'textures' / folder / f'{name}.png'
            (assets / 'textures' / folder / f'{name}_0.png').write_bytes(src.read_bytes())
            for stage in range(4):
                if stage:
                    recolour(name, folder, stage)
                write_parts(name, out, f'_{stage}', f'evanscomputermod:{folder}/{name}_{stage}')
            src.unlink()
        return
    if kind == 'item':
        out['parent'] = 'minecraft:item/generated' if not out['elements'] else 'minecraft:block/block'
        write(models / f'{name}.json', out)
        return
    if kind in ('facing_lit', 'facing_active'):
        prop = 'lit' if kind == 'facing_lit' else 'active'
        write(models / f'{name}.json', {**out, 'elements': [e for e in out['elements'] if not e['name'].startswith('lit_')]})
        write(models / f'{name}_{prop}.json', out)
        state = {'variants': {}}
        for d, y in Y_ROT.items():
            for on in ['false', 'true']:
                v = {'model': model_ref('block', name + (f'_{prop}' if on == 'true' else ''))}
                if y:
                    v['y'] = y
                key = f'facing={d},lit={on}' if prop == 'lit' else f'active={on},facing={d}'
                state['variants'][key] = v
    elif kind == 'facing':
        write(models / f'{name}.json', out)
        state = {'variants': {f'facing={d}': ({'model': model_ref('block', name), 'y': y} if y else {'model': model_ref('block', name)})
                              for d, y in Y_ROT.items()}}
    elif kind == 'connector6':
        for part in ['center', *DIRECTIONS]:
            write(models / f'{name}_{part}.json', {**out, 'elements': [e for e in out['elements'] if e['name'].startswith(part)]})
        write(models / f'{name}.json', {**out, 'elements': [e for e in out['elements'] if e['name'].startswith(('center', 'north', 'south'))]})
        state = {'multipart': [{'apply': {'model': model_ref('block', name + '_center')}},
                               *[{'when': {d: 'true'}, 'apply': {'model': model_ref('block', f'{name}_{d}')}} for d in DIRECTIONS]]}
    else:
        write(models / f'{name}.json', out)
        state = {'variants': {'': {'model': model_ref('block', name)}}}
    write(assets / 'blockstates' / f'{name}.json', state)
    write(assets / 'models' / 'item' / f'{name}.json', {'parent': model_ref('block', name)})


if __name__ == '__main__':
    for n in (sys.argv[1:] or PROJECTS):
        export(n)
        print('exported', n)
