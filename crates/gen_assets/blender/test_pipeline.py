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
            self.assertEqual(set(manifest['assets']), {'actor.player', 'structure.house',
                'scenery.tree.0', 'scenery.rock.0', 'weapon.knife'})
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
