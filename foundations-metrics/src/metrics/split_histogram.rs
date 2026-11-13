use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::RwLock;

use crate::{EncodeMetricValue, MetricFamily};

use super::histogram::{HistogramSnapshot, encode_snapshot};
use super::update_f64;

/// A classic fixed-bucket histogram aggregated from explicitly created shards.
///
/// Each shard records independently using relaxed atomics. Collection sums all
/// shards registered through [`SplitHistogram::shard`] and emits one ordinary
/// classic Prometheus histogram.
///
/// # Examples
///
/// ```
/// use std::sync::LazyLock;
///
/// use foundations_metrics::{
///     EncodeMetric, NamedMetric, SplitHistogram, SplitHistogramShard,
/// };
///
/// static HISTOGRAM: LazyLock<SplitHistogram> =
///     LazyLock::new(|| SplitHistogram::new([0.1, 1.0, 10.0]));
///
/// std::thread_local! {
///     static HISTOGRAM_SHARD: SplitHistogramShard = HISTOGRAM.shard();
/// }
///
/// HISTOGRAM_SHARD.with(|shard| shard.observe(0.5));
///
/// // Register a clone of the collecting metric. The clone shares its shard list.
/// let registered = NamedMetric::new(
///     "request_duration_seconds",
///     "Request duration.",
///     (*HISTOGRAM).clone(),
/// );
/// assert_eq!(registered.encode()[0].name.as_deref(), Some("request_duration_seconds"));
/// ```
#[derive(Clone, Debug)]
pub struct SplitHistogram {
    state: Arc<SplitHistogramState>,
}

#[derive(Debug)]
struct SplitHistogramState {
    bounds: Arc<[f64]>,
    shards: RwLock<Vec<Arc<ShardState>>>,
}

/// A cloneable handle to one shard of a [`SplitHistogram`].
///
/// Create shards with [`SplitHistogram::shard`]. Clones share the same
/// per-shard counters, and the parent histogram retains the shard for its
/// lifetime so observations remain collectable after handles are dropped.
#[derive(Clone, Debug)]
pub struct SplitHistogramShard {
    state: Arc<ShardState>,
}

#[derive(Debug)]
struct ShardState {
    bounds: Arc<[f64]>,
    sum: AtomicU64,
    count: AtomicU64,
    buckets: Box<[AtomicU64]>,
}

impl SplitHistogram {
    /// Creates a split histogram with the provided inclusive upper bounds.
    ///
    /// Bounds may be given in any order; they are sorted ascending. A terminal
    /// `f64::INFINITY` bucket is appended automatically.
    pub fn new(bounds: impl IntoIterator<Item = f64>) -> Self {
        let mut bounds: Vec<_> = bounds.into_iter().collect();
        bounds.push(f64::INFINITY);
        bounds.sort_by(f64::total_cmp);

        Self {
            state: Arc::new(SplitHistogramState {
                bounds: bounds.into(),
                shards: RwLock::new(Vec::new()),
            }),
        }
    }

    /// Creates and registers a compatible shard with this histogram.
    pub fn shard(&self) -> SplitHistogramShard {
        let state = Arc::new(ShardState {
            bounds: Arc::clone(&self.state.bounds),
            sum: AtomicU64::new(0.0_f64.to_bits()),
            count: AtomicU64::new(0),
            buckets: (0..self.state.bounds.len())
                .map(|_| AtomicU64::new(0))
                .collect(),
        });

        self.state.shards.write().push(Arc::clone(&state));
        SplitHistogramShard { state }
    }

    /// Returns a point-in-time aggregate snapshot of all registered shards.
    pub fn snapshot(&self) -> HistogramSnapshot {
        let shards = self.state.shards.read();
        let mut sum = 0.0;
        let mut count = 0_u64;
        let mut buckets = vec![0_u64; self.state.bounds.len()];

        for shard in shards.iter() {
            sum += f64::from_bits(shard.sum.load(Ordering::Relaxed));
            count = count.wrapping_add(shard.count.load(Ordering::Relaxed));

            for (total, shard_count) in buckets.iter_mut().zip(shard.buckets.iter()) {
                *total = total.wrapping_add(shard_count.load(Ordering::Relaxed));
            }
        }

        HistogramSnapshot::from_parts(
            sum,
            count,
            self.state.bounds.iter().copied().zip(buckets).collect(),
        )
    }
}

impl SplitHistogramShard {
    /// Records an observed value in this shard.
    #[inline]
    pub fn observe(&self, value: f64) {
        update_f64(&self.state.sum, |sum| sum + value);
        self.state.count.fetch_add(1, Ordering::Relaxed);

        if value.is_nan() {
            return;
        }

        let index = self
            .state
            .bounds
            .partition_point(|upper_bound| *upper_bound < value);
        if let Some(bucket) = self.state.buckets.get(index) {
            bucket.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl EncodeMetricValue for SplitHistogram {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        encode_snapshot(self.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::MetricType;

    use super::*;
    use crate::{EncodeMetric, NamedMetric};

    #[test]
    fn shard_clones_share_storage() {
        let histogram = SplitHistogram::new([1.0]);
        let shard = histogram.shard();
        let clone = shard.clone();

        shard.observe(0.25);
        clone.observe(0.75);
        drop(shard);
        drop(clone);

        let parent_clone = histogram.clone();
        assert_eq!(parent_clone.snapshot().count(), 2);
        assert_eq!(
            parent_clone.snapshot().buckets(),
            &[(1.0, 2), (f64::INFINITY, 0)]
        );
    }

    #[test]
    fn aggregates_shards_updated_by_multiple_threads() {
        let histogram = SplitHistogram::new([1.0, 2.0]);
        let shards: Vec<_> = (0..4).map(|_| histogram.shard()).collect();
        let threads: Vec<_> = shards
            .into_iter()
            .map(|shard| {
                std::thread::spawn(move || {
                    for _ in 0..1_000 {
                        shard.observe(1.5);
                    }
                })
            })
            .collect();

        for thread in threads {
            thread.join().expect("shard thread should not panic");
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
        let histogram = SplitHistogram::new([2.0, 1.0]);
        let first = histogram.shard();
        let second = histogram.shard();

        for value in [0.5, 1.0, 1.5, 2.0, 3.0] {
            first.observe(value);
        }
        second.observe(f64::NAN);

        let snapshot = histogram.snapshot();
        assert_eq!(snapshot.count(), 6);
        assert!(snapshot.sum().is_nan());
        assert_eq!(
            snapshot.buckets(),
            &[(1.0, 2), (2.0, 2), (f64::INFINITY, 1)]
        );
    }

    #[test]
    fn encodes_one_classic_histogram_with_cumulative_buckets() {
        let histogram = SplitHistogram::new([1.0, 2.0]);
        let first = histogram.shard();
        let second = histogram.shard();
        first.observe(0.5);
        first.observe(1.5);
        second.observe(3.0);

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
