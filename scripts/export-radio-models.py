"""Export the radio feature's Blockbench projects (models/<name>.bbmodel) to game assets.

Each project is authored through the Blockbench MCP with its texture embedded.
Kinds:
  facing       one model, blockstate variants for the 4 horizontal facings (model faces north)
  facing_lit   like facing; cubes named lit_* only appear in <name>_lit, used when lit=true
  connector6   cubes named center*/north*/south*/west*/east*/down*/up* -> multipart on 6 sides
  simple       one model, one variant
  item         an item model (no blockstate)
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
}

DIRECTIONS = ['north', 'south', 'west', 'east', 'down', 'up']
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


def model_ref(folder, model):
    return f'evanscomputermod:{folder}/{model}'


def export(name):
    kind, folder = PROJECTS[name]
    out = convert(name, folder)
    models = assets / 'models' / ('item' if kind == 'item' else 'block')
    if kind == 'item':
        out['parent'] = 'minecraft:item/generated' if not out['elements'] else 'minecraft:block/block'
        write(models / f'{name}.json', out)
        return
    if kind == 'facing_lit':
        write(models / f'{name}.json', {**out, 'elements': [e for e in out['elements'] if not e['name'].startswith('lit_')]})
        write(models / f'{name}_lit.json', out)
        state = {'variants': {}}
        for d, y in Y_ROT.items():
            for lit in ['false', 'true']:
                v = {'model': model_ref('block', name + ('_lit' if lit == 'true' else ''))}
                if y:
                    v['y'] = y
                state['variants'][f'facing={d},lit={lit}'] = v
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
