use super::IntoF64;
use crate::{EncodeMetricValue, MetricFamily};
use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use thread_local::ThreadLocal;

/// Interface for types that can be represented by a [`ThreadLocalCounter`].
trait CounterType: fmt::Debug + IntoF64 {
    const ZERO: Self;
    const ONE: Self;
    fn add(v: Self, raw: u64) -> Self;
    fn to_bits(self) -> u64;
}

impl CounterType for u64 {
    const ZERO: Self = 0;
    const ONE: Self = 1;

    #[inline(always)]
    fn add(v: Self, raw: u64) -> Self {
        // Use wrapping arithmetic to match AtomicU64::fetch_add
        v.wrapping_add(raw)
    }
    #[inline(always)]
    fn to_bits(self) -> u64 {
        self
    }
}

impl CounterType for f64 {
    const ZERO: Self = 0.0;
    const ONE: Self = 1.0;

    #[inline(always)]
    fn add(v: Self, raw: u64) -> Self {
        v + Self::from_bits(raw)
    }
    #[inline(always)]
    fn to_bits(self) -> u64 {
        self.to_bits()
    }
}

/// Sharded [`Counter`][super::Counter] with thread-local storage.
///
/// The counter can be used for integers (`N = u64`, the default) or for
/// floating-point numbers (`N = f64`).
///
/// Thread-local storage eliminates contention between multiple threads
/// concurrently incrementing the same counter. This can yield significant
/// performance benefits on very frequently incrementing counters, such as
/// bandwidth counters in a web server.
///
/// The downside to sharding is an increase in memory usage. This implementation
/// allocates as many `u64`-sized counters as there are live threads. However,
/// allocations happen lazily on first use in each thread and are reused when
/// a thread exits. See the [`mod@thread_local`] docs for details.
pub struct ThreadLocalCounter<N = u64> {
    // `ThreadLocal` packs values for many threads in a tight array.
    // We must avoid sharing cachelines, so either we need to pad each
    // value out (16x size) or add another layer of indirection (Box).
    // Good allocators (incl. jemalloc by default) will use per-thread
    // arenas for allocations this small.
    //
    // We do not need to do anything special for thread exits: `ThreadLocal`
    // does not drop any values until the struct itself is dropped.
    value: Arc<ThreadLocal<Box<AtomicU64>>>,
    _marker: PhantomData<N>,
}

#[expect(
    private_bounds,
    reason = "Abstraction over a closed set of value types"
)]
impl<N: CounterType> ThreadLocalCounter<N> {
    /// Increase the [`ThreadLocalCounter`] by 1.
    #[inline]
    pub fn inc(&self) {
        self.inc_by(N::ONE);
    }

    /// Increase the [`ThreadLocalCounter`] by `v`.
    pub fn inc_by(&self, v: N) {
        let c = self.value.get_or_default();

        // c is a thread-local and we don't modify it from other threads, so
        // a non-atomic read-modify-write has the same effect as `fetch_add`.
        // This avoids expensive memory barriers.
        let old = c.load(Ordering::Relaxed);
        let new = N::add(v, old).to_bits();
        c.store(new, Ordering::Relaxed);
    }

    /// Get the current value of the [`ThreadLocalCounter`].
    pub fn get(&self) -> N {
        self.value
            .iter()
            .map(|c| c.load(Ordering::Relaxed))
            .fold(N::ZERO, N::add)
    }
}

impl<N: CounterType> fmt::Debug for ThreadLocalCounter<N> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let total = self.get();
        f.debug_struct("ThreadLocalCounter")
            .field("value", &total)
            .finish()
    }
}

impl<N> Clone for ThreadLocalCounter<N> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
            _marker: PhantomData,
        }
    }
}

impl<N> Default for ThreadLocalCounter<N> {
    fn default() -> Self {
        Self {
            value: Arc::new(ThreadLocal::new()),
            _marker: PhantomData,
        }
    }
}

impl<N: CounterType> EncodeMetricValue for ThreadLocalCounter<N> {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let value = self.get();
        super::counter::encode_counter(value.into_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::LazyLock;

    #[test]
    fn inc_and_get() {
        let counter = ThreadLocalCounter::default();
        assert_eq!(0, counter.get());

        counter.inc();
        assert_eq!(1, counter.get());

        counter.inc_by(199);
        assert_eq!(200, counter.get());

        let float_counter = ThreadLocalCounter::<f64>::default();
        assert_eq!(0.0, float_counter.get());

        float_counter.inc();
        assert_eq!(1.0, float_counter.get());

        float_counter.inc_by(199.0);
        assert_eq!(200.0, float_counter.get());
    }

    #[test]
    fn threaded_inc() {
        static COUNTER: LazyLock<ThreadLocalCounter> = LazyLock::new(ThreadLocalCounter::default);
        let counter = &*COUNTER;
        assert_eq!(0, counter.get());

        let handles: Vec<_> = (0..5)
            .map(|_| {
                std::thread::spawn(|| {
                    for _ in 0..1000 {
                        counter.inc();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("test thread paniced");
        }

        assert_eq!(5000, counter.get());

        // Test wrap-around: adding u64::MAX is equivalent to subtracting 1
        counter.inc_by(u64::MAX);
        assert_eq!(4999, counter.get());
    }
}
