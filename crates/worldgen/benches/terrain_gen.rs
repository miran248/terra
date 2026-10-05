use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use terra_geometry::sphere::SpherePos;
use terra_worldgen::terrain::TerrainGen;

fn bench_terrain_gen(c: &mut Criterion) {
    let mut g = c.benchmark_group("terrain_gen");

    g.bench_function("new", |b| b.iter(|| black_box(TerrainGen::new(1337))));

    g.bench_function("elevation_10k", |b| {
        let tg = TerrainGen::new(1337);
        let dirs: Vec<SpherePos> = (0..10000)
            .map(|i| {
                let u = (i as f32 * 0.6180339) % 1.0;
                let v = (i as f32 * 0.7548776) % 1.0;
                let theta = u * std::f32::consts::TAU;
                let z = v * 2.0 - 1.0;
                let r = (1.0 - z * z).max(0.0).sqrt();
                SpherePos::new(bevy_math::Vec3::new(r * theta.cos(), z, r * theta.sin()))
            })
            .collect();
        b.iter(|| {
            for d in &dirs {
                black_box(tg.elevation_at(*d));
            }
        })
    });

    g.bench_function("surface_radius_10k", |b| {
        let tg = TerrainGen::new(1337);
        let dirs: Vec<SpherePos> = (0..10000)
            .map(|i| {
                let u = (i as f32 * 0.6180339) % 1.0;
                let v = (i as f32 * 0.7548776) % 1.0;
                let theta = u * std::f32::consts::TAU;
                let z = v * 2.0 - 1.0;
                let r = (1.0 - z * z).max(0.0).sqrt();
                SpherePos::new(bevy_math::Vec3::new(r * theta.cos(), z, r * theta.sin()))
            })
            .collect();
        b.iter(|| {
            for d in &dirs {
                black_box(tg.surface_radius(*d));
            }
        })
    });

    g.bench_function("base_classify_10k", |b| {
        let tg = TerrainGen::new(1337);
        let dirs: Vec<SpherePos> = (0..10000)
            .map(|i| {
                let u = (i as f32 * 0.6180339) % 1.0;
                let v = (i as f32 * 0.7548776) % 1.0;
                let theta = u * std::f32::consts::TAU;
                let z = v * 2.0 - 1.0;
                let r = (1.0 - z * z).max(0.0).sqrt();
                SpherePos::new(bevy_math::Vec3::new(r * theta.cos(), z, r * theta.sin()))
            })
            .collect();
        b.iter(|| {
            for d in &dirs {
                black_box(tg.base_classify(*d));
            }
        })
    });

    g.bench_function("nearest_vert_10k", |b| {
        let tg = TerrainGen::new(1337);
        let dirs: Vec<SpherePos> = (0..10000)
            .map(|i| {
                let u = (i as f32 * 0.6180339) % 1.0;
                let v = (i as f32 * 0.7548776) % 1.0;
                let theta = u * std::f32::consts::TAU;
                let z = v * 2.0 - 1.0;
                let r = (1.0 - z * z).max(0.0).sqrt();
                SpherePos::new(bevy_math::Vec3::new(r * theta.cos(), z, r * theta.sin()))
            })
            .collect();
        b.iter(|| {
            for d in &dirs {
                black_box(tg.nearest_vert(*d));
            }
        })
    });

    g.finish();
}

criterion_group!(benches, bench_terrain_gen);
criterion_main!(benches);
