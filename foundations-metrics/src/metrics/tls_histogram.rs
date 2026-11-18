use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use thread_local::ThreadLocal;

use crate::{EncodeMetricValue, MetricFamily};

use super::MetricConstructor;
use super::histogram::{
    ClassicHistogramBounds, HistogramBuilder, HistogramSnapshot, encode_snapshot,
};

/// A classic fixed-bucket histogram with thread-local storage.
///
/// Each thread records independently using relaxed atomics stored in a
/// [`ThreadLocal`]. Collection iterates over the thread-local values and emits
/// one ordinary classic Prometheus histogram. Values belonging to exited
/// threads remain part of the aggregate, and their storage can be reused by
/// later threads.
///
/// # Examples
///
/// ```
/// use foundations_metrics::{EncodeMetric, NamedMetric, ThreadLocalHistogram};
///
/// // ThreadLocalHistogram is an ordinary runtime-created value.
/// let histogram = ThreadLocalHistogram::new([0.1, 1.0, 10.0]);
///
/// std::thread::scope(|scope| {
///     for value in [0.5, 2.0] {
///         let histogram = histogram.clone();
///         scope.spawn(move || histogram.observe(value));
///     }
/// });
///
/// let registered = NamedMetric::new(
///     "request_duration_seconds",
///     "Request duration.",
///     histogram,
/// );
/// assert_eq!(registered.encode()[0].name.as_deref(), Some("request_duration_seconds"));
/// ```
#[derive(Clone)]
pub struct ThreadLocalHistogram {
    state: Arc<ThreadLocalHistogramState>,
}

struct ThreadLocalHistogramState {
    bounds: ClassicHistogramBounds,
    shards: ThreadLocal<Box<ShardState>>,
}

struct ShardState {
    sum: AtomicU64,
    count: AtomicU64,
    buckets: Box<[AtomicU64]>,
}

impl ThreadLocalHistogram {
    /// Creates a thread-local histogram with the provided inclusive upper bounds.
    ///
    /// Bounds may be given in any order; they are sorted ascending. A terminal
    /// `f64::INFINITY` bucket is appended automatically.
    pub fn new(bounds: impl IntoIterator<Item = f64>) -> Self {
        Self {
            state: Arc::new(ThreadLocalHistogramState {
                bounds: ClassicHistogramBounds::new(bounds),
                shards: ThreadLocal::new(),
            }),
        }
    }

    /// Records an observed value in the current thread's histogram storage.
    #[inline]
    pub fn observe(&self, value: f64) {
        let shard = self.state.shards.get_or(|| {
            Box::new(ShardState {
                sum: AtomicU64::new(0.0_f64.to_bits()),
                count: AtomicU64::new(0),
                buckets: (0..self.state.bounds.len())
                    .map(|_| AtomicU64::new(0))
                    .collect(),
            })
        });

        let sum = f64::from_bits(shard.sum.load(Ordering::Relaxed));
        shard.sum.store((sum + value).to_bits(), Ordering::Relaxed);
        let count = shard.count.load(Ordering::Relaxed);
        shard.count.store(count.wrapping_add(1), Ordering::Relaxed);

        let Some(index) = self.state.bounds.bucket_index(value) else {
            return;
        };
        if let Some(bucket) = shard.buckets.get(index) {
            let count = bucket.load(Ordering::Relaxed);
            bucket.store(count.wrapping_add(1), Ordering::Relaxed);
        }
    }

    /// Returns a point-in-time aggregate snapshot of all thread-local values.
    pub fn snapshot(&self) -> HistogramSnapshot {
        let mut sum = 0.0;
        let mut count = 0_u64;
        let mut buckets = vec![0_u64; self.state.bounds.len()];

        for shard in self.state.shards.iter() {
            sum += f64::from_bits(shard.sum.load(Ordering::Relaxed));
            count = count.wrapping_add(shard.count.load(Ordering::Relaxed));

            for (total, shard_count) in buckets.iter_mut().zip(shard.buckets.iter()) {
                *total = total.wrapping_add(shard_count.load(Ordering::Relaxed));
            }
        }

        self.state.bounds.snapshot(sum, count, &buckets)
    }
}

impl fmt::Debug for ThreadLocalHistogram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let snapshot = self.snapshot();
        f.debug_struct("ThreadLocalHistogram")
            .field("sum", &snapshot.sum())
            .field("count", &snapshot.count())
            .field("buckets", &snapshot.buckets())
            .finish()
    }
}

impl EncodeMetricValue for ThreadLocalHistogram {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        encode_snapshot(self.snapshot())
    }
}

impl MetricConstructor<ThreadLocalHistogram> for HistogramBuilder {
    fn new_metric(&self) -> ThreadLocalHistogram {
        ThreadLocalHistogram::new(self.buckets.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use foundations_metrics_registry::proto::MetricType;

    use super::*;
    use crate::{EncodeMetric, NamedMetric};

    #[test]
    fn clones_share_storage() {
        let histogram = ThreadLocalHistogram::new([1.0]);
        let clone = histogram.clone();

        histogram.observe(0.25);
        clone.observe(0.75);

        assert_eq!(histogram.snapshot().count(), 2);
        assert_eq!(
            histogram.snapshot().buckets(),
            &[(1.0, 2), (f64::INFINITY, 0)]
        );
    }

    #[test]
    fn aggregates_values_from_multiple_threads() {
        let histogram = ThreadLocalHistogram::new([1.0, 2.0]);
        let barrier = Arc::new(Barrier::new(5));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let histogram = histogram.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..1_000 {
                        histogram.observe(1.5);
                    }
                    barrier.wait();
                })
            })
            .collect();

        barrier.wait();
        barrier.wait();
        for thread in threads {
            thread.join().expect("histogram thread should not panic");
        }

        let snapshot = histogram.snapshot();
        assert_eq!(snapshot.count(), 4_000);
        assert_eq!(snapshot.sum(), 6_000.0);
        assert_eq!(
            snapshot.buckets(),
            &[(1.0, 0), (2.0, 4_000), (f64::INFINITY, 0)]
        );
    }

    #[test]
    fn sorts_bounds_and_uses_inclusive_bucket_boundaries() {
        let histogram = ThreadLocalHistogram::new([2.0, 1.0]);

        for value in [0.5, 1.0, 1.5, 2.0, 3.0, f64::NAN] {
            histogram.observe(value);
        }

        let snapshot = histogram.snapshot();
        assert_eq!(snapshot.count(), 6);
        assert!(snapshot.sum().is_nan());
        assert_eq!(
            snapshot.buckets(),
            &[(1.0, 2), (2.0, 2), (f64::INFINITY, 1)]
        );
    }

    #[test]
    fn histogram_builder_constructs_thread_local_histograms() {
        let histogram: ThreadLocalHistogram = HistogramBuilder {
            buckets: &[0.5, 1.0],
        }
        .new_metric();

        histogram.observe(0.75);
        assert_eq!(histogram.snapshot().buckets()[1], (1.0, 1));
    }

    #[test]
    fn encodes_one_classic_histogram_with_cumulative_buckets() {
        let histogram = ThreadLocalHistogram::new([1.0, 2.0]);
        histogram.observe(0.5);
        histogram.observe(1.5);
        histogram.observe(3.0);

        let families =
            NamedMetric::new("request_duration_seconds", "Request duration.", histogram).encode();
        assert_eq!(families.len(), 1);
        assert_eq!(
            families[0].name.as_deref(),
            Some("request_duration_seconds")
        );
        assert_eq!(families[0].help.as_deref(), Some("Request duration."));
        assert_eq!(families[0].r#type, Some(MetricType::Histogram as i32));

        let encoded = families[0].metric[0]
            .histogram
            .as_ref()
            .expect("encoded histogram should be present");
        assert_eq!(encoded.sample_count, Some(3));
        assert_eq!(encoded.sample_sum, Some(5.0));
        assert_eq!(
            encoded
                .bucket
                .iter()
                .map(|bucket| (bucket.upper_bound, bucket.cumulative_count))
                .collect::<Vec<_>>(),
            vec![
                (Some(1.0), Some(1)),
                (Some(2.0), Some(2)),
                (Some(f64::INFINITY), Some(3)),
            ]
        );
    }
}
