# Procedural asset generator

- Owns deterministic procedural GLB construction, serialization, validation, and CLI behavior.
- Consumes canonical names, dimensions, and collider contracts from `shared::art`.
- Output remains texture-free glTF 2.0 binary in meters, Y-up, forward -Z, with ground pivots, flat normals, linear vertex colors, and high-roughness PBR materials.
- Generation is byte-for-byte deterministic; `--check` validates without writing.

Read the [GLB pipeline](../../docs/glb-pipeline.md) for generation and verification procedures.
