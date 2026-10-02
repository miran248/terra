#!/usr/bin/env python3
"""THROWAWAY: generate the current catalogs and launch the native scale workbench.

Run from anywhere: python3 crates/main/examples/asset_preview_prototype.py
The temporary manifest contains rest-pose bounds, never user edits.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "crates/shared/tools"))
from glb import bounds, read


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline-only', action='store_true')
    args = parser.parse_args()
    candidate_dir = ROOT / 'crates/main/assets/models/candidates'
    candidate_manifest = candidate_dir / 'manifest.json'
    candidates = {} if args.baseline_only or not candidate_manifest.exists() else json.loads(candidate_manifest.read_text())['assets']
    subprocess.run(["cargo", "run", "-p", "gen_assets", "--", "--out-dir",
                    "crates/main/assets/models"], cwd=ROOT, check=True)
    rows = []
    for name in ("environment", "structures", "items", "actors"):
        path = ROOT / f"crates/main/assets/models/{name}.glb"
        doc = read(path)
        for index, scene in enumerate(doc["scenes"]):
            row = [f"models/{name}.glb#Scene{index}", scene["name"], *map(str, bounds(doc, scene))]
            if scene['name'] in candidates:
                filename = candidates[scene['name']]['file']
                candidate = read(candidate_dir / filename)
                candidate_index = next(i for i, s in enumerate(candidate['scenes']) if s['name'] == scene['name'])
                row += [f"models/candidates/{filename}#Scene{candidate_index}", *map(str, bounds(candidate, candidate['scenes'][candidate_index]))]
            rows.append("\t".join(row))
    with tempfile.TemporaryDirectory(prefix="terra-asset-preview-") as temporary:
        manifest = Path(temporary) / "catalog.tsv"
        manifest.write_text("\n".join(sorted(rows, key=lambda row: row.split("\t")[1])))
        env = dict(os.environ, TERRA_PREVIEW_MANIFEST=str(manifest))
        subprocess.run(["cargo", "run", "-p", "main", "--example", "asset_preview_prototype"],
                       cwd=ROOT, env=env, check=True)


if __name__ == "__main__":
    main()
