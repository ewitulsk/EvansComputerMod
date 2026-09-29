"""Generate terminal bay / cartridge models, the multipart blockstate and the
Redstone Link Interface model.

Geometry is described in the terminal's north-facing frame (front = north,
left bay on the west face x=0..3, right bay on the east face x=13..16). The
right side is the left side mirrored in x. Face UVs are projected from world
coordinates, so adjacent elements tile a texture exactly like one full cube.
"""
import json, os

A = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'src', 'main', 'resources',
                 'assets', 'evanscomputermod')
M = A + '/models/block'
os.makedirs(M + '/terminal', exist_ok=True)

TEX = {
    "front": "evanscomputermod:block/terminal_front",
    "back": "evanscomputermod:block/terminal_back",
    "side": "evanscomputermod:block/terminal_side",
    "rim": "evanscomputermod:block/terminal_bay_rim",
    "inner": "evanscomputermod:block/terminal_bay_inner",
    "bayback": "evanscomputermod:block/terminal_bay_back",
    "particle": "evanscomputermod:block/terminal_side",
}


def proj(face, f, t):
    x1, y1, z1 = f
    x2, y2, z2 = t
    return {
        "north": [16 - x2, 16 - y2, 16 - x1, 16 - y1],
        "south": [x1, 16 - y2, x2, 16 - y1],
        "west": [z1, 16 - y2, z2, 16 - y1],
        "east": [16 - z2, 16 - y2, 16 - z1, 16 - y1],
        "up": [x1, z1, x2, z2],
        "down": [x1, 16 - z2, x2, 16 - z1],
    }[face]


def rnd(v):
    v = round(v, 4)
    return int(v) if v == int(v) else v


def el(name, f, t, faces):
    """faces: {dir: (texture, cull?) or (texture, cull, uv)}"""
    out = {"name": name, "from": [rnd(v) for v in f], "to": [rnd(v) for v in t], "faces": {}}
    for d, spec in faces.items():
        tex, cull = spec[0], spec[1]
        uv = spec[2] if len(spec) > 2 else proj(d, f, t)
        face = {"uv": [rnd(v) for v in uv], "texture": "#" + tex}
        if cull:
            face["cullface"] = d
        out["faces"][d] = face
    return out


MIRROR_DIR = {"west": "east", "east": "west"}


def mirror(e):
    """Mirror an element (built for the west side) to the east side, re-projecting UVs."""
    f, t = e["from"], e["to"]
    nf = [16 - t[0], f[1], f[2]]
    nt = [16 - f[0], t[1], t[2]]
    faces = {}
    for d, face in e["faces"].items():
        nd = MIRROR_DIR.get(d, d)
        spec = (face["texture"][1:], "cullface" in face)
        if face.get("_explicit"):
            uv = face["uv"]
            if d in ("west", "east"):
                # Keep cartridge face art reading the same way on both sides.
                uv = [uv[2], uv[1], uv[0], uv[3]]
            spec = spec + (uv,)
        faces[nd] = spec
    return el(e["name"].replace("left", "right"), nf, nt, faces)


def model(elements, textures, parent="block/block"):
    m = {"parent": parent, "textures": dict(textures), "elements": elements}
    return m


def write(rel, data):
    p = M + '/' + rel + '.json'
    with open(p, 'w', encoding='utf-8', newline='\n') as fh:
        json.dump(data, fh, indent=2)
        fh.write('\n')


def used(textures, elements):
    keys = {face["texture"][1:] for e in elements for face in e["faces"].values()}
    return {k: v for k, v in textures.items() if k in keys or k == "particle"}


# ---------------------------------------------------------------- terminal parts
core = [el("core", [3, 0, 0], [13, 16, 16], {
    "north": ("front", True), "south": ("back", True), "up": ("side", True), "down": ("side", True)})]

side_left = [el("side_left", [0, 0, 0], [3, 16, 16], {
    "north": ("front", True), "south": ("back", True), "west": ("side", True),
    "up": ("side", True), "down": ("side", True)})]

bay_left = [
    el("left_front_rim", [0, 0, 0], [3, 16, 2], {
        "north": ("front", True), "west": ("rim", True), "up": ("side", True), "down": ("side", True),
        "south": ("inner", False)}),
    el("left_back_rim", [0, 0, 14], [3, 16, 16], {
        "south": ("back", True), "west": ("rim", True), "up": ("side", True), "down": ("side", True),
        "north": ("inner", False)}),
    el("left_top_rim", [0, 15, 2], [3, 16, 14], {
        "west": ("rim", True), "up": ("side", True), "down": ("inner", False)}),
    el("left_bottom_rim", [0, 0, 2], [3, 1, 14], {
        "west": ("rim", True), "down": ("side", True), "up": ("inner", False)}),
    el("left_divider", [0, 7, 2], [3, 9, 14], {
        "west": ("rim", True), "up": ("inner", False), "down": ("inner", False)}),
    el("left_back_wall", [2, 1, 2], [3, 15, 14], {"west": ("bayback", False)}),
]


def cartridge(visual, upper):
    """Cartridge in a left-bay slot (upper y 9..15, lower y 1..7)."""
    y0 = 10 if upper else 2
    f, t = [0.5, y0, 3], [2, y0 + 4, 13]
    els = [el("left_cartridge", f, t, {
        "west": ("module", False, [0, 0, 10, 4]),
        "up": ("module", False, [0, 4, 1.5, 6]),
        "down": ("module", False, [0, 6, 1.5, 8]),
        "north": ("module", False, [10, 0, 11.5, 4]),
        "south": ("module", False, [12, 0, 13.5, 4]),
    })]
    if visual == "redstone_link":
        af, at = [0, y0 + 1, 11.5], [0.5, y0 + 3.5, 12.5]
        els.append(el("left_antenna", af, at, {
            "west": ("module", False, [15, 0, 16, 2.5]),
            "north": ("module", False, [15, 0, 15.5, 2.5]),
            "south": ("module", False, [15, 0, 15.5, 2.5]),
            "up": ("module", False, [15, 0, 15.5, 1]),
        }))
    for e in els:
        for face in e["faces"].values():
            face["_explicit"] = True
    return els


def clean(elements):
    for e in elements:
        for face in e["faces"].values():
            face.pop("_explicit", None)
    return elements


write("terminal/core", model(core, used(TEX, core)))
write("terminal/side_left", model(side_left, used(TEX, side_left)))
write("terminal/side_right", model([mirror(e) for e in side_left], used(TEX, side_left)))
write("terminal/bay_left", model(bay_left, used(TEX, bay_left)))
write("terminal/bay_right", model([mirror(e) for e in bay_left], used(TEX, bay_left)))

SLOT_NAMES = ["left_bay_1", "left_bay_2", "right_bay_1", "right_bay_2"]
VISUALS = {"generic": "evanscomputermod:block/module_generic",
           "redstone_link": "evanscomputermod:block/module_redstone_link"}
for visual, tex in VISUALS.items():
    for slot in SLOT_NAMES:
        upper = slot.endswith("_1")
        els = cartridge(visual, upper)
        if slot.startswith("right"):
            els = [mirror(e) for e in els]
        write("terminal/cartridge_%s_%s" % (visual, slot),
              model(clean(els), {"module": tex, "particle": tex}))

# Full model (item + particle source): core with two plain sides.
full = core + side_left + [mirror(e) for e in side_left]
write("terminal_block", model(full, used(TEX, full)))

# ---------------------------------------------------------------- blockstate
ROT = {"north": 0, "east": 90, "south": 180, "west": 270}
parts = []
for facing, rot in ROT.items():
    def apply(name):
        a = {"model": "evanscomputermod:block/terminal/" + name}
        if rot:
            a["y"] = rot
        return a
    parts.append({"when": {"facing": facing}, "apply": apply("core")})
    for side in ("left", "right"):
        parts.append({"when": {"facing": facing, side + "_bay": "false"}, "apply": apply("side_" + side)})
        parts.append({"when": {"facing": facing, side + "_bay": "true"}, "apply": apply("bay_" + side)})
    for slot in SLOT_NAMES:
        for visual in VISUALS:
            parts.append({"when": {"facing": facing, slot: visual},
                          "apply": apply("cartridge_%s_%s" % (visual, slot))})
with open(A + '/blockstates/terminal_block.json', 'w', encoding='utf-8', newline='\n') as fh:
    json.dump({"multipart": parts}, fh, indent=2)
    fh.write('\n')

# ---------------------------------------------------------------- link interface (Create only)
casing = "create:block/brass_casing"
iface = [el("base", [0, 0, 0], [16, 8, 16], {
    "north": ("casing", True, [0, 8, 16, 16]), "south": ("casing", True, [0, 8, 16, 16]),
    "west": ("casing", True, [0, 8, 16, 16]), "east": ("casing", True, [0, 8, 16, 16]),
    "up": ("top", False, [0, 0, 16, 16]), "down": ("casing", True, [0, 0, 16, 16])})]


def antenna(x, z):
    # Create's Redstone Link antenna (models/block/redstone_link/transmitter.json), raised onto the base.
    return [
        el("antenna_top", [x + 1, 16, z + 1], [x + 2, 17, z + 2], {"up": ("antenna", False, [1, 1, 2, 2])}),
        el("antenna_z", [x + 1, 8, z], [x + 2, 18, z + 3], {
            "east": ("antenna", False, [0, 0, 3, 10]), "west": ("antenna", False, [0, 0, 3, 10])}),
        el("antenna_x", [x, 8, z + 1], [x + 3, 18, z + 2], {
            "north": ("antenna", False, [0, 0, 3, 10]), "south": ("antenna", False, [0, 0, 3, 10]),
            "down": ("antenna", False, [0, 9, 3, 10])}),
    ]


iface += antenna(1, 1) + antenna(12, 12)
write("redstone_link_interface", model(iface, {
    "casing": casing, "top": "evanscomputermod:block/redstone_link_interface_top",
    "antenna": "create:block/redstone_antenna", "particle": casing}))
with open(A + '/blockstates/redstone_link_interface.json', 'w', encoding='utf-8', newline='\n') as fh:
    json.dump({"variants": {"": {"model": "evanscomputermod:block/redstone_link_interface"}}}, fh, indent=2)
    fh.write('\n')

# ---------------------------------------------------------------- item models
I = A + '/models/item'
for name in ("module_expansion_card", "redstone_link_module"):
    with open(I + '/' + name + '.json', 'w', encoding='utf-8', newline='\n') as fh:
        json.dump({"parent": "minecraft:item/generated", "textures": {"layer0": "evanscomputermod:item/" + name}}, fh, indent=2)
        fh.write('\n')
with open(I + '/redstone_link_interface.json', 'w', encoding='utf-8', newline='\n') as fh:
    json.dump({"parent": "evanscomputermod:block/redstone_link_interface"}, fh, indent=2)
    fh.write('\n')
# 26.1 item definitions (Create items don't exist on 26.1)
with open(A + '/items/module_expansion_card.json', 'w', encoding='utf-8', newline='\n') as fh:
    json.dump({"model": {"type": "minecraft:model", "model": "evanscomputermod:item/module_expansion_card"}}, fh, indent=2)
    fh.write('\n')
print('models ok,', len(parts), 'multipart entries')
