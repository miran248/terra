# Launch the separate asset preview using the production catalog.
asset-preview:
    python3 crates/main/examples/asset_preview_prototype.py

# Generate candidate assets using the running Blender MCP add-on.
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
    TERRA_ASSET_SHOWCASE=1 cargo run -p main --features asset-review

# Generate meter-scale scenery review sheets through the running Blender MCP.
asset-scenery-sheets: asset-candidates
    python3 crates/gen_assets/blender/contact_sheet.py

# Render the structure family sheets using the running Blender MCP.
asset-structure-sheets: asset-candidates
    python3 crates/gen_assets/blender/contact_sheet.py --family structures --out-dir /tmp/terra-structures-review

# Inspect candidate structures and repeated modules on the actual terrain.
asset-structure-showcase:
    TERRA_ASSET_SHOWCASE=1 TERRA_STRUCTURE_SHOWCASE=1 cargo run -p main --features asset-review

# Render items and actor variants at their actual meter dimensions.
asset-item-sheets: asset-candidates
    python3 crates/gen_assets/blender/contact_sheet.py --family items --out-dir /tmp/terra-items-review
    python3 crates/gen_assets/blender/contact_sheet.py --family actors --out-dir /tmp/terra-items-review

# Inspect all actors/actions, held weapons and dropped items on the terrain.
asset-item-showcase:
    TERRA_ASSET_SHOWCASE=1 TERRA_ITEM_SHOWCASE=1 cargo run -p main --features asset-review

# Generate the complete production catalog through the running Blender MCP.
assets:
    cargo run -p gen_assets

# Regenerate into scratch storage and compare production bytes without writing.
assets-check:
    cargo run -p gen_assets -- --check

# Start the normal game with the generated production catalog.
run:
    cargo run -p main

# Compare current and proposed on-foot controls on the actual planet.
on-foot-prototype:
    cargo run -p main --features on-foot-prototype
