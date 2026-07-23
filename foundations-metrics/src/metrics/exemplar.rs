use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::SystemTime;

use foundations_metrics_registry::proto;
use parking_lot::{MappedRwLockReadGuard, RwLock, RwLockReadGuard};
use prost_types::Timestamp;
use serde::Serialize;

use super::counter::encode_counter;
use super::histogram::encode_snapshot;
use super::{
    CounterAtomic, Histogram, HistogramBuilder, IntoF64, MetricConstructor, NativeHistogram,
    NativeHistogramBuilder,
};
use crate::diagnostics::report_collect_error;
use crate::labels::to_label_pairs;
use crate::validation::EXEMPLAR_SERIALIZATION_ERROR_LABEL;
use crate::{MetricFamily, value::EncodeMetricValue};

/// Labels and a sampled value associated with a metric observation.
///
/// Exemplar values are exposed through [`CounterWithExemplar::get`]. Their fields
/// remain private; exemplars are created by recording labeled metric updates.
#[derive(Debug)]
pub struct Exemplar<S, V> {
    label_set: Arc<S>,
    value: V,
    timestamp: Option<Timestamp>,
}

impl<S, V> Exemplar<S, V> {
    /// Returns the exemplar's label set.
    pub fn label_set(&self) -> &S {
        &self.label_set
    }

    /// Returns the sampled increment or observation.
    pub fn value(&self) -> &V {
        &self.value
    }
}

impl<S, V> Exemplar<S, V> {
    fn new(label_set: S, value: V, timestamp: Option<Timestamp>) -> Self {
        Self {
            label_set: Arc::new(label_set),
            value,
            timestamp,
        }
    }
}

impl<S, V: Clone> Clone for Exemplar<S, V> {
    fn clone(&self) -> Self {
        Self {
            label_set: Arc::clone(&self.label_set),
            value: self.value.clone(),
            timestamp: self.timestamp,
        }
    }
}

impl<S, V> Exemplar<S, V> {
    fn encode(&self) -> Result<proto::Exemplar, String>
    where
        S: Serialize,
        V: Clone + IntoF64,
    {
        Ok(proto::Exemplar {
            label: to_label_pairs(self.label_set.as_ref()).map_err(|error| error.to_string())?,
            value: Some(self.value.clone().into_f64()),
            timestamp: self.timestamp,
        })
    }
}

fn finish_exemplar(exemplar: Option<Result<proto::Exemplar, String>>) -> Option<proto::Exemplar> {
    match exemplar {
        Some(Ok(exemplar)) => Some(exemplar),
        // Defer reporting to validation, after any enclosing Family lock has
        // been released.
        Some(Err(error)) => Some(proto::Exemplar {
            label: vec![proto::LabelPair {
                name: Some(EXEMPLAR_SERIALIZATION_ERROR_LABEL.to_owned()),
                value: Some(error),
            }],
            ..Default::default()
        }),
        None => None,
    }
}

/// A monotonically increasing counter that records an exemplar with an update.
///
/// Calling [`inc_by`](Self::inc_by) with a label set replaces the previous
/// exemplar. Calling it without a label set clears the previous exemplar. The
/// exemplar value is the increment. [`get`](Self::get) returns the cumulative
/// counter value and read-only access to the current exemplar. Clones share both
/// values.
///
/// Exemplar labels are serialized with [`serde::Serialize`] during collection.
#[derive(Debug)]
pub struct CounterWithExemplar<S, N = u64, A = AtomicU64> {
    state: Arc<RwLock<CounterWithExemplarState<S, N, A>>>,
    marker: PhantomData<N>,
}

#[derive(Debug)]
struct CounterWithExemplarState<S, N, A> {
    value: A,
    exemplar: Option<Exemplar<S, N>>,
}

impl<S, N, A> CounterWithExemplar<S, N, A>
where
    N: Clone,
    A: CounterAtomic<N>,
{
    /// Increments the counter by `value`, returning the previous total.
    ///
    /// A provided label set replaces the stored exemplar. `None` clears it.
    pub fn inc_by(&self, value: N, label_set: Option<S>) -> N {
        let exemplar = label_set.map(|label_set| Exemplar::new(label_set, value.clone(), None));
        let mut state = self.state.write();
        state.exemplar = exemplar;
        state.value.inc_by(value)
    }

    /// Returns the cumulative counter value and the current exemplar.
    ///
    /// The exemplar guard keeps the counter read-locked until it is dropped.
    pub fn get(&self) -> (N, MappedRwLockReadGuard<'_, Option<Exemplar<S, N>>>) {
        let state = self.state.read();
        let value = state.value.get();
        let exemplar = RwLockReadGuard::map(state, |state| &state.exemplar);
        (value, exemplar)
    }

    /// Returns read-only access to the underlying counter storage.
    ///
    /// The returned guard prevents updates until it is dropped.
    pub fn inner(&self) -> MappedRwLockReadGuard<'_, A> {
        RwLockReadGuard::map(self.state.read(), |state| &state.value)
    }
}

impl<S, N, A> Clone for CounterWithExemplar<S, N, A> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            marker: PhantomData,
        }
    }
}

impl<S, N, A: Default> Default for CounterWithExemplar<S, N, A> {
    fn default() -> Self {
        Self {
            state: Arc::new(RwLock::new(CounterWithExemplarState {
                value: A::default(),
                exemplar: None,
            })),
            marker: PhantomData,
        }
    }
}

impl<S, N, A> EncodeMetricValue for CounterWithExemplar<S, N, A>
where
    S: Serialize + Send + Sync + 'static,
    N: Clone + IntoF64,
    A: CounterAtomic<N> + Send + Sync + 'static,
{
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let state = self.state.read();
        let value = state.value.get().into_f64();
        let exemplar = state.exemplar.clone();
        drop(state);

        let mut families = encode_counter(value);
        let counter = families[0].metric[0]
            .counter
            .as_mut()
            .expect("counter encoder always creates a counter value");
        counter.exemplar = finish_exemplar(exemplar.as_ref().map(Exemplar::encode));
        families
    }
}

/// A classic fixed-bucket histogram that retains one exemplar per bucket.
///
/// A labeled observation replaces the exemplar in its bucket. An unlabeled
/// observation updates the histogram without clearing an existing exemplar.
/// Clones share histogram and exemplar storage.
#[derive(Clone, Debug)]
pub struct HistogramWithExemplars<S> {
    state: Arc<RwLock<HistogramWithExemplarsState<S>>>,
}

#[derive(Debug)]
struct HistogramWithExemplarsState<S> {
    histogram: Histogram,
    exemplars: HashMap<usize, Exemplar<S, f64>>,
}

impl<S> HistogramWithExemplars<S> {
    /// Creates a histogram with the provided inclusive upper bounds.
    ///
    /// Bounds are sorted and a terminal `f64::MAX` bucket is appended.
    pub fn new(bounds: impl IntoIterator<Item = f64>) -> Self {
        Self {
            state: Arc::new(RwLock::new(HistogramWithExemplarsState {
                histogram: Histogram::new(bounds),
                exemplars: HashMap::new(),
            })),
        }
    }

    /// Records an observation and optionally associates it with an exemplar.
    pub fn observe(&self, value: f64, label_set: Option<S>) {
        let exemplar = label_set.map(|label_set| Exemplar::new(label_set, value, None));
        let mut state = self.state.write();
        let bucket = state.histogram.observe_and_bucket(value);

        if let (Some(bucket), Some(exemplar)) = (bucket, exemplar) {
            state.exemplars.insert(bucket, exemplar);
        }
    }
}

impl<S> EncodeMetricValue for HistogramWithExemplars<S>
where
    S: Serialize + Send + Sync + 'static,
{
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let state = self.state.read();
        let snapshot = state.histogram.snapshot();
        let exemplars: Vec<_> = state
            .exemplars
            .iter()
            .map(|(&index, exemplar)| (index, exemplar.clone()))
            .collect();
        drop(state);

        let mut families = encode_snapshot(snapshot);
        let buckets = &mut families[0].metric[0]
            .histogram
            .as_mut()
            .expect("histogram encoder always creates a histogram value")
            .bucket;

        for (index, exemplar) in exemplars {
            if let Some(bucket) = buckets.get_mut(index) {
                bucket.exemplar = finish_exemplar(Some(exemplar.encode()));
            }
        }

        families
    }
}

impl<S> MetricConstructor<HistogramWithExemplars<S>> for HistogramBuilder {
    fn new_metric(&self) -> HistogramWithExemplars<S> {
        HistogramWithExemplars::new(self.buckets.iter().copied())
    }
}

/// Constructs native histograms with exemplar support.
///
/// This mirrors [`NativeHistogramBuilder`] without adding another constructor
/// implementation to that type, which keeps existing constructor inference
/// unambiguous.
#[derive(Clone, Copy, Debug)]
pub struct NativeHistogramWithExemplarsBuilder {
    inner: NativeHistogramBuilder,
}

impl NativeHistogramWithExemplarsBuilder {
    /// Creates a builder with the given bucket growth `factor`.
    pub fn new(factor: f64) -> Self {
        Self {
            inner: NativeHistogramBuilder::new(factor),
        }
    }

    /// Sets the zero-bucket threshold.
    pub fn with_zero_threshold(mut self, zero_threshold: f64) -> Self {
        self.inner = self.inner.with_zero_threshold(zero_threshold);
        self
    }

    /// Sets a best-effort maximum number of populated buckets.
    pub fn with_max_buckets(mut self, max_buckets: usize) -> Self {
        self.inner = self.inner.with_max_buckets(max_buckets);
        self
    }
}

/// A native histogram that retains its latest labeled observation as an exemplar.
///
/// Native histogram exemplars require timestamps in the Prometheus protobuf
/// model. The timestamp is captured when the labeled observation is recorded.
/// Native histograms require protobuf exposition; OpenMetrics text encoding
/// does not expose their sparse buckets or exemplars.
#[derive(Clone, Debug)]
pub struct NativeHistogramWithExemplars<S> {
    state: Arc<RwLock<NativeHistogramWithExemplarsState<S>>>,
}

#[derive(Debug)]
struct NativeHistogramWithExemplarsState<S> {
    histogram: NativeHistogram,
    exemplar: Option<Exemplar<S, f64>>,
}

impl<S> NativeHistogramWithExemplars<S> {
    /// Creates a native histogram with the given bucket growth `factor`.
    ///
    /// # Panics
    ///
    /// Panics if `factor` is not greater than `1.0`.
    pub fn new(factor: f64) -> Self {
        NativeHistogramWithExemplarsBuilder::new(factor).new_metric()
    }

    /// Records an observation and optionally retains it as the latest exemplar.
    pub fn observe(&self, value: f64, label_set: Option<S>) {
        let exemplar = label_set.map(|label_set| Exemplar::new(label_set, value, None));
        let mut state = self.state.write();
        state.histogram.observe(value);

        if let Some(mut exemplar) = exemplar {
            exemplar.timestamp = Some(SystemTime::now().into());
            state.exemplar = Some(exemplar);
        }
    }
}

impl<S> EncodeMetricValue for NativeHistogramWithExemplars<S>
where
    S: Serialize + Send + Sync + 'static,
{
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let state = self.state.read();
        let families = state.histogram.try_encode_metric_value();
        let exemplar = state.exemplar.clone();
        drop(state);

        let mut families = match families {
            Ok(families) => families,
            Err(error) => {
                report_collect_error(format_args!(
                    "non-fatal error while collecting metrics: skipped a native histogram; protobuf encoding failed: {error}"
                ));
                return Vec::new();
            }
        };

        if let Some(exemplar) = finish_exemplar(exemplar.as_ref().map(Exemplar::encode)) {
            for histogram in families
                .iter_mut()
                .flat_map(|family| &mut family.metric)
                .filter_map(|metric| metric.histogram.as_mut())
            {
                histogram.exemplars.push(exemplar.clone());
            }
        }

        families
    }
}

impl<S> MetricConstructor<NativeHistogramWithExemplars<S>> for NativeHistogramWithExemplarsBuilder {
    fn new_metric(&self) -> NativeHistogramWithExemplars<S> {
        NativeHistogramWithExemplars {
            state: Arc::new(RwLock::new(NativeHistogramWithExemplarsState {
                histogram:
                    <NativeHistogramBuilder as MetricConstructor<NativeHistogram>>::new_metric(
                        &self.inner,
                    ),
                exemplar: None,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::MetricType;
    use prost::Message;
    use serde::Serialize;

    use super::*;
    use crate::{
        CollectionOptions, EncodeMetric, Family, NamedMetric, RegistrationMetadata,
        ServiceNameFormat, collect, encode_to_protobuf, encode_to_text, register,
    };

    #[derive(Clone, Debug, Serialize)]
    struct TraceLabels {
        trace_id: &'static str,
    }

    fn trace_id(exemplar: &proto::Exemplar) -> Option<&str> {
        exemplar
            .label
            .iter()
            .find(|label| label.name.as_deref() == Some("trace_id"))
            .and_then(|label| label.value.as_deref())
    }

    #[test]
    fn counter_replaces_and_clears_exemplars_and_clones_share_storage() {
        let counter = CounterWithExemplar::<TraceLabels>::default();
        let clone = counter.clone();

        assert_eq!(
            counter.inc_by(2, Some(TraceLabels { trace_id: "first" })),
            0
        );
        assert_eq!(clone.inc_by(3, Some(TraceLabels { trace_id: "latest" })), 2);

        let families = counter.encode_metric_value();
        let encoded = families[0].metric[0].counter.as_ref().unwrap();
        let exemplar = encoded.exemplar.as_ref().unwrap();
        assert_eq!(encoded.value, Some(5.0));
        assert_eq!(exemplar.value, Some(3.0));
        assert_eq!(trace_id(exemplar), Some("latest"));
        assert!(exemplar.timestamp.is_none());

        let (value, exemplar) = counter.get();
        assert_eq!(value, 5);
        assert!(exemplar.is_some());
        drop(exemplar);
        assert_eq!(
            counter.inner().load(std::sync::atomic::Ordering::Relaxed),
            5
        );

        assert_eq!(counter.inc_by(1, None), 5);
        assert_eq!(counter.get().0, 6);
        assert!(
            counter.encode_metric_value()[0].metric[0]
                .counter
                .as_ref()
                .unwrap()
                .exemplar
                .is_none()
        );
    }

    #[test]
    fn classic_histogram_retains_latest_exemplar_per_bucket() {
        let histogram = HistogramWithExemplars::new([1.0, 2.0]);
        histogram.observe(0.5, Some(TraceLabels { trace_id: "first" }));
        histogram.observe(
            0.75,
            Some(TraceLabels {
                trace_id: "replacement",
            }),
        );
        histogram.observe(0.8, None);
        histogram.observe(
            1.5,
            Some(TraceLabels {
                trace_id: "second_bucket",
            }),
        );

        let families = histogram.encode_metric_value();
        let encoded = families[0].metric[0].histogram.as_ref().unwrap();
        assert_eq!(encoded.sample_count, Some(4));
        assert_eq!(encoded.sample_sum, Some(3.55));
        assert_eq!(
            encoded
                .bucket
                .iter()
                .map(|bucket| bucket.cumulative_count)
                .collect::<Vec<_>>(),
            [Some(3), Some(4), Some(4)]
        );
        assert_eq!(
            trace_id(encoded.bucket[0].exemplar.as_ref().unwrap()),
            Some("replacement")
        );
        assert_eq!(
            encoded.bucket[0].exemplar.as_ref().unwrap().value,
            Some(0.75)
        );
        assert_eq!(
            trace_id(encoded.bucket[1].exemplar.as_ref().unwrap()),
            Some("second_bucket")
        );
        assert!(encoded.bucket[2].exemplar.is_none());
    }

    #[test]
    fn native_histogram_retains_latest_timestamped_exemplar() {
        let histogram = NativeHistogramWithExemplars::new(1.1);
        let clone = histogram.clone();
        histogram.observe(0.5, Some(TraceLabels { trace_id: "first" }));
        clone.observe(2.0, None);
        clone.observe(3.0, Some(TraceLabels { trace_id: "latest" }));

        let families = histogram.encode_metric_value();
        let encoded = families[0].metric[0].histogram.as_ref().unwrap();
        assert_eq!(encoded.sample_count, Some(3));
        assert_eq!(encoded.sample_sum, Some(5.5));
        assert!(!encoded.positive_span.is_empty());
        assert_eq!(encoded.exemplars.len(), 1);
        assert_eq!(encoded.exemplars[0].value, Some(3.0));
        assert_eq!(trace_id(&encoded.exemplars[0]), Some("latest"));
        assert!(encoded.exemplars[0].timestamp.is_some());
    }

    #[test]
    fn exemplar_label_failure_drops_only_the_exemplar() {
        let counter = CounterWithExemplar::<&'static str>::default();
        counter.inc_by(2, Some("not a label set"));

        let families = NamedMetric::new("serialization_failure", "", counter).encode();
        let sentinel = &families[0].metric[0]
            .counter
            .as_ref()
            .unwrap()
            .exemplar
            .as_ref()
            .unwrap()
            .label[0];
        assert_eq!(
            sentinel.name.as_deref(),
            Some(EXEMPLAR_SERIALIZATION_ERROR_LABEL)
        );
        assert_eq!(
            sentinel.value.as_deref(),
            Some("metric labels must serialize as a struct or unit")
        );

        let payload = encode_to_protobuf(&families);
        let encoded = MetricFamily::decode_length_delimited(payload.as_slice()).unwrap();
        let encoded = encoded.metric[0].counter.as_ref().unwrap();
        assert_eq!(encoded.value, Some(2.0));
        assert!(encoded.exemplar.is_none());
    }

    #[test]
    fn empty_exemplar_label_sets_are_retained_in_text() {
        let counter = CounterWithExemplar::<()>::default();
        counter.inc_by(2, Some(()));

        let families = NamedMetric::new("empty_exemplar", "", counter).encode();
        assert!(encode_to_text(&families).contains("empty_exemplar 2.0 # {} 2.0\n"));
    }

    #[test]
    fn accepts_legacy_sequence_label_sets() {
        let counter = CounterWithExemplar::<Vec<(&'static str, &'static str)>>::default();
        counter.inc_by(1, Some(vec![("trace_id", "legacy")]));

        let families = counter.encode_metric_value();
        let exemplar = families[0].metric[0]
            .counter
            .as_ref()
            .unwrap()
            .exemplar
            .as_ref()
            .unwrap();
        assert_eq!(trace_id(exemplar), Some("legacy"));
    }

    #[test]
    fn updates_do_not_require_serializable_label_sets() {
        struct OpaqueLabels;

        let counter = CounterWithExemplar::<OpaqueLabels>::default();
        counter.inc_by(1, Some(OpaqueLabels));
        assert_eq!(counter.get().0, 1);

        let classic = HistogramWithExemplars::new([1.0]);
        classic.observe(0.5, Some(OpaqueLabels));

        let native = NativeHistogramWithExemplars::new(1.1);
        native.observe(0.5, Some(OpaqueLabels));
    }

    #[test]
    fn families_keep_series_and_exemplar_labels_separate() {
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
        struct SeriesLabels {
            method: &'static str,
        }

        let family = Family::<SeriesLabels, CounterWithExemplar<TraceLabels>>::default();
        family
            .get_or_create(&SeriesLabels { method: "GET" })
            .inc_by(1, Some(TraceLabels { trace_id: "abc" }));

        let families = family.encode_metric_value();
        let metric = &families[0].metric[0];
        assert!(metric.label.iter().any(|label| {
            label.name.as_deref() == Some("method") && label.value.as_deref() == Some("GET")
        }));
        assert_eq!(
            trace_id(metric.counter.as_ref().unwrap().exemplar.as_ref().unwrap()),
            Some("abc")
        );
    }

    #[test]
    fn histogram_builders_construct_exemplar_metrics() {
        let classic: HistogramWithExemplars<TraceLabels> = HistogramBuilder {
            buckets: &[0.5, 1.0],
        }
        .new_metric();
        classic.observe(
            0.75,
            Some(TraceLabels {
                trace_id: "classic",
            }),
        );
        assert!(
            classic.encode_metric_value()[0].metric[0]
                .histogram
                .as_ref()
                .unwrap()
                .bucket[1]
                .exemplar
                .is_some()
        );

        let native: NativeHistogramWithExemplars<TraceLabels> =
            NativeHistogramWithExemplarsBuilder::new(1.1)
                .with_max_buckets(160)
                .new_metric();
        native.observe(0.75, Some(TraceLabels { trace_id: "native" }));
        assert_eq!(
            native.encode_metric_value()[0].metric[0]
                .histogram
                .as_ref()
                .unwrap()
                .exemplars
                .len(),
            1
        );
    }

    #[test]
    fn registered_exemplar_counter_is_collected_and_encoded() {
        let counter = CounterWithExemplar::<TraceLabels>::default();
        counter.inc_by(
            4,
            Some(TraceLabels {
                trace_id: "registered",
            }),
        );
        register(
            Box::new(NamedMetric::new(
                "registered_counter_with_exemplar",
                "A registered exemplar counter.",
                counter,
            )) as Box<dyn EncodeMetric>,
            RegistrationMetadata::default(),
        );

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: None,
            service_name_format: ServiceNameFormat::MetricPrefix,
        });
        let family = families
            .iter()
            .find(|family| family.name.as_deref() == Some("registered_counter_with_exemplar"))
            .expect("registered exemplar counter is collected");
        assert_eq!(family.r#type, Some(MetricType::Counter as i32));
        assert_eq!(
            trace_id(
                family.metric[0]
                    .counter
                    .as_ref()
                    .unwrap()
                    .exemplar
                    .as_ref()
                    .unwrap()
            ),
            Some("registered")
        );

        let text = encode_to_text(std::slice::from_ref(family));
        assert!(text.contains(
            "registered_counter_with_exemplar 4.0 # {\"trace_id\"=\"registered\"} 4.0\n"
        ));
    }

    #[test]
    fn registered_native_exemplar_family_round_trips_through_protobuf() {
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
        struct SeriesLabels {
            method: &'static str,
        }

        let family = Family::<
            SeriesLabels,
            NativeHistogramWithExemplars<TraceLabels>,
            NativeHistogramWithExemplarsBuilder,
        >::new_with_constructor(NativeHistogramWithExemplarsBuilder::new(1.1));
        family
            .get_or_create(&SeriesLabels { method: "GET" })
            .observe(0.25, Some(TraceLabels { trace_id: "native" }));
        register(
            Box::new(NamedMetric::new(
                "registered_native_histogram_with_exemplars",
                "A registered native histogram with exemplars.",
                family,
            )) as Box<dyn EncodeMetric>,
            RegistrationMetadata::default(),
        );

        let families = collect(CollectionOptions {
            include_optional: false,
            service_name: None,
            service_name_format: ServiceNameFormat::MetricPrefix,
        });
        let family = families
            .iter()
            .find(|family| {
                family.name.as_deref() == Some("registered_native_histogram_with_exemplars")
            })
            .expect("registered native histogram family is collected");
        let payload = encode_to_protobuf(std::slice::from_ref(family));
        let mut bytes = payload.as_slice();
        let decoded = MetricFamily::decode_length_delimited(&mut bytes).unwrap();
        let metric = &decoded.metric[0];
        assert!(metric.label.iter().any(|label| {
            label.name.as_deref() == Some("method") && label.value.as_deref() == Some("GET")
        }));
        let exemplars = &metric.histogram.as_ref().unwrap().exemplars;
        assert_eq!(exemplars.len(), 1);
        assert_eq!(trace_id(&exemplars[0]), Some("native"));
        assert!(exemplars[0].timestamp.is_some());
        assert!(bytes.is_empty());
    }
}
