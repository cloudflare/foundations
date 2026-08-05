use std::time::SystemTime;

use foundations_metrics_registry::proto::{self, Bucket, BucketSpan, LabelPair, MetricType};
use prometheus_client::encoding::prometheus_protobuf::{
    self, prometheus_data_model as prometheus_proto,
};
use prometheus_client::metrics::histogram::{
    Histogram as PrometheusHistogram, NativeHistogramConfig,
};
use prometheus_client::registry::Registry;
use serde::Serialize;

use crate::diagnostics::report_collect_error;
use crate::{MetricFamily, value::EncodeMetricValue};

use super::MetricConstructor;
use super::exemplar::{Exemplar, ExemplarStorage, StoredExemplar, WithExemplar, finish_exemplar};
use super::histogram::{HistogramTimer, ObserveNanos, seconds};

/// A native (exponential-bucket) histogram.
///
/// Unlike a classic [`Histogram`](super::Histogram), whose buckets are a fixed
/// list of upper bounds, a native histogram places observations into
/// exponentially sized buckets whose resolution is chosen by a growth factor.
/// Native-only histograms require the Prometheus protobuf exposition format.
/// Use [`NativeHistogram::new_classic_and_native`] or
/// [`NativeHistogramBuilder::with_classic_buckets`] to retain classic buckets
/// for text exposition.
///
/// Clones share the same storage.
///
/// # Examples
///
/// ```
/// use foundations_metrics::NativeHistogram;
///
/// let request_latency = NativeHistogram::new(1.1);
/// request_latency.observe(0.25);
/// request_latency.observe(4.2);
/// ```
#[derive(Clone, Debug)]
pub struct NativeHistogram {
    inner: PrometheusHistogram,
}

impl NativeHistogram {
    /// Creates a native histogram with the given bucket growth `factor`.
    ///
    /// The factor bounds the ratio between adjacent bucket boundaries; a
    /// smaller factor gives finer resolution. The zero bucket uses the
    /// Prometheus-recommended default threshold and the number of buckets is
    /// unbounded. Use [`NativeHistogramBuilder`] for full control.
    ///
    /// # Panics
    ///
    /// Panics if `factor` is not greater than `1.0`.
    #[track_caller]
    pub fn new(factor: f64) -> Self {
        NativeHistogramBuilder::new(factor).new_metric()
    }

    /// Creates a histogram with both classic and native buckets.
    ///
    /// Classic bucket bounds are sorted ascending.
    ///
    /// Protobuf exposition includes both representations. Text exposition uses
    /// the classic buckets as a fallback.
    ///
    /// # Panics
    ///
    /// Panics if `factor` is not greater than `1.0`.
    #[track_caller]
    pub fn new_classic_and_native(buckets: impl IntoIterator<Item = f64>, factor: f64) -> Self {
        Self {
            inner: PrometheusHistogram::new_classic_and_native(
                sorted_buckets(buckets),
                NativeHistogramBuilder::new(factor).config(),
            ),
        }
    }

    /// Records an observed value.
    #[inline]
    pub fn observe(&self, value: f64) {
        self.inner.observe(value);
    }

    pub(super) fn try_encode_metric_value(&self) -> Result<Vec<MetricFamily>, std::fmt::Error> {
        // prometheus_client keeps native bucket state private. A cloned histogram shares
        // storage, so a temporary registry can drive its protobuf encoder.
        let mut registry = Registry::default();
        registry.register("native_histogram", "", self.inner.clone());

        prometheus_protobuf::encode(&registry)
            .map(|families| families.into_iter().map(convert_native_family).collect())
    }
}

impl<S> WithExemplar<NativeHistogram, S> {
    /// Records `value` and retains it as the latest exemplar.
    pub fn observe_with_exemplar(&self, label_set: S, value: f64) {
        let mut exemplar = Exemplar::new(label_set, value, None);
        let mut exemplars = self.exemplars.lock();
        self.inner.observe(value);
        exemplar.timestamp = Some(SystemTime::now().into());
        let exemplar = StoredExemplar::F64(exemplar);

        match &mut *exemplars {
            ExemplarStorage::Empty => {
                *exemplars = ExemplarStorage::Single(Some(exemplar));
            }
            ExemplarStorage::Single(stored) => *stored = Some(exemplar),
            ExemplarStorage::PerBucket(_) => {
                unreachable!("native histogram uses single exemplar storage")
            }
        }
    }
}

/// A native (exponential-bucket) histogram for tracking time.
///
/// This is the native-bucket counterpart to [`TimeHistogram`](super::TimeHistogram):
/// it records durations in nanoseconds and exposes [`start_timer`] for
/// measuring a scope. Unlike `TimeHistogram`, whose resolution is fixed by its
/// bucket list, resolution here follows the growth factor and the observed
/// range, so a latency distribution does not have to be bounded in advance.
///
/// Durations are converted to seconds before bucketing, matching the
/// Prometheus convention and [`TimeHistogram`](super::TimeHistogram).
///
/// Clones share the same storage.
///
/// [`start_timer`]: NativeTimeHistogram::start_timer
///
/// # Examples
///
/// ```
/// use foundations_metrics::NativeTimeHistogram;
///
/// let request_latency = NativeTimeHistogram::new(1.1);
/// request_latency.observe(1_500_000);
///
/// let timer = request_latency.start_timer();
/// // ... work being measured ...
/// drop(timer);
/// ```
#[derive(Clone, Debug)]
pub struct NativeTimeHistogram {
    inner: NativeHistogram,
}

impl NativeTimeHistogram {
    /// Creates a native time histogram with the given bucket growth `factor`.
    ///
    /// The factor bounds the ratio between adjacent bucket boundaries; a
    /// smaller factor gives finer resolution. Use [`NativeHistogramBuilder`]
    /// for full control.
    ///
    /// # Panics
    ///
    /// Panics if `factor` is not greater than `1.0`.
    #[track_caller]
    pub fn new(factor: f64) -> Self {
        NativeHistogramBuilder::new(factor).new_metric()
    }

    /// Starts a timer that records its duration when stopped or dropped.
    pub fn start_timer(&self) -> HistogramTimer<Self> {
        HistogramTimer::start(self.clone())
    }

    /// Records an observed duration in nanoseconds.
    #[inline]
    pub fn observe(&self, nanos: u64) {
        self.inner.observe(seconds(nanos));
    }
}

impl ObserveNanos for NativeTimeHistogram {
    #[inline]
    fn observe_nanos(&self, nanos: u64) {
        self.observe(nanos);
    }
}

impl EncodeMetricValue for NativeTimeHistogram {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        self.inner.encode_metric_value()
    }
}

impl MetricConstructor<NativeTimeHistogram> for NativeHistogramBuilder {
    fn new_metric(&self) -> NativeTimeHistogram {
        NativeTimeHistogram {
            inner: self.new_metric(),
        }
    }
}

/// Constructs [`NativeHistogram`]s with a fixed configuration.
///
/// Use this with [`Family`](crate::Family) or a metric's `#[ctor = ...]` when a
/// native histogram needs bucket configuration at creation time.
///
/// # Examples
///
/// ```
/// use foundations_metrics::{Family, NativeHistogram, NativeHistogramBuilder};
/// use serde::Serialize;
///
/// #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
/// struct Labels {
///     method: &'static str,
/// }
///
/// let builder = NativeHistogramBuilder::new(1.1)
///     .with_classic_buckets(&[0.1, 0.5, 1.0])
///     .with_max_buckets(160);
/// let latencies = Family::<Labels, NativeHistogram, _>::new_with_constructor(builder);
/// latencies.get_or_create(&Labels { method: "GET" }).observe(0.5);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct NativeHistogramBuilder {
    /// Optional upper bounds for classic histogram buckets.
    ///
    /// When present, each observation updates both the classic and native
    /// representations so that text exposition can use the classic buckets.
    pub classic_buckets: Option<&'static [f64]>,

    /// Bucket growth factor; must be greater than `1.0`. Smaller factors give
    /// finer resolution.
    pub bucket_factor: f64,

    /// Width of the zero bucket, which absorbs observations close to zero.
    ///
    /// `0.0` keeps the Prometheus-recommended default threshold; a negative
    /// value configures a zero-width zero bucket.
    pub zero_threshold: f64,

    /// Best-effort upper bound on the number of populated buckets across both
    /// the positive and negative ranges.
    ///
    /// `0` leaves the bucket count unbounded.
    pub max_buckets: usize,
}

impl NativeHistogramBuilder {
    /// Creates a builder with the given bucket growth `factor`, the default zero
    /// threshold, and an unbounded number of buckets.
    pub fn new(factor: f64) -> Self {
        Self {
            classic_buckets: None,
            bucket_factor: factor,
            zero_threshold: 0.0,
            max_buckets: 0,
        }
    }

    /// Adds classic buckets as a fallback for text exposition.
    ///
    /// Bucket bounds are sorted ascending when the histogram is constructed.
    ///
    /// Each observation updates both the classic and native representations,
    /// and protobuf exposition includes both.
    pub fn with_classic_buckets(mut self, buckets: &'static [f64]) -> Self {
        self.classic_buckets = Some(buckets);
        self
    }

    /// Sets the zero-bucket threshold.
    ///
    /// `0.0` keeps the default threshold; a negative value configures a
    /// zero-width zero bucket.
    pub fn with_zero_threshold(mut self, zero_threshold: f64) -> Self {
        self.zero_threshold = zero_threshold;
        self
    }

    /// Sets a best-effort maximum number of populated buckets.
    ///
    /// `0` leaves the count unbounded.
    pub fn with_max_buckets(mut self, max_buckets: usize) -> Self {
        self.max_buckets = max_buckets;
        self
    }

    /// Translates this builder into the wrapped crate's configuration.
    ///
    /// # Panics
    ///
    /// Panics if `bucket_factor` is not greater than `1.0` or if
    /// `zero_threshold` is not finite.
    #[track_caller]
    fn config(&self) -> NativeHistogramConfig {
        // Validated here rather than left to `prometheus-client`: its assertions
        // are not `#[track_caller]`, so they report a location inside that crate
        // instead of the code that supplied the invalid configuration.
        assert!(
            self.bucket_factor > 1.0,
            "native histogram bucket factor must be greater than 1.0, but was {}",
            self.bucket_factor
        );
        assert!(
            self.zero_threshold.is_finite(),
            "native histogram zero threshold must be finite, but was {}",
            self.zero_threshold
        );

        NativeHistogramConfig::new(self.bucket_factor)
            .zero_threshold(self.zero_threshold)
            .max_buckets(self.max_buckets)
    }
}

impl MetricConstructor<NativeHistogram> for NativeHistogramBuilder {
    #[track_caller]
    fn new_metric(&self) -> NativeHistogram {
        let config = self.config();
        NativeHistogram {
            inner: match self.classic_buckets {
                Some(buckets) => PrometheusHistogram::new_classic_and_native(
                    sorted_buckets(buckets.iter().copied()),
                    config,
                ),
                None => PrometheusHistogram::new_native(config),
            },
        }
    }
}

impl<S> MetricConstructor<WithExemplar<NativeHistogram, S>> for NativeHistogramBuilder {
    fn new_metric(&self) -> WithExemplar<NativeHistogram, S> {
        WithExemplar::new(NativeHistogramBuilder::new_metric(self))
    }
}

fn sorted_buckets(buckets: impl IntoIterator<Item = f64>) -> Vec<f64> {
    let mut buckets: Vec<_> = buckets.into_iter().collect();
    buckets.sort_by(f64::total_cmp);
    buckets
}

impl EncodeMetricValue for NativeHistogram {
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        match self.try_encode_metric_value() {
            Ok(families) => families,
            Err(error) => {
                report_collect_error(format_args!(
                    "non-fatal error while collecting metrics: skipped a native histogram; protobuf encoding failed: {error}"
                ));
                Vec::new()
            }
        }
    }
}

impl<S> EncodeMetricValue for WithExemplar<NativeHistogram, S>
where
    S: Serialize + Send + Sync + 'static,
{
    fn encode_metric_value(&self) -> Vec<MetricFamily> {
        let exemplars = self.exemplars.lock();
        let exemplar = match &*exemplars {
            ExemplarStorage::Empty => None,
            ExemplarStorage::Single(exemplar) => exemplar.clone(),
            ExemplarStorage::PerBucket(_) => {
                unreachable!("native histogram uses single exemplar storage")
            }
        };
        let families = self.inner.try_encode_metric_value();
        drop(exemplars);

        let mut families = match families {
            Ok(families) => families,
            Err(error) => {
                report_collect_error(format_args!(
                    "non-fatal error while collecting metrics: skipped a native histogram; protobuf encoding failed: {error}"
                ));
                return Vec::new();
            }
        };

        if let Some(exemplar) = finish_exemplar(exemplar.as_ref().map(StoredExemplar::encode)) {
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

fn convert_native_family(family: prometheus_proto::MetricFamily) -> MetricFamily {
    MetricFamily {
        name: Some(String::new()),
        help: None,
        r#type: Some(MetricType::Histogram as i32),
        metric: family
            .metric
            .into_iter()
            .map(convert_native_metric)
            .collect(),
        unit: (!family.unit.is_empty()).then_some(family.unit),
    }
}

fn convert_native_metric(metric: prometheus_proto::Metric) -> proto::Metric {
    proto::Metric {
        label: metric
            .label
            .into_iter()
            .map(|label| LabelPair {
                name: Some(label.name),
                value: Some(label.value),
            })
            .collect(),
        histogram: metric.histogram.map(convert_native_histogram),
        timestamp_ms: (metric.timestamp_ms != 0).then_some(metric.timestamp_ms),
        ..Default::default()
    }
}

fn convert_native_histogram(histogram: prometheus_proto::Histogram) -> proto::Histogram {
    proto::Histogram {
        sample_count: Some(histogram.sample_count),
        sample_count_float: (histogram.sample_count_float > 0.0)
            .then_some(histogram.sample_count_float),
        sample_sum: Some(histogram.sample_sum),
        bucket: histogram
            .bucket
            .into_iter()
            .map(|bucket| Bucket {
                cumulative_count: Some(bucket.cumulative_count),
                cumulative_count_float: (bucket.cumulative_count_float > 0.0)
                    .then_some(bucket.cumulative_count_float),
                upper_bound: Some(bucket.upper_bound),
                ..Default::default()
            })
            .collect(),
        created_timestamp: histogram.start_timestamp,
        schema: Some(histogram.schema),
        zero_threshold: Some(histogram.zero_threshold),
        zero_count: Some(histogram.zero_count),
        zero_count_float: (histogram.zero_count_float > 0.0).then_some(histogram.zero_count_float),
        negative_span: histogram
            .negative_span
            .into_iter()
            .map(convert_native_span)
            .collect(),
        negative_delta: histogram.negative_delta,
        negative_count: histogram.negative_count,
        positive_span: histogram
            .positive_span
            .into_iter()
            .map(convert_native_span)
            .collect(),
        positive_delta: histogram.positive_delta,
        positive_count: histogram.positive_count,
        ..Default::default()
    }
}

fn convert_native_span(span: prometheus_proto::BucketSpan) -> BucketSpan {
    BucketSpan {
        offset: Some(span.offset),
        length: Some(span.length),
    }
}

#[cfg(test)]
mod tests {
    use foundations_metrics_registry::proto::{self, MetricType};
    use prost::Message;
    use serde::Serialize;

    use super::*;
    use crate::{EncodeMetric, Family, NamedMetric, encode_to_protobuf, encode_to_text};

    fn encoded_histogram(families: &[MetricFamily]) -> &proto::Histogram {
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].r#type, Some(MetricType::Histogram as i32));
        assert_eq!(families[0].metric.len(), 1);
        families[0].metric[0]
            .histogram
            .as_ref()
            .expect("encoded native histogram is present")
    }

    #[test]
    fn clones_share_storage() {
        let histogram = NativeHistogram::new(1.1);
        let clone = histogram.clone();

        histogram.observe(1.0);
        clone.observe(3.5);

        let families = histogram.encode_metric_value();
        let encoded = encoded_histogram(&families);
        assert_eq!(encoded.sample_count, Some(2));
        assert_eq!(encoded.sample_sum, Some(4.5));
    }

    #[test]
    fn encodes_relative_name_and_native_fields() {
        let histogram = NativeHistogram::new(2.0);
        histogram.observe(1.0);
        histogram.observe(2.0);
        histogram.observe(4.0);

        let families = histogram.encode_metric_value();
        assert_eq!(families[0].name.as_deref(), Some(""));
        assert_eq!(families[0].help, None);

        let encoded = encoded_histogram(&families);
        assert_eq!(encoded.sample_count, Some(3));
        assert_eq!(encoded.sample_sum, Some(7.0));
        assert!(encoded.schema.is_some());
        assert!(encoded.zero_threshold.is_some());
        assert_eq!(encoded.zero_count, Some(0));
        assert!(!encoded.positive_span.is_empty());
        assert!(!encoded.positive_delta.is_empty());
        assert!(encoded.negative_span.is_empty());
        assert!(encoded.bucket.is_empty());
    }

    #[test]
    fn empty_histogram_encodes_a_valid_family() {
        let families = NativeHistogram::new(1.1).encode_metric_value();
        let encoded = encoded_histogram(&families);

        assert_eq!(encoded.sample_count, Some(0));
        assert_eq!(encoded.sample_sum, Some(0.0));
        assert!(encoded.schema.is_some());
    }

    #[test]
    fn builder_applies_configuration() {
        let histogram: NativeHistogram = NativeHistogramBuilder::new(1.5)
            .with_zero_threshold(0.001)
            .with_max_buckets(160)
            .new_metric();
        histogram.observe(0.5);

        let families = histogram.encode_metric_value();
        let encoded = encoded_histogram(&families);
        assert_eq!(encoded.sample_count, Some(1));
        assert_eq!(encoded.zero_threshold, Some(0.001));
    }

    #[test]
    fn classic_and_native_histogram_uses_each_exposition_format() {
        let histogram: NativeHistogram = NativeHistogramBuilder::new(2.0)
            .with_classic_buckets(&[1.0, 2.0])
            .new_metric();
        histogram.observe(0.5);
        histogram.observe(1.5);
        histogram.observe(3.0);

        let families =
            NamedMetric::new("request_latency_seconds", "Request latency.", histogram).encode();
        let encoded = encoded_histogram(&families);

        assert_eq!(encoded.sample_count, Some(3));
        assert_eq!(encoded.bucket.len(), 3);
        assert_eq!(encoded.bucket[0].cumulative_count, Some(1));
        assert_eq!(encoded.bucket[1].cumulative_count, Some(2));
        assert_eq!(encoded.bucket[2].cumulative_count, Some(3));
        assert!(encoded.schema.is_some());
        assert!(!encoded.positive_span.is_empty());

        let text = encode_to_text(&families);
        assert!(text.contains("request_latency_seconds_bucket{le=\"1.0\"} 1\n"));
        assert!(text.contains("request_latency_seconds_bucket{le=\"2.0\"} 2\n"));
        assert!(text.contains("request_latency_seconds_bucket{le=\"+Inf\"} 3\n"));

        let payload = encode_to_protobuf(&families);
        let mut payload = payload.as_slice();
        let decoded = MetricFamily::decode_length_delimited(&mut payload)
            .expect("combined histogram protobuf should decode");
        assert!(payload.is_empty());

        let decoded = decoded.metric[0]
            .histogram
            .as_ref()
            .expect("decoded histogram is present");
        assert_eq!(decoded.bucket.len(), 3);
        assert!(decoded.schema.is_some());
        assert!(!decoded.positive_span.is_empty());
    }

    #[test]
    fn classic_and_native_constructor_uses_the_default_native_configuration() {
        let histogram = NativeHistogram::new_classic_and_native([2.0, 1.0], 2.0);
        histogram.observe(1.5);

        let families = histogram.encode_metric_value();
        let encoded = encoded_histogram(&families);
        assert_eq!(
            encoded
                .bucket
                .iter()
                .map(|bucket| (bucket.upper_bound, bucket.cumulative_count))
                .collect::<Vec<_>>(),
            vec![
                (Some(1.0), Some(0)),
                (Some(2.0), Some(1)),
                (Some(f64::MAX), Some(1)),
            ]
        );
        assert!(encoded.schema.is_some());
    }

    #[test]
    fn named_metric_rewrites_name_and_fills_help() {
        let histogram = NativeHistogram::new(1.1);
        histogram.observe(1.0);

        let named = NamedMetric::new("request_latency_seconds", "Latency of requests.", histogram);

        let families = named.encode();
        assert_eq!(families.len(), 1);
        assert_eq!(families[0].name.as_deref(), Some("request_latency_seconds"));
        assert_eq!(families[0].help.as_deref(), Some("Latency of requests."));
        assert_eq!(families[0].r#type, Some(MetricType::Histogram as i32));
    }

    #[test]
    fn family_adds_labels_to_histogram_rows() {
        #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
        struct Labels {
            method: &'static str,
        }

        let family =
            Family::<Labels, NativeHistogram, NativeHistogramBuilder>::new_with_constructor(
                NativeHistogramBuilder::new(1.1),
            );
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

    // Each `#[track_caller]` between the caller and the assertion is
    // load-bearing: dropping any one of them collapses the reported location
    // onto that frame instead of the code that supplied the configuration.
    #[test]
    fn invalid_bucket_factor_panics_at_the_callers_location() {
        // Relies on each test running in its own process, as `cargo nextest`
        // does, since the panic hook is process-wide.
        let previous = std::panic::take_hook();
        let location = std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured = std::sync::Arc::clone(&location);

        std::panic::set_hook(Box::new(move |info| {
            *captured.lock().unwrap() = info
                .location()
                .map(|location| (location.file().to_owned(), location.line()));
        }));

        let expected_line = line!() + 1;
        let result = std::panic::catch_unwind(|| NativeHistogram::new(1.0));

        std::panic::set_hook(previous);

        assert!(result.is_err(), "a bucket factor of 1.0 must be rejected");

        let (file, line) = location
            .lock()
            .unwrap()
            .take()
            .expect("the panic location was captured");

        assert!(
            file.ends_with("native_histogram.rs"),
            "reported file: {file}"
        );
        assert_eq!(line, expected_line);
    }

    #[test]
    fn time_histogram_records_nanos_as_seconds() {
        let histogram = NativeTimeHistogram::new(1.1);
        histogram.observe(1_500_000);

        let families = histogram.encode_metric_value();
        let encoded = encoded_histogram(&families);
        assert_eq!(encoded.sample_count, Some(1));
        assert_eq!(encoded.sample_sum, Some(0.0015));
    }

    // The whole point of the wrapper is that it changes the input unit and
    // nothing else, so the encoded output must be indistinguishable from
    // driving a `NativeHistogram` with the equivalent value in seconds.
    #[test]
    fn time_histogram_buckets_identically_to_native_histogram() {
        let nanos = [1_000u64, 250_000, 1_500_000, 40_000_000, 2_500_000_000];

        let timed = NativeTimeHistogram::new(1.1);
        let plain = NativeHistogram::new(1.1);
        for value in nanos {
            timed.observe(value);
            plain.observe(value as f64 * 1e-9);
        }

        let timed = timed.encode_metric_value();
        let plain = plain.encode_metric_value();
        let timed = encoded_histogram(&timed);
        let plain = encoded_histogram(&plain);

        assert_eq!(timed.sample_count, plain.sample_count);
        assert_eq!(timed.sample_sum, plain.sample_sum);
        assert_eq!(timed.schema, plain.schema);
        assert_eq!(timed.zero_threshold, plain.zero_threshold);
        assert_eq!(timed.zero_count, plain.zero_count);
        assert_eq!(timed.positive_span, plain.positive_span);
        assert_eq!(timed.positive_delta, plain.positive_delta);
        assert_eq!(timed.negative_span, plain.negative_span);
        assert_eq!(timed.negative_delta, plain.negative_delta);
    }

    #[test]
    fn time_histogram_clones_share_storage() {
        let histogram = NativeTimeHistogram::new(1.1);
        let clone = histogram.clone();

        histogram.observe(1_000_000);
        clone.observe(3_000_000);

        let families = histogram.encode_metric_value();
        let encoded = encoded_histogram(&families);
        assert_eq!(encoded.sample_count, Some(2));
        assert_eq!(encoded.sample_sum, Some(0.004));
    }

    #[test]
    fn time_histogram_timer_records_on_drop() {
        let histogram = NativeTimeHistogram::new(1.1);
        drop(histogram.start_timer());

        let families = histogram.encode_metric_value();
        assert_eq!(encoded_histogram(&families).sample_count, Some(1));
    }

    #[test]
    fn time_histogram_timer_can_be_discarded() {
        let histogram = NativeTimeHistogram::new(1.1);
        histogram.start_timer().stop_and_discard();

        let families = histogram.encode_metric_value();
        assert_eq!(encoded_histogram(&families).sample_count, Some(0));
    }

    #[test]
    fn time_histogram_timer_records_once_when_stopped_then_dropped() {
        let histogram = NativeTimeHistogram::new(1.1);
        let timer = histogram.start_timer();
        let _ = timer.stop_and_record();

        let families = histogram.encode_metric_value();
        assert_eq!(encoded_histogram(&families).sample_count, Some(1));
    }

    #[test]
    fn time_histogram_timer_excludes_paused_time() {
        let histogram = NativeTimeHistogram::new(1.1);

        let mut timer = histogram.start_timer();
        timer.pause();
        std::thread::sleep(std::time::Duration::from_millis(50));
        timer.resume();
        let elapsed = timer.stop_and_record();

        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "paused time must not be counted, measured {elapsed:?}"
        );
    }

    #[test]
    fn time_histogram_is_constructible_from_the_builder() {
        let histogram: NativeTimeHistogram = NativeHistogramBuilder::new(2.0)
            .with_zero_threshold(0.001)
            .new_metric();
        histogram.observe(2_000_000_000);

        let families = histogram.encode_metric_value();
        let encoded = encoded_histogram(&families);
        assert_eq!(encoded.sample_count, Some(1));
        assert_eq!(encoded.zero_threshold, Some(0.001));
    }
}
