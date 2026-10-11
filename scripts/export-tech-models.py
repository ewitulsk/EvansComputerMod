"""Export checked-in Blockbench projects without replacing their authored geometry."""
import base64,json
from pathlib import Path
root=Path(__file__).resolve().parent.parent
assets=root/'src/main/resources/assets/evanscomputermod'
for name in ['fiber_span','fiber_patch_panel','always_on_module']:
    project=json.loads((root/'models'/f'{name}.bbmodel').read_text())
    folder='item' if name=='always_on_module' else 'block'
    textures=project['textures']
    names={str(t['uuid']):str(t.get('id',i)) for i,t in enumerate(textures)}
    resolution=project['resolution']
    out={'parent':'minecraft:block/block','credit':'Made with Blockbench','textures':{},'elements':[]}
    for i,t in enumerate(textures):
        key=str(t.get('id',i)); filename=t['name'].removesuffix('.png')
        out['textures'][key]=f'evanscomputermod:{folder}/{filename}'
        (assets/'textures'/folder/f'{filename}.png').write_bytes(base64.b64decode(t['source'].split(',')[1]))
    out['textures']['particle']=next(iter(out['textures'].values()))
    for e in project['elements']:
        cube={k:e[k] for k in ['name','from','to']}
        cube['faces']={}
        for direction,face in e['faces'].items():
            texture=face.get('texture')
            if texture is None:continue
            # .bbmodel texture references are indices, while the native editor uses UUIDs.
            key=names.get(str(texture),str(textures[int(texture)].get('id',texture)) if isinstance(texture,int) else str(texture))
            uv=face['uv']; cube['faces'][direction]={'uv':[v*16/resolution['width' if j%2==0 else 'height'] for j,v in enumerate(uv)],'texture':'#'+key}
        out['elements'].append(cube)
    if name=='always_on_module': out['display']={'gui':{'rotation':[0,0,0],'translation':[0,0,0],'scale':[1,1,1]}}
    def write(model,data): (assets/'models'/folder/f'{model}.json').write_text(json.dumps(data,indent=2)+'\n')
    write(name,out)
    if name=='fiber_span':
        directions=['north','south','west','east','down','up']
        for component in ['center',*directions]:write(name+'_'+component,{**out,'elements':[e for e in out['elements'] if e['name'].startswith(component)]})
        state={'multipart':[{'apply':{'model':'evanscomputermod:block/fiber_span_center'}},*[{'when':{d:'true'},'apply':{'model':f'evanscomputermod:block/fiber_span_{d}'}} for d in directions]]}
    elif name=='fiber_patch_panel':
        state={'variants':{f'facing={d}':{'model':'evanscomputermod:block/fiber_patch_panel','y':rotation} for d,rotation in [('north',0),('east',90),('south',180),('west',270)]}}
    else:continue
    (assets/'blockstates'/f'{name}.json').write_text(json.dumps(state,indent=2)+'\n')
