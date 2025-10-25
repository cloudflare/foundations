use prometheus_client::encoding::text::{EncodeMetric, Encoder};
use prometheus_client::metrics::{MetricType, TypedMetric};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use thread_local::ThreadLocal;

/// Sharded Prometheus [`Counter`][counter] with thread-local storage.
///
/// Thread-local storage eliminates contention between multiple threads
/// concurrently incrementing the same counter. This can yield significant
/// performance benefits on very frequently incrementing counters, such as
/// bandwidth counters in a web server.
///
/// The downside to sharding is an increase in memory usage. This implementation
/// allocates as many `u64`-sized counters as there are live threads. However,
/// allocations happen lazily on first use in each thread and are reused when
/// a thread exits. See the [`thread_local`] docs for details.
///
/// [counter]: prometheus_client::metrics::counter::Counter
pub struct ThreadLocalCounter {
    // `ThreadLocal` packs values for many threads in a tight array.
    // We must avoid sharing cachelines, so either we need to pad each
    // value out (16x size) or add another layer of indirection (Box).
    // Good allocators (incl. jemalloc by default) will use per-thread
    // arenas for allocations this small.
    //
    // We do not need to do anything special for thread exits: `ThreadLocal`
    // does not drop any values until the struct itself is dropped.
    value: ThreadLocal<Box<AtomicU64>>,
}

impl ThreadLocalCounter {
    /// Create a new [`ThreadLocalCounter`] instance.
    pub const fn new() -> Self {
        Self {
            value: ThreadLocal::new(),
        }
    }

    /// Increase the [`ThreadLocalCounter`] by 1.
    #[inline]
    pub fn inc(&self) {
        self.inc_by(1)
    }

    /// Increase the [`ThreadLocalCounter`] by `v`.
    pub fn inc_by(&self, v: u64) {
        let c = self.value.get_or_default();

        // c is a thread-local and we don't modify it from other threads, so
        // a non-atomic read-modify-write has the same effect as `fetch_add`.
        // This avoids expensive memory barriers.
        let old = c.load(Ordering::Relaxed);
        let new = old.wrapping_add(v);
        c.store(new, Ordering::Relaxed);
    }

    /// Get the current value of the [`ThreadLocalCounter`].
    pub fn get(&self) -> u64 {
        // Use wrapping arithmetic to emulate a single counter
        self.value
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .fold(0, u64::wrapping_add)
    }
}

impl fmt::Debug for ThreadLocalCounter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let total = self.get();
        f.debug_struct("ThreadLocalCounter")
            .field("value", &total)
            .finish()
    }
}

impl Default for ThreadLocalCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedMetric for ThreadLocalCounter {
    const TYPE: MetricType = MetricType::Counter;
}

impl EncodeMetric for ThreadLocalCounter {
    fn encode(&self, mut encoder: Encoder) -> std::io::Result<()> {
        let total = self.get();
        encoder
            .encode_suffix("total")?
            .no_bucket()?
            .encode_value(total)?
            .no_exemplar()
    }

    fn metric_type(&self) -> MetricType {
        Self::TYPE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inc_and_get() {
        let counter = ThreadLocalCounter::default();
        assert_eq!(0, counter.get());

        counter.inc();
        assert_eq!(1, counter.get());

        counter.inc_by(199);
        assert_eq!(200, counter.get());
    }

    #[test]
    fn threaded_inc() {
        static COUNTER: ThreadLocalCounter = ThreadLocalCounter::new();
        assert_eq!(0, COUNTER.get());

        let handles: Vec<_> = (0..5)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..1000 {
                        COUNTER.inc();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("test thread paniced");
        }

        assert_eq!(5000, COUNTER.get());

        // Test wrap-around: adding u64::MAX is equivalent to subtracing 1
        COUNTER.inc_by(u64::MAX);
        assert_eq!(4999, COUNTER.get());
    }
}
