# Keep scenery placement independent of visibility settings

Procedural scenery must return to the same positions when an area is revisited, regardless of graphics settings. Density-driven placement will therefore be deterministic, while rendering may vary the visible subset or representation with distance and quality settings. Collision-bearing scenery must remain consistent so changing visibility cannot change which obstacles exist in the world.

This rules out unseeded resampling on chunk entry and quality-dependent obstacle placement. [Generation-time sampling](0003-sample-scenery-density-during-generation.md) specifies when placements are produced; exact storage and rendering details remain implementation choices.
