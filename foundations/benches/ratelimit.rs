use std::sync::Arc;

use criterion::{Bencher, BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use foundations::{RateLimiter, RateLimiterConfig};

fn bench_uncontended(c: &mut Criterion, group_name: &str, bench_name: &str, rate: f64, burst: u64) {
    let mut group = c.benchmark_group(group_name);
    group.bench_function(bench_name, |b| {
        let config = RateLimiterConfig::new(rate, burst);
        let limiter = RateLimiter::new(&config);
        b.iter(|| black_box(limiter.is_ratelimited()))
    });
    group.finish();
}

fn bench_contended(c: &mut Criterion, group_name: &str, bench_name: &str, rate: f64, burst: u64) {
    let mut group = c.benchmark_group(group_name);
    let config: &'static RateLimiterConfig =
        Box::leak(Box::new(RateLimiterConfig::new(rate, burst)));

    for num_threads in [2, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new(bench_name, num_threads),
            &num_threads,
            |b, &num_threads| {
                run_contended_iter(b, config, num_threads);
            },
        );
    }

    group.finish();
}

fn run_contended_iter(b: &mut Bencher, config: &'static RateLimiterConfig, num_threads: usize) {
    let limiter = Arc::new(RateLimiter::new(config));

    b.iter_custom(|iters| {
        let barrier = Arc::new(std::sync::Barrier::new(num_threads + 1));

        let handles: Vec<_> = (0..num_threads)
            .map(|_| spawn_thread(Arc::clone(&limiter), Arc::clone(&barrier), iters))
            .collect();

        // All threads ready, start timing
        barrier.wait();

        let start = std::time::Instant::now();
        handles.into_iter().for_each(|h| h.join().unwrap());
        start.elapsed()
    });
}

fn spawn_thread(
    limiter: Arc<RateLimiter<'static>>,
    barrier: Arc<std::sync::Barrier>,
    iters: u64,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        barrier.wait();
        for _ in 0..iters {
            let _ = black_box(limiter.is_ratelimited());
        }
    })
}

fn bench_uncontended_low_rate(c: &mut Criterion) {
    bench_uncontended(
        c,
        "ratelimiter/uncontended_low_rate",
        "1_per_min_1_burst",
        1.0 / 60.0,
        1,
    );
}

fn bench_contended_low_rate(c: &mut Criterion) {
    bench_contended(
        c,
        "ratelimiter/contended_low_rate",
        "1_per_min_1_burst",
        1.0 / 60.0,
        1,
    );
}

fn bench_uncontended_high_rate(c: &mut Criterion) {
    bench_uncontended(
        c,
        "ratelimiter/uncontended_high_rate",
        "1000rps_10_burst",
        1_000.0,
        10,
    );
}

fn bench_contended_high_rate(c: &mut Criterion) {
    bench_contended(
        c,
        "ratelimiter/contended_high_rate",
        "1000rps_10_burst",
        1_000.0,
        10,
    );
}

criterion_group!(
    benches,
    bench_uncontended_low_rate,
    bench_contended_low_rate,
    bench_uncontended_high_rate,
    bench_contended_high_rate
);
criterion_main!(benches);
