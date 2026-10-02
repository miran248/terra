# Script-first asset refresh with a character scale reference

Author the replacement asset catalog through scripts executed in Blender, retaining a pastel, low-poly style while adding believable anatomy, construction, and shape detail. Use an approximately 1 m tall humanoid as the scale reference, keep visual dimensions and simplified collision shapes aligned, and validate proportions in an asset preview before replacing the catalog; interface graphics and gameplay mechanics remain outside the refresh, and animation work initially covers existing idle, walk, and attack actions.

Scripts are the preferred editable source because the catalog needs consistent, repeatable changes across asset families. Revisit this requirement if character modeling, rigging, or animation makes scripts disproportionately complex; hand-authored Blender assets remain an alternative rather than a second source of truth.

This records the authoring direction, not a completed pipeline migration. The Rust generator remains the current baseline; Blender export reproducibility and compatibility with the existing GLB contract must be demonstrated before replacing it.
