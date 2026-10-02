"""Scripted segmented humanoid rig and the three existing one-second actions."""
import math
import bpy
from mathutils import Vector, Quaternion


def rig_humanoid(model, owner):
    armature = bpy.data.armatures.new(model.name + '.skeleton')
    armature[owner] = True
    rig = bpy.data.objects.new(model.name + '.rig', armature)
    rig[owner] = True
    model.scene.collection.objects.link(rig)
    rig.parent = model.root
    bpy.context.view_layer.objects.active = rig
    rig.select_set(True)
    bpy.ops.object.mode_set(mode='EDIT')

    def bone(name, head, tail, parent=None):
        item = armature.edit_bones.new(name)
        item.head, item.tail = head, tail
        if parent:
            item.parent = armature.edit_bones[parent]
        return item

    bone('root', (0,0,0), (0,0,.48))
    bone('spine', (0,0,.48), (0,0,.78), 'root')
    bone('head', (0,0,.78), (0,0,.96), 'spine')
    for sign, side in [(-1,'left'), (1,'right')]:
        bone('thigh.'+side, (sign*.068,0,.46), (sign*.068,0,.26), 'root')
        bone('shin.'+side, (sign*.068,0,.26), (sign*.068,0,.065), 'thigh.'+side)
        bone('foot.'+side, (sign*.068,0,.065), (sign*.068,.06,.025), 'shin.'+side)
        bone('arm.'+side, (sign*.142,0,.745), (sign*.179,.006,.59), 'spine')
        bone('forearm.'+side, (sign*.179,.006,.59), (sign*.184,.015,.474), 'arm.'+side)
        bone('hand.'+side, (sign*.184,.015,.474), (sign*.184,.015,.445), 'forearm.'+side)
    bpy.ops.object.mode_set(mode='OBJECT')
    for obj in list(model.scene.objects):
        if obj.type != 'MESH':
            continue
        part = obj.name.removeprefix(model.name + '.')
        side, _, limb = part.partition('.')
        if side in ('left','right') and limb in ('boot','leg','sleeve','forearm','hand'):
            target = {'boot':'foot','leg':'thigh','sleeve':'arm','forearm':'forearm','hand':'hand'}[limb] + '.' + side
        elif part in ('head','hair','nose','mouth','neck') or limb in ('eye','brow','ear'):
            target = 'head'
        elif part in ('belt','buckle'):
            target = 'root'
        else:
            target = 'spine'
        group = obj.vertex_groups.new(name=target)
        if limb == 'leg':
            lower = obj.vertex_groups.new(name='shin.'+side)
            for vertex in obj.data.vertices:
                upper_weight = max(0., min(1., (vertex.co.z-.23)/.06))
                if upper_weight:
                    group.add([vertex.index], upper_weight, 'REPLACE')
                if upper_weight < 1:
                    lower.add([vertex.index], 1-upper_weight, 'REPLACE')
        else:
            group.add(list(range(len(obj.data.vertices))), 1., 'REPLACE')
        modifier = obj.modifiers.new('Armature', 'ARMATURE')
        modifier.object = rig
    socket = next(o for o in model.scene.objects if o.name == 'socket.hand')
    bpy.context.view_layer.update()
    world = socket.matrix_world.copy()
    socket.parent = rig
    socket.parent_type = 'BONE'
    socket.parent_bone = 'hand.right'
    bpy.context.view_layer.update()
    socket.matrix_world = world
    rig.animation_data_create()
    model.scene.render.fps = 24
    model.scene.frame_start = 0
    model.scene.frame_end = 24
    bounds_by_action = {}

    def rotate(name, angle, axis=(1,0,0)):
        pose = rig.pose.bones[name]
        local_axis = pose.bone.matrix_local.to_quaternion().inverted() @ Vector(axis)
        pose.rotation_quaternion = Quaternion(local_axis, angle)

    for name in ('idle','walk','attack'):
        action = bpy.data.actions.new(owner + '.' + name)
        action[owner] = True
        rig.animation_data.action = action
        action.use_fake_user = True
        envelope = [math.inf]*3 + [-math.inf]*3
        for frame in range(25):
            model.scene.frame_set(frame)
            for pose in rig.pose.bones:
                pose.rotation_mode = 'QUATERNION'
                pose.rotation_quaternion = Quaternion()
                pose.location = (0,0,0)
            phase = frame/24 * math.tau
            if name == 'idle':
                rotate('spine', .018*math.sin(phase))
                rotate('head', -.012*math.sin(phase))
            elif name == 'walk':
                for sign, side in [(-1,'left'), (1,'right')]:
                    swing = sign*math.sin(phase)
                    rotate('thigh.'+side, .42*swing)
                    rotate('shin.'+side, -.55*max(0.,swing))
                    rotate('foot.'+side, -.42*swing+.55*max(0.,swing))
                    rotate('arm.'+side, -.32*swing)
                    rotate('forearm.'+side, -.10-.08*max(0.,-swing))
                rotate('spine', .025*math.sin(phase), (0,0,1))
            else:
                # Anticipation, strike, recovery; identical boundary pose for safe transitions.
                t = frame/24
                windup = math.sin(math.pi*min(t/.35,1)) if t < .35 else 0
                strike = math.sin(math.pi*(t-.35)/.65) if t >= .35 else 0
                rotate('arm.right', -.6*windup+1.15*strike)
                rotate('forearm.right', -.85*windup-.35*strike)
                rotate('spine', -.10*strike)
                rotate('arm.left', .12*strike)
            bpy.context.view_layer.update()
            # Keep the lowest boot on the ground while preserving an in-place clip.
            depsgraph = bpy.context.evaluated_depsgraph_get()
            boots = [o for o in model.scene.objects if o.type == 'MESH' and '.boot' in o.name]
            lowest = min((o.evaluated_get(depsgraph).matrix_world @ v.co).z
                for o in boots for v in o.evaluated_get(depsgraph).data.vertices)
            rig.pose.bones['root'].location = armature.bones['root'].matrix_local.to_quaternion().inverted() @ Vector((0,0,-lowest))
            for pose in rig.pose.bones:
                pose.keyframe_insert('rotation_quaternion', frame=frame, group=pose.name)
                pose.keyframe_insert('location', frame=frame, group=pose.name)
            bpy.context.view_layer.update()
            depsgraph = bpy.context.evaluated_depsgraph_get()
            for obj in model.scene.objects:
                if obj.type != 'MESH':
                    continue
                evaluated = obj.evaluated_get(depsgraph)
                for vertex in evaluated.data.vertices:
                    p = evaluated.matrix_world @ vertex.co
                    point = (p.x,p.z,-p.y)
                    for i in range(3):
                        envelope[i] = min(envelope[i], point[i])
                        envelope[i+3] = max(envelope[i+3], point[i])
        bounds_by_action[name] = envelope
        track = rig.animation_data.nla_tracks.new()
        track.name = name
        track.strips.new(name, 0, action)
        track.mute = True
    rig.animation_data.action = None
    for pose in rig.pose.bones:
        pose.rotation_quaternion = Quaternion()
        pose.location = (0,0,0)
    model.scene.frame_set(0)
    return bounds_by_action
