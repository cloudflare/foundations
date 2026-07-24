use std::iter::once;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use foundations_metrics_registry::proto::{self, Bucket, MetricType};
use parking_lot::Mutex;

use crate::{MetricFamily, value::EncodeMetricValue};

use super::MetricConstructor;

/// A classic fixed-bucket histogram.
///
/// Each observation contributes to the sum and count and increments the first
/// bucket whose inclusive upper bound contains it. Clones share the same
/// storage.
///
/// # Examples
///
/// ```
/// use foundations_metrics::Histogram;
///
/// let response_size = Histogram::new([100.0, 1_000.0, 10_000.0]);
/// response_size.observe(750.0);
/// response_size.observe(2_500.0);
/// ```
#[derive(Clone, Debug)]
pub struct Histogram {
    state: Arc<Mutex<HistogramState>>,
}

#[derive(Debug)]
struct HistogramState {
    sum: f64,
    count: u64,
    buckets: Vec<(f64, u64)>,
}

/// A point-in-time view of a histogram's sum, count, and buckets.
#[derive(Debug, PartialEq)]
pub struct HistogramSnapshot {
    sum: f64,
    count: u64,
    buckets: Vec<(f64, u64)>,
}

impl Histogram {
    /// Creates a histogram with the provided inclusive upper bounds.
    ///
    /// Bounds may be given in any order; they are sorted ascending. A terminal
    /// `f64::MAX` bucket is appended automatically.
    pub fn new(bounds: impl IntoIterator<Item = f64>) -> Self {
        let mut buckets: Vec<_> = bounds.into_iter().map(|bound| (bound, 0)).collect();
        buckets.push((f64::MAX, 0));
        buckets.sort_by(|(a, _), (b, _)| a.total_cmp(b));

        Self {
            state: Arc::new(Mutex::new(HistogramState {
                sum: 0.0,
                count: 0,
                buckets,
            })),
        }
    }

    /// Records an observed value.
    pub fn observe(&self, value: f64) {
        self.observe_and_bucket(value);
    }

    pub(super) fn observe_and_bucket(&self, value: f64) -> Option<usize> {
        let mut state = self.state.lock();
        state.sum += value;
        state.count = state.count.wrapping_add(1);

        if value.is_nan() {
            return None;
        }

        let index = state
            .buckets
            .partition_point(|(upper_bound, _)| *upper_bound < value);

        if let Some((_, count)) = state.buckets.get_mut(index) {
            *count = count.wrapping_add(1);
            Some(index)
        } else {
            None
        }
    }

    pub(super) fn snapshot(&self) -> HistogramSnapshot {
        let state = self.state.lock();
        HistogramSnapshot {
            sum: state.sum,
            count: state.count,
            buckets: state.buckets.clone(),
        }
    }
}

impl EncodeMetricValue for Histogram {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        encode_snapshot(self.snapshot())
    }
}

pub(super) fn encode_snapshot(snapshot: HistogramSnapshot) -> Vec<MetricFamily> {
    let mut cumulative_count = 0_u64;
    let buckets = snapshot
        .buckets
        .into_iter()
        .map(|(upper_bound, count)| {
            cumulative_count = cumulative_count.wrapping_add(count);
            Bucket {
                cumulative_count: Some(cumulative_count),
                upper_bound: Some(upper_bound),
                ..Default::default()
            }
        })
        .collect();

    vec![MetricFamily {
        name: Some(String::new()),
        help: None,
        r#type: Some(MetricType::Histogram as i32),
        metric: vec![proto::Metric {
            histogram: Some(proto::Histogram {
                sample_count: Some(snapshot.count),
                sample_sum: Some(snapshot.sum),
                bucket: buckets,
                ..Default::default()
            }),
            ..Default::default()
        }],
        unit: None,
    }]
}

/// Constructs classic and time histograms with a fixed set of buckets.
#[derive(Clone, Debug)]
pub struct HistogramBuilder {
    /// Inclusive upper bounds for the histogram buckets.
    pub buckets: &'static [f64],
}

impl MetricConstructor<Histogram> for HistogramBuilder {
    fn new_metric(&self) -> Histogram {
        Histogram::new(self.buckets.iter().copied())
    }
}

impl MetricConstructor<TimeHistogram> for HistogramBuilder {
    fn new_metric(&self) -> TimeHistogram {
        TimeHistogram::new(self.buckets.iter().copied())
    }
}

/// A faster, lock-free histogram for tracking time.
// Adapted from prometools' `histogram::TimeHistogram`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
#[derive(Debug)]
pub struct TimeHistogram {
    state: Arc<TimeHistogramState>,
}

/// Timer to measure and record the duration of an event.
///
/// This timer can be stopped and observed at most once, either automatically
/// (when it goes out of scope) or manually. Alternatively, it can be manually
/// stopped and discarded in order to not record its value.
// Adapted from prometools' `histogram::HistogramTimer`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
#[must_use = "HistogramTimer measures on Drop so should be assigned to named variable"]
pub struct HistogramTimer {
    histogram: TimeHistogram,
    observed: bool,
    start: Option<Instant>,
    accumulated: Duration,
}

#[derive(Debug)]
struct TimeHistogramState {
    sum: AtomicU64,
    count: AtomicU64,
    buckets: Vec<(f64, AtomicU64)>,
}

impl HistogramTimer {
    /// Pauses elapsed-time tracking.
    ///
    /// Calling this while the timer is already paused has no effect.
    pub fn pause(&mut self) {
        self.accumulated += self.start.map_or(Duration::ZERO, |value| {
            Instant::now().saturating_duration_since(value)
        });
        self.start = None
    }

    /// Resumes elapsed-time tracking.
    ///
    /// Calling this while the timer is already running has no effect.
    pub fn resume(&mut self) {
        if self.start.is_none() {
            self.start = Some(Instant::now());
        }
    }

    /// Stops the timer, records its duration, and returns the duration.
    pub fn stop_and_record(self) -> Duration {
        let mut timer = self;
        timer.observe(true)
    }

    /// Stops the timer without recording and returns its duration.
    pub fn stop_and_discard(self) -> Duration {
        let mut timer = self;
        timer.observe(false)
    }

    fn observe(&mut self, record: bool) -> Duration {
        let elapsed_since_start: Duration = self.start.map_or(Duration::ZERO, |value| {
            Instant::now().saturating_duration_since(value)
        });
        let elapsed = elapsed_since_start + self.accumulated;

        self.observed = true;
        if record {
            self.histogram.observe(elapsed.as_nanos() as u64);
        }

        elapsed
    }
}

impl Drop for HistogramTimer {
    fn drop(&mut self) {
        if !self.observed {
            self.observe(true);
        }
    }
}

impl Clone for TimeHistogram {
    fn clone(&self) -> Self {
        TimeHistogram {
            state: Arc::clone(&self.state),
        }
    }
}

impl TimeHistogram {
    /// Creates a time histogram with inclusive bucket bounds in seconds.
    ///
    /// Bounds may be given in any order; they are sorted ascending. A terminal
    /// `f64::MAX` bucket is appended automatically.
    pub fn new(buckets: impl IntoIterator<Item = f64>) -> Self {
        let mut buckets: Vec<_> = buckets
            .into_iter()
            .chain(once(f64::MAX))
            .map(|upper_bound| (upper_bound, AtomicU64::new(0)))
            .collect();
        buckets.sort_by(|(a, _), (b, _)| a.total_cmp(b));

        Self {
            state: Arc::new(TimeHistogramState {
                sum: Default::default(),
                count: Default::default(),
                buckets,
            }),
        }
    }

    /// Starts a timer that records its duration when stopped or dropped.
    pub fn start_timer(&self) -> HistogramTimer {
        HistogramTimer {
            histogram: self.clone(),
            observed: false,
            start: Some(Instant::now()),
            accumulated: Duration::new(0, 0),
        }
    }

    /// Records an observed duration in nanoseconds.
    pub fn observe(&self, nanos: u64) {
        self.observe_and_bucket(nanos);
    }

    fn observe_and_bucket(&self, v: u64) -> Option<usize> {
        self.state.sum.fetch_add(v, Ordering::Relaxed);
        self.state.count.fetch_add(1, Ordering::Relaxed);

        let first_bucket = self
            .state
            .buckets
            .iter()
            .enumerate()
            .find(|(_i, (upper_bound, _value))| upper_bound >= &(seconds(v)));

        match first_bucket {
            Some((i, (_upper_bound, value))) => {
                value.fetch_add(1, Ordering::Relaxed);
                Some(i)
            }
            None => None,
        }
    }

    /// Returns a snapshot whose sum and bucket bounds are expressed in seconds.
    pub fn snapshot(&self) -> HistogramSnapshot {
        let sum = seconds(self.state.sum.load(Ordering::Relaxed));
        let count = self.state.count.load(Ordering::Relaxed);
        let buckets = self
            .state
            .buckets
            .iter()
            .map(|(k, v)| (*k, v.load(Ordering::Relaxed)))
            .collect();

        HistogramSnapshot {
            sum,
            count,
            buckets,
        }
    }
}

impl HistogramSnapshot {
    /// Returns the sum of all observations.
    pub fn sum(&self) -> f64 {
        self.sum
    }

    /// Returns the number of observations.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// Returns each inclusive upper bound and its non-cumulative count.
    pub fn buckets(&self) -> &[(f64, u64)] {
        &self.buckets
    }
}

// Adapted from prometools' private `histogram::seconds`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
#[inline(always)]
fn seconds(val: u64) -> f64 {
    (val as f64) * 1E-9
}

impl EncodeMetricValue for TimeHistogram {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        encode_snapshot(self.snapshot())
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;

    use super::*;
    use crate::Family;

    #[test]
    fn appends_terminal_bucket_and_clones_share_storage() {
        let histogram = Histogram::new([1.0, 2.0]);
        let clone = histogram.clone();

        clone.observe(0.5);
        clone.observe(1.0);
        histogram.observe(1.5);
        histogram.observe(3.0);

        assert_eq!(
            histogram.snapshot(),
            HistogramSnapshot {
                sum: 6.0,
                count: 4,
                buckets: vec![(1.0, 2), (2.0, 1), (f64::MAX, 1)],
            }
        );
    }

    #[test]
    fn excludes_nan_and_sorts_unsorted_bounds() {
        let nan = Histogram::new([1.0]);
        nan.observe(f64::NAN);
        let snapshot = nan.snapshot();
        assert!(snapshot.sum.is_nan());
        assert_eq!(snapshot.count, 1);
        assert!(snapshot.buckets.iter().all(|(_, count)| *count == 0));

        // Bounds are sorted ascending regardless of the order they were given in,
        // so `0.5` lands in the `1.0` bucket rather than the leading `10.0` one.
        let unsorted = Histogram::new([10.0, 1.0]);
        unsorted.observe(0.5);
        let snapshot = unsorted.snapshot();
        assert_eq!(snapshot.buckets, vec![(1.0, 1), (10.0, 0), (f64::MAX, 0)],);
    }

    #[test]
    fn encodes_one_protobuf_histogram_with_cumulative_buckets() {
        let histogram = Histogram::new([1.0, 2.0]);
        histogram.observe(0.5);
        histogram.observe(1.5);
        histogram.observe(3.0);

        let families = histogram.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].name.as_deref(), Some(""));
        assert_eq!(families[0].help, None);
        assert_eq!(families[0].r#type, Some(MetricType::Histogram as i32));
        assert_eq!(families[0].unit, None);

        let encoded = families[0].metric[0]
            .histogram
            .as_ref()
            .expect("encoded histogram is present");
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
                (Some(f64::MAX), Some(3)),
            ]
        );
    }

    #[test]
    fn family_adds_labels_to_histogram_rows() {
        #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
        struct Labels {
            method: &'static str,
        }

        let family =
            Family::<Labels, Histogram, HistogramBuilder>::new_with_constructor(HistogramBuilder {
                buckets: &[0.1, 1.0],
            });
        family.get_or_create(&Labels { method: "GET" }).observe(0.5);
        family
            .get_or_create(&Labels { method: "POST" })
            .observe(2.0);

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].metric.len(), 2);
        assert!(families[0].metric.iter().all(|metric| {
            metric.histogram.is_some()
                && metric
                    .label
                    .iter()
                    .any(|label| label.name.as_deref() == Some("method"))
        }));
    }

    #[test]
    fn time_histogram_encodes_seconds_and_cumulative_buckets() {
        let histogram = TimeHistogram::new([1.0, 2.0]);
        histogram.observe(500_000_000);
        histogram.observe(1_500_000_000);
        histogram.observe(3_000_000_000);

        let families = histogram.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].name.as_deref(), Some(""));
        assert_eq!(families[0].r#type, Some(MetricType::Histogram as i32));

        let encoded = families[0].metric[0]
            .histogram
            .as_ref()
            .expect("encoded time histogram is present");
        assert_eq!(encoded.sample_count, Some(3));
        assert_eq!(encoded.sample_sum, Some(5.0));
        assert_eq!(
            encoded
                .bucket
                .iter()
                .map(|bucket| bucket.cumulative_count)
                .collect::<Vec<_>>(),
            vec![Some(1), Some(2), Some(3)]
        );
    }

    #[test]
    fn time_histogram_sorts_unsorted_buckets() {
        let histogram = TimeHistogram::new([2.0, 1.0]);
        histogram.observe(500_000_000);

        assert_eq!(
            histogram.snapshot().buckets(),
            &[(1.0, 1), (2.0, 0), (f64::MAX, 0)]
        );
    }

    #[test]
    fn time_histogram_tracks_seconds_and_clones_share_storage() {
        let histogram = TimeHistogram::new([1.0, 2.0, 4.0, 8.0, 16.0]);
        let clone = histogram.clone();

        for nanos in [
            1_000_000_000,
            1_500_000_000,
            2_500_000_000,
            8_500_000_000,
            500_000_000,
        ] {
            clone.observe(nanos);
        }

        let snapshot = histogram.snapshot();
        assert_eq!(snapshot.sum(), 14.0);
        assert_eq!(snapshot.count(), 5);
        assert_eq!(
            snapshot.buckets(),
            &[
                (1.0, 2),
                (2.0, 1),
                (4.0, 1),
                (8.0, 0),
                (16.0, 1),
                (f64::MAX, 0),
            ]
        );
    }

    #[test]
    fn histogram_builder_constructs_time_histograms() {
        let histogram: TimeHistogram = HistogramBuilder {
            buckets: &[0.5, 1.0],
        }
        .new_metric();

        histogram.observe(750_000_000);
        assert_eq!(histogram.snapshot().buckets()[1], (1.0, 1));
    }

    #[test]
    fn timer_records_once_or_discards() {
        let recorded = TimeHistogram::new([1.0]);
        let _duration = recorded.start_timer().stop_and_record();
        assert_eq!(recorded.snapshot().count(), 1);

        let discarded = TimeHistogram::new([1.0]);
        discarded.start_timer().stop_and_discard();
        assert_eq!(discarded.snapshot().count(), 0);

        let dropped = TimeHistogram::new([1.0]);
        drop(dropped.start_timer());
        assert_eq!(dropped.snapshot().count(), 1);
    }

    #[test]
    fn timer_pause_and_resume_are_idempotent() {
        let histogram = TimeHistogram::new([1.0]);
        let mut timer = histogram.start_timer();

        timer.pause();
        let paused_duration = timer.accumulated;
        assert!(timer.start.is_none());
        timer.pause();
        assert_eq!(timer.accumulated, paused_duration);

        timer.resume();
        let resumed_at = timer.start;
        assert!(resumed_at.is_some());
        timer.resume();
        assert_eq!(timer.start, resumed_at);

        timer.stop_and_discard();
        assert_eq!(histogram.snapshot().count(), 0);
    }

    #[test]
    fn family_adds_labels_to_time_histogram_rows() {
        #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
        struct Labels {
            operation: &'static str,
        }

        let family = Family::<Labels, TimeHistogram, HistogramBuilder>::new_with_constructor(
            HistogramBuilder {
                buckets: &[0.1, 1.0],
            },
        );
        family
            .get_or_create(&Labels { operation: "read" })
            .observe(500_000_000);
        family
            .get_or_create(&Labels { operation: "write" })
            .observe(1_500_000_000);

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].metric.len(), 2);
        assert!(families[0].metric.iter().all(|metric| {
            metric.histogram.is_some()
                && metric
                    .label
                    .iter()
                    .any(|label| label.name.as_deref() == Some("operation"))
        }));
    }
}
