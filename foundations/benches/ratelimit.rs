use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use foundations::{RateLimiter, RateLimiterConfig};

fn bench_uncontended_low_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("ratelimiter/uncontended_low_rate");

    // 1 event per minute, 1 burst
    group.bench_function("1_per_min_1_burst", |b| {
        let config = RateLimiterConfig::new(1.0 / 60.0, 1);
        let limiter = RateLimiter::new(&config);
        b.iter(|| black_box(limiter.is_ratelimited()))
    });

    group.finish();
}

fn bench_contended_low_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("ratelimiter/contended_low_rate");

    // 1 event per minute, 1 burst
    let config: &'static RateLimiterConfig = Box::leak(Box::new(RateLimiterConfig::new(1.0 / 60.0, 1)));

    for num_threads in [2, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new("1_per_min_1_burst", num_threads),
            &num_threads,
            |b, &num_threads| {
                let limiter = Arc::new(RateLimiter::new(config));

                b.iter_custom(|iters| {
                    let iters_per_thread = iters / num_threads as u64;
                    let barrier = Arc::new(std::sync::Barrier::new(num_threads));

                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let limiter = Arc::clone(&limiter);
                            let barrier = Arc::clone(&barrier);
                            std::thread::spawn(move || {
                                barrier.wait();
                                let start = std::time::Instant::now();
                                for _ in 0..iters_per_thread {
                                    let _ = black_box(limiter.is_ratelimited());
                                }
                                start.elapsed()
                            })
                        })
                        .collect();

                    let total_duration: std::time::Duration =
                        handles.into_iter().map(|h| h.join().unwrap()).sum();

                    // Return average duration across all threads
                    total_duration / num_threads as u32
                });
            },
        );
    }

    group.finish();
}

fn bench_uncontended_high_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("ratelimiter/uncontended_high_rate");

    // 1000 rps, 10 burst
    group.bench_function("1000rps_10_burst", |b| {
        let config = RateLimiterConfig::new(1_000.0, 10);
        let limiter = RateLimiter::new(&config);
        b.iter(|| black_box(limiter.is_ratelimited()))
    });

    group.finish();
}

fn bench_contended_high_rate(c: &mut Criterion) {
    let mut group = c.benchmark_group("ratelimiter/contended_high_rate");

    // 1000 rps, 10 burst
    let config: &'static RateLimiterConfig = Box::leak(Box::new(RateLimiterConfig::new(1_000.0, 10)));

    for num_threads in [2, 4, 8, 16] {
        group.bench_with_input(
            BenchmarkId::new("1000rps_10_burst", num_threads),
            &num_threads,
            |b, &num_threads| {
                let limiter = Arc::new(RateLimiter::new(config));

                b.iter_custom(|iters| {
                    let iters_per_thread = iters / num_threads as u64;
                    let barrier = Arc::new(std::sync::Barrier::new(num_threads));

                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let limiter = Arc::clone(&limiter);
                            let barrier = Arc::clone(&barrier);
                            std::thread::spawn(move || {
                                barrier.wait();
                                let start = std::time::Instant::now();
                                for _ in 0..iters_per_thread {
                                    let _ = black_box(limiter.is_ratelimited());
                                }
                                start.elapsed()
                            })
                        })
                        .collect();

                    let total_duration: std::time::Duration =
                        handles.into_iter().map(|h| h.join().unwrap()).sum();

                    // Return average duration across all threads
                    total_duration / num_threads as u32
                });
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_uncontended_low_rate,
    bench_contended_low_rate,
    bench_uncontended_high_rate,
    bench_contended_high_rate
);
criterion_main!(benches);
