"""Render review sheets through the running Blender MCP; never modifies source recipes."""
import argparse
from pathlib import Path


def render(output, family='scenery'):
    import bpy
    import math
    from mathutils import Vector, Matrix
    owner = 'terra_blender_pilot'
    original = bpy.context.window.scene
    scene = bpy.data.scenes.new('terra.scenery.review')
    scene[owner] = True
    owned_data=[]
    def remove_review_object(obj):
        data=obj.data
        bpy.data.objects.remove(obj,do_unlink=True)
        if data is not None and data.users==0:
            if isinstance(data,bpy.types.Curve):bpy.data.curves.remove(data)
            elif isinstance(data,bpy.types.Mesh):bpy.data.meshes.remove(data)
    try:
        bpy.context.window.scene = scene
        scene.render.engine = 'CYCLES'
        scene.cycles.samples = 16
        scene.cycles.use_denoising = True
        scene.render.resolution_percentage = 100
        scene.render.image_settings.file_format = 'PNG'
        scene.view_settings.view_transform = 'Standard'
        world = bpy.data.worlds.new('terra.review.world')
        owned_data.append((bpy.data.worlds,world))
        scene.world = world
        world.use_nodes = True
        world.node_tree.nodes['Background'].inputs[0].default_value = (.72,.77,.83,1)
        world.node_tree.nodes['Background'].inputs[1].default_value = .7
        ink = bpy.data.materials.new('terra.review.ink')
        owned_data.append((bpy.data.materials,ink))
        ink.diffuse_color = (.075,.1,.13,1)
        ink.use_nodes=True
        ink.node_tree.nodes['Principled BSDF'].inputs['Base Color'].default_value=(.075,.1,.13,1)
        key=bpy.data.lights.new('terra.review.key','AREA')
        owned_data.append((bpy.data.lights,key))
        key.energy=2500
        key.shape='DISK';key.size=12
        lamp=bpy.data.objects.new(key.name,key);scene.collection.objects.link(lamp)
        lamp.location=(-5,-8,14)
        camera_data=bpy.data.cameras.new('terra.review.camera')
        owned_data.append((bpy.data.cameras,camera_data))
        camera=bpy.data.objects.new(camera_data.name,camera_data);scene.collection.objects.link(camera)
        camera.location=(0,-30,15)
        camera.rotation_euler=(-camera.location).to_track_quat('-Z','Y').to_euler()
        camera_data.type='ORTHO';scene.camera=camera
        up=camera.rotation_euler.to_quaternion() @ Vector((0,1,0))
        right=Vector((1,0,0))
        prefixes = {'scenery': ('scenery.',), 'structures': ('structure.',), 'items': ('material.', 'weapon.'), 'actors': ('actor.',)}[family]
        sources=sorted((s for s in bpy.data.scenes if s.name.startswith(prefixes) and s.get(owner)), key=lambda s:s.name)
        tall=[s for s in sources if s.name.startswith(('scenery.tree.','scenery.dead_tree.'))]
        ground=[s for s in sources if s not in tall]
        groups=[(tall,'trees',3,4.2),(ground,'ground',5,1.7)]
        if family=='structures':
            buildings=[s for s in sources if s.name.split('.')[-1] in ['house','watchtower','well','tent','ruin','suspension']]
            props=[s for s in sources if s not in buildings]
            groups=[(buildings,'buildings',3,4.8),(props,'props',3,3.6)]
        if family=='items':
            groups=[([s for s in sources if s.name.startswith('material.')],'materials',4,.7),
                    ([s for s in sources if s.name.startswith('weapon.')],'weapons',5,1.55)]
        elif family=='actors':
            groups=[(sources,'actors',3,1.55)]
        output=Path(output);output.mkdir(parents=True,exist_ok=True)
        def label(body,position,size):
            curve=bpy.data.curves.new('review.label','FONT');curve.body=body;curve.size=size
            obj=bpy.data.objects.new('review.label',curve);scene.collection.objects.link(obj)
            obj.location=position;obj.rotation_euler=camera.rotation_euler
            curve.materials.append(ink)
        for group,title,columns,cell in groups:
            rows=math.ceil(len(group)/columns)
            width=columns*cell
            height=rows*cell*1.03+.8
            scene.render.resolution_x=2400
            scene.render.resolution_y=round(2400*height/width)
            camera_data.ortho_scale=max(width,height)*1.06
            before=set(scene.objects)
            reference = .25 if title=='materials' else 1
            label('TERRA / '+title.upper()+f' / meters / dark bars = {reference:g} m',right*(-width/2+.13)+up*(height/2-.3),cell*.055)
            for index,source in enumerate(group):
                col=index%columns;row=index//columns
                origin=right*((col-(columns-1)/2)*cell)+up*((rows/2-row-.85)*cell)
                bpy.context.window.scene=source
                bpy.context.view_layer.update()
                points=[]
                for obj in source.objects:
                    if obj.type!='MESH':continue
                    clone=obj.copy();clone.data=obj.data
                    scene.collection.objects.link(clone)
                    clone.parent=None
                    clone.modifiers.clear()
                    angle = math.pi + (1.15 if source.name in ('weapon.pistol','weapon.rifle') else 0)
                    clone.matrix_world=Matrix.Rotation(angle,4,'Z') @ obj.matrix_world
                    clone.location+=origin
                    points += [obj.matrix_world @ v.co for v in obj.data.vertices]
                bpy.context.window.scene=scene
                # Shared scale within a sheet, actual meter labels, no per-object normalization.
                dims=[max(p[i] for p in points)-min(p[i] for p in points) for i in range(3)]
                label(source.name.split('.',1)[1],origin+right*(-cell*.45)-up*(cell*(.22 if family=='structures' else .14)),cell*.061)
                label(f'{dims[0]:.2f} x {dims[2]:.2f} x {dims[1]:.2f} m',origin+right*(-cell*.45)-up*(cell*(.29 if family=='structures' else .22)),cell*.043)
                bpy.ops.mesh.primitive_cube_add(size=1,location=origin+right*(-cell*.44)+Vector((0,0,reference/2)))
                bar=bpy.context.object;bar.scale=(.012,.012,reference);bar.data.materials.append(ink)
            scene.render.filepath=str(output/f'terra-{family}-{title}.png')
            bpy.ops.render.render(write_still=True)
            for obj in set(scene.objects)-before:
                remove_review_object(obj)
        return {'sheets':[str(output/f'terra-{family}-{g[1]}.png') for g in groups]}
    finally:
        bpy.context.window.scene=original
        for obj in list(scene.objects):
            remove_review_object(obj)
        bpy.data.scenes.remove(scene)
        for container, data in owned_data:
            if data.users==0:container.remove(data)


if __name__=='__main__':
    import generate
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out-dir',type=Path,default=Path('/tmp/terra-scenery-review'))
    parser.add_argument('--family', choices=['scenery','structures','items','actors'], default='scenery')
    args=parser.parse_args()
    script=Path(__file__).resolve()
    print(generate.execute(f"import runpy\nmodule = runpy.run_path({str(script)!r})\nresult = module['render']({str(args.out_dir.resolve())!r}, {args.family!r})\nresult"))
