"""Blender MCP entry point. Editable source for the static humanoid/house pilot.

Model recipes use Z-up, forward -Y; the mesh helper reflects Y so Blender's
Y-up exporter produces glTF forward -Z. Reflected face winding is corrected.
Only scenes/datablocks tagged by this generator are replaced on regeneration.
"""
import json
import math
from pathlib import Path
import sys

import bpy
from mathutils import Vector

ROOT = Path(__file__).resolve().parents[3]
if str(ROOT / "crates/shared/tools") not in sys.path:
    sys.path.insert(0, str(ROOT / "crates/shared/tools"))
from glb import bounds, read

VERSION = (5, 2, 2)
OWNER = "terra_blender_pilot"
PALETTE = {
    "skin": (0.83, 0.64, 0.49), "hair": (0.28, 0.23, 0.24),
    "sage": (0.60, 0.72, 0.64), "sage_light": (0.70, 0.80, 0.71),
    "lavender": (0.51, 0.54, 0.65), "linen": (0.91, 0.84, 0.68),
    "clay": (0.78, 0.48, 0.40), "clay_light": (0.87, 0.59, 0.48),
    "wood": (0.63, 0.46, 0.34), "dark_wood": (0.39, 0.31, 0.29),
    "plaster": (0.89, 0.83, 0.69), "stone": (0.63, 0.65, 0.63),
    "glass": (0.39, 0.58, 0.63), "dark": (0.17, 0.22, 0.25),
}


def linear(value):
    return value / 12.92 if value <= .04045 else ((value + .055) / 1.055) ** 2.4


class Model:
    def __init__(self, name):
        self.name = name
        self.scene = bpy.data.scenes.new(name)
        self.scene[OWNER] = True
        self.scene.unit_settings.system = 'METRIC'
        self.scene.unit_settings.scale_length = 1
        bpy.context.window.scene = self.scene
        self.root = self.empty(name)
        self.material = bpy.data.materials.new(name + ".material")
        self.material[OWNER] = True
        self.material.use_nodes = True
        nodes = self.material.node_tree.nodes
        shader = nodes.get("Principled BSDF")
        shader.inputs['Base Color'].default_value = (1, 1, 1, 1)
        shader.inputs['Roughness'].default_value = .95
        shader.inputs['Metallic'].default_value = 0
        color = nodes.new('ShaderNodeVertexColor')
        color.layer_name = "Color"
        self.material.node_tree.links.new(color.outputs['Color'], shader.inputs['Base Color'])

    def empty(self, name, position=(0, 0, 0)):
        obj = bpy.data.objects.new(name, None)
        obj[OWNER] = True
        self.scene.collection.objects.link(obj)
        obj.location = (position[0], -position[1], position[2])
        return obj

    def mesh(self, part, vertices, faces, color):
        name = self.name + "." + part
        mesh = bpy.data.meshes.new(name + ".mesh")
        mesh[OWNER] = True
        mesh.from_pydata([(x,-y,z) for x,y,z in vertices], [], [tuple(reversed(face)) for face in faces])
        mesh.update()
        mesh.materials.append(self.material)
        attr = mesh.color_attributes.new(name="Color", type='FLOAT_COLOR', domain='CORNER')
        rgba = (*map(linear, PALETTE[color]), 1)
        for value in attr.data:
            value.color = rgba
        for face in mesh.polygons:
            face.use_smooth = False
        obj = bpy.data.objects.new(name, mesh)
        obj[OWNER] = True
        self.scene.collection.objects.link(obj)
        obj.parent = self.root
        return obj

    def box(self, name, center, size, color):
        x, y, z = center
        a, b, c = (v / 2 for v in size)
        vertices = [(x-a,y-b,z-c),(x+a,y-b,z-c),(x+a,y+b,z-c),(x-a,y+b,z-c),
                    (x-a,y-b,z+c),(x+a,y-b,z+c),(x+a,y+b,z+c),(x-a,y+b,z+c)]
        return self.mesh(name, vertices, [(0,3,2,1),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7),(4,5,6,7)], color)

    def rings(self, name, rings, color, sides=8):
        # Rings are (center_x, center_y, z, radius_x, radius_y).
        if rings[0][2] > rings[-1][2]:
            rings = list(reversed(rings))
        vertices = [(x+rx*math.cos(i*math.tau/sides), y+ry*math.sin(i*math.tau/sides), z)
                    for x,y,z,rx,ry in rings for i in range(sides)]
        faces = [tuple(reversed(range(sides)))]
        for ring in range(len(rings)-1):
            for i in range(sides):
                j=(i+1)%sides
                faces.append((ring*sides+i,ring*sides+j,(ring+1)*sides+j,(ring+1)*sides+i))
        faces.append(tuple((len(rings)-1)*sides+i for i in range(sides)))
        return self.mesh(name, vertices, faces, color)

    def beam(self, name, start, end, width, depth, color):
        start, end = (Vector((p[0], -p[1], p[2])) for p in [start, end])
        obj = self.box(name, (0,0,0), (width, depth, (end-start).length), color)
        rotation = (end-start).to_track_quat('Z', 'Y')
        center = (start+end)*.5
        for vertex in obj.data.vertices:
            vertex.co = rotation @ vertex.co + center
        obj.data.update()
        return obj


def humanoid():
    m = Model('actor.player')
    for sign, side in [(-1, 'left'), (1, 'right')]:
        x = sign*.068
        m.rings(side+'.boot', [(x,-.020,0,.041,.077),(x,-.023,.043,.041,.077),(x,0,.095,.037,.041)], 'dark_wood')
        m.rings(side+'.leg', [(x,0,.065,.031,.036),(x,0,.26,.038,.042),(sign*.058,0,.48,.051,.051)], 'lavender')
        m.rings(side+'.sleeve', [(sign*.142,0,.755,.050,.050),(sign*.173,0,.64,.043,.043),(sign*.179,-.006,.58,.038,.038)], 'sage')
        m.rings(side+'.forearm', [(sign*.179,-.006,.59,.027,.028),(sign*.183,-.013,.49,.023,.025)], 'skin')
        m.rings(side+'.hand', [(sign*.183,-.013,.51,.027,.023),(sign*.184,-.015,.445,.025,.020)], 'skin')
    m.rings('torso', [(0,0,.44,.116,.065),(0,0,.54,.090,.054),(0,0,.70,.132,.066),(0,0,.77,.115,.057)], 'sage', 8).data.name = 'actor.player.mesh'
    m.rings('belt', [(0,0,.51,.095,.06),(0,0,.535,.095,.06)], 'wood')
    m.box('buckle', (0,-.064,.523), (.036,.012,.030), 'linen')
    m.rings('neck', [(0,0,.75,.035,.033),(0,0,.83,.035,.033)], 'skin')
    m.rings('scarf', [(0,-.004,.76,.070,.047),(0,-.004,.80,.051,.04)], 'clay_light')
    m.box('scarf.tail', (.043,-.064,.716), (.045,.012,.100), 'clay')
    m.rings('head', [(0,-.005,.811,.038,.044),(0,-.005,.85,.063,.059),(0,0,.934,.070,.067),(0,0,.967,.055,.055)], 'skin', 10)
    m.rings('hair', [(0,.009,.935,.072,.067),(0,.008,.976,.060,.056),(0,.005,1.0,.037,.035)], 'hair', 10)
    for sign, side in [(-1,'left'),(1,'right')]:
        m.box(side+'.eye', (sign*.026,-.063,.905), (.012,.006,.008), 'dark')
        m.box(side+'.brow', (sign*.026,-.064,.924), (.023,.005,.006), 'hair')
        m.rings(side+'.ear', [(sign*.064,.002,.872,.010,.014),(sign*.068,.002,.908,.010,.014)], 'skin', 6)
    m.box('nose', (0,-.069,.887), (.016,.021,.023), 'skin')
    m.box('mouth', (0,-.061,.864), (.021,.004,.004), 'wood')
    socket = m.empty('socket.hand', (.184,-.015,.474))
    socket.parent = m.root
    return m


def house():
    m = Model('structure.house')
    # A single storey. Foundation top .14, door lintel underside 1.34:
    # 1.20 m clear height for the approximately 1 m character.
    m.box('foundation', (0,0,.07), (2.80,2.20,.14), 'stone')
    m.box('floor', (0,0,.145), (2.62,2.02,.01), 'wood')
    m.box('rear.wall', (0,.97,.875), (2.60,.12,1.47), 'plaster')
    m.box('left.wall', (-1.24,0,.875), (.12,1.94,1.47), 'plaster')
    m.box('right.wall', (1.24,0,.875), (.12,1.94,1.47), 'plaster')
    # The entrance is centered; two wall sections leave an actual opening.
    m.box('front.left.wall', (-.805,-.97,.875), (.99,.12,1.47), 'plaster').data.name = 'structure.house.mesh'
    m.box('front.right.wall', (.805,-.97,.875), (.99,.12,1.47), 'plaster')
    m.box('front.lintel.wall', (0,-.97,1.49), (.62,.12,.24), 'plaster')
    for i,x in enumerate([-1.27,-.34,.34,1.27]):
        m.box(f'front.post.{i}', (x,-1.048,.87), (.072,.085,1.46), 'wood')
    m.box('door.lintel', (0,-1.048,1.38), (.75,.095,.08), 'wood')
    m.box('door.recess', (0,-.906,.74), (.60,.035,1.20), 'dark_wood')
    for i in range(5):
        m.box(f'door.plank.{i}', (-.232+i*.116,-.932,.728), (.109,.026,1.17), 'sage' if i%2 else 'sage_light')
    for z in [.35,1.08]:
        m.box(f'door.strap.{z}', (0,-.950,z), (.56,.015,.025), 'dark_wood')
    m.box('door.handle', (.205,-.97,.75), (.025,.025,.055), 'wood')
    # Foundation blocks keep the silhouettes simple but explain construction.
    for i in range(9):
        m.box(f'foundation.block.{i}', (-1.24+i*.31,-1.11,.07), (.293,.035,.10), 'stone' if i%2 else 'linen')
    for side in [-1,1]:
        x = side*.80
        m.box(f'window.{side}.recess', (x,-1.036,.98), (.51,.020,.54), 'dark_wood')
        m.box(f'window.{side}.glass', (x,-1.05,.99), (.43,.015,.46), 'glass')
        for sx in [-1,1]:
            m.box(f'window.{side}.jamb.{sx}', (x+sx*.237,-1.071,.99), (.037,.044,.54), 'linen')
        for z in [.74,1.24]:
            m.box(f'window.{side}.frame.{z}', (x,-1.072,z), (.51,.045,.037), 'linen')
        m.box(f'window.{side}.mullion', (x,-1.082,.99), (.025,.025,.49), 'linen')
        m.box(f'window.{side}.crossbar', (x,-1.084,.99), (.46,.025,.025), 'linen')
        m.box(f'window.{side}.sill', (x,-1.092,.70), (.60,.15,.055), 'wood')
        for sx in [-1,1]:
            m.box(f'window.{side}.shutter.{sx}', (x+sx*.325,-1.062,.99), (.12,.035,.52), 'sage')
    # Solid gables, then two thick roof planes and a few readable courses.
    for i,y in enumerate([-.976,.976]):
        m.mesh(f'gable.{i}', [(-1.30,y,1.59),(1.30,y,1.59),(0,y,2.32)], [(0,1,2)] if i else [(2,1,0)], 'plaster')
        m.beam(f'gable.{i}.left', (-1.31,y,1.61),(0,y,2.34), .06,.10,'wood')
        m.beam(f'gable.{i}.right', (0,y,2.34),(1.31,y,1.61), .06,.10,'wood')
        m.box(f'gable.{i}.tie', (0,y,1.60),(2.70,.12,.085),'wood')
        m.box(f'gable.{i}.king', (0,y,1.98),(.07,.10,.71),'wood')
    for side in [-1,1]:
        # Sloped roof slabs: explicit thickness, front -Y overhang.
        x = side*1.49
        vertices = [(0,-1.22,2.38),(x,-1.22,1.56),(x,1.22,1.56),(0,1.22,2.38),
                    (0,-1.22,2.46),(x,-1.22,1.64),(x,1.22,1.64),(0,1.22,2.46)]
        faces = [(0,3,2,1),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7),(4,5,6,7)]
        if side < 0: faces=[tuple(reversed(f)) for f in faces]
        m.mesh(f'roof.{side}', vertices, faces, 'clay')
        for row in range(1,5):
            xx=side*1.49*row/5
            zz=2.46-.82*row/5
            m.box(f'roof.{side}.course.{row}', (xx,0,zz+.012),(.033,2.45,.026),'clay_light')
        m.box(f'eave.{side}', (side*1.48,0,1.595),(.07,2.51,.10),'wood')
    m.box('ridge', (0,0,2.46),(.12,2.50,.07),'clay_light')
    return m


def clear_owned():
    for scene in list(bpy.data.scenes):
        if scene.get(OWNER):
            bpy.data.scenes.remove(scene)
    for container in [bpy.data.objects, bpy.data.meshes, bpy.data.materials]:
        for item in list(container):
            if item.get(OWNER):
                container.remove(item, do_unlink=True)


def generate(out_dir):
    if bpy.app.version != VERSION:
        raise RuntimeError(f"Pinned Blender version is {VERSION}; running {bpy.app.version}")
    output = Path(out_dir).resolve()
    baseline = ROOT / 'crates/main/assets/models'
    if output == baseline:
        raise ValueError('Candidate output must not be the baseline catalog directory')
    original = bpy.context.window.scene
    # Regenerate only our own scratch scenes; never clear the user's active scene.
    if original.get(OWNER):
        original = next((s for s in bpy.data.scenes if not s.get(OWNER)), None)
        if original is None:
            original = bpy.data.scenes.new('Scene')
        bpy.context.window.scene = original
    clear_owned()
    output.mkdir(parents=True, exist_ok=True)
    manifest = {'blender': '.'.join(map(str, VERSION)), 'seed': 0, 'assets': {}}
    try:
        for build in [humanoid, house]:
            model = build()
            bpy.context.view_layer.update()
            filename = model.name + '.glb'
            bpy.ops.export_scene.gltf(filepath=str(output / filename), export_format='GLB',
                use_active_scene=True, export_yup=True, export_apply=True,
                export_normals=True, export_texcoords=False, export_materials='EXPORT',
                export_vertex_color='ACTIVE', export_all_vertex_colors=False,
                export_animations=False, export_extras=False, export_cameras=False,
                export_lights=False)
            doc = read(output / filename)
            extent = bounds(doc, doc['scenes'][0])
            manifest['assets'][model.name] = {'file': filename, 'scene': model.name,
                'bounds': extent, 'dimensions': [extent[i+3]-extent[i] for i in range(3)],
                'runtime_scale': [1,1,1], 'collider': None}
            if model.name == 'structure.house':
                manifest['assets'][model.name]['door_clear_height'] = 1.2
        (output / 'manifest.json').write_text(json.dumps(manifest, indent=2, sort_keys=True) + '\n')
        return manifest
    finally:
        bpy.context.window.scene = original
