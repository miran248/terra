"""Integration contracts for the public MCP generation command (Blender must be open)."""
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("generate.py")
sys.path.insert(0, str(SCRIPT.parents[3] / "crates/shared/tools"))
from glb import bounds


def document(path):
    data = path.read_bytes()
    magic, version, length, json_length, kind = struct.unpack_from("<5I", data)
    assert magic == 0x46546C67 and version == 2 and length == len(data)
    assert kind == 0x4E4F534A
    return json.loads(data[20:20 + json_length])


def float_accessor(path, doc, index):
    data = path.read_bytes()
    json_length = struct.unpack_from('<I', data, 12)[0]
    binary = 20 + json_length + 8
    accessor = doc['accessors'][index]
    view = doc['bufferViews'][accessor['bufferView']]
    width = {'SCALAR':1, 'VEC3':3, 'VEC4':4}[accessor['type']]
    stride = view.get('byteStride', width*4)
    offset = binary + view.get('byteOffset',0) + accessor.get('byteOffset',0)
    return [struct.unpack_from('<'+'f'*width, data, offset+i*stride) for i in range(accessor['count'])]


class PipelineTests(unittest.TestCase):
    def test_exports_isolated_named_pair_at_meter_scale(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = subprocess.run([sys.executable, str(SCRIPT), "--out-dir", temporary],
                                    capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            actor = document(Path(temporary) / "actor.player.glb")
            house = document(Path(temporary) / "structure.house.glb")
            self.assertEqual([s["name"] for s in actor["scenes"]], ["actor.player"])
            self.assertEqual([s["name"] for s in house["scenes"]], ["structure.house"])
            self.assertIn("socket.hand", [n.get("name") for n in actor["nodes"]])
            manifest = json.loads((Path(temporary) / "manifest.json").read_text())
            self.assertEqual(manifest["blender"], "5.2.2")
            self.assertAlmostEqual(manifest["assets"]["actor.player"]["dimensions"][1], 1.0, places=4)
            self.assertAlmostEqual(bounds(actor, actor["scenes"][0])[4], 1.0, places=4)
            self.assertAlmostEqual(manifest["assets"]["structure.house"]["door_clear_height"], 1.2)
            for doc in [actor, house]:
                self.assertFalse(doc.get("textures"))
                self.assertFalse(doc.get("images"))
                for mesh in doc["meshes"]:
                    for primitive in mesh["primitives"]:
                        self.assertIn("COLOR_0", primitive["attributes"])
                        self.assertIn("NORMAL", primitive["attributes"])
                self.assertTrue(all(m["pbrMetallicRoughness"]["roughnessFactor"] >= .9 for m in doc["materials"]))

    def test_representative_set_has_clear_canopy_and_hand_sized_weapon(self):
        with tempfile.TemporaryDirectory() as temporary:
            subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary], check=True, capture_output=True)
            output = Path(temporary)
            manifest = json.loads((output / 'manifest.json').read_text())
            self.assertTrue({'actor.player', 'structure.house',
                'scenery.tree.0', 'scenery.rock.0', 'weapon.knife'} <= set(manifest['assets']))
            for name in ['scenery.tree.0', 'scenery.rock.0', 'weapon.knife']:
                doc = document(output / (name + '.glb'))
                self.assertEqual(doc['scenes'][0]['name'], name)
                self.assertIn(name + '.mesh', [m.get('name') for m in doc['meshes']])
                self.assertIn(name + '.material', [m.get('name') for m in doc['materials']])
                self.assertAlmostEqual(bounds(doc, doc['scenes'][0])[1], 0, places=5)
            tree = document(output / 'scenery.tree.0.glb')
            canopy = [i for i,n in enumerate(tree['nodes']) if '.canopy.' in n.get('name', '')]
            self.assertGreaterEqual(len(canopy), 3)
            self.assertGreater(bounds(tree, {'nodes': canopy})[1], 1.2)
            knife = manifest['assets']['weapon.knife']
            self.assertGreater(knife['dimensions'][1], .2)
            self.assertLess(knife['dimensions'][1], .35)
            self.assertIn('socket.grip', [n.get('name') for n in document(output / 'weapon.knife.glb')['nodes']])

    def test_scenery_catalog_preserves_all_37_scenes_at_grounded_meter_scale(self):
        with tempfile.TemporaryDirectory() as temporary:
            baseline_dir = Path(temporary) / 'baseline'
            subprocess.run(['cargo', 'run', '-p', 'gen_assets', '--', '--out-dir', str(baseline_dir)],
                           cwd=SCRIPT.parents[3], check=True, capture_output=True)
            expected = {scene['name'] for scene in document(baseline_dir / 'environment.glb')['scenes']}
            self.assertEqual(len(expected), 37)
            subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary], check=True, capture_output=True)
            assets = json.loads((Path(temporary) / 'manifest.json').read_text())['assets']
            self.assertEqual({name for name in assets if name.startswith('scenery.')}, expected)
            barrel = assets['scenery.cactus.1']['dimensions']
            self.assertLess(barrel[1], .8, 'Variant 1 retains the short barrel-cactus silhouette')
            self.assertGreater(barrel[0] / barrel[1], .7)
            for name in sorted(expected):
                with self.subTest(scene=name):
                    doc = document(Path(temporary) / (name + '.glb'))
                    self.assertEqual([s['name'] for s in doc['scenes']], [name])
                    self.assertAlmostEqual(bounds(doc, doc['scenes'][0])[1], 0, places=5)
                    self.assertEqual(assets[name]['runtime_scale'], [1, 1, 1])
                    self.assertTrue(all(0 < d < 4 for d in assets[name]['dimensions']))
                    self.assertFalse(doc.get('textures'))
                    self.assertTrue(all(m['pbrMetallicRoughness']['roughnessFactor'] >= .9 for m in doc['materials']))
                    extent = assets[name]['bounds']
                    for part in assets[name]['collider']:
                        if part['shape'] == 'box':
                            half = [d / 2 for d in part['size']]
                        else:
                            half = [part['radius'], part['length']/2, part['radius']]
                            if part['shape'] == 'capsule':
                                half[1] += part['radius']
                        for axis in range(3):
                            self.assertGreaterEqual(part['center'][axis]-half[axis], extent[axis]-.025)
                            self.assertLessEqual(part['center'][axis]+half[axis], extent[axis+3]+.025)
                    self.assertIn(name + '.mesh', [m.get('name') for m in doc['meshes']])
                    if name not in ('scenery.tree.0', 'scenery.rock.0'):
                        self.assertEqual(sum(len(m['primitives']) for m in doc['meshes']), 1,
                                         'Static scenery must batch compatible authoring parts')
                    for mesh in doc['meshes']:
                        for primitive in mesh['primitives']:
                            self.assertIn('COLOR_0', primitive['attributes'])
                            self.assertIn('NORMAL', primitive['attributes'])

    def test_visual_exports_consume_shared_dimensions_and_collision_contract(self):
        contracts = json.loads((SCRIPT.parents[3] / 'crates/shared/asset_dimensions.json').read_text())
        with tempfile.TemporaryDirectory() as temporary:
            subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary], check=True, capture_output=True)
            assets = json.loads((Path(temporary) / 'manifest.json').read_text())['assets']
            for name, contract in contracts.items():
                self.assertEqual(assets[name]['collider'], contract['colliders'])
                for actual, expected in zip(assets[name]['dimensions'], contract['dimensions']):
                    self.assertAlmostEqual(actual, expected, places=5)

    def test_humanoid_exports_a_skin_distinct_actions_and_bone_attached_socket(self):
        with tempfile.TemporaryDirectory() as temporary:
            subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary], check=True, capture_output=True)
            actor = document(Path(temporary) / 'actor.player.glb')
            self.assertTrue(actor.get('skins'), 'The actor must export a skeleton and skin')
            clips = {a['name']: a for a in actor.get('animations', [])}
            self.assertEqual(set(clips), {'idle', 'walk', 'attack'})
            self.assertEqual(len({json.dumps(a['samplers'], sort_keys=True) for a in clips.values()}), 3)
            for clip in clips.values():
                duration = max(actor['accessors'][s['input']]['max'][0] for s in clip['samplers'])
                self.assertAlmostEqual(duration, 1.0)
            socket = next(i for i,n in enumerate(actor['nodes']) if n.get('name') == 'socket.hand')
            parent = next(n for n in actor['nodes'] if socket in n.get('children', []))
            self.assertEqual(parent['name'], 'hand.right')
            payloads = []
            for clip in clips.values():
                values = [float_accessor(Path(temporary) / 'actor.player.glb', actor, sampler['output'])
                          for sampler in clip['samplers']]
                payloads.append(values)
                for channel in values:
                    for first, last in zip(channel[0], channel[-1]):
                        self.assertAlmostEqual(first, last, places=5, msg='Clip must loop without a jump')
            self.assertNotEqual(payloads[0], payloads[1])
            self.assertNotEqual(payloads[1], payloads[2])
            manifest = json.loads((Path(temporary) / 'manifest.json').read_text())
            for extent in manifest['assets']['actor.player']['animation_bounds'].values():
                self.assertGreaterEqual(extent[1], -0.00001, 'Feet must not penetrate the ground')
                self.assertLess(extent[4], 1.05, 'Body remains close to its one-meter collider')

    def test_existing_user_actions_do_not_rename_or_leak_into_exports(self):
        import generate
        created = generate.execute("import bpy\nresult = {'names': []}\nfor name in ['idle', 'walk', 'attack', 'unrelated.user.action']:\n    if name not in bpy.data.actions:\n        bpy.data.actions.new(name)\n        result['names'].append(name)\nresult")['names']
        try:
            with tempfile.TemporaryDirectory() as temporary:
                subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary], check=True, capture_output=True)
                actor = document(Path(temporary) / 'actor.player.glb')
                self.assertEqual({a['name'] for a in actor['animations']}, {'idle', 'walk', 'attack'})
            present = generate.execute("import bpy\nresult = {'names': [a.name for a in bpy.data.actions if not a.get('terra_blender_pilot')]}\nresult")["names"]
            self.assertTrue(set(created) <= set(present))
        finally:
            generate.execute(f"import bpy\nfor name in {created!r}:\n    bpy.data.actions.remove(bpy.data.actions[name])\nresult = {{'ok': True}}\nresult")

    def test_actor_faces_negative_z_and_keeps_ground_pivot(self):
        with tempfile.TemporaryDirectory() as temporary:
            subprocess.run([sys.executable, str(SCRIPT), "--out-dir", temporary], check=True, capture_output=True)
            actor = document(Path(temporary) / "actor.player.glb")
            nose = next(i for i,n in enumerate(actor['nodes']) if n.get('name') == 'actor.player.nose')
            nose_bounds = bounds(actor, {'nodes': [nose]})
            self.assertLess(nose_bounds[5], 0, 'The face must look toward glTF -Z')
            self.assertAlmostEqual(bounds(actor, actor['scenes'][0])[1], 0, places=5)

    def test_generation_is_byte_reproducible_and_check_does_not_write(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary)
            subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary], check=True, capture_output=True)
            before = {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in output.iterdir()}
            checked = subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary, '--check'], capture_output=True, text=True)
            self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)
            self.assertEqual(before, {p.name: (p.read_bytes(), p.stat().st_mtime_ns) for p in output.iterdir()})
            actor = output / 'actor.player.glb'
            actor.write_bytes(b'corrupted export')
            checked = subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary, '--check'], capture_output=True, text=True)
            self.assertNotEqual(checked.returncode, 0)
            self.assertEqual(actor.read_bytes(), b'corrupted export')

    def test_generation_preserves_the_users_scene(self):
        import generate
        code = """import bpy
result = {'active': bpy.context.window.scene.name,
          'scenes': {s.name: sorted(o.name for o in s.objects) for s in bpy.data.scenes if not s.get('terra_blender_pilot')}}
result"""
        before = generate.execute(code)
        with tempfile.TemporaryDirectory() as temporary:
            subprocess.run([sys.executable, str(SCRIPT), '--out-dir', temporary], check=True, capture_output=True)
        self.assertEqual(generate.execute(code), before)


if __name__ == "__main__":
    unittest.main()
