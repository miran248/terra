#!/usr/bin/env python3
"""THROWAWAY: generate the current catalogs and launch the native scale workbench.

Run from anywhere: python3 crates/main/examples/asset_preview_prototype.py
The temporary manifest contains rest-pose bounds, never user edits.
"""
import itertools
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]


def multiply(a, b):
    return [sum(a[k * 4 + row] * b[col * 4 + k] for k in range(4))
            for col in range(4) for row in range(4)]


def matrix(node):
    if "matrix" in node:
        return node["matrix"]
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    sx, sy, sz = node.get("scale", [1, 1, 1])
    tx, ty, tz = node.get("translation", [0, 0, 0])
    return [(1-2*y*y-2*z*z)*sx, (2*x*y+2*z*w)*sx, (2*x*z-2*y*w)*sx, 0,
            (2*x*y-2*z*w)*sy, (1-2*x*x-2*z*z)*sy, (2*y*z+2*x*w)*sy, 0,
            (2*x*z+2*y*w)*sz, (2*y*z-2*x*w)*sz, (1-2*x*x-2*y*y)*sz, 0,
            tx, ty, tz, 1]


def bounds(doc, scene):
    points = []

    def visit(index, parent):
        node = doc["nodes"][index]
        transform = multiply(parent, matrix(node))
        if "mesh" in node:
            for primitive in doc["meshes"][node["mesh"]]["primitives"]:
                accessor = doc["accessors"][primitive["attributes"]["POSITION"]]
                for point in itertools.product(*zip(accessor["min"], accessor["max"])):
                    points.append([sum(transform[k*4+r] * point[k] for k in range(3))
                                   + transform[12+r] for r in range(3)])
        for child in node.get("children", []):
            visit(child, transform)

    for node in scene["nodes"]:
        visit(node, IDENTITY)
    return [min(p[i] for p in points) for i in range(3)] + [max(p[i] for p in points) for i in range(3)]


def main():
    subprocess.run(["cargo", "run", "-p", "gen_assets", "--", "--out-dir",
                    "crates/main/assets/models"], cwd=ROOT, check=True)
    rows = []
    for name in ("environment", "structures", "items", "actors"):
        path = ROOT / f"crates/main/assets/models/{name}.glb"
        data = path.read_bytes()
        length, = struct.unpack_from("<I", data, 12)
        doc = json.loads(data[20:20+length])
        for index, scene in enumerate(doc["scenes"]):
            rows.append("\t".join([f"models/{name}.glb#Scene{index}", scene["name"],
                                    *map(str, bounds(doc, scene))]))
    with tempfile.TemporaryDirectory(prefix="terra-asset-preview-") as temporary:
        manifest = Path(temporary) / "catalog.tsv"
        manifest.write_text("\n".join(sorted(rows, key=lambda row: row.split("\t")[1])))
        env = dict(os.environ, TERRA_PREVIEW_MANIFEST=str(manifest))
        subprocess.run(["cargo", "run", "-p", "main", "--example", "asset_preview_prototype"],
                       cwd=ROOT, env=env, check=True)


if __name__ == "__main__":
    main()
