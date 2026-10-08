use std::fmt;
use std::num::NonZeroUsize;
use std::sync::{Arc, OnceLock};

use serde::Serialize;

use super::family::MetricConstructor;
use crate::diagnostics::report_collect_error;
use crate::{MetricFamily, labels::to_label_pairs, value::EncodeMetricValue};

/// A finite label set with bounded, compile-time known cardinality.
///
/// Fieldless enums are typical low-cardinality label types.
///
/// Every value must return an index less than `CARDINALITY.get()`, and distinct
/// values must return distinct indices. Violating these requirements can merge
/// metric series or panic, but cannot cause memory unsafety. Indices are
/// process-local implementation details and must not be serialized or persisted.
pub trait LowCardinalityLabel: PartialEq {
    /// The number of possible values of this label type.
    const CARDINALITY: NonZeroUsize;

    /// Returns this value's unique index in `0..CARDINALITY.get()`.
    fn index(&self) -> usize;
}

/// A lazily initialized metric family indexed directly by low-cardinality labels.
///
/// Lookup performs no hashing and acquires no family lock. Storage is allocated
/// for `S::CARDINALITY` slots up front, while metrics and exported series are
/// created only when their labels are first accessed. Clones share the same
/// slots. Unlike [`Family`](crate::Family), this family cannot remove or clear
/// entries.
pub struct LowCardinalityFamily<S, M, C = fn() -> M> {
    slots: Arc<[OnceLock<(S, M)>]>,
    constructor: C,
}

impl<S, M, C> LowCardinalityFamily<S, M, C>
where
    S: LowCardinalityLabel,
{
    /// Creates an empty family that uses `constructor` for new label values.
    pub fn new_with_constructor(constructor: C) -> Self {
        let slots = std::iter::repeat_with(OnceLock::new)
            .take(S::CARDINALITY.get())
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            slots: Arc::from(slots),
            constructor,
        }
    }
}

impl<S, M> Default for LowCardinalityFamily<S, M>
where
    S: LowCardinalityLabel,
    M: Default,
{
    fn default() -> Self {
        Self::new_with_constructor(M::default)
    }
}

impl<S, M, C> LowCardinalityFamily<S, M, C>
where
    S: LowCardinalityLabel,
    C: MetricConstructor<M>,
{
    /// Returns the metric for `label_set`, creating it on first access.
    ///
    /// The label set is cloned only when its metric is first initialized.
    /// Concurrent first accesses initialize the slot once.
    ///
    /// # Panics
    ///
    /// Panics if [`LowCardinalityLabel::index`] returns an out-of-range index.
    pub fn get_or_create(&self, label_set: &S) -> &M
    where
        S: Clone,
    {
        let index = label_set.index();
        let slot = self.slots.get(index).unwrap_or_else(|| {
            panic!(
                "LowCardinalityLabel contract violated: index {index} is outside 0..{}",
                self.slots.len()
            )
        });

        let (stored_label, metric) =
            slot.get_or_init(|| (label_set.clone(), self.constructor.new_metric()));

        debug_assert!(
            stored_label == label_set,
            "LowCardinalityLabel contract violated: distinct values returned index {index}"
        );

        metric
    }
}

impl<S, M, C> Clone for LowCardinalityFamily<S, M, C>
where
    C: Clone,
{
    fn clone(&self) -> Self {
        Self {
            slots: Arc::clone(&self.slots),
            constructor: self.constructor.clone(),
        }
    }
}

impl<S, M, C> fmt::Debug for LowCardinalityFamily<S, M, C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let initialized_slots = self
            .slots
            .iter()
            .filter(|slot| slot.get().is_some())
            .count();

        formatter
            .debug_struct("LowCardinalityFamily")
            .field("cardinality", &self.slots.len())
            .field("initialized_slots", &initialized_slots)
            .finish_non_exhaustive()
    }
}

impl<S, M, C> EncodeMetricValue for LowCardinalityFamily<S, M, C>
where
    S: Serialize + Send + Sync + 'static,
    M: EncodeMetricValue,
    C: Send + Sync + 'static,
{
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let metric_count = self
            .slots
            .iter()
            .filter(|slot| slot.get().is_some())
            .count();
        let mut grouped: Vec<MetricFamily> = Vec::new();
        let mut first_label_error = None;
        let mut label_error_count = 0;
        let mut first_metadata_error = None;
        let mut metadata_error_count = 0;

        for (label_set, metric) in self.slots.iter().filter_map(OnceLock::get) {
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    use foundations_metrics_registry::proto::MetricType;
    use serde::Serialize;

    use super::*;
    use crate::{Counter, RangeGauge};

    #[derive(Clone, Debug, PartialEq, Serialize)]
    struct Labels {
        state: State,
    }

    #[derive(Clone, Debug, PartialEq, Serialize)]
    enum State {
        Ready,
        Busy,
    }

    impl LowCardinalityLabel for Labels {
        const CARDINALITY: NonZeroUsize = NonZeroUsize::new(2).unwrap();

        fn index(&self) -> usize {
            match self.state {
                State::Ready => 0,
                State::Busy => 1,
            }
        }
    }

    fn labels(state: State) -> Labels {
        Labels { state }
    }

    #[test]
    fn untouched_family_encodes_nothing() {
        let family = LowCardinalityFamily::<Labels, Counter>::default();
        assert!(family.encode_metric_value().is_empty());
    }

    #[test]
    fn reuses_metrics_and_keeps_indices_independent() {
        let family = LowCardinalityFamily::<Labels, Counter>::default();
        let ready = family.get_or_create(&labels(State::Ready));
        ready.inc_by(3);
        let ready_again = family.get_or_create(&labels(State::Ready));
        let busy = family.get_or_create(&labels(State::Busy));
        busy.inc_by(7);

        assert!(std::ptr::eq(ready, ready_again));
        assert_eq!(ready_again.get(), 3);
        assert_eq!(busy.get(), 7);
        assert!(!std::ptr::eq(ready, busy));
    }

    #[test]
    fn clones_share_initialized_slots() {
        let family = LowCardinalityFamily::<Labels, Counter>::default();
        let clone = family.clone();
        family.get_or_create(&labels(State::Ready)).inc();
        clone.get_or_create(&labels(State::Ready)).inc_by(2);

        assert_eq!(family.get_or_create(&labels(State::Ready)).get(), 3);
    }

    #[test]
    fn custom_constructor_runs_once_per_index() {
        let calls = Arc::new(AtomicUsize::new(0));
        let family = LowCardinalityFamily::<Labels, Counter, _>::new_with_constructor({
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::Relaxed);
                Counter::default()
            }
        });

        family.get_or_create(&labels(State::Ready));
        family.get_or_create(&labels(State::Ready));
        family.get_or_create(&labels(State::Busy));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    }

    struct CloneCounted(Arc<AtomicUsize>);

    impl Clone for CloneCounted {
        fn clone(&self) -> Self {
            self.0.fetch_add(1, Ordering::Relaxed);
            Self(Arc::clone(&self.0))
        }
    }

    impl PartialEq for CloneCounted {
        fn eq(&self, _other: &Self) -> bool {
            true
        }
    }

    impl LowCardinalityLabel for CloneCounted {
        const CARDINALITY: NonZeroUsize = NonZeroUsize::new(1).unwrap();

        fn index(&self) -> usize {
            0
        }
    }

    #[test]
    fn clones_label_only_during_initialization() {
        let clone_count = Arc::new(AtomicUsize::new(0));
        let labels = CloneCounted(Arc::clone(&clone_count));
        let family = LowCardinalityFamily::<CloneCounted, Counter>::default();

        family.get_or_create(&labels);
        family.get_or_create(&labels);

        assert_eq!(clone_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn concurrent_first_access_constructs_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let family = LowCardinalityFamily::<Labels, Counter, _>::new_with_constructor({
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::Relaxed);
                Counter::default()
            }
        });
        let barrier = Barrier::new(8);

        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    barrier.wait();
                    family.get_or_create(&labels(State::Ready)).inc();
                });
            }
        });

        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(family.get_or_create(&labels(State::Ready)).get(), 8);
    }

    #[test]
    fn encodes_only_touched_labels() {
        let family = LowCardinalityFamily::<Labels, Counter>::default();
        family.get_or_create(&labels(State::Busy)).inc_by(5);

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].metric.len(), 1);
        assert_eq!(
            families[0].metric[0].label[0].value.as_deref(),
            Some("Busy")
        );
        assert_eq!(
            families[0].metric[0]
                .counter
                .as_ref()
                .and_then(|counter| counter.value),
            Some(5.0)
        );
    }

    #[derive(Clone, PartialEq, Serialize)]
    struct PairLabels {
        protocol: Protocol,
        outcome: Outcome,
    }

    #[derive(Clone, PartialEq, Serialize)]
    enum Protocol {
        Tcp,
        Udp,
    }

    #[derive(Clone, PartialEq, Serialize)]
    enum Outcome {
        Success,
        Failure,
    }

    impl LowCardinalityLabel for PairLabels {
        const CARDINALITY: NonZeroUsize = NonZeroUsize::new(4).unwrap();

        fn index(&self) -> usize {
            let protocol = match self.protocol {
                Protocol::Tcp => 0,
                Protocol::Udp => 1,
            };
            let outcome = match self.outcome {
                Outcome::Success => 0,
                Outcome::Failure => 1,
            };
            protocol * 2 + outcome
        }
    }

    #[test]
    fn multiple_fields_encode_independent_values() {
        let family = LowCardinalityFamily::<PairLabels, Counter>::default();
        family
            .get_or_create(&PairLabels {
                protocol: Protocol::Tcp,
                outcome: Outcome::Success,
            })
            .inc_by(2);
        family
            .get_or_create(&PairLabels {
                protocol: Protocol::Udp,
                outcome: Outcome::Failure,
            })
            .inc_by(9);

        let families = family.encode_metric_value();
        assert_eq!(families[0].metric.len(), 2);
        assert!(families[0].metric.iter().all(|row| row.label.len() == 2));
        let values: Vec<_> = families[0]
            .metric
            .iter()
            .map(|row| {
                row.counter
                    .as_ref()
                    .and_then(|counter| counter.value)
                    .unwrap()
            })
            .collect();
        assert_eq!(values, [2.0, 9.0]);
    }

    #[test]
    fn groups_range_gauge_suffixes() {
        let family = LowCardinalityFamily::<Labels, RangeGauge>::default();
        family.get_or_create(&labels(State::Ready)).inc_by(3);
        family.get_or_create(&labels(State::Busy)).inc_by(5);

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 3);
        for (family, suffix) in families.iter().zip(["", "_min", "_max"]) {
            assert_eq!(family.name.as_deref(), Some(suffix));
            assert_eq!(family.r#type, Some(MetricType::Gauge as i32));
            assert_eq!(family.metric.len(), 2);
        }
    }

    #[derive(Clone, PartialEq, Serialize)]
    struct FallibleLabels {
        value: FallibleValue,
    }

    #[derive(Clone, PartialEq, Serialize)]
    enum FallibleValue {
        Valid,
        Invalid(Vec<u8>),
    }

    impl LowCardinalityLabel for FallibleLabels {
        const CARDINALITY: NonZeroUsize = NonZeroUsize::new(2).unwrap();

        fn index(&self) -> usize {
            match self.value {
                FallibleValue::Valid => 0,
                FallibleValue::Invalid(_) => 1,
            }
        }
    }

    #[test]
    fn skips_only_label_sets_that_fail_to_serialize() {
        let family = LowCardinalityFamily::<FallibleLabels, Counter>::default();
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
        let family = LowCardinalityFamily::<Labels, MetadataMetric, _>::new_with_constructor({
            let metric_count = Arc::clone(&metric_count);
            move || MetadataMetric {
                unit: if metric_count.fetch_add(1, Ordering::Relaxed) == 0 {
                    "seconds"
                } else {
                    "bytes"
                },
            }
        });

        family.get_or_create(&labels(State::Ready));
        family.get_or_create(&labels(State::Busy));

        let families = family.encode_metric_value();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].metric.len(), 1);
    }

    #[derive(Clone, PartialEq, Serialize)]
    struct OutOfRange;

    impl LowCardinalityLabel for OutOfRange {
        const CARDINALITY: NonZeroUsize = NonZeroUsize::new(1).unwrap();

        fn index(&self) -> usize {
            1
        }
    }

    #[test]
    #[should_panic(expected = "index 1 is outside 0..1")]
    fn out_of_range_index_panics_clearly() {
        LowCardinalityFamily::<OutOfRange, Counter>::default().get_or_create(&OutOfRange);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "distinct values returned index 0")]
    fn colliding_indices_are_detected() {
        #[derive(Clone)]
        struct Colliding(bool);

        impl PartialEq for Colliding {
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl LowCardinalityLabel for Colliding {
            const CARDINALITY: NonZeroUsize = NonZeroUsize::new(1).unwrap();

            fn index(&self) -> usize {
                0
            }
        }

        let family = LowCardinalityFamily::<Colliding, Counter>::default();
        family.get_or_create(&Colliding(false));
        family.get_or_create(&Colliding(true));
    }
}
