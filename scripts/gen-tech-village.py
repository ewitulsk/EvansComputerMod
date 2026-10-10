#!/usr/bin/env python3
"""Reproducible Tech Village buildings, one set per vanilla village style: the ISP
(start piece) and the Data Center attached to it.

Each ISP template is the start piece of a jigsaw structure that grows a normal vanilla
village: three street connectors use vanilla's `minecraft:street` name/target and the
style's `minecraft:village/<style>/streets` pool, the fourth (east) attaches the
style's Data Center first (higher selection priority, so it always fits), and the
`minecraft:bottom` jigsaws spawn the style's vanilla villagers.

ISP layout (unrotated, front = south):
  * 21 x 25 x 21 template; the lattice mast stands on the exact centre column
    (10, 1..23, 10), so a structure rotation never moves it. The start jigsaw
    `evanscomputermod:mast_anchor` sits at the mast base: JigsawStructure puts that
    block on the ring site's (x, z), so the mast top is known before the chunk
    generates (WorldNetwork.plan).
  * No patch panel and no cable in the template: the village network piece places the
    two Fiber Patch Panels beside the mast top on world-fixed sides (toward the
    previous and the next village) and routes the router's cables knowing the
    building's rotation: eth2/eth3 up the mast to the two panels, eth1 along the
    ceiling and through the east wall to the Data Center. The mast-side columns, the
    ceiling layer (y = 4) and the east wall cell (15, 4, 10) are kept free for them.
  * ISP router (role isp.router) at (7, 1, 8) facing south with an Interface Block on
    its west side (9 NICs). Its DOWN face (eth0, village access LAN) is cabled under the
    floor by the network piece.

Data Center layout (unrotated; its jigsaw at (0, 1, 6) faces west, onto the ISP):
  * 14 x 11 x 13 template, walls x 1..12, z 1..11, door in the west wall.
  * The server LAN arrives overhead at (0, 4, 6) (from the ISP's east wall), runs
    along the ceiling and drops onto the rack row: cable over seven rack slots at
    x = 11, z = 3..9 (y = 2). Slot z = 3 is the web server (role datacenter.web), slot
    z = 4 the chat server (datacenter.chat, removed by the provisioning processor except
    in the chat village); the rest are free: a computer placed there (screen to the
    aisle) has its UP face (eth1) on the LAN.
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
MAST_TOP = 23                # last lattice block; the two patch panels hang beside it
ROUTER, IFACE = (7, 1, 8), (6, 1, 8)
DC_SIZE = (14, 11, 13)
DX0, DX1, DZ0, DZ1 = 1, 12, 1, 11   # data center walls
DC_DOOR_Z = 6
RACK_X, RACK_Z = 11, range(3, 10)

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
    def jigsaw(self, x, y, z, orientation, name, target, pool, final, joint='aligned', priority=0):
        self.put(x, y, z, 'minecraft:jigsaw', {'orientation': orientation},
                 compound(text('id', 'minecraft:jigsaw'), text('name', name), text('target', target), text('pool', pool),
                          text('joint', joint), text('final_state', final), integer('selection_priority', priority),
                          integer('placement_priority', priority)))
    def cable(self, pos, dirs):
        self.put(*pos, 'evanscomputermod:network_cable', {d: 'true' if d in dirs else 'false'
                                                         for d in ('north', 'south', 'east', 'west', 'up', 'down')})
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
    for x, z, o in ((0, C, 'west_up'), (C, 0, 'north_up'), (C, SIZE[2] - 1, 'south_up')):
        t.jigsaw(x, 1, z, o, 'minecraft:street', 'minecraft:street', pool, 'minecraft:structure_void')
    # East: the Data Center, placed before any street (selection priority 1).
    t.jigsaw(SIZE[0] - 1, 1, C, 'east_up', 'evanscomputermod:isp_datacenter', 'evanscomputermod:datacenter',
             f'evanscomputermod:tech_village/datacenter_{style}', 'minecraft:structure_void', priority=1)
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
                    accent = (x in (LO, HI) and z in (LO, HI)) or x == C or z == C
                    t.put(x, 6, z, s['trim'] if accent else s['found'])
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
    # ---- lattice mast through the roof apex; the panels and riser cables are placed
    # by the network piece beside it (the four mast-side columns stay free)
    for y in range(1, MAST_TOP + 1):
        t.put(C, y, C, 'evanscomputermod:lattice_mast',
              {'up': 'true' if y < MAST_TOP else 'false', 'down': 'true' if y > 1 else 'false', 'north': 'false',
               'south': 'false', 'east': 'false', 'west': 'false', 'waterlogged': 'false'})
    # ---- equipment: router + interface block (cables come from the network piece)
    t.terminal(ROUTER, 'isp.router'); t.put(*IFACE, 'evanscomputermod:interface_block')
    # ---- interior furnishing and workstations
    t.put(12, 1, 6, 'minecraft:barrel', {'facing': 'up', 'open': 'false'})
    t.put(13, 1, 6, 'minecraft:lectern', {'facing': 'south', 'has_book': 'false', 'powered': 'false'})
    t.put(13, 1, 13, 'minecraft:cartography_table')
    t.put(7, 1, 13, 'minecraft:bookshelf'); t.put(7, 2, 13, 'minecraft:lantern', {'hanging': 'false', 'waterlogged': 'false'})
    t.put(6, 1, 13, 'minecraft:lantern', {'hanging': 'false', 'waterlogged': 'false'})
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

def build_datacenter(style, s):
    """The Data Center: rack row with the server LAN pre-run over every slot."""
    t = Template(DC_SIZE)
    air = 'minecraft:air'
    W, H, D = DC_SIZE
    # ---- ground: foundation, floor, rack plinth, path from the ISP to the door
    for x in range(DX0, DX1 + 1):
        for z in range(DZ0, DZ1 + 1):
            edge = x in (DX0, DX1) or z in (DZ0, DZ1)
            t.put(x, 0, z, s['found'] if edge else s['floor'])
    for z in RACK_Z:
        t.put(RACK_X, 0, z, 'minecraft:polished_andesite')
        t.put(RACK_X - 1, 0, z, 'minecraft:polished_andesite')
    for z in (DC_DOOR_Z - 1, DC_DOOR_Z, DC_DOOR_Z + 1):
        t.put(0, 0, z, s['path'])
    t.jigsaw(0, 1, DC_DOOR_Z, 'west_up', 'evanscomputermod:datacenter', 'minecraft:empty', 'minecraft:empty',
             'minecraft:structure_void')
    # ---- walls
    for y in range(1, 5):
        for x in range(DX0, DX1 + 1):
            for z in range(DZ0, DZ1 + 1):
                if not (x in (DX0, DX1) or z in (DZ0, DZ1)):
                    t.put(x, y, z, air); continue
                corner = x in (DX0, DX1) and z in (DZ0, DZ1)
                if corner or (z in (DZ0, DZ1) and x == 6):
                    t.put(x, y, z, s['pillar'], axis_y(s['pillar'])); continue
                if y == 4: t.put(x, y, z, s['trim'], axis_x_or_z(s['trim'], x, z, DZ0, DZ1)); continue
                if y in (2, 3) and z in (DZ0, DZ1) and x in (3, 4, 8, 9):
                    props = {'east': 'true', 'west': 'true', 'north': 'false', 'south': 'false', 'waterlogged': 'false'}
                    if x in (3, 8): props['west'] = 'false'
                    if x in (4, 9): props['east'] = 'false'
                    t.put(x, y, z, s['window'], props)
                else:
                    t.put(x, y, z, s['wall'])
    for y, half in ((1, 'lower'), (2, 'upper')):
        t.put(DX0, y, DC_DOOR_Z, s['door'], {'facing': 'east', 'half': half, 'hinge': 'left', 'open': 'false', 'powered': 'false'})
    # ---- roof
    if s['flat']:
        for x in range(DX0, DX1 + 1):
            for z in range(DZ0, DZ1 + 1):
                t.put(x, 5, z, s['roof_block'])
                if x in (DX0, DX1) or z in (DZ0, DZ1):
                    accent = (x in (DX0, DX1) and z in (DZ0, DZ1)) or z == DC_DOOR_Z
                    t.put(x, 6, z, s['trim'] if accent else s['found'])
    else:
        ridge = (DZ0 + DZ1) // 2          # z of the ridge line
        for k in range(0, ridge):
            y = 5 + k
            for x in range(0, W):
                t.put(x, y, k, s['roof'], {'facing': 'south', 'half': 'bottom', 'shape': 'straight', 'waterlogged': 'false'})
                t.put(x, y, D - 1 - k, s['roof'], {'facing': 'north', 'half': 'bottom', 'shape': 'straight', 'waterlogged': 'false'})
            for z in range(k + 1, D - 1 - k):
                for x in (DX0, DX1):          # gable ends
                    t.put(x, y, z, s['wall'])
                for x in range(DX0 + 1, DX1):  # attic
                    t.put(x, y, z, air)
        for x in range(0, W):
            t.put(x, 4 + ridge, ridge, s['roof_block'])
    # ---- the server LAN: in overhead from the ISP, along the ceiling, down onto the racks
    t.cable((0, 4, DC_DOOR_Z), {'west', 'east'})
    for x in range(DX0, RACK_X):
        t.cable((x, 4, DC_DOOR_Z), {'west', 'east'})
    t.cable((RACK_X, 4, DC_DOOR_Z), {'west', 'down'})
    t.cable((RACK_X, 3, DC_DOOR_Z), {'up', 'down'})
    for z in RACK_Z:
        dirs = {'down'}
        if z > RACK_Z[0]: dirs.add('north')
        if z < RACK_Z[-1]: dirs.add('south')
        if z == DC_DOOR_Z: dirs.add('up')
        t.cable((RACK_X, 2, z), dirs)
        if z != DC_DOOR_Z:
            t.put(RACK_X, 3, z, 'minecraft:smooth_stone_slab', {'type': 'bottom', 'waterlogged': 'false'})
    t.terminal((RACK_X, 1, RACK_Z[0]), 'datacenter.web', 'west')
    t.terminal((RACK_X, 1, RACK_Z[1]), 'datacenter.chat', 'west')
    for z in RACK_Z[2:]:
        t.put(RACK_X, 1, z, air)
    # ---- interior
    for x in range(6, 9):
        for z in range(3, 10):
            t.put(x, 1, z, s['carpet'])
    t.put(2, 1, 2, 'minecraft:lantern', {'hanging': 'false', 'waterlogged': 'false'})
    t.put(2, 1, 10, 'minecraft:lantern', {'hanging': 'false', 'waterlogged': 'false'})
    t.put(3, 1, 10, 'minecraft:barrel', {'facing': 'up', 'open': 'false'})
    t.put(4, 1, 10, 'minecraft:lectern', {'facing': 'north', 'has_book': 'false', 'powered': 'false'})
    t.put(10, 2, 2, s['sign'], {'facing': 'south', 'waterlogged': 'false'},
          compound(text('id', 'minecraft:sign'), text('ecmSign', 'rack')))
    # ---- outside: sign by the door
    t.put(0, 2, DC_DOOR_Z + 2, s['sign'], {'facing': 'west', 'waterlogged': 'false'},
          compound(text('id', 'minecraft:sign'), text('ecmSign', 'datacenter')))
    t.write(f'datacenter_{style}')

def axis_x_or_z(name, x, z, lo=LO, hi=HI):
    if not name.endswith('_log'): return None
    return {'axis': 'x' if z in (lo, hi) else 'z'}

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
    build_datacenter(style, palette)
    write_json(Path('worldgen/template_pool') / f'tech_village/datacenter_{style}.json', {
        'name': f'evanscomputermod:tech_village/datacenter_{style}', 'fallback': 'minecraft:empty',
        'elements': [{'weight': 1, 'element': {'element_type': 'minecraft:single_pool_element',
                                               'location': f'evanscomputermod:tech_village/datacenter_{style}',
                                               'processors': 'evanscomputermod:tech_village', 'projection': 'rigid'}}]})
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
print('Generated', len(STYLES), 'ISP start templates, data centers, pools and structures')
