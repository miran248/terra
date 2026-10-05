# World model crate

## Ownership

- Owns terrain classifications, settlement configuration, world records, `LevelData`, and the versioned artifact codec and validation.
- Depends on `terra-geometry` for typed geometry identities. Keep generation, `shared`, and application binaries out of this crate's dependency graph.
- Preserve serialized field and enum ordering, the TERA header, schema version, Postcard representation, validation behavior, and generated artifact bytes.

## Verification

Use the small model-owned fixture for codec and validation tests. Run `cargo test -p terra-world`; verify generation fingerprints and the seed-1337 artifact comparison through the [level pipeline](../../docs/level-pipeline.md).
