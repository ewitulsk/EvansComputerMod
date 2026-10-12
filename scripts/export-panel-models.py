"""Export the display panel Blockbench projects (P1, docs/gui) to game assets.

  models/terminal_panels.bbmodel  cubes named <visual>_* (phosphor, amber,
      color_lcd, hd, none) -> models/block/terminal/panel_<visual>.json, the
      overlay on a Terminal's screen face. scripts/gen-module-models.py adds
      them to the Terminal's multipart blockstate on its `panel` property.
  models/display_panels.bbmodel   cubes named <prefix>_* -> one item model per
      panel item (phosphor_panel, amber_phosphor_panel, color_lcd_panel,
      hd_panel), plus the 26.1 item definitions.
  models/phosphor_screen.bbmodel, models/hd_screen.bbmodel  facing blocks,
      exported by scripts/export-radio-models.py's 'facing' kind (the Tier 2
      Screen keeps its own project, models/screen_block.bbmodel).

The textures embedded in the projects are painted by scripts/gen-panel-textures.py.
Run: py -3 scripts/export-panel-models.py
"""
import importlib.util
import json
from pathlib import Path

root = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location('radio_export', root / 'scripts' / 'export-radio-models.py')
rx = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rx)
assets = rx.assets

VISUALS = ['phosphor', 'amber', 'color_lcd', 'hd', 'none']
ITEMS = {'phosphor': 'phosphor_panel', 'amber': 'amber_phosphor_panel', 'color_lcd': 'color_lcd_panel',
         'hd': 'hd_panel'}


def only(out, prefix):
    els = [e for e in out['elements'] if e['name'].startswith(prefix + '_')]
    used = {f['texture'][1:] for e in els for f in e['faces'].values()}
    tex = {k: v for k, v in out['textures'].items() if k in used}
    tex['particle'] = next(iter(tex.values()))
    return {**out, 'textures': tex, 'elements': els}


def item_definition(name):
    rx.write(assets / 'items' / f'{name}.json',
             {'model': {'type': 'minecraft:model', 'model': f'evanscomputermod:item/{name}'}})


def main():
    out = rx.convert('terminal_panels', 'block')
    out.pop('display', None)
    for v in VISUALS:
        m = only(out, v)
        if not m['elements']:
            raise SystemExit(f'terminal_panels has no {v}_* cubes')
        rx.write(assets / 'models' / 'block' / 'terminal' / f'panel_{v}.json', m)
        print('exported terminal/panel_' + v)

    out = rx.convert('display_panels', 'item')
    for prefix, item in ITEMS.items():
        m = only(out, prefix)
        rx.write(assets / 'models' / 'item' / f'{item}.json', m)
        item_definition(item)
        print('exported item/' + item)

    for name in ('phosphor_screen', 'hd_screen'):
        rx.PROJECTS[name] = ('facing', 'block')
        rx.export(name)
        item_definition(name)
        # Same item model the block uses, like screen_block's.
        print('exported', name)


if __name__ == '__main__':
    main()
