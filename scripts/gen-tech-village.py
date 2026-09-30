#!/usr/bin/env python3
"""Reproducible role-marked village jigsaws. No world UUIDs are baked into NBT."""
import gzip, json, struct
from pathlib import Path
ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'src/main/resources-mc1.21.1/data/evanscomputermod'
def string(s):
    b=s.encode(); return struct.pack('>H',len(b))+b
def tag(t,n,b): return bytes([t])+string(n)+b
def text(n,s): return tag(8,n,string(s))
def integer(n,v): return tag(3,n,struct.pack('>i',v))
def compound(*parts): return b''.join(parts)+b'\0'
def obj(n,*parts): return tag(10,n,compound(*parts))
def array(n,t,items): return tag(9,n,bytes([t])+struct.pack('>i',len(items))+b''.join(items))
def ints(n,values): return array(n,3,[struct.pack('>i',v) for v in values])
def doubles(n,values): return array(n,6,[struct.pack('>d',v) for v in values])
def write_json(path,data):
    p=OUT/path;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(data,indent=2)+'\n')
class Template:
    def __init__(self,size): self.size=size;self.palette=[];self.blocks={};self.entities=[]
    def put(self,x,y,z,name,props=None,nbt=None):
        state=(name,tuple(sorted((props or {}).items())))
        if state not in self.palette: self.palette.append(state)
        self.blocks[x,y,z]=(self.palette.index(state),nbt)
    def room(self,x,z,w,d):
        for a in range(x,x+w):
            for b in range(z,z+d):
                self.put(a,0,b,'minecraft:stone_bricks')
                self.put(a,5,b,'minecraft:oak_planks')
                for y in range(1,5):
                    wall=a in (x,x+w-1) or b in (z,z+d-1)
                    self.put(a,y,b,'minecraft:glass_pane' if wall and y==3 else 'minecraft:oak_planks' if wall else 'minecraft:air')
        self.put(x+w//2,1,z,'minecraft:air');self.put(x+w//2,2,z,'minecraft:air')
    def terminal(self,x,y,z,role,expanded=False):
        self.put(x,y,z,'evanscomputermod:terminal_block',{'facing':'north'},compound(text('id','evanscomputermod:terminal_block'),text('ecmRole',role)))
        if expanded: self.put(x+1,y,z,'evanscomputermod:interface_block')
    def jigsaw(self,x,z,pool,name,target,orientation):
        self.put(x,1,z,'minecraft:jigsaw',{'orientation':orientation},compound(text('id','minecraft:jigsaw'),text('name',name),text('target',target),text('pool',pool),text('joint','aligned'),text('final_state','minecraft:gravel')))
    def villager(self,x,z,profession):
        nbt=compound(text('id','minecraft:villager'),obj('VillagerData',text('type','minecraft:plains'),text('profession',profession),integer('level',1)),integer('PersistenceRequired',1))
        self.entities.append(compound(doubles('pos',[x+.5,1,z+.5]),ints('blockPos',[x,1,z]),tag(10,'nbt',nbt)))
    def write(self,name):
        palette=[compound(text('Name',n),obj('Properties',*[text(k,v) for k,v in p])) for n,p in self.palette]
        blocks=[compound(ints('pos',pos),integer('state',s),tag(10,'nbt',nbt) if nbt else b'') for pos,(s,nbt) in sorted(self.blocks.items())]
        data=tag(10,'',compound(integer('DataVersion',3955),ints('size',self.size),array('palette',10,palette),array('blocks',10,blocks),array('entities',10,self.entities)))
        p=OUT/'structure'/f'tech_village/{name}.nbt';p.parent.mkdir(parents=True,exist_ok=True)
        with p.open('wb') as f:
            with gzip.GzipFile(fileobj=f,mode='wb',mtime=0) as gz: gz.write(data)
def pool(name):
    write_json(Path('worldgen/template_pool')/f'tech_village/{name}.json',{'name':f'evanscomputermod:tech_village/{name}','fallback':'minecraft:empty','elements':[{'weight':1,'element':{'element_type':'minecraft:single_pool_element','location':f'evanscomputermod:tech_village/{name}','processors':'evanscomputermod:tech_village','projection':'rigid'}}]})
isp=Template((49,7,49))
isp.room(17,17,15,15)
isp.terminal(21,1,23,'isp.router',True);isp.terminal(25,1,23,'isp.server');isp.terminal(28,1,23,'isp.access',True)
isp.put(21,2,23,'evanscomputermod:fiber_patch_panel');isp.put(21,0,23,'evanscomputermod:fiber_patch_panel')
isp.put(23,1,26,'minecraft:lectern');isp.put(26,1,26,'minecraft:cartography_table')
isp.villager(23,25,'minecraft:librarian');isp.villager(26,25,'minecraft:cartographer')
for x in range(49):
    for z in range(49):
        if x in range(23,26) or z in range(23,26):
            if not (17<=x<=31 and 17<=z<=31): isp.put(x,0,z,'minecraft:gravel')
connections=[(8,0,'north_up'),(40,0,'north_up'),(48,8,'east_up'),(48,40,'east_up'),(8,48,'south_up'),(40,48,'south_up')]
for i,(x,z,o) in enumerate(connections,1):
    isp.jigsaw(x,z,f'evanscomputermod:tech_village/house{i}','evanscomputermod:street','evanscomputermod:house',o)
isp.write('isp');pool('isp')
for i in range(1,7):
    house=Template((11,7,13));house.room(0,1,11,11)
    house.jigsaw(5,12,'minecraft:empty','evanscomputermod:house','evanscomputermod:street','south_up')
    house.terminal(2,1,5,f'house{i}.router');house.terminal(4,1,5,f'house{i}.pc')
    house.put(7,1,7,'minecraft:red_bed',{'facing':'north','part':'foot','occupied':'false'})
    house.put(7,1,6,'minecraft:red_bed',{'facing':'north','part':'head','occupied':'false'})
    house.put(8,1,4,'minecraft:lectern');house.villager(7,4,'minecraft:librarian')
    house.write(f'house{i}');pool(f'house{i}')
write_json(Path('worldgen/processor_list/tech_village.json'),{'processors':[{'processor_type':'evanscomputermod:provision'}]})
write_json(Path('worldgen/structure/tech_village.json'),{'type':'minecraft:jigsaw','biomes':'#minecraft:is_overworld','step':'surface_structures','spawn_overrides':{},'terrain_adaptation':'beard_thin','start_pool':'evanscomputermod:tech_village/isp','size':1,'start_height':{'absolute':0},'project_start_to_heightmap':'WORLD_SURFACE_WG','max_distance_from_center':96,'use_expansion_hack':False})
write_json(Path('worldgen/structure_set/tech_villages.json'),{'structures':[{'structure':'evanscomputermod:tech_village','weight':1}],'placement':{'type':'evanscomputermod:tech_ring'}})
print('Generated seven role-marked templates and jigsaw definitions')
