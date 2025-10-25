use criterion::measurement::WallTime;
use criterion::{Criterion, criterion_group, criterion_main};
use crossbeam_utils::CachePadded;
use foundations::telemetry::metrics::Counter as AtomicCounter;
use foundations::telemetry::metrics::ThreadLocalCounter;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

type BenchmarkGroup<'a> = criterion::BenchmarkGroup<'a, WallTime>;

trait Counter: Default + Clone + Send + 'static {
    const NAME: &str;
    fn inc(&self);
}
impl Counter for AtomicCounter {
    const NAME: &str = "AtomicCounter";

    #[inline(always)]
    fn inc(&self) {
        AtomicCounter::inc(self);
    }
}
impl Counter for ThreadLocalCounter {
    const NAME: &str = "ThreadLocalCounter";

    #[inline(always)]
    fn inc(&self) {
        ThreadLocalCounter::inc(self);
    }
}

fn bench_counter_with_threads<C: Counter>(c: &mut BenchmarkGroup, threads: usize) {
    // CachePadded so it doesn't get invalidated by counter increments
    let stop = Arc::new(CachePadded::new(AtomicBool::new(false)));
    let counter = C::default();

    // n-1 threads to generate load
    let handles: Vec<_> = (1..threads)
        .map(|_| {
            let stop = Arc::clone(&stop);
            let counter = C::clone(&counter);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..1000 {
                        counter.inc();
                    }
                }
            })
        })
        .collect();

    c.bench_function(C::NAME, |b| b.iter(|| counter.inc()));

    stop.store(true, Ordering::Relaxed);
    for h in handles {
        h.join().expect("bench thread paniced");
    }
}

fn bench_counters(c: &mut Criterion) {
    for threads in [1, 2, 4, 6, 10, 100] {
        let mut group = c.benchmark_group(format!("{threads} threads"));
        bench_counter_with_threads::<AtomicCounter>(&mut group, threads);
        bench_counter_with_threads::<ThreadLocalCounter>(&mut group, threads);
    }
}

criterion_group!(benches, bench_counters);
criterion_main!(benches);
