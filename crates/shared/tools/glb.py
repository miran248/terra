"""Small, standard-library GLB readers shared by generation and asset preview."""
import itertools
import json
import math
from pathlib import Path
import struct

IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]


def read(path):
    data = Path(path).read_bytes()
    magic, version, length, size, kind = struct.unpack_from("<5I", data)
    if (magic, version, length, kind) != (0x46546C67, 2, len(data), 0x4E4F534A):
        raise ValueError(f"Invalid GLB: {path}")
    return json.loads(data[20:20 + size])


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
    if not points or not all(math.isfinite(v) for p in points for v in p):
        raise ValueError("Scene must have finite geometry")
    return [min(p[i] for p in points) for i in range(3)] + [max(p[i] for p in points) for i in range(3)]
