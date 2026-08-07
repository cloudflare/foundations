use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use foundations_metrics_registry::proto;
use parking_lot::Mutex;
use prost_types::Timestamp;
use serde::Serialize;

use super::IntoF64;
use crate::labels::to_label_pairs;
use crate::validation::EXEMPLAR_SERIALIZATION_ERROR_LABEL;

/// Labels and a sampled value associated with a metric observation.
#[derive(Debug)]
pub(super) struct Exemplar<S, V> {
    label_set: Arc<S>,
    value: V,
    pub(super) timestamp: Option<Timestamp>,
}

impl<S, V> Exemplar<S, V> {
    pub(super) fn new(label_set: S, value: V, timestamp: Option<Timestamp>) -> Self {
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
    pub(super) fn encode(&self) -> Result<proto::Exemplar, String>
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

pub(super) fn finish_exemplar(
    exemplar: Option<Result<proto::Exemplar, String>>,
) -> Option<proto::Exemplar> {
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

/// A metric paired with exemplar storage.
///
/// The wrapper [`Deref`]s to the inner metric, so unlabeled updates go straight
/// to it and never touch the exemplar lock:
///
/// ```
/// use foundations_metrics::{Counter, WithExemplar};
/// # #[derive(serde::Serialize)]
/// # struct TraceLabels { trace_id: &'static str }
///
/// let requests: WithExemplar<Counter, TraceLabels> = WithExemplar::default();
///
/// // Unlabeled: identical to `Counter::inc`, and leaves any exemplar in place.
/// requests.inc();
///
/// // Labeled: the exemplar and the increment are applied together.
/// requests.inc_by_with_exemplar(TraceLabels { trace_id: "abc" }, 4);
///
/// assert_eq!(requests.get(), 5);
/// ```
///
/// A histogram exemplar can link an unusual observation to a trace:
///
/// ```
/// use foundations_metrics::{Histogram, WithExemplar};
/// # #[derive(serde::Serialize)]
/// # struct TraceLabels { trace_id: &'static str }
///
/// let request_latency = WithExemplar::<Histogram, TraceLabels>::new(
///     Histogram::new([0.1, 0.5, 1.0]),
/// );
///
/// request_latency.observe(0.05);
/// // Attach the trace ID of a slow request to the matching latency bucket.
/// request_latency.observe_with_exemplar(TraceLabels { trace_id: "abc123" }, 0.8);
/// ```
///
/// Cloning a supported metric shares both its inner metric and exemplar storage.
///
/// Exemplar labels are serialized with [`serde::Serialize`] during collection.
pub struct WithExemplar<T, S> {
    pub(super) inner: T,
    pub(super) exemplars: Arc<Mutex<ExemplarStorage<S>>>,
}

#[derive(Debug)]
pub(super) enum ExemplarStorage<S> {
    Empty,
    Single(Option<StoredExemplar<S>>),
    PerBucket(HashMap<usize, Exemplar<S, f64>>),
}

#[derive(Debug)]
pub(super) enum StoredExemplar<S> {
    I64(Exemplar<S, i64>),
    U64(Exemplar<S, u64>),
    F64(Exemplar<S, f64>),
}

impl<S> Clone for StoredExemplar<S> {
    fn clone(&self) -> Self {
        match self {
            Self::I64(exemplar) => Self::I64(exemplar.clone()),
            Self::U64(exemplar) => Self::U64(exemplar.clone()),
            Self::F64(exemplar) => Self::F64(exemplar.clone()),
        }
    }
}

impl<S> StoredExemplar<S> {
    pub(super) fn encode(&self) -> Result<proto::Exemplar, String>
    where
        S: Serialize,
    {
        match self {
            Self::I64(exemplar) => exemplar.encode(),
            Self::U64(exemplar) => exemplar.encode(),
            Self::F64(exemplar) => exemplar.encode(),
        }
    }
}

impl<S> ExemplarStorage<S> {
    fn clear(&mut self) {
        match self {
            Self::Empty => {}
            Self::Single(exemplar) => *exemplar = None,
            Self::PerBucket(exemplars) => exemplars.clear(),
        }
    }
}

impl<T, S> WithExemplar<T, S> {
    /// Wraps `metric` in exemplar storage.
    pub fn new(metric: T) -> Self {
        Self {
            inner: metric,
            exemplars: Arc::new(Mutex::new(ExemplarStorage::Empty)),
        }
    }

    /// Returns the wrapped metric.
    pub fn metric(&self) -> &T {
        &self.inner
    }

    /// Discards all stored exemplars.
    pub fn clear_exemplars(&self) {
        self.exemplars.lock().clear();
    }
}

impl<T, S> Deref for WithExemplar<T, S> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T, S> Clone for WithExemplar<T, S>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            exemplars: Arc::clone(&self.exemplars),
        }
    }
}

impl<T, S> Default for WithExemplar<T, S>
where
    T: Default,
{
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T, S> fmt::Debug for WithExemplar<T, S>
where
    T: fmt::Debug,
    S: fmt::Debug,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WithExemplar")
            .field("inner", &self.inner)
            .field("exemplars", &self.exemplars)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::MetricType;
    use prost::Message;
    use serde::Serialize;

    use super::*;
    use crate::value::EncodeMetricValue;
    use crate::{
        CollectionOptions, Counter, CounterAtomic, EncodeMetric, Family, Histogram,
        HistogramBuilder, MetricConstructor, MetricFamily, NamedMetric, NativeHistogram,
        NativeHistogramBuilder, RegistrationMetadata, ServiceNameFormat, collect,
        encode_to_protobuf, encode_to_text, register,
    };

    #[derive(Clone, Debug, Serialize)]
    struct TraceLabels {
        trace_id: &'static str,
    }

    #[derive(Default)]
    struct BlockingCounterAtomic {
        value: std::sync::atomic::AtomicU64,
        block_get: std::sync::atomic::AtomicBool,
        entered_get: (std::sync::Mutex<bool>, std::sync::Condvar),
        release_get: (std::sync::Mutex<bool>, std::sync::Condvar),
    }

    impl CounterAtomic<u64> for BlockingCounterAtomic {
        fn inc(&self) -> u64 {
            self.inc_by(1)
        }

        fn inc_by(&self, value: u64) -> u64 {
            self.value
                .fetch_add(value, std::sync::atomic::Ordering::Relaxed)
        }

        fn get(&self) -> u64 {
            if self.block_get.load(std::sync::atomic::Ordering::Relaxed) {
                let (entered, entered_condvar) = &self.entered_get;
                *entered.lock().unwrap() = true;
                entered_condvar.notify_one();

                let (release, release_condvar) = &self.release_get;
                let mut release = release.lock().unwrap();
                while !*release {
                    release = release_condvar.wait(release).unwrap();
                }
            }

            self.value.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    fn trace_id(exemplar: &proto::Exemplar) -> Option<&str> {
        exemplar
            .label
            .iter()
            .find(|label| label.name.as_deref() == Some("trace_id"))
            .and_then(|label| label.value.as_deref())
    }

    #[test]
    fn reads_and_updates_interleave_without_deadlock() {
        let counter = WithExemplar::<Counter, TraceLabels>::new(Counter::default());
        counter.inc_by_with_exemplar(TraceLabels { trace_id: "a" }, 1);

        let value = counter.get();
        assert_eq!(value, 1);
        counter.inc();
        assert_eq!(counter.get(), 2);
        let _inner = counter.inner();
        counter.inc_by_with_exemplar(TraceLabels { trace_id: "b" }, 1);
        assert_eq!(counter.get(), 3);
    }

    #[test]
    fn unlabeled_updates_do_not_take_the_exemplar_lock() {
        use std::sync::Arc as StdArc;
        use std::sync::mpsc;
        use std::time::Duration;

        let counter = StdArc::new(WithExemplar::<Counter, TraceLabels>::default());
        let exemplars = counter.exemplars.lock();

        let writer = StdArc::clone(&counter);
        let (updated_tx, updated_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            updated_tx.send(writer.inc()).unwrap();
        });

        assert_eq!(updated_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 0);
        drop(exemplars);
        handle.join().unwrap();
        assert_eq!(counter.get(), 1);
    }

    #[test]
    fn counter_replaces_exemplars_and_clones_share_storage() {
        let counter = WithExemplar::<Counter, TraceLabels>::default();
        let clone = counter.clone();

        assert_eq!(
            counter.inc_by_with_exemplar(TraceLabels { trace_id: "first" }, 2),
            0
        );
        assert_eq!(
            clone.inc_by_with_exemplar(TraceLabels { trace_id: "latest" }, 3),
            2
        );

        let families = counter.encode_metric_value();
        let encoded = families[0].metric[0].counter.as_ref().unwrap();
        let exemplar = encoded.exemplar.as_ref().unwrap();
        assert_eq!(encoded.value, Some(5.0));
        assert_eq!(exemplar.value, Some(3.0));
        assert_eq!(trace_id(exemplar), Some("latest"));
        assert!(exemplar.timestamp.is_none());

        assert_eq!(counter.get(), 5);
        assert_eq!(
            counter.inner().load(std::sync::atomic::Ordering::Relaxed),
            5
        );

        // Unlabeled increments go straight to the inner counter and leave the
        // exemplar in place.
        assert_eq!(counter.inc(), 5);
        assert_eq!(counter.get(), 6);
        assert!(
            counter.encode_metric_value()[0].metric[0]
                .counter
                .as_ref()
                .unwrap()
                .exemplar
                .is_some()
        );

        counter.clear_exemplars();
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
    fn counter_exemplars_preserve_their_value_type() {
        let unsigned = WithExemplar::<Counter, TraceLabels>::default();
        unsigned.inc_by_with_exemplar(
            TraceLabels {
                trace_id: "unsigned",
            },
            u64::MAX,
        );
        let unsigned_exemplars = unsigned.exemplars.lock();
        let ExemplarStorage::Single(Some(StoredExemplar::U64(unsigned_exemplar))) =
            &*unsigned_exemplars
        else {
            panic!("expected an unsigned counter exemplar");
        };
        assert_eq!(unsigned_exemplar.value, u64::MAX);
        assert_eq!(unsigned_exemplar.label_set.trace_id, "unsigned");
        drop(unsigned_exemplars);

        let float = WithExemplar::<Counter<f64>, TraceLabels>::default();
        float.inc_by_with_exemplar(TraceLabels { trace_id: "float" }, 1.5);
        let float_exemplars = float.exemplars.lock();
        let ExemplarStorage::Single(Some(StoredExemplar::F64(float_exemplar))) = &*float_exemplars
        else {
            panic!("expected a floating-point counter exemplar");
        };
        assert_eq!(float_exemplar.value, 1.5);
    }

    #[test]
    fn collection_keeps_labeled_counter_updates_atomic() {
        use std::sync::mpsc::{self, RecvTimeoutError};
        use std::time::Duration;

        let counter = Arc::new(WithExemplar::<
            Counter<u64, BlockingCounterAtomic>,
            TraceLabels,
        >::default());
        counter.inc_by_with_exemplar(TraceLabels { trace_id: "old" }, 1);
        counter
            .inner()
            .block_get
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let collector = Arc::clone(&counter);
        let collector = std::thread::spawn(move || collector.encode_metric_value());

        let (entered, entered_condvar) = &counter.inner().entered_get;
        let mut entered = entered.lock().unwrap();
        while !*entered {
            entered = entered_condvar.wait(entered).unwrap();
        }
        drop(entered);

        let updater = Arc::clone(&counter);
        let (updated_tx, updated_rx) = mpsc::channel();
        let updater = std::thread::spawn(move || {
            let previous = updater.inc_by_with_exemplar(TraceLabels { trace_id: "new" }, 1);
            updated_tx.send(previous).unwrap();
        });

        let early_update = updated_rx.recv_timeout(Duration::from_millis(50));
        let update_blocked = matches!(&early_update, Err(RecvTimeoutError::Timeout));

        let (release, release_condvar) = &counter.inner().release_get;
        *release.lock().unwrap() = true;
        release_condvar.notify_one();

        let previous = match early_update {
            Ok(previous) => previous,
            Err(RecvTimeoutError::Timeout) => updated_rx.recv().unwrap(),
            Err(RecvTimeoutError::Disconnected) => panic!("updater disconnected"),
        };
        updater.join().unwrap();
        let families = collector.join().unwrap();

        assert!(update_blocked);
        assert_eq!(previous, 1);
        let encoded = families[0].metric[0].counter.as_ref().unwrap();
        assert_eq!(encoded.value, Some(1.0));
        assert_eq!(trace_id(encoded.exemplar.as_ref().unwrap()), Some("old"));
        assert_eq!(counter.get(), 2);
    }

    #[test]
    fn classic_histogram_retains_latest_exemplar_per_bucket() {
        let histogram = WithExemplar::new(Histogram::new([1.0, 2.0]));
        histogram.observe_with_exemplar(TraceLabels { trace_id: "first" }, 0.5);
        histogram.observe_with_exemplar(
            TraceLabels {
                trace_id: "replacement",
            },
            0.75,
        );
        histogram.observe(0.8);
        histogram.observe_with_exemplar(
            TraceLabels {
                trace_id: "second_bucket",
            },
            1.5,
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
        let histogram = WithExemplar::new(NativeHistogram::new(1.1));
        let clone = histogram.clone();
        histogram.observe_with_exemplar(TraceLabels { trace_id: "first" }, 0.5);
        clone.observe(2.0);
        clone.observe_with_exemplar(TraceLabels { trace_id: "latest" }, 3.0);

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
        let counter = WithExemplar::<Counter, &'static str>::default();
        counter.inc_by_with_exemplar("not a label set", 2);

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
        let counter = WithExemplar::<Counter, ()>::default();
        counter.inc_by_with_exemplar((), 2);

        let families = NamedMetric::new("empty_exemplar", "", counter).encode();
        assert!(encode_to_text(&families).contains("empty_exemplar 2 # {} 2.0\n"));
    }

    #[test]
    fn accepts_legacy_sequence_label_sets() {
        let counter = WithExemplar::<Counter, Vec<(&'static str, &'static str)>>::default();
        counter.inc_by_with_exemplar(vec![("trace_id", "legacy")], 1);

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

        let counter = WithExemplar::<Counter, OpaqueLabels>::default();
        counter.inc_by_with_exemplar(OpaqueLabels, 1);
        assert_eq!(counter.get(), 1);

        let classic = WithExemplar::<Histogram, OpaqueLabels>::new(Histogram::new([1.0]));
        classic.observe_with_exemplar(OpaqueLabels, 0.5);

        let native = WithExemplar::<NativeHistogram, OpaqueLabels>::new(NativeHistogram::new(1.1));
        native.observe_with_exemplar(OpaqueLabels, 0.5);
    }

    #[test]
    fn families_keep_series_and_exemplar_labels_separate() {
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
        struct SeriesLabels {
            method: &'static str,
        }

        let family = Family::<SeriesLabels, WithExemplar<Counter, TraceLabels>>::default();
        family
            .get_or_create(&SeriesLabels { method: "GET" })
            .inc_by_with_exemplar(TraceLabels { trace_id: "abc" }, 1);

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
        let classic: WithExemplar<Histogram, TraceLabels> = HistogramBuilder {
            buckets: &[0.5, 1.0],
        }
        .new_metric();
        classic.observe_with_exemplar(
            TraceLabels {
                trace_id: "classic",
            },
            0.75,
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

        let native: WithExemplar<NativeHistogram, TraceLabels> = NativeHistogramBuilder::new(1.1)
            .with_max_buckets(160)
            .new_metric();
        native.observe_with_exemplar(TraceLabels { trace_id: "native" }, 0.75);
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
        let counter = WithExemplar::<Counter, TraceLabels>::default();
        counter.inc_by_with_exemplar(
            TraceLabels {
                trace_id: "registered",
            },
            4,
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
        assert!(
            text.contains("registered_counter_with_exemplar 4 # {trace_id=\"registered\"} 4.0\n")
        );
    }

    #[test]
    fn registered_native_exemplar_family_round_trips_through_protobuf() {
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
        struct SeriesLabels {
            method: &'static str,
        }

        let family = Family::<
            SeriesLabels,
            WithExemplar<NativeHistogram, TraceLabels>,
            NativeHistogramBuilder,
        >::new_with_constructor(NativeHistogramBuilder::new(1.1));
        family
            .get_or_create(&SeriesLabels { method: "GET" })
            .observe_with_exemplar(TraceLabels { trace_id: "native" }, 0.25);
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
