# Generate the baseline catalogs and launch the asset preview prototype.
asset-preview:
    python3 crates/main/examples/asset_preview_prototype.py

# Generate the humanoid/house candidates using the running Blender MCP add-on.
asset-candidates:
    python3 crates/gen_assets/blender/generate.py

# Prove that the candidate exports still match their scripts, without writing them.
asset-candidates-check:
    python3 crates/gen_assets/blender/generate.py --check

# Run the exported-asset integration contracts (requires Blender MCP).
asset-candidates-test:
    python3 -m unittest discover -s crates/gen_assets/blender -p test_pipeline.py

# Review candidate animation on the actual planet without changing the catalog.
asset-showcase:
    TERRA_ASSET_SHOWCASE=1 cargo run -p main
