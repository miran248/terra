use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use terra_geometry::planet::{PlanetMesh, ray_triangle_radius, unit_icosphere_tris};
use terra_geometry::sphere::PLANET_RADIUS;

/// Deterministic spread of query directions over the sphere.
fn query_dirs(n: usize) -> Vec<bevy::prelude::Vec3> {
    (0..n)
        .map(|i| {
            let u = (i as f32 * 0.6180339) % 1.0;
            let v = (i as f32 * 0.7548776) % 1.0;
            let theta = u * std::f32::consts::TAU;
            let z = v * 2.0 - 1.0;
            let r = (1.0 - z * z).max(0.0).sqrt();
            bevy::prelude::Vec3::new(r * theta.cos(), z, r * theta.sin())
        })
        .collect()
}

fn bench(c: &mut Criterion) {
    // Subdivision 5 ≈ 20*4^5 = 20480 tris, close to the in-game ico(40) budget.
    let tris = unit_icosphere_tris(5);
    let mesh = PlanetMesh::new(tris.clone());
    let dirs = query_dirs(256);
    eprintln!("triangles: {}", mesh.triangle_count());

    let mut g = c.benchmark_group("facet_radius");

    g.bench_function("grid", |b| {
        b.iter(|| {
            for d in &dirs {
                black_box(mesh.facet_radius(black_box(*d), PLANET_RADIUS));
            }
        })
    });

    g.bench_function("brute_force", |b| {
        b.iter(|| {
            for d in &dirs {
                let r = tris
                    .iter()
                    .find_map(|t| ray_triangle_radius(black_box(*d), t))
                    .unwrap_or(PLANET_RADIUS);
                black_box(r);
            }
        })
    });

    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
