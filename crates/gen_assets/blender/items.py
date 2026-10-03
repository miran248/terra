"""Small, readable scrap and weapons at the approved one-meter character scale.

Authoring uses Z for the weapon's long axis. The shared grip transform turns that
axis forward in the hand; dropped scenes retain the catalog's grounded pivot.
"""
import math
from functools import partial


def coil(m, name, center, radius, turns, pitch, thickness, color):
    x, y, z = center
    points = [(x+radius*math.cos(t*math.tau/16), y+radius*math.sin(t*math.tau/16), z+pitch*t/16)
              for t in range(turns*16+1)]
    for i, (a, b) in enumerate(zip(points, points[1:])):
        m.beam(f'{name}.{i}', a, b, thickness, thickness, color)


def material(Model, kind):
    m = Model('material.'+kind)
    if kind == 'metal':
        # A short angle offcut, flat plate and visible rivets.
        m.box('plate', (0,0,.018), (.24,.16,.036), 'lavender')
        m.box('angle.foot', (.02,.015,.046), (.22,.064,.02), 'stone')
        m.box('angle.web', (.02,.042,.087), (.22,.018,.10), 'lavender')
        for x in [-.065,.07]:
            m.rings('rivet.'+str(x), [(x,-.044,.036,.012,.012),(x,-.044,.046,.009,.009)], 'linen', 6)
    elif kind == 'wood':
        for i, (x,y,z) in enumerate([(-.06,0,.034),(.055,.014,.040),(0,.005,.098)]):
            m.box(f'plank.{i}', (x,y,z), (.095,.29,.064), 'wood' if i%2 else 'dark_wood')
            for side in [-1,1]:
                m.box(f'endgrain.{i}.{side}', (x,y+side*.146,z), (.073,.003,.042), 'linen')
        for y in [-.084,.084]:
            m.box('binding.top.'+str(y), (0,y,.134), (.22,.018,.009), 'sage')
            for x in [-.11,.11]:
                m.box(f'binding.side.{y}.{x}', (x,y,.071), (.012,.018,.135), 'sage')
    elif kind == 'rope':
        coil(m, 'coil', (0,0,.012), .09, 3, .023, .023, 'linen')
        m.beam('loose.end', (.09,0,.083), (.15,-.045,.03), .021,.021,'wood')
        m.beam('tie', (-.04,-.105,.04), (.04,-.105,.055), .012,.012,'wood')
    else:
        for i in range(3):
            m.box(f'fold.{i}', (i*.008,0,.016+i*.027), (.21-i*.008,.16,.03), 'linen' if i%2 else 'lavender')
        m.box('flap', (.065,0,.088), (.09,.163,.012), 'linen')
        for y in [-.061,.061]:
            m.box('hem.'+str(y), (0,y,.097), (.18,.006,.003), 'sage')
    return m


def weapon(Model, kind):
    m = Model('weapon.'+kind)
    if kind == 'spear':
        m.rings('shaft', [(0,0,0,.016,.016),(0,0,1.02,.012,.012)], 'wood', 8)
        m.rings('butt', [(0,0,0,.018,.018),(0,0,.05,.018,.018)], 'dark_wood', 8)
        for i in range(9):
            z=.48+i*.019
            m.rings(f'grip.wrap.{i}', [(0,0,z,.018,.018),(0,0,z+.011,.018,.018)], 'linen', 8)
        m.rings('ferrule', [(0,0,.97,.021,.021),(0,0,1.03,.018,.018)], 'lavender', 8)
        m.mesh('head', [(0,0,1.23),(-.055,0,1.065),(0,-.012,1.06),(.055,0,1.065),(0,.012,1.06),(0,0,1.01)],
               [(0,1,2),(0,2,3),(0,3,4),(0,4,1),(5,2,1),(5,3,2),(5,4,3),(5,1,4)], 'stone')
    elif kind in ('pistol','rifle'):
        rifle = kind == 'rifle'
        length = .68 if rifle else .25
        # Reflect the finished gun below so its grip hangs below the forward barrel.
        m.box('receiver', (0,0,length*.51), (.047,.05,length*.30), 'lavender')
        m.rings('barrel', [(0,-.007,length*.59,.016,.016),(0,-.007,length,.016,.016)], 'dark', 8)
        m.rings('muzzle', [(0,-.007,length-.012,.019,.019),(0,-.007,length,.019,.019)], 'stone', 8)
        m.beam('grip', (0,.005,length*.43), (0,.083,length*.33), .039,.041,'wood')
        m.beam('guard.front', (0,.012,length*.61), (0,.057,length*.60), .012,.01,'dark')
        m.beam('guard.bottom', (0,.057,length*.60), (0,.063,length*.42), .012,.01,'dark')
        m.box('trigger', (0,.029,length*.52), (.009,.027,.009), 'stone')
        m.box('front.sight', (0,-.029,length*.90), (.011,.012,.012), 'stone')
        m.box('rear.sight', (0,-.032,length*.40), (.032,.012,.013), 'stone')
        if rifle:
            m.beam('stock.neck', (0,.004,.27), (0,.022,.09), .046,.047,'wood')
            m.box('stock.butt', (0,.035,.043), (.048,.108,.086), 'wood')
            m.box('butt.plate', (0,.035,.008), (.052,.112,.016), 'dark_wood')
            m.box('forestock', (0,.016,.475), (.042,.040,.20), 'wood')
            for z in [.40,.51]:
                m.box('band.'+str(z), (0,.016,z), (.046,.044,.012), 'dark_wood')
        else:
            m.box('slide', (0,-.018,.095), (.052,.045,.18), 'lavender')
            for z in [.032,.045,.058]:
                m.box('slide.ridge.'+str(z), (0,-.043,z), (.053,.004,.005), 'stone')
    else:
        # A leather pouch suspended from two cords, distinct from a slingshot.
        m.rings('pouch', [(0,0,.015,.027,.012),(0,0,.055,.053,.023),(0,0,.11,.036,.014)], 'wood', 8)
        for side in [-1,1]:
            points=[(side*.044,0,.07),(side*.066,0,.17),(side*.025,0,.28),(side*.015,0,.35)]
            for i,(a,b) in enumerate(zip(points,points[1:])):
                m.beam(f'cord.{side}.{i}',a,b,.010,.010,'linen')
        m.beam('finger.loop.left', (-.015,0,.35), (-.016,0,.385), .009,.009,'linen')
        m.beam('finger.loop.right', (.015,0,.35), (.016,0,.385), .009,.009,'linen')
        m.beam('finger.loop.end', (-.016,0,.385), (.016,0,.385), .009,.009,'linen')
    if kind in ('pistol','rifle'):
        for obj in m.scene.objects:
            if obj.type == 'MESH':
                for vertex in obj.data.vertices:
                    vertex.co.y *= -1
                for polygon in obj.data.polygons:
                    polygon.flip()
    return m


def recipes(Model):
    return [partial(material, Model, kind) for kind in ['metal','wood','rope','cloth']] + [
        partial(weapon, Model, kind) for kind in ['spear','pistol','sling','rifle']]
