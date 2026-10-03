"""Meter-scale exploration vehicles and rigidly weighted moving parts."""
import math
import bpy
from mathutils import Vector


def recipes(Model):
    def loft(m, name, sections, color):
        # Chamfered longitudinal sections: y, half-width, bottom, top, bevel.
        vertices=[]
        for y,w,b,t,c in sections:
            vertices += [(-w,y,b),(w,y,b),(w,y,t-c),(w-c,y,t),(-w+c,y,t),(-w,y,t-c)]
        n=6
        faces=[tuple(range(n)),tuple(reversed(range((len(sections)-1)*n,len(sections)*n)))]
        for ring in range(len(sections)-1):
            for i in range(n):
                j=(i+1)%n;a=ring*n;b=(ring+1)*n
                faces.append((a+i,b+i,b+j,a+j))
        return m.mesh(name,vertices,faces,color)

    def car():
        m = Model('vehicle.car')
        loft(m,'body',[(-1.6,.60,.28,.53,.1),(-1.3,.78,.25,.68,.12),(-.65,.8,.25,.72,.1),(.8,.8,.25,.7,.1),(1.5,.72,.30,.61,.1)],'sage')
        loft(m,'cabin',[(-.73,.61,.63,.72,.04),(-.22,.58,.64,1.13,.1),(.63,.57,.63,1.16,.1),(1.12,.61,.62,.74,.05)],'glass')
        loft(m,'roof',[(-.24,.5,1.08,1.19,.03),(.62,.5,1.11,1.21,.03),(.77,.49,1.02,1.09,.03)],'linen')
        # Opaque pillars and lower sills define individual windows.
        for x in [-.59,.59]:
            m.beam('pillar',(x,.25,.66),(x*.9,.25,1.15),.06,.06,'sage')
        m.box('bumper',(0,-1.54,.31),(1.2,.12,.12),'dark')
        for x in [-.47,.47]:
            m.box('headlight',(x,-1.63,.49),(.25,.09,.13),'linen')
            m.box('taillight',(x,1.53,.49),(.22,.06,.10),'clay')
        for side, x in [('left', -.78), ('right', .78)]:
            for end, y in [('front', -.98), ('rear', .98)]:
                # Wheel axle is X; alternating hub/spoke colors expose rotation.
                points=[]
                for dx in [-.13,.13]:
                    for i in range(12):
                        a=i*math.tau/12
                        points.append((x+dx,y+.31*math.cos(a),.31+.31*math.sin(a)))
                faces=[tuple(reversed(range(12))),tuple(range(12,24))]+[(i,(i+1)%12,(i+1)%12+12,i+12) for i in range(12)]
                m.mesh('wheel.'+end+'.'+side, points, faces, 'dark')
                m.box('hub.'+end+'.'+side,(x,y,.31),(.28,.12,.43),'stone')
        return m

    def plane():
        m=Model('vehicle.plane')
        loft(m,'body',[(-2.45,.22,.4,.72,.07),(-1.8,.48,.15,.93,.2),(-.5,.53,.10,1.02,.23),(.8,.35,.24,.79,.16),(2.45,.07,.48,.6,.04)],'linen')
        loft(m,'nose',[(-2.48,.22,.4,.72,.06),(-2.15,.34,.30,.82,.12)],'clay')
        loft(m,'canopy',[(-1.0,.35,.82,.94,.07),(-.5,.36,.85,1.34,.16),(.2,.29,.75,1.23,.13),(.65,.18,.7,.84,.06)],'glass')
        # Tapered swept wings with a thicker root and thin clipped tips.
        for sign in [-1,1]:
            verts=[(sign*.3,-.55,.72),(sign*3.8,-.08,.87),(sign*4.,.30,.87),(sign*3.85,.60,.87),(sign*.3,.68,.72),
                   (sign*.3,-.55,.83),(sign*3.8,-.08,.93),(sign*4.,.30,.93),(sign*3.85,.60,.93),(sign*.3,.68,.83)]
            faces=[(0,4,3,2,1),(5,6,7,8,9),(0,1,6,5),(1,2,7,6),(2,3,8,7),(3,4,9,8),(4,0,5,9)]
            if sign<0:faces=[tuple(reversed(f)) for f in faces]
            m.mesh('wing.'+str(sign),verts,faces,'sage')
        m.mesh('tail',[(-1.5,2.25,.72),(0,1.65,.73),(1.5,2.25,.72),(1.35,2.55,.72),(-1.35,2.55,.72),(-1.5,2.25,.8),(0,1.65,.81),(1.5,2.25,.8),(1.35,2.55,.8),(-1.35,2.55,.8)],[(0,4,3,2,1),(5,6,7,8,9),(0,1,6,5),(1,2,7,6),(2,3,8,7),(3,4,9,8),(4,0,5,9)],'sage')
        m.mesh('fin',[(-.06,1.65,.6),(-.06,2.3,1.625),(-.06,2.55,.6),(.06,1.65,.6),(.06,2.3,1.625),(.06,2.55,.6)],[(0,2,1),(3,4,5),(0,1,4,3),(1,2,5,4),(2,0,3,5)],'clay')
        m.box('propeller',(0,-2.5,.65),(1.65,.06,.13),'dark')
        return m
    return [car, plane]


def rig_vehicle(model, owner):
    # Apply the same contract fit to geometry and articulation centers before
    # skinning; all moving parts remain one consolidated compatible primitive.
    fit=Vector(model.root.scale)
    for obj in model.scene.objects:
        if obj.type=='MESH':
            for v in obj.data.vertices:
                v.co=Vector(tuple(v.co[i]*fit[i] for i in range(3)))
    model.root.scale=(1,1,1)
    data=bpy.data.armatures.new(model.name+'.skeleton');data[owner]=True
    rig=bpy.data.objects.new(model.name+'.rig',data);rig[owner]=True
    model.scene.collection.objects.link(rig);rig.parent=model.root
    bpy.context.view_layer.objects.active=rig;rig.select_set(True)
    bpy.ops.object.mode_set(mode='EDIT')
    root=data.edit_bones.new('vehicle.root');root.head=(0,0,0);root.tail=(0,0,.1)
    centers={}
    for obj in model.scene.objects:
        part=obj.name.removeprefix(model.name+'.')
        if obj.type=='MESH' and (part.startswith('wheel.') or part=='propeller'):
            centers['vehicle.'+part]=sum((v.co for v in obj.data.vertices),Vector())/len(obj.data.vertices)
    for name,point in centers.items():
        bone=data.edit_bones.new(name)
        bone.head=tuple(point);bone.tail=bone.head+Vector((0,0,.1));bone.parent=root
    bpy.ops.object.mode_set(mode='OBJECT')
    for obj in list(model.scene.objects):
        if obj.type!='MESH':continue
        part=obj.name.removeprefix(model.name+'.')
        if part.startswith('wheel.') or part.startswith('hub.'):
            target='vehicle.wheel.'+part.partition('.')[2]
        elif part=='propeller':target='vehicle.propeller'
        else:target='vehicle.root'
        obj.vertex_groups.new(name=target).add(list(range(len(obj.data.vertices))),1.,'REPLACE')
        obj.modifiers.new('Armature','ARMATURE').object=rig
