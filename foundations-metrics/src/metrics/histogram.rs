use std::sync::Arc;

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
    state: Arc<Mutex<State>>,
}

#[derive(Debug)]
struct State {
    sum: f64,
    count: u64,
    buckets: Vec<(f64, u64)>,
    sorted: bool,
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    sum: f64,
    count: u64,
    buckets: Vec<(f64, u64)>,
}

impl Histogram {
    /// Creates a histogram with the provided inclusive upper bounds.
    ///
    /// Bounds should be ordered from smallest to largest. A terminal
    /// `f64::MAX` bucket is appended automatically.
    pub fn new(bounds: impl IntoIterator<Item = f64>) -> Self {
        let mut buckets: Vec<_> = bounds.into_iter().map(|bound| (bound, 0)).collect();
        buckets.push((f64::MAX, 0));

        let sorted = buckets.windows(2).all(|window| window[0].0 <= window[1].0);

        Self {
            state: Arc::new(Mutex::new(State {
                sum: 0.0,
                count: 0,
                buckets,
                sorted,
            })),
        }
    }

    /// Records an observed value.
    pub fn observe(&self, value: f64) {
        let mut state = self.state.lock();
        state.sum += value;
        state.count = state.count.wrapping_add(1);

        let bucket = if value.is_nan() {
            None
        } else if state.sorted {
            let index = state
                .buckets
                .partition_point(|(upper_bound, _)| *upper_bound < value);
            state.buckets.get_mut(index)
        } else {
            state
                .buckets
                .iter_mut()
                .find(|(upper_bound, _)| *upper_bound >= value)
        };

        if let Some((_, count)) = bucket {
            *count = count.wrapping_add(1);
        }
    }

    fn snapshot(&self) -> Snapshot {
        let state = self.state.lock();
        Snapshot {
            sum: state.sum,
            count: state.count,
            buckets: state.buckets.clone(),
        }
    }
}

impl EncodeMetricValue for Histogram {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let snapshot = self.snapshot();
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
}

/// Constructs classic histograms with a fixed set of buckets.
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
            Snapshot {
                sum: 6.0,
                count: 4,
                buckets: vec![(1.0, 2), (2.0, 1), (f64::MAX, 1)],
            }
        );
    }

    #[test]
    fn preserves_legacy_behavior_for_nan_and_unsorted_bounds() {
        let nan = Histogram::new([1.0]);
        nan.observe(f64::NAN);
        let snapshot = nan.snapshot();
        assert!(snapshot.sum.is_nan());
        assert_eq!(snapshot.count, 1);
        assert!(snapshot.buckets.iter().all(|(_, count)| *count == 0));

        let unsorted = Histogram::new([10.0, 1.0]);
        unsorted.observe(0.5);
        assert_eq!(unsorted.snapshot().buckets[0], (10.0, 1));
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
}
