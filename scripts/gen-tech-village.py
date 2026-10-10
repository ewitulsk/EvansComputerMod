#!/usr/bin/env python3
"""Reproducible Tech Village ISP buildings, one per vanilla village style.

Each template is the start piece of a jigsaw structure that grows a normal vanilla
village: its four street connectors use vanilla's `minecraft:street` name/target and
the style's `minecraft:village/<style>/streets` pool, and its `minecraft:bottom`
jigsaws spawn the style's vanilla villagers.

Layout (unrotated, front = south):
  * 21 x 25 x 21 template; the lattice mast stands on the exact centre column
    (10, *, 10), so a structure rotation never moves it. The start jigsaw
    `evanscomputermod:mast_anchor` sits at the mast base: JigsawStructure puts that
    block on the ring site's (x, z), and the fiber endpoint is known before the
    chunk generates (WorldNetwork.plan).
  * Fiber Patch Panel on the mast top; the long-distance fiber leaves above it.
  * ISP router (role isp.router) with an Interface Block on its west side (9 NICs),
    village server (isp.server) two blocks east; a patch cable over their tops joins
    the router's UP face (eth1, server LAN) to the server's UP face (eth1). The
    router's DOWN face (eth0, village access LAN) is cabled under the floor by the
    structure's network piece. No other cable touches either terminal.
No world UUIDs are baked into NBT: the provisioning processor derives identities.
"""
import gzip, json, struct
from pathlib import Path
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'src/main/resources-mc1.21.1/data/evanscomputermod'

def string(s):
    b = s.encode(); return struct.pack('>H', len(b)) + b
def tag(t, n, b): return bytes([t]) + string(n) + b
def text(n, s): return tag(8, n, string(s))
def integer(n, v): return tag(3, n, struct.pack('>i', v))
def byte(n, v): return tag(1, n, struct.pack('>b', v))
def compound(*parts): return b''.join(parts) + b'\0'
def obj(n, *parts): return tag(10, n, compound(*parts))
def array(n, t, items): return tag(9, n, bytes([t]) + struct.pack('>i', len(items)) + b''.join(items))
def ints(n, values): return array(n, 3, [struct.pack('>i', v) for v in values])
def doubles(n, values): return array(n, 6, [struct.pack('>d', v) for v in values])
def strings(n, values): return array(n, 8, [string(v) for v in values])

def write_json(path, data):
    p = OUT / path; p.parent.mkdir(parents=True, exist_ok=True); p.write_text(json.dumps(data, indent=2) + '\n')

SIZE = (21, 25, 21)
C = 10                       # centre column
LO, HI = 5, 15               # building walls
MAST_TOP = 22                # last lattice block; patch panel at MAST_TOP + 1
ROUTER, IFACE, SERVER = (8, 1, 6), (7, 1, 6), (10, 1, 6)

STYLES = {
    'plains': dict(found='minecraft:cobblestone', floor='minecraft:oak_planks', wall='minecraft:oak_planks',
                   pillar='minecraft:oak_log', roof='minecraft:spruce_stairs', roof_block='minecraft:spruce_planks',
                   trim='minecraft:stripped_oak_log', window='minecraft:glass_pane', door='minecraft:oak_door',
                   path='minecraft:dirt_path', fence='minecraft:oak_fence', sign='minecraft:oak_wall_sign',
                   carpet='minecraft:light_blue_carpet', flat=False),
    'desert': dict(found='minecraft:cut_sandstone', floor='minecraft:smooth_sandstone', wall='minecraft:smooth_sandstone',
                   pillar='minecraft:cut_sandstone', roof=None, roof_block='minecraft:smooth_sandstone',
                   trim='minecraft:orange_terracotta', window='minecraft:glass_pane', door='minecraft:jungle_door',
                   path='minecraft:smooth_sandstone', fence='minecraft:sandstone_wall', sign='minecraft:jungle_wall_sign',
                   carpet='minecraft:cyan_carpet', flat=True),
    'savanna': dict(found='minecraft:orange_terracotta', floor='minecraft:acacia_planks', wall='minecraft:acacia_planks',
                    pillar='minecraft:acacia_log', roof='minecraft:acacia_stairs', roof_block='minecraft:acacia_planks',
                    trim='minecraft:yellow_terracotta', window='minecraft:glass_pane', door='minecraft:acacia_door',
                    path='minecraft:dirt_path', fence='minecraft:acacia_fence', sign='minecraft:acacia_wall_sign',
                    carpet='minecraft:orange_carpet', flat=False),
    'snowy': dict(found='minecraft:stone_bricks', floor='minecraft:spruce_planks', wall='minecraft:white_terracotta',
                  pillar='minecraft:stripped_spruce_log', roof='minecraft:dark_oak_stairs', roof_block='minecraft:dark_oak_planks',
                  trim='minecraft:spruce_planks', window='minecraft:glass_pane', door='minecraft:spruce_door',
                  path='minecraft:dirt_path', fence='minecraft:spruce_fence', sign='minecraft:spruce_wall_sign',
                  carpet='minecraft:red_carpet', flat=False, snow=True),
    'taiga': dict(found='minecraft:mossy_cobblestone', floor='minecraft:spruce_planks', wall='minecraft:spruce_planks',
                  pillar='minecraft:spruce_log', roof='minecraft:spruce_stairs', roof_block='minecraft:spruce_planks',
                  trim='minecraft:cobblestone', window='minecraft:glass_pane', door='minecraft:spruce_door',
                  path='minecraft:dirt_path', fence='minecraft:spruce_fence', sign='minecraft:spruce_wall_sign',
                  carpet='minecraft:green_carpet', flat=False),
}

class Template:
    def __init__(self, size): self.size = size; self.palette = []; self.blocks = {}; self.entities = []
    def put(self, x, y, z, name, props=None, nbt=None):
        state = (name, tuple(sorted((props or {}).items())))
        if state not in self.palette: self.palette.append(state)
        self.blocks[x, y, z] = (self.palette.index(state), nbt)
    def get(self, x, y, z):
        v = self.blocks.get((x, y, z)); return self.palette[v[0]][0] if v else None
    def jigsaw(self, x, y, z, orientation, name, target, pool, final, joint='aligned'):
        self.put(x, y, z, 'minecraft:jigsaw', {'orientation': orientation},
                 compound(text('id', 'minecraft:jigsaw'), text('name', name), text('target', target), text('pool', pool),
                          text('joint', joint), text('final_state', final), integer('selection_priority', 0),
                          integer('placement_priority', 0)))
    def terminal(self, pos, role, facing='south'):
        self.put(*pos, 'evanscomputermod:terminal_block', {'facing': facing},
                 compound(text('id', 'evanscomputermod:terminal_block'), text('ecmRole', role)))
    def write(self, name):
        palette = [compound(text('Name', n), obj('Properties', *[text(k, v) for k, v in p])) if p else compound(text('Name', n))
                   for n, p in self.palette]
        blocks = [compound(ints('pos', pos), integer('state', s), tag(10, 'nbt', nbt) if nbt else b'')
                  for pos, (s, nbt) in sorted(self.blocks.items())]
        data = tag(10, '', compound(integer('DataVersion', 3955), ints('size', self.size), array('palette', 10, palette),
                                    array('blocks', 10, blocks), array('entities', 10, self.entities)))
        p = OUT / 'structure' / f'tech_village/{name}.nbt'; p.parent.mkdir(parents=True, exist_ok=True)
        with p.open('wb') as f:
            with gzip.GzipFile(fileobj=f, mode='wb', mtime=0) as gz: gz.write(data)

def axis_y(name): return {'axis': 'y'} if name.endswith('_log') else None

def build(style, s):
    t = Template(SIZE)
    air = 'minecraft:air'
    # ---- ground: foundation ring + floor, street paths, start anchor under the mast
    for x in range(LO, HI + 1):
        for z in range(LO, HI + 1):
            edge = x in (LO, HI) or z in (LO, HI)
            t.put(x, 0, z, s['found'] if edge else s['floor'])
    for i in range(0, LO):
        for w in (C - 1, C, C + 1):
            for x, z in ((i, w), (SIZE[0] - 1 - i, w), (w, i), (w, SIZE[2] - 1 - i)):
                t.put(x, 0, z, s['path'])
    t.jigsaw(C, 0, C, 'up_north', 'evanscomputermod:mast_anchor', 'minecraft:empty', 'minecraft:empty', s['found'], 'rollable')
    pool = f'minecraft:village/{style}/streets'
    for x, z, o in ((0, C, 'west_up'), (SIZE[0] - 1, C, 'east_up'), (C, 0, 'north_up'), (C, SIZE[2] - 1, 'south_up')):
        t.jigsaw(x, 1, z, o, 'minecraft:street', 'minecraft:street', pool, 'minecraft:structure_void')
    # ---- walls with timber/stone pillars, a trim band and windows
    for y in range(1, 5):
        for x in range(LO, HI + 1):
            for z in range(LO, HI + 1):
                if not (x in (LO, HI) or z in (LO, HI)):
                    t.put(x, y, z, air); continue
                corner = x in (LO, HI) and z in (LO, HI)
                mid = (x == C and z in (LO, HI)) or (z == C and x in (LO, HI))
                if corner or (mid and not (x == C and z == HI)):
                    t.put(x, y, z, s['pillar'], axis_y(s['pillar'])); continue
                if y == 4: t.put(x, y, z, s['trim'], axis_x_or_z(s['trim'], x, z)); continue
                along_x = z in (LO, HI)
                k = x if along_x else z
                if y in (2, 3) and k in (7, 8, 12, 13):
                    props = {'east': 'true', 'west': 'true', 'north': 'false', 'south': 'false'} if along_x else \
                            {'north': 'true', 'south': 'true', 'east': 'false', 'west': 'false'}
                    props['waterlogged'] = 'false'
                    if k in (7, 12): props['west' if along_x else 'north'] = 'false'
                    if k in (8, 13): props['east' if along_x else 'south'] = 'false'
                    t.put(x, y, z, s['window'], props)
                else:
                    t.put(x, y, z, s['wall'])
    # front door (south wall centre)
    for y, half in ((1, 'lower'), (2, 'upper')):
        t.put(C, y, HI, s['door'], {'facing': 'north', 'half': half, 'hinge': 'left', 'open': 'false', 'powered': 'false'})
    t.put(C, 3, HI, s['wall'])
    # ---- roof
    if s['flat']:
        for x in range(LO, HI + 1):
            for z in range(LO, HI + 1):
                t.put(x, 5, z, s['roof_block'])
        for x in range(LO, HI + 1):
            for z in range(LO, HI + 1):
                if x in (LO, HI) or z in (LO, HI):
                    t.put(x, 6, z, s['trim'] if (x + z) % 2 == 0 else s['found'])
        # stepped desert cupola around the mast
        for x in range(C - 1, C + 2):
            for z in range(C - 1, C + 2):
                t.put(x, 6, z, s['pillar'])
        t.put(C, 7, C, s['pillar'])
        roof_top = 7
    else:
        for k in range(0, 6):
            y = 5 + k; a, b = LO - 1 + k, HI + 1 - k
            for x in range(a, b + 1):
                for z in range(a, b + 1):
                    if not (x in (a, b) or z in (a, b)): continue
                    if x in (a, b) and z in (a, b):
                        t.put(x, y, z, s['roof_block']); continue
                    facing = 'south' if z == a else 'north' if z == b else 'east' if x == a else 'west'
                    t.put(x, y, z, s['roof'], {'facing': facing, 'half': 'bottom', 'shape': 'straight', 'waterlogged': 'false'})
        roof_top = 10
    # ---- lattice mast through the roof apex, patch panel on top
    for y in range(1, MAST_TOP + 1):
        t.put(C, y, C, 'evanscomputermod:lattice_mast',
              {'up': 'true', 'down': 'true' if y > 1 else 'false', 'north': 'false', 'south': 'false',
               'east': 'false', 'west': 'false', 'waterlogged': 'false'})
    t.put(C, MAST_TOP + 1, C, 'evanscomputermod:fiber_patch_panel',
          {'facing': 'south', 'north': 'false', 'south': 'false', 'east': 'false', 'west': 'false', 'up': 'false', 'down': 'false'})
    # aviation-style collar lights on the mast
    for y in (roof_top + 4, MAST_TOP - 2):
        for x, z in ((C - 1, C), (C + 1, C), (C, C - 1), (C, C + 1)):
            t.put(x, y, z, 'minecraft:red_stained_glass_pane' if y == MAST_TOP - 2 else 'minecraft:iron_bars',
                  pane_props(x, z))
    # ---- equipment: router + interface block, patch cable over the tops, server
    t.terminal(ROUTER, 'isp.router'); t.put(*IFACE, 'evanscomputermod:interface_block')
    t.terminal(SERVER, 'isp.server')
    def cable(pos, dirs):
        t.put(*pos, 'evanscomputermod:network_cable', {d: 'true' if d in dirs else 'false'
                                                       for d in ('north', 'south', 'east', 'west', 'up', 'down')})
    cable((ROUTER[0], 2, ROUTER[2]), {'east', 'down'})
    cable((ROUTER[0] + 1, 2, ROUTER[2]), {'east', 'west'})
    cable((SERVER[0], 2, SERVER[2]), {'west', 'down'})
    t.put(ROUTER[0] + 1, 1, ROUTER[2], 'minecraft:air')
    # ---- interior furnishing and workstations
    t.put(12, 1, 6, 'minecraft:barrel', {'facing': 'up', 'open': 'false'})
    t.put(13, 1, 6, 'minecraft:lectern', {'facing': 'south', 'has_book': 'false', 'powered': 'false'})
    t.put(13, 1, 13, 'minecraft:cartography_table')
    t.put(7, 1, 13, 'minecraft:bookshelf'); t.put(7, 2, 13, 'minecraft:lantern', {'hanging': 'false', 'waterlogged': 'false'})
    t.put(6, 1, 6, 'minecraft:lantern', {'hanging': 'false', 'waterlogged': 'false'})
    t.put(14, 1, 9, 'minecraft:lantern', {'hanging': 'false', 'waterlogged': 'false'})
    for x in range(9, 12):
        for z in range(11, 14):
            t.put(x, 1, z, s['carpet'])
    # villagers from the style's vanilla pool
    for x, z in ((12, 9), (8, 11)):
        t.jigsaw(x, 0, z, 'up_north', 'minecraft:bottom', 'minecraft:bottom', f'minecraft:village/{style}/villagers',
                 s['floor'], 'rollable')
    # ---- outside: sign over the door, lamp posts by the path
    t.put(C, 3, HI + 1, s['sign'], {'facing': 'south', 'waterlogged': 'false'},
          compound(text('id', 'minecraft:sign'), text('ecmSign', 'isp')))
    for x in (C - 2, C + 2):
        t.put(x, 1, HI + 2, s['fence'], fence_props(s['fence']))
        t.put(x, 2, HI + 2, 'minecraft:lantern', {'hanging': 'false', 'waterlogged': 'false'})
    t.write(f'isp_{style}')

def axis_x_or_z(name, x, z):
    if not name.endswith('_log'): return None
    return {'axis': 'x' if z in (LO, HI) else 'z'}

def pane_props(x, z):
    p = {'north': 'false', 'south': 'false', 'east': 'false', 'west': 'false', 'waterlogged': 'false'}
    if x < C: p['east'] = 'true'
    if x > C: p['west'] = 'true'
    if z < C: p['south'] = 'true'
    if z > C: p['north'] = 'true'
    return p

def fence_props(name):
    p = {'north': 'false', 'south': 'false', 'east': 'false', 'west': 'false', 'waterlogged': 'false'}
    if name.endswith('_wall'):
        p = {'north': 'none', 'south': 'none', 'east': 'none', 'west': 'none', 'up': 'true', 'waterlogged': 'false'}
    return p

for style, palette in STYLES.items():
    build(style, palette)
    write_json(Path('worldgen/template_pool') / f'tech_village/isp_{style}.json', {
        'name': f'evanscomputermod:tech_village/isp_{style}', 'fallback': 'minecraft:empty',
        'elements': [{'weight': 1, 'element': {'element_type': 'minecraft:single_pool_element',
                                               'location': f'evanscomputermod:tech_village/isp_{style}',
                                               'processors': 'evanscomputermod:tech_village', 'projection': 'rigid'}}]})
    write_json(Path('worldgen/structure') / f'tech_village_{style}.json', {
        'type': 'evanscomputermod:tech_village', 'style': style, 'biomes': '#minecraft:is_overworld',
        'step': 'surface_structures', 'spawn_overrides': {}, 'terrain_adaptation': 'beard_thin',
        'start_pool': f'evanscomputermod:tech_village/isp_{style}', 'start_jigsaw_name': 'evanscomputermod:mast_anchor',
        'size': 6, 'max_distance_from_center': 80})
write_json(Path('worldgen/processor_list/tech_village.json'), {'processors': [{'processor_type': 'evanscomputermod:provision'}]})
write_json(Path('worldgen/structure_set/tech_villages.json'), {
    'structures': [{'structure': f'evanscomputermod:tech_village_{s}', 'weight': 1} for s in STYLES],
    'placement': {'type': 'evanscomputermod:tech_ring'}})
print('Generated', len(STYLES), 'ISP start templates, pools and structures')
