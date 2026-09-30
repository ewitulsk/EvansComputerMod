"""Generate fiber/module models, recipes and drops using vanilla textures."""
import json
from pathlib import Path
ROOT=Path(__file__).resolve().parent.parent/'src/main/resources'
def write(path,data):
    if 'recipe' in path.parts:
        for version in ['1.21.1','26.1']:
            variant=json.loads(json.dumps(data))
            if version=='26.1':variant['ingredients']=[i['item'] for i in variant['ingredients']]
            p=ROOT.parent/f'resources-mc{version}'/path;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(variant,indent=2)+'\n')
        return
    p=ROOT/path;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(data,indent=2)+'\n')
def rod(box,texture):
    return {'from':box[:3],'to':box[3:],'faces':{f:{'texture':'#all'} for f in ['north','south','west','east','up','down']}}
for block,texture,boxes in [('utility_pole','minecraft:block/stripped_oak_log',[[6,0,6,10,16,10]]),('fiber_span','minecraft:block/black_concrete',[[0,7,7,16,9,9],[7,7,0,9,9,16]]),('fiber_patch_panel','minecraft:block/iron_block',[[1,1,1,15,15,15]])]:
    write(Path(f'assets/evanscomputermod/blockstates/{block}.json'),{'variants':{'':{'model':f'evanscomputermod:block/{block}'}}})
    write(Path(f'assets/evanscomputermod/models/block/{block}.json'),{'textures':{'all':texture,'particle':texture},'elements':[rod(b,texture) for b in boxes]})
    write(Path(f'assets/evanscomputermod/models/item/{block}.json'),{'parent':f'evanscomputermod:block/{block}'})
    write(Path(f'assets/evanscomputermod/items/{block}.json'),{'model':{'type':'minecraft:model','model':f'evanscomputermod:item/{block}'}})
    write(Path(f'data/evanscomputermod/loot_table/blocks/{block}.json'),{'type':'minecraft:block','pools':[{'rolls':1,'entries':[{'type':'minecraft:item','name':f'evanscomputermod:{block}'}],'conditions':[{'condition':'minecraft:survives_explosion'}]}]})
    ingredients={'utility_pole':['minecraft:oak_log'],'fiber_span':['minecraft:glass','minecraft:iron_nugget'],'fiber_patch_panel':['evanscomputermod:fiber_span','minecraft:iron_ingot']}[block]
    write(Path(f'data/evanscomputermod/recipe/{block}.json'),{'type':'minecraft:crafting_shapeless','ingredients':[{'item':i} for i in ingredients],'result':{'id':f'evanscomputermod:{block}','count':8 if block=='fiber_span' else 1}})
write(Path('assets/evanscomputermod/models/item/always_on_module.json'),{'parent':'minecraft:item/generated','textures':{'layer0':'minecraft:item/clock_00'}})
write(Path('assets/evanscomputermod/items/always_on_module.json'),{'model':{'type':'minecraft:model','model':'evanscomputermod:item/always_on_module'}})
write(Path('data/evanscomputermod/recipe/always_on_module.json'),{'type':'minecraft:crafting_shapeless','ingredients':[{'item':'minecraft:clock'},{'item':'minecraft:redstone'},{'item':'evanscomputermod:module_expansion_card'}],'result':{'id':'evanscomputermod:always_on_module','count':1}})
p=ROOT/'assets/evanscomputermod/lang/en_us.json';lang=json.loads(p.read_text());lang.update({'block.evanscomputermod.utility_pole':'Utility Pole','block.evanscomputermod.fiber_span':'Fiber Span','block.evanscomputermod.fiber_patch_panel':'Fiber Patch Panel','item.evanscomputermod.always_on_module':'Always-On Module'});write(Path('assets/evanscomputermod/lang/en_us.json'),lang)
