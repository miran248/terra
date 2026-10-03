"""Construction-focused structure recipes at the approved one-meter character scale."""
import math


def roof(m, prefix, width, depth, eaves, ridge, color='clay'):
    """Two closed roof slabs, explicit outer dimensions and .04 m thickness."""
    for side in [-1,1]:
        x=side*width/2
        vertices=[(0,-depth/2,ridge-.04),(x,-depth/2,eaves-.04),
            (x,depth/2,eaves-.04),(0,depth/2,ridge-.04),
            (0,-depth/2,ridge),(x,-depth/2,eaves),
            (x,depth/2,eaves),(0,depth/2,ridge)]
        faces=[(0,3,2,1),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7),(4,5,6,7)]
        if side<0:faces=[tuple(reversed(f)) for f in faces]
        m.mesh(f'{prefix}.{side}',vertices,faces,color)
        for i in range(1,4):
            t=i/4
            # Flush color courses describe construction without changing the envelope.
            z=ridge+(eaves-ridge)*t+.002
            m.box(f'{prefix}.course.{side}.{i}',(x*t,0,z),(.022,depth-.05,.015),'clay_light' if color=='clay' else color)


def post(m,name,x,y,height,width=.08,color='wood'):
    m.box(name,(x,y,height/2),(width,width,height),color)


def ruin(Model):
    m=Model('structure.ruin')
    m.box('foundation',(0,0,.06),(2,1.65,.12),'stone')
    # Stepped remnants form an L rather than an opaque solid footprint.
    for row in range(4):
        for col in range(5-row):
            x=-.8+col*.36+(row%2)*.10
            m.box(f'rear.block.{row}.{col}',(x,.61,.12+row*.27+.13),(.34,.27,.25),'stone' if (row+col)%2 else 'linen')
    for row in range(3):
        for col in range(3-row):
            m.box(f'left.block.{row}.{col}',(-.8,.24-col*.36,.12+row*.27+.13),(.28,.34,.25),'stone')
    for i in range(4):
        m.box(f'rubble.{i}',(-.30+i*.28,-.49+(i%2)*.18,.18),(.22,.24,.12),'stone' if i%2 else 'linen')
    m.rings('broken.column',[(-.76,.59,.95,.16,.16),(-.76,.59,1.35,.14,.14)],'plaster',7)
    return m


def watchtower(Model):
    m=Model('structure.watchtower')
    for x in [-.65,.65]:
        for y in [-.65,.65]:
            post(m,f'leg.{x}.{y}',x,y,3.25,.10)
            m.box(f'foot.{x}.{y}',(x,y,.07),(.19,.19,.14),'stone')
    for y in [-.65,.65]:
        m.beam(f'brace.{y}.a',(-.65,y,.35),(.65,y,1.8),.055,.045,'wood')
        m.beam(f'brace.{y}.b',(.65,y,.35),(-.65,y,1.8),.055,.045,'wood')
    for i in range(9):
        m.box(f'deck.plank.{i}',(-.72+i*.18,0,2.04),(.174,1.62,.12),'wood' if i%2 else 'linen')
    for y in [-.72,.72]:
        m.box(f'rail.{y}',(0,y,2.67),(1.6,.06,.065),'wood')
        for x in [-.65,-.32,0,.32,.65]:
            post_start=2.1
            m.box(f'baluster.{x}.{y}',(x,y,post_start+.25),(.045,.045,.5),'wood')
    for x in [-.74,.74]:
        m.box(f'side.rail.{x}',(x,0,2.67),(.06,1.55,.065),'wood')
    # Ladder stays visual; the refresh does not add a climbing mechanic.
    for x in [-.18,.18]:
        m.beam(f'ladder.side.{x}',(x,-.91,.02),(x,-.72,2.12),.035,.035,'wood')
    for i in range(10):
        z=.12+i*.20
        m.box(f'ladder.rung.{i}',(0,-.91+z/2.12*.19,z),(.38,.035,.032),'linen')
    roof(m,'roof',1.9,1.9,3.26,3.65)
    return m


def dock(Model):
    m=Model('structure.dock')
    for x in [-.57,.57]:
        for y in [-.98,.98]:
            post(m,f'pile.{x}.{y}',x,y,.42,.12)
        m.box(f'joist.{x}',(x,0,.32),(.10,2.4,.10),'dark_wood')
    for i in range(12):
        m.box(f'plank.{i}',(0,-1.1+i*.2,.41),(1.4,.195,.08),'wood' if i%3 else 'linen')
        for x in [-.57,.57]:
            m.box(f'peg.{i}.{x}',(x,-1.1+i*.2,.451),(.017,.017,.004),'dark_wood')
    # Explicit end caps meet the next module exactly at +/-1.2 m.
    for y in [-1.195,1.195]:
        m.box(f'end.{y}',(0,y,.41),(1.4,.01,.08),'wood')
    return m


def farm(Model):
    m=Model('structure.farm')
    m.box('soil',(0,0,.025),(2.4,2.4,.05),'dark_wood')
    for x in [-1.17,1.17]:m.box(f'bed.edge.x.{x}',(x,0,.06),(.06,2.4,.12),'wood')
    for y in [-1.17,1.17]:m.box(f'bed.edge.y.{y}',(0,y,.06),(2.28,.06,.12),'wood')
    for row in range(4):
        x=-.84+row*.56
        m.box(f'furrow.{row}',(x,0,.052),(.28,2.18,.025),'wood')
        for col in range(6):
            y=-.9+col*.36
            m.rings(f'crop.{row}.{col}',[(x,y,.05,.04,.04),(x,y,.22,.10,.075),(x,y,.36,.018,.018)],'sage_light' if (row+col)%2 else 'sage',5)
    return m


def wall(Model):
    m=Model('structure.wall')
    m.box('mortar',(0,0,.49),(2,.29,.98),'linen')
    for row in range(4):
        breaks=[-1,-.6,-.2,.2,.6,1] if row%2==0 else [-1,-.8,-.4,0,.4,.8,1]
        for i,(a,b) in enumerate(zip(breaks,breaks[1:])):
            m.box(f'block.{row}.{i}',((a+b)/2,0,.12+row*.235),(b-a-.008,.31,.225),'stone' if i%3 else 'plaster')
    for i in range(5):m.box(f'cap.{i}',(-.8+i*.4,0,1.015),(.398,.32,.07),'linen')
    return m


def well(Model):
    m=Model('structure.well')
    for row in range(3):
        for i in range(8):
            a=(i+(row%2)*.5)*math.tau/8
            # Tangential stone blocks preserve an actual open center.
            obj=m.box(f'stone.{row}.{i}',(0,0,0),(.30,.16,.15),'stone' if i%2 else 'linen')
            from mathutils import Matrix, Vector
            rotation=Matrix.Rotation(a,3,'Z')
            center=Vector((math.sin(a)*.39,-math.cos(a)*.39,.075+row*.15))
            for vertex in obj.data.vertices:vertex.co=rotation @ vertex.co+center
    m.rings('water',[(0,0,.14,.28,.28),(0,0,.15,.28,.28)],'glass',12)
    for x in [-.47,.47]:post(m,f'post.{x}',x,0,1.16,.07)
    m.beam('windlass',(-.52,0,.91),(.52,0,.91),.07,.07,'wood')
    m.beam('rope',(0,0,.89),(0,0,.29),.012,.012,'linen')
    m.rings('bucket',[(0,0,.25,.08,.08),(0,0,.4,.10,.10)],'wood',8)
    m.box('crank',(.57,0,.85),(.05,.04,.18),'dark_wood')
    roof(m,'roof',1.18,.94,1.19,1.55)
    return m


def campfire(Model):
    m=Model('structure.campfire')
    for i in range(8):
        a=i*math.tau/8
        x,y=math.cos(a)*.29,math.sin(a)*.29
        m.rings(f'stone.{i}',[(x,y,0,.065,.06),(x,y,.085,.075,.06),(x,y,.12,.04,.04)],'stone',6)
    for i in [-1,1]:
        m.beam(f'log.{i}',(-.23,i*.10,.075),(.23,-i*.10,.075),.07,.07,'wood')
    for i,(x,y,h) in enumerate([(-.07,0,.31),(.07,.025,.46),(0,-.07,.27)]):
        m.rings(f'flame.{i}',[(x,y,.10,.07,.06),(x+.025,y,h*.68,.055,.04),(x-.025,y,h,.002,.002)],'clay_light' if i%2 else 'linen',5)
    return m


def tent(Model):
    m=Model('structure.tent')
    m.box('groundsheet',(0,0,.015),(1.65,1.85,.03),'sage')
    # An open A-frame with separate canvas planes and rolled-back front flaps.
    roof(m,'canvas',1.65,1.85,.045,1.35,'sage_light')
    m.mesh('rear.canvas',[(-.825,.91,.03),(.825,.91,.03),(0,.91,1.31)],[(0,1,2),(2,1,0)],'sage')
    for y in [-.9,.9]:
        m.beam(f'pole.left.{y}',(-.76,y,.03),(0,y,1.31),.025,.025,'wood')
        m.beam(f'pole.right.{y}',(0,y,1.31),(.76,y,.03),.025,.025,'wood')
    for side in [-1,1]:
        m.mesh(f'flap.{side}',[(side*.78,-.92,.05),(side*.52,-.92,.05),(side*.18,-.92,1.02)],[(0,1,2),(2,1,0)],'sage')
    m.beam('ridge.pole',(0,-.92,1.31),(0,.92,1.31),.026,.026,'wood')
    return m


def crate(Model):
    m=Model('structure.crate')
    m.box('body',(0,0,.25),(.58,.50,.50),'wood')
    for side in [-1,1]:
        for i in range(5):
            m.box(f'front.plank.{side}.{i}',(-.232+i*.116,side*.252,.25),(.11,.025,.48),'wood' if i%2 else 'linen')
        for z in [.07,.43]:m.box(f'band.{side}.{z}',(0,side*.273,z),(.62,.025,.04),'lavender')
    for i in range(5):m.box(f'lid.{i}',(-.232+i*.116,0,.512),(.11,.54,.025),'wood' if i%2 else 'linen')
    for x in [-.282,.282]:
        m.box(f'lid.strap.{x}',(x,0,.53),(.04,.56,.015),'lavender')
        for y in [-.24,.24]:m.box(f'rivet.{x}.{y}',(x,y,.54),(.016,.016,.006),'stone')
    return m


def barrier(Model, kind):
    m=Model('structure.'+kind)
    h={'fence':.75,'guardrail':.60,'railing':.65,'barricade':.72}[kind]
    width=1.7 if kind=='barricade' else 2.0
    color='lavender' if kind=='guardrail' else 'wood'
    for x in [-width/2+.06,width/2-.06]:
        post(m,f'post.{x}',x,0,h,.10,color)
        if kind=='barricade':
            m.box(f'foot.{x}',(x,0,.04),(.14,.44,.08),'wood')
    levels=[.30,h-.08] if kind!='guardrail' else [.41,.53]
    for i,z in enumerate(levels):
        m.box(f'rail.{i}',(0,-.035,z),(width,.07,.075),color)
    if kind=='railing':
        for i in range(1,8):
            m.box(f'spindle.{i}',(-1+i*.25,0,.31),(.026,.04,.52),'wood')
    if kind in ('fence','barricade'):
        m.beam('diagonal',(-width/2+.09,-.075,.24),(width/2-.09,-.075,h-.11),.048,.028,'linen')
    if kind=='barricade':
        for z in [.17,.39,.57]:m.box(f'board.{z}',(0,-.025,z),(width,.08,.14),'wood')
    for side in [-1,1]:
        for z in levels:m.box(f'bolt.{side}.{z}',(side*(width/2-.06),-.076,z),(.022,.012,.022),'stone')
    return m


def lamp_post(Model):
    m=Model('structure.lamp_post')
    m.rings('base',[(0,0,0,.13,.13),(0,0,.15,.10,.10),(0,0,.19,.055,.055)],'stone',8)
    post(m,'shaft',0,0,1.55,.065,'dark_wood')
    m.beam('arm',(0,0,1.5),(.34,0,1.5),.05,.05,'dark_wood')
    m.box('lantern.glass',(.31,0,1.34),(.16,.16,.24),'linen')
    for x in [.22,.40]:
        for y in [-.09,.09]:
            m.box(f'frame.{x}.{y}',(x,y,1.34),(.018,.018,.27),'dark_wood')
    for z in [1.2,1.48]:m.box(f'lantern.cap.{z}',(.31,0,z),(.22,.22,.025),'lavender')
    return m


def signpost(Model):
    m=Model('structure.signpost')
    post(m,'post',0,0,1.2,.07)
    for i,side in enumerate([-1,1]):
        z=.94+i*.19
        vertices=[(-.38,-.06,z-.065),(.30,-.06,z-.065),(.40,-.06,z),(.30,-.06,z+.065),(-.38,-.06,z+.065)]
        vertices=[(x*side,y,z) for x,y,z in vertices]
        m.mesh(f'board.{i}',vertices,[(0,1,2,3,4),(4,3,2,1,0)],'sage' if i else 'linen')
        for j in range(3):
            m.box(f'mark.{i}.{j}',(-.18+j*.1,-.063,z),(.065,.012,.016),'wood')
    return m


def suspension(Model):
    m=Model('structure.suspension')
    # A decorative pylon/cable module, never a replacement for a runtime bridge deck.
    for x in [-.91,.91]:
        post(m,f'pylon.{x}',x,0,2.4,.14,'stone')
        m.box(f'foot.{x}',(x,0,.07),(.18,.28,.14),'stone')
    m.box('crossbar',(0,0,2.25),(2,.16,.12),'stone')
    points=[(-.985,0,2.23),(-.5,0,1.94),(0,0,1.83),(.5,0,1.94),(.985,0,2.23)]
    for i,(a,b) in enumerate(zip(points,points[1:])):
        m.beam(f'cable.{i}',a,b,.022,.022,'dark_wood')
    for i,x in enumerate([-.65,-.32,0,.32,.65]):
        top=1.83+abs(x)*.4
        m.beam(f'hanger.{i}',(x,0,.75),(x,0,top),.014,.014,'dark_wood')
    return m


def recipes(Model):
    result=[lambda f=f:f(Model) for f in [ruin,watchtower,dock,farm,wall,well,campfire,tent,crate,lamp_post,signpost,suspension]]
    result += [lambda k=k:barrier(Model,k) for k in ['fence','barricade','guardrail','railing']]
    return result
