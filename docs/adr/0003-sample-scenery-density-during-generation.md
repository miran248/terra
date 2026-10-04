# Sample scenery density during world generation

Sample procedural scenery density maps during world generation to produce deterministic placements, then instance or batch repeated scenery for rendering. This preserves Terra's offline world-generation boundary and makes collision-bearing scenery independent of chunk entry and graphics settings, consistent with [the placement invariant](0002-deterministic-scenery-placement.md).

Runtime sampling could reduce stored placement data, but would introduce runtime placement work and require coordination with persistent obstacle identity. Defer that alternative unless measured placement storage warrants it. This decision accepts storing generated placements; it does not require storing the source density maps in the serialized level or prescribe their encoding.
