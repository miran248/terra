use std::hint::black_box;
use std::time::Instant;

fn main() {
    let seed = std::env::var("PLANET_SEED")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(1337);
    let runs = std::env::var("WORLDGEN_BENCH_RUNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(3)
        .max(1);
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let started = Instant::now();
        let world = shared::worldgen::run(black_box(seed), |_| {});
        black_box(world.level_data());
        samples.push(started.elapsed());
    }
    let mean = samples
        .iter()
        .map(|sample| sample.as_secs_f64())
        .sum::<f64>()
        / runs as f64;
    println!("worldgen/{seed}: {runs} runs, mean {mean:.3}s, samples {samples:?}");
}
