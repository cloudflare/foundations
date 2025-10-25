use criterion::measurement::WallTime;
use criterion::{criterion_group, criterion_main, Criterion};
use foundations::telemetry::metrics::ThreadLocalCounter;
use prometheus_client::metrics::counter::Counter as PromCounter;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

type BenchmarkGroup<'a> = criterion::BenchmarkGroup<'a, WallTime>;

trait Counter: Default + Clone + Send + 'static {
    const NAME: &str;
    fn inc(&self);
}
impl Counter for PromCounter {
    const NAME: &str = "PromCounter";

    #[inline(always)]
    fn inc(&self) {
        PromCounter::inc(self);
    }
}
impl Counter for Arc<ThreadLocalCounter> {
    const NAME: &str = "ThreadLocalCounter";

    #[inline(always)]
    fn inc(&self) {
        ThreadLocalCounter::inc(&*self);
    }
}

fn bench_counter_with_threads<C: Counter>(c: &mut BenchmarkGroup, threads: usize) {
    let stop = Arc::new(AtomicBool::new(false));
    let counter = C::default();

    let handles: Vec<_> = (1..threads)
        .map(|_| {
            let stop = Arc::clone(&stop);
            let counter = C::clone(&counter);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    counter.inc();
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
        bench_counter_with_threads::<PromCounter>(&mut group, threads);
        bench_counter_with_threads::<Arc<ThreadLocalCounter>>(&mut group, threads);
    }
}

criterion_group!(benches, bench_counters);
criterion_main!(benches);
