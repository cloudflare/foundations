use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::ops::Deref;
use std::sync::Arc;

use parking_lot::{MappedRwLockReadGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};
use serde::Serialize;

use crate::diagnostics::report_collect_error;
use crate::{MetricFamily, labels::to_label_pairs, value::EncodeMetricValue};

// Adapted from prometools' `serde::Family`
// (https://github.com/nox/prometools, licensed MIT OR Apache-2.0).
/// A set of metrics differentiated by their label values.
///
/// Clones share the same metrics. Calling [`Family::get_or_create`] creates a
/// metric for a label set on first use and returns the existing metric on later
/// calls.
///
/// # Examples
///
/// ```
/// use foundations_metrics::{Counter, Family};
/// use serde::Serialize;
///
/// #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
/// struct Labels {
///     method: &'static str,
///     status: u16,
/// }
///
/// let requests = Family::<Labels, Counter>::default();
/// let success = Labels {
///     method: "GET",
///     status: 200,
/// };
///
/// requests.get_or_create(&success).inc();
/// requests.get_or_create(&success).inc_by(2);
/// assert_eq!(requests.get_or_create(&success).get(), 3);
///
/// // Drop any returned guards before removing metrics from the family.
/// assert!(requests.remove(&success));
/// assert_eq!(requests.get_or_create(&success).get(), 0);
/// ```
pub struct Family<S, M, C = fn() -> M> {
    metrics: Arc<RwLock<HashMap<S, M>>>,
    constructor: C,
}

/// Read-only access to a metric stored in a [`Family`].
///
/// The family remains read-locked until this guard is dropped.
#[must_use = "if unused the family read lock will immediately unlock"]
pub struct FamilyMetricGuard<'a, M: ?Sized> {
    guard: MappedRwLockReadGuard<'a, M>,
}

impl<'a, M: ?Sized> FamilyMetricGuard<'a, M> {
    fn new(guard: MappedRwLockReadGuard<'a, M>) -> Self {
        Self { guard }
    }
}

impl<M: ?Sized> Deref for FamilyMetricGuard<'_, M> {
    type Target = M;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<M: fmt::Debug + ?Sized> fmt::Debug for FamilyMetricGuard<'_, M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl<M: fmt::Display + ?Sized> fmt::Display for FamilyMetricGuard<'_, M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&**self, formatter)
    }
}

/// Constructs metrics for a [`Family`].
///
/// Custom constructors are useful for metrics that need configuration when
/// created, such as histograms with a specific set of buckets.
pub trait MetricConstructor<M> {
    /// Creates a new metric.
    fn new_metric(&self) -> M;
}

impl<M, F> MetricConstructor<M> for F
where
    F: Fn() -> M,
{
    fn new_metric(&self) -> M {
        self()
    }
}

impl<S, M, C> Family<S, M, C> {
    /// Creates an empty family that uses `constructor` for new label sets.
    ///
    /// ```
    /// use foundations_metrics::{Counter, Family};
    ///
    /// let counters = Family::<&'static str, Counter, _>::new_with_constructor(|| {
    ///     let counter = Counter::default();
    ///     counter.inc_by(10);
    ///     counter
    /// });
    ///
    /// assert_eq!(counters.get_or_create(&"priority").get(), 10);
    /// ```
    pub fn new_with_constructor(constructor: C) -> Self {
        Self {
            metrics: Arc::new(RwLock::new(HashMap::new())),
            constructor,
        }
    }
}

impl<S, M> Default for Family<S, M>
where
    M: Default,
{
    fn default() -> Self {
        Self::new_with_constructor(M::default)
    }
}

impl<S, M, C> Family<S, M, C>
where
    S: Clone + Eq + Hash,
    C: MetricConstructor<M>,
{
    /// Returns the metric for `label_set`, creating it when absent.
    ///
    /// The returned guard keeps the family read-locked. Holding it while
    /// accessing another label set in the same family can deadlock if that
    /// second label set needs to be created.
    pub fn get_or_create(&self, label_set: &S) -> FamilyMetricGuard<'_, M> {
        if let Ok(metric) =
            RwLockReadGuard::try_map(self.metrics.read(), |metrics| metrics.get(label_set))
        {
            return FamilyMetricGuard::new(metric);
        }

        let mut metrics = self.metrics.write();
        metrics
            .entry(label_set.clone())
            .or_insert_with(|| self.constructor.new_metric());

        let metrics = RwLockWriteGuard::downgrade(metrics);
        FamilyMetricGuard::new(RwLockReadGuard::map(metrics, |metrics| {
            metrics
                .get(label_set)
                .expect("metric exists after it was inserted")
        }))
    }

    /// Removes a label set, returning whether it was present.
    pub fn remove(&self, label_set: &S) -> bool {
        self.metrics.write().remove(label_set).is_some()
    }

    /// Removes every label set from this family.
    pub fn clear(&self) {
        self.metrics.write().clear();
    }
}

impl<S, M, C> Clone for Family<S, M, C>
where
    C: Clone,
{
    fn clone(&self) -> Self {
        Self {
            metrics: Arc::clone(&self.metrics),
            constructor: self.constructor.clone(),
        }
    }
}

impl<S, M, C> fmt::Debug for Family<S, M, C>
where
    S: fmt::Debug,
    M: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Family")
            .field("metrics", &self.metrics)
            .finish_non_exhaustive()
    }
}

impl<S, M, C> EncodeMetricValue for Family<S, M, C>
where
    S: Serialize + Send + Sync + 'static,
    M: EncodeMetricValue,
    C: Send + Sync + 'static,
{
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let metrics = self.metrics.read();
        // Each label set contributes one row per group, so a freshly created
        // group will grow to roughly this many rows.
        let metric_count = metrics.len();
        let mut grouped: Vec<MetricFamily> = Vec::new();
        let mut first_label_error = None;
        let mut label_error_count = 0;
        let mut first_metadata_error = None;
        let mut metadata_error_count = 0;

        for (label_set, metric) in metrics.iter() {
            let mut labels = match to_label_pairs(label_set) {
                Ok(labels) => labels,
                Err(error) => {
                    label_error_count += 1;
                    first_label_error.get_or_insert(error);
                    continue;
                }
            };

            let encoded = metric.encode_metric_value();
            let mut remaining_rows: usize = encoded
                .iter()
                .filter(|family| !family.metric.is_empty())
                .map(|family| family.metric.len())
                .sum();

            for mut family in encoded {
                if family.metric.is_empty() {
                    continue;
                }

                for row in &mut family.metric {
                    remaining_rows -= 1;

                    // Prepend so family labels stay before any metric-specific
                    // labels (e.g. a histogram's `le`). Move into the final
                    // consumer; clone for the rest.
                    if remaining_rows == 0 {
                        row.label.splice(0..0, labels.drain(..));
                    } else {
                        row.label.splice(0..0, labels.iter().cloned());
                    }
                }

                if let Some(existing) = grouped
                    .iter_mut()
                    .find(|existing| existing.name == family.name)
                {
                    if existing.help != family.help
                        || existing.r#type != family.r#type
                        || existing.unit != family.unit
                    {
                        metadata_error_count += 1;
                        first_metadata_error.get_or_insert_with(|| family.name.clone());
                        continue;
                    }

                    existing.metric.append(&mut family.metric);
                } else {
                    family
                        .metric
                        .reserve(metric_count.saturating_sub(family.metric.len()));
                    grouped.push(family);
                }
            }
        }

        drop(metrics);

        if let Some(error) = first_label_error {
            report_collect_error(format_args!(
                "non-fatal error while collecting metrics: skipped {label_error_count} label set(s); first serialization error: {error}"
            ));
        }

        if let Some(name) = first_metadata_error {
            report_collect_error(format_args!(
                "non-fatal error while collecting metrics: skipped {metadata_error_count} metric group(s) with inconsistent metadata; first relative name: {name:?}"
            ));
        }

        grouped
    }
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::MetricType;
    use serde::Serialize;

    use super::*;
    use crate::{Counter, RangeGauge};

    #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
    struct Labels {
        method: &'static str,
        status: u16,
    }

    #[test]
    fn creates_reuses_removes_and_clears_metrics() {
        let family = Family::<Labels, Counter>::default();
        let get = Labels {
            method: "GET",
            status: 200,
        };
        let post = Labels {
            method: "POST",
            status: 201,
        };

        family.get_or_create(&get).inc();
        family.get_or_create(&get).inc_by(2);
        family.get_or_create(&post).inc_by(4);

        assert_eq!(family.get_or_create(&get).get(), 3);
        assert_eq!(family.get_or_create(&post).get(), 4);
        assert!(family.remove(&post));
        assert!(!family.remove(&post));
        assert_eq!(family.get_or_create(&post).get(), 0);

        family.clear();
        assert_eq!(family.get_or_create(&get).get(), 0);
    }

    #[test]
    fn clones_share_metrics() {
        let family = Family::<Labels, Counter>::default();
        let clone = family.clone();
        let labels = Labels {
            method: "GET",
            status: 200,
        };

        family.get_or_create(&labels).inc();
        clone.get_or_create(&labels).inc();

        assert_eq!(family.get_or_create(&labels).get(), 2);
    }

    #[test]
    fn custom_constructor_creates_metrics() {
        let family = Family::<Labels, Counter, _>::new_with_constructor(|| {
            let counter = Counter::default();
            counter.inc_by(10);
            counter
        });
        let labels = Labels {
            method: "GET",
            status: 200,
        };

        assert_eq!(family.get_or_create(&labels).get(), 10);
    }

    #[test]
    fn encodes_one_row_per_label_set() {
        let family = Family::<Labels, Counter>::default();
        family
            .get_or_create(&Labels {
                method: "GET",
                status: 200,
            })
            .inc_by(3);
        family
            .get_or_create(&Labels {
                method: "POST",
                status: 201,
            })
            .inc_by(5);

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].name.as_deref(), Some(""));
        assert_eq!(families[0].r#type, Some(MetricType::Counter as i32));
        assert_eq!(families[0].metric.len(), 2);

        for row in &families[0].metric {
            let method = row
                .label
                .iter()
                .find(|label| label.name.as_deref() == Some("method"))
                .and_then(|label| label.value.as_deref())
                .expect("row has a method label");
            let value = row
                .counter
                .as_ref()
                .and_then(|counter| counter.value)
                .expect("row has a counter value");

            match method {
                "GET" => assert_eq!(value, 3.0),
                "POST" => assert_eq!(value, 5.0),
                method => panic!("unexpected method {method}"),
            }
        }
    }

    #[test]
    fn groups_each_range_gauge_suffix() {
        let family = Family::<Labels, RangeGauge>::default();
        let first = Labels {
            method: "GET",
            status: 200,
        };
        let second = Labels {
            method: "POST",
            status: 201,
        };

        family.get_or_create(&first).inc_by(3);
        family.get_or_create(&first).dec_by(2);
        family.get_or_create(&second).inc_by(5);

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 3);

        for (family, suffix) in families.iter().zip(["", "_min", "_max"]) {
            assert_eq!(family.name.as_deref(), Some(suffix));
            assert_eq!(family.r#type, Some(MetricType::Gauge as i32));
            assert_eq!(family.metric.len(), 2);
            assert!(family.metric.iter().all(|row| row.label.len() == 2));
        }
    }

    #[test]
    fn skips_only_label_sets_that_fail_to_serialize() {
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
        enum FallibleValue {
            Valid,
            Invalid(Vec<u8>),
        }

        #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
        struct FallibleLabels {
            value: FallibleValue,
        }

        let family = Family::<FallibleLabels, Counter>::default();
        family
            .get_or_create(&FallibleLabels {
                value: FallibleValue::Valid,
            })
            .inc_by(3);
        family
            .get_or_create(&FallibleLabels {
                value: FallibleValue::Invalid(vec![1, 2, 3]),
            })
            .inc_by(5);

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].metric.len(), 1);
        assert_eq!(
            families[0].metric[0].label[0].value.as_deref(),
            Some("Valid")
        );
        assert_eq!(
            families[0].metric[0]
                .counter
                .as_ref()
                .and_then(|counter| counter.value),
            Some(3.0)
        );
    }

    #[test]
    fn skips_metric_groups_with_inconsistent_metadata() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct MetadataMetric {
            unit: &'static str,
        }

        impl EncodeMetricValue for MetadataMetric {
            fn encode_metric_value(&self) -> Vec<MetricFamily> {
                let counter: Counter = Counter::default();
                let mut families = counter.encode_metric_value();
                families[0].unit = Some(self.unit.to_owned());
                families
            }
        }

        let metric_count = Arc::new(AtomicUsize::new(0));
        let family = Family::<Labels, MetadataMetric, _>::new_with_constructor({
            let metric_count = Arc::clone(&metric_count);
            move || MetadataMetric {
                unit: if metric_count.fetch_add(1, Ordering::Relaxed) == 0 {
                    "seconds"
                } else {
                    "bytes"
                },
            }
        });

        drop(family.get_or_create(&Labels {
            method: "GET",
            status: 200,
        }));
        drop(family.get_or_create(&Labels {
            method: "POST",
            status: 201,
        }));

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].metric.len(), 1);
    }

    #[test]
    fn empty_family_encodes_nothing() {
        let family = Family::<Labels, Counter>::default();
        assert!(family.encode_metric_value().is_empty());
    }
}
