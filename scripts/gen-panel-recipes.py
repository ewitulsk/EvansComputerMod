"""Generate the display panel / tiered Screen / Terminal recipes for both
Minecraft versions (docs/gui/DISPLAY_PIPELINE.md section 6, "Recipes").

Each tier is crafted from the one below. A newly crafted Terminal comes
with a Tier 1 panel installed (the evanscomputermod:installed_panel
component on the result). Vanilla ingredients only.

1.21.1 writes ingredients as {"item": id} (a list of those for "any of");
26.1 writes plain ids (a list of ids for "any of").
"""
import json, os

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'src', 'main')
NS = 'evanscomputermod:'
ANY_T1 = [NS + 'phosphor_panel', NS + 'amber_phosphor_panel']


def ing(v, old):
    if isinstance(v, list):
        return [ing(x, old) for x in v] if old else list(v)
    return {"item": v} if old else v


def result(item, count=1, panel=None):
    r = {"id": NS + item, "count": count}
    if panel:
        r["components"] = {NS + "installed_panel": {"panel": {"id": NS + panel, "count": 1}}}
    return r


def shaped(pattern, key, res):
    return lambda old: {"type": "minecraft:crafting_shaped", "category": "redstone", "pattern": pattern,
                        "key": {k: ing(v, old) for k, v in key.items()}, "result": res}


def shapeless(items, res):
    return lambda old: {"type": "minecraft:crafting_shapeless", "category": "redstone",
                        "ingredients": [ing(i, old) for i in items], "result": res}


R = {
    # Tier 1: 3 glass panes, 2 redstone, 2 iron, green (or orange) dye.
    "phosphor_panel": shaped(["GGG", "RDR", "I I"], {"G": "minecraft:glass_pane", "R": "minecraft:redstone",
                                                     "D": "minecraft:green_dye", "I": "minecraft:iron_ingot"},
                             result("phosphor_panel")),
    "amber_phosphor_panel": shaped(["GGG", "RDR", "I I"], {"G": "minecraft:glass_pane", "R": "minecraft:redstone",
                                                           "D": "minecraft:orange_dye", "I": "minecraft:iron_ingot"},
                                   result("amber_phosphor_panel")),
    "amber_phosphor_panel_from_green": shapeless([NS + "phosphor_panel", "minecraft:orange_dye"],
                                                 result("amber_phosphor_panel")),
    "phosphor_panel_from_amber": shapeless([NS + "amber_phosphor_panel", "minecraft:green_dye"],
                                           result("phosphor_panel")),
    # Tier 2: Tier 1 panel, 2 gold, red, green and blue (lapis), 2 redstone.
    "color_lcd_panel": shapeless([ANY_T1, "minecraft:gold_ingot", "minecraft:gold_ingot", "minecraft:red_dye",
                                  "minecraft:green_dye", "minecraft:lapis_lazuli", "minecraft:redstone",
                                  "minecraft:redstone"], result("color_lcd_panel")),
    # Tier 3: Tier 2 panel, diamond, 2 glowstone dust, 2 quartz, 2 redstone.
    "hd_panel": shapeless([NS + "color_lcd_panel", "minecraft:diamond", "minecraft:glowstone_dust",
                           "minecraft:glowstone_dust", "minecraft:quartz", "minecraft:quartz",
                           "minecraft:redstone", "minecraft:redstone"], result("hd_panel")),
    # Terminal: the old recipe with a Tier 1 panel in the top middle; comes with it installed.
    "terminal_block": shaped(["IPI", "IRI", "IGI"], {"I": "minecraft:iron_ingot", "P": NS + "phosphor_panel",
                                                     "R": "minecraft:redstone_block", "G": "minecraft:gold_block"},
                             result("terminal_block", panel="phosphor_panel")),
    "terminal_block_amber": shaped(["IPI", "IRI", "IGI"], {"I": "minecraft:iron_ingot", "P": NS + "amber_phosphor_panel",
                                                           "R": "minecraft:redstone_block", "G": "minecraft:gold_block"},
                                   result("terminal_block", panel="amber_phosphor_panel")),
    # Screens, 4 per craft: that tier's panel in an iron and glass frame.
    "phosphor_screen": shaped(["IGI", "GPG", "IGI"], {"I": "minecraft:iron_ingot", "G": "minecraft:glass_pane",
                                                      "P": ANY_T1}, result("phosphor_screen", 4)),
    "screen_block": shaped(["IGI", "GPG", "IGI"], {"I": "minecraft:iron_ingot", "G": "minecraft:glass_pane",
                                                   "P": NS + "color_lcd_panel"}, result("screen_block", 4)),
    "hd_screen": shaped(["IGI", "GPG", "IGI"], {"I": "minecraft:iron_ingot", "G": "minecraft:glass_pane",
                                                "P": NS + "hd_panel"}, result("hd_screen", 4)),
    # Or upgrade one Screen at a time (walls can be upgraded block by block).
    "screen_block_upgrade": shapeless([NS + "phosphor_screen", "minecraft:gold_ingot", "minecraft:red_dye",
                                       "minecraft:lapis_lazuli", "minecraft:redstone"], result("screen_block")),
    "hd_screen_upgrade": shapeless([NS + "screen_block", "minecraft:glowstone_dust", "minecraft:quartz",
                                    "minecraft:redstone"], result("hd_screen")),
}

for ver, old in (("resources-mc1.21.1", True), ("resources-mc26.1", False)):
    d = os.path.join(ROOT, ver, 'data', 'evanscomputermod', 'recipe')
    os.makedirs(d, exist_ok=True)
    for name, fn in R.items():
        with open(os.path.join(d, name + '.json'), 'w', encoding='utf-8', newline='\n') as fh:
            json.dump(fn(old), fh, indent=2)
            fh.write('\n')
print('recipes ok:', len(R), 'per version')
