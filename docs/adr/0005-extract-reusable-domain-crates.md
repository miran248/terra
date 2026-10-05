---
status: accepted
---

# Extract reusable domain crates within Terra

Keep reusable domain crates in Terra's workspace initially, for use by Terra and other Bevy projects. Analyze the full domain map, then migrate the planetary foundations first; extracting unrelated domains and moving libraries to a separate repository are later work.

Bevy is the intended engine for consumers. Engine independence is optional: preserve compatible Bevy math types and separate rendering dependencies where useful, without requiring engine-neutral interfaces or adapters solely for hypothetical consumers.

The initial extraction preserves existing generation behavior and world rules, including deterministic output. Generalizing planet parameters or generation policy is separate work driven by concrete consumer requirements.

Implementation in this effort is limited to Terra. External consumers migrate separately; their usage informs interface design without coupling the libraries to any particular consuming project.

The initial library split is:

| Crate | Ownership |
| --- | --- |
| `terra-geometry` | Spherical coordinates, planet mesh queries, topology, and geometric road helpers. |
| `terra-world` | Terrain classifications, settlement configuration, regions, placement records, `LevelData`, and artifact encoding. |
| `terra-worldgen` | Terrain generation, zones, river and road planning, and the private generation pipeline. |

`terra-world` depends on `terra-geometry`; `terra-worldgen` depends on both. Moving terrain classifications and settlement configuration into the world model removes the current mutual dependency between terrain and level modules. Generation internals remain private behind finalized world output. `TerrainGen` retains its current query methods in `terra-worldgen`; separating a read-only query object is deferred, while `PlanetMesh` queries remain independently available through geometry.

Consumers use the new crates directly. Remove extracted planetary exports from `shared` without compatibility re-exports; external consumers may update their imports in a separate migration. Keep existing artifact formats and deterministic output unchanged. Asset contracts, planetary presentation, gameplay mechanics, and prototypes remain outside this first extraction.

This replaces the single-`shared` ownership direction in the repository contract and [the planet-view ADR](0004-live-planet-view.md) with domain-specific library ownership as extraction proceeds. Binary crates continue to orchestrate libraries. Update affected ownership instructions and pipeline documentation with the implementation. This decision records the agreed architecture, not a completed migration.
