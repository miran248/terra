"""Meter-scale scenery recipes. Coordinates are Blender Z-up; no external dependencies."""
import math


def crown(m, name, x, y, z, radius, color='sage', height=None):
    h = height or radius * 1.4
    return m.rings(name, [(x,y,z-h*.5,radius*.36,radius*.32),
        (x-.03,y,z-h*.12,radius,radius*.86),
        (x+.03,y,z+h*.25,radius*.76,radius*.7),
        (x,y,z+h*.5,radius*.15,radius*.13)], color, 7)


def leaf(m, name, start, end, width, color='sage'):
    """A closed, folded lanceolate leaf; readable from both sides without alpha."""
    a = start
    b = end
    dx, dy = b[0]-a[0], b[1]-a[1]
    length = math.hypot(dx,dy) or 1
    sx, sy = -dy/length*width, dx/length*width
    mid = tuple(a[i]+(b[i]-a[i])*.55 for i in range(3))
    vertices = [a, (mid[0]+sx,mid[1]+sy,mid[2]), b,
                (mid[0]-sx,mid[1]-sy,mid[2]),
                (mid[0],mid[1],mid[2]+width*.3),
                (mid[0],mid[1],mid[2]-width*.08)]
    return m.mesh(name, vertices, [(0,1,4),(1,2,4),(2,3,4),(3,0,4),
        (1,0,5),(2,1,5),(3,2,5),(0,3,5)], color)


def radial(angle, radius, z):
    return (math.cos(angle)*radius,math.sin(angle)*radius,z)


def tree(Model, variant):
    m = Model(f'scenery.tree.{variant}')
    height = {1:3.1,2:2.9,3:2.7,4:2.5,5:2.3,6:2.63}[variant]
    m.rings('trunk', [(0,0,0,.14,.13),(.015,0,.25,.10,.09),
        (-.03,.01,height*.6,.065,.06),(.06,0,height*.87,.035,.03)], 'wood', 7)
    for i in range(5):
        m.beam(f'root.{i}', (0,0,.13), radial(i*math.tau/5,.23,.025), .05,.035,'wood')
    if variant == 2:
        for i,(z,r) in enumerate([(1.30,.72),(1.78,.61),(2.21,.46),(2.58,.28)]):
            m.rings(f'needle.tier.{i}', [(0,0,z,r*.85,r*.85),(0,0,z+.08,r,r),
                (.02,0,z+.65,.015,.015)], 'sage' if i%2 else 'sage_light', 9)
    elif variant == 3:
        for i in range(9):
            a=i*math.tau/9
            start=(.06,0,2.32)
            mid=radial(a,.44,2.58)
            end=radial(a,1.02,2.1+(i%2)*.12)
            m.beam(f'frond.stem.{i}',start,mid,.025,.025,'sage')
            leaf(m,f'frond.{i}',mid,end,.16,'sage_light' if i%2 else 'sage')
        for i in range(3):
            crown(m,f'coconut.{i}', .06+(i-1)*.075,-.07,2.28,.07,'wood')
        for i in range(11):
            z=.25+i*.17
            t=min(1,(z-.25)/(height*.6-.25))
            x=.015-.045*t
            radius=.1-.035*t
            m.rings(f'trunk.ring.{i}',[(x,.01*t,z,radius+.002,radius-.008),
                (x,.01*t,z+.018,radius+.003,radius-.007)],'dark_wood',7)
    else:
        spread={1:.66,4:.77,5:.53,6:.54}[variant]
        for i in range(5 if variant != 5 else 7):
            a=i*2.4
            x,y,_=radial(a,spread*(.55 if i==0 else 1),0)
            z=height-.38-(i%3)*.17
            m.beam(f'branch.{i}',(0,0,1.22+i*.06),(x,y,z),.06,.05,'wood')
            color=('clay_light' if i%2 else 'linen') if variant==6 else ('sage_light' if i%2 else 'sage')
            crown(m,f'canopy.{i}',x,y,z,.24 if variant==5 else spread*.82,color,
                  .47 if variant==5 else .75)
    return m


def shrubs(Model, name):
    m=Model(name)
    berry=name=='scenery.berry'
    variant=name.endswith('.1')
    for i in range(5):
        a=i*2.4
        x,y,_=radial(a,.22,0)
        m.beam(f'branch.{i}',(0,0,0),(x,y,.35),.026,.022,'wood')
        crown(m,f'foliage.{i}',x,y,.3+(i%2)*.1,.24,'sage_light' if variant or i%2 else 'sage')
        if berry:
            for j in range(3):
                crown(m,f'berry.{i}.{j}',x+(j-1)*.055,y-.18,.37+(j%2)*.045,.037,'clay',.06)
    return m


def ground_flora(Model, kind):
    m=Model('scenery.'+kind)
    if kind in ('grass','reed','cattail','seaweed','kelp'):
        for i in range(7 if kind=='grass' else 5):
            a=i*2.4
            x,y,_=radial(a,.11,0)
            h={'grass':.28,'reed':.74,'cattail':.85,'seaweed':.52,'kelp':1.25}[kind]*(.7+(i%3)*.15)
            if kind in ('reed','cattail'):
                m.beam(f'stem.{i}',(x,y,0),(x+.035,y,h),.014,.014,'sage')
                leaf(m,f'leaf.{i}',(x,y,h*.15),(x+.24*math.cos(a),y+.24*math.sin(a),h*.8),.035)
                if kind=='cattail':
                    m.rings(f'head.{i}',[(x+.035,y,h*.76,.024,.024),(x+.035,y,h,.024,.024)],'wood',7)
            else:
                leaf(m,f'blade.{i}',(x,y,0),(x+.17*math.cos(a),y+.17*math.sin(a),h),.033 if kind=='grass' else .07,
                     'sage_light' if i%2 else 'sage')
                if kind=='kelp':
                    for j in range(3):
                        z=h*(.25+j*.2)
                        leaf(m,f'frond.{i}.{j}',(x,y,z),(x+(-1)**j*.25,y+.05,z+.20),.075)
    elif kind=='fern':
        for i in range(7):
            a=i*math.tau/7
            end=radial(a,.43,.23+(i%2)*.1)
            m.beam(f'rachis.{i}',(0,0,0),end,.013,.013,'sage')
            for j in range(1,6):
                t=j/6
                base=tuple(v*t for v in end)
                for side in [-1,1]:
                    tip=(base[0]+math.cos(a+side*.8)*.12*(1-t*.65),base[1]+math.sin(a+side*.8)*.12*(1-t*.65),base[2]+.04)
                    leaf(m,f'pinna.{i}.{j}.{side}',base,tip,.024,'sage_light' if j%2 else 'sage')
    elif kind=='flower':
        for i in range(3):
            x,y,_=radial(i*2.4,.10,0); z=.27+i*.065
            m.beam(f'stem.{i}',(x,y,0),(x,y,z),.012,.012,'sage')
            leaf(m,f'leaf.{i}',(x,y,.07),(x+.13,y,.17),.04)
            for j in range(6):
                a=j*math.tau/6
                leaf(m,f'petal.{i}.{j}',(x,y,z),(x+math.cos(a)*.095,y+math.sin(a)*.095,z+.025),.045,'clay_light' if i%2 else 'lavender')
            crown(m,f'heart.{i}',x,y,z+.025,.025,'linen',.025)
    elif kind=='vine':
        for i in range(8):
            a=(i/7-.5)*2.5
            start=(math.sin(a)*.3,.06*math.cos(a),i*.085)
            end=(math.sin(a+.3)*.3,.06*math.cos(a+.3),(i+1)*.085)
            m.beam(f'stem.{i}',start,end,.018,.018,'wood')
            leaf(m,f'leaf.{i}',end,(end[0]+(-1)**i*.17,end[1]-.05,end[2]+.08),.06)
    elif kind=='lilypad':
        # Notched disk with folded rim and a small bloom.
        vertices=[(0,0,.015)]+[(math.cos(a)*.28,math.sin(a)*.24,.025) for a in [(.25+i*(math.tau-.5)/12) for i in range(13)]]
        m.mesh('pad',vertices,[(0,i,i+1) for i in range(1,13)]+[(0,i+1,i) for i in range(1,13)],'sage')
        for i in range(7):
            a=i*math.tau/7
            leaf(m,f'bloom.{i}',(0,0,.03),radial(a,.09,.11),.034,'clay_light')
    return m


def cactus(Model, variant):
    m=Model(f'scenery.cactus.{variant}')
    if variant==0:
        m.rings('trunk',[(0,0,0,.10,.10),(0,0,1.05,.11,.11),(0,0,1.14,.025,.025)],'sage',9)
        for i,side in enumerate([-1,1]):
            m.beam(f'arm.{i}',(0,0,.40+i*.15),(side*.24,0,.45+i*.15),.095,.095,'sage')
            m.rings(f'finger.{i}',[(side*.24,0,.41+i*.15,.055,.055),(side*.24,0,.81+i*.12,.055,.055),(side*.24,0,.86+i*.12,.018,.018)],'sage_light',7)
    else:
        m.rings('barrel',[(0,0,0,.16,.15),(0,0,.13,.26,.25),
            (0,0,.44,.26,.25),(0,0,.57,.17,.16),(0,0,.60,.07,.07)],'sage_light',12)
        for i in range(12):
            a=i*math.tau/12
            for j in range(3):
                x,y,z=radial(a,.264,.17+j*.12)
                m.box(f'areole.{i}.{j}',(x,y,z),(.012,.012,.02),'linen')
        crown(m,'blossom',0,0,.625,.075,'clay_light',.07)
    if variant==0:
        for i in range(8):
            z=.1+i*.1
            m.box(f'areole.{i}',(.045*(-1)**i,-.102,z),(.009,.007,.012),'linen')
        crown(m,'blossom',0,0,1.15,.06,'clay_light',.07)
    return m


def wood(Model, kind, variant=0):
    name='scenery.'+kind+(f'.{variant}' if kind=='dead_tree' else '')
    m=Model(name)
    if kind=='log':
        # Rings built on Z, then rotated into a horizontal X-axis log.
        from mathutils import Matrix, Vector
        rotation=Matrix.Rotation(math.pi/2,3,'Y')
        for part,lo,hi,radius,color in [('bark',-.55,.55,.13,'wood'),
            ('cut.left',-.556,-.55,.112,'linen'),('cut.right',.55,.556,.112,'linen'),
            ('heart.left',-.56,-.556,.061,'wood'),('heart.right',.556,.56,.061,'wood')]:
            obj=m.rings(part,[(0,0,lo,radius,radius*.92),(0,0,hi,radius,radius*.92)],color,9)
            for vertex in obj.data.vertices:
                vertex.co=rotation @ vertex.co + Vector((0,0,.13))
        for i in range(5):
            a=i*math.pi/4
            y=math.cos(a)*.124; z=.13+math.sin(a)*.12
            m.beam(f'bark.ridge.{i}',(-.5,y,z),(.5,y,z),.009,.009,'dark_wood')
        m.beam('broken.branch',(.08,0,.17),(.19,.07,.4),.06,.05,'wood')
    else:
        h=.32 if kind=='stump' else (1.65 if variant==0 else 1.35)
        m.rings('trunk',[(0,0,0,.19,.17),(.01,0,.13,.14,.12),(.035,.015,h,.075,.065)],'wood',8)
        m.rings('cut',[(.035,.015,h,.071,.061),(.035,.015,h+.006,.071,.061)],'linen',8)
        for i in range(5):
            m.beam(f'root.{i}',(0,0,.1),radial(i*2.4,.26,.022),.07,.04,'wood')
        if kind=='dead_tree':
            for i in range(4):
                a=i*2.4
                end=radial(a,.45,h*.65+i*.1)
                m.beam(f'branch.{i}',(0,0,h*.4+i*.1),end,.065,.055,'wood')
                m.beam(f'twig.{i}',end,(end[0]*1.13,end[1]*1.13,end[2]+.19),.032,.028,'wood')
                if variant==1:
                    m.beam(f'snow.branch.{i}',(0,0,h*.4+i*.1+.035),
                        (end[0],end[1],end[2]+.035),.07,.025,'plaster')
            if variant==1:
                crown(m,'snow.cap',.035,.015,h+.025,.085,'plaster',.05)
    return m


def marine(Model, kind):
    m=Model('scenery.'+kind)
    if kind=='coral':
        for i in range(5):
            a=i*2.4; x,y,_=radial(a,.17,0); h=.25+i*.05
            m.beam(f'stalk.{i}',(x,y,0),(x,y,h),.055,.05,'clay_light')
            for j in [-1,1]:
                m.beam(f'fork.{i}.{j}',(x,y,h*.65),(x+j*.12,y+.04,h+.10),.035,.035,'clay')
    elif kind=='anemone':
        crown(m,'foot',0,0,.07,.18,'clay',.14)
        for i in range(12):
            a=i*math.tau/12
            m.beam(f'tentacle.{i}',radial(a,.1,.07),radial(a+.2,.20,.23+(i%3)*.025),.024,.024,'lavender')
            crown(m,f'tip.{i}',*radial(a+.2,.20,.23+(i%3)*.025),.022,'clay_light',.035)
    elif kind=='starfish':
        for i in range(5):
            leaf(m,f'arm.{i}',(0,0,.028),radial(i*math.tau/5,.23,.015),.066,'clay_light')
        crown(m,'center',0,0,.033,.07,'clay',.065)
    elif kind=='shell':
        for i in range(9):
            a=(i/8-.5)*2.5
            leaf(m,f'rib.{i}',(0,-.12,.008),(math.sin(a)*.20,math.cos(a)*.22,.035),.039,'linen' if i%2 else 'clay_light')
    return m


def other(Model, kind):
    m=Model('scenery.'+kind)
    if kind=='rock.1':
        m.rings('mass',[(0,0,0,.32,.3),(-.05,0,.18,.39,.28),(.015,.01,.48,.26,.21),(-.015,0,.60,.1,.09)],'lavender',7)
        m.rings('vein',[(0,-.23,.16,.16,.025),(.035,-.19,.34,.12,.025)],'stone',5)
    elif kind=='mushroom':
        for i,(x,y,h) in enumerate([(-.08,0,.27),(.12,.045,.17),(.06,-.11,.13)]):
            m.rings(f'stem.{i}',[(x,y,0,.025,.025),(x,y,h*.75,.022,.022)],'linen',7)
            m.rings(f'cap.{i}',[(x,y,h*.68,h*.44,h*.44),(x,y,h*.82,h*.48,h*.48),(x+.01,y,h,h*.14,h*.14)],'clay_light' if i%2 else 'clay',9)
            for j in range(3):
                a=j*2.4
                crown(m,f'spot.{i}.{j}',x+math.cos(a)*h*.22,y+math.sin(a)*h*.22,h*.94,.013,'linen',.008)
    elif kind=='tumbleweed':
        for i in range(9):
            a=i*math.tau/9
            for j in range(5):
                t=j*math.pi/5; u=(j+1)*math.pi/5
                def p(t): return (math.sin(t)*math.cos(a)*.25,math.sin(t)*math.sin(a)*.25,.25-math.cos(t)*.24)
                m.beam(f'twig.{i}.{j}',p(t),p(u),.013,.013,'linen' if i%2 else 'wood')
    elif kind=='skull':
        crown(m,'cranium',0,.025,.105,.11,'linen',.18)
        m.box('muzzle',(0,-.10,.055),(.11,.14,.065),'linen')
        for side in [-1,1]:
            m.box(f'eye.{side}',(side*.067,-.057,.135),(.045,.012,.038),'dark_wood')
            m.beam(f'horn.{side}',(side*.075,.025,.17),(side*.20,.05,.25),.035,.025,'linen')
        for i in range(4):
            m.box(f'tooth.{i}',(-.039+i*.026,-.17,.028),(.02,.023,.026),'plaster')
    elif kind=='snowdrift':
        m.rings('bank',[(0,0,0,.62,.34),(-.08,0,.12,.53,.30),(-.12,.025,.25,.3,.18),(-.1,.03,.31,.07,.05)],'plaster',9)
    elif kind=='icicle':
        for i in range(4):
            x=(i-1.5)*.085; h=.5-i*.08
            m.rings(f'ice.{i}',[(x,0,0,.012,.01),(x,0,h,.06,.05)],'glass' if i%2 else 'plaster',6)
    elif kind=='snowman':
        for i,(z,r) in enumerate([(.23,.24),(.55,.18),(.79,.13)]):
            crown(m,f'snow.{i}',0,0,z,r,'plaster',r*2)
        m.box('hat.brim',(0,0,.91),(.32,.28,.025),'lavender')
        m.box('hat',(0,0,1.005),(.20,.19,.17),'lavender')
        for side in [-1,1]:
            m.box(f'eye.{side}',(side*.045,-.108,.82),(.02,.013,.021),'dark')
            m.beam(f'arm.{side}',(side*.12,0,.55),(side*.37,0,.65),.025,.025,'wood')
        m.beam('nose',(0,-.10,.78),(0,-.23,.78),.038,.038,'clay_light')
        for z in [.48,.57,.65]:
            m.box(f'button.{z}',(0,-.15,z),(.024,.02,.024),'dark_wood')
        m.rings('scarf',[(0,0,.675,.123,.12),(0,0,.71,.125,.12)],'clay',8)
        m.box('scarf.tail',(.08,-.16,.59),(.06,.023,.22),'clay_light')
    return m


def recipes(Model):
    result=[lambda v=v: tree(Model,v) for v in range(1,7)]
    result += [lambda n=n: shrubs(Model,n) for n in ['scenery.bush.0','scenery.bush.1','scenery.berry']]
    result += [lambda k=k: ground_flora(Model,k) for k in ['grass','reed','cattail','seaweed','kelp','fern','flower','vine','lilypad']]
    result += [lambda v=v: cactus(Model,v) for v in range(2)]
    result += [lambda k=k,v=v: wood(Model,k,v) for k,v in [('log',0),('stump',0),('dead_tree',0),('dead_tree',1)]]
    result += [lambda k=k: marine(Model,k) for k in ['coral','anemone','starfish','shell']]
    result += [lambda k=k: other(Model,k) for k in ['rock.1','mushroom','tumbleweed','skull','snowdrift','icicle','snowman']]
    return result
