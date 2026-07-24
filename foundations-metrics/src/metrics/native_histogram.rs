use foundations_metrics_registry::proto::{self, Bucket, BucketSpan, LabelPair, MetricType};
use prometheus_client::encoding::prometheus_protobuf::{
    self, prometheus_data_model as prometheus_proto,
};
use prometheus_client::metrics::histogram::{
    Histogram as PrometheusHistogram, NativeHistogramConfig,
};
use prometheus_client::registry::Registry;

use crate::diagnostics::report_collect_error;
use crate::{MetricFamily, value::EncodeMetricValue};

use super::MetricConstructor;

/// A native (exponential-bucket) histogram.
///
/// Unlike a classic [`Histogram`](super::Histogram), whose buckets are a fixed
/// list of upper bounds, a native histogram places observations into
/// exponentially sized buckets whose resolution is chosen by a growth factor.
/// Native histograms require the Prometheus protobuf exposition format.
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

    /// Records an observed value.
    #[inline]
    pub fn observe(&self, value: f64) {
        self.inner.observe(value);
    }

    pub(super) fn try_encode_metric_value(&self) -> Result<Vec<MetricFamily>, std::fmt::Error> {
        // Upstream keeps native bucket state private. A cloned histogram shares
        // storage, so a temporary registry can drive its protobuf encoder.
        let mut registry = Registry::default();
        registry.register("native_histogram", "", self.inner.clone());

        prometheus_protobuf::encode(&registry)
            .map(|families| families.into_iter().map(convert_native_family).collect())
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
/// let builder = NativeHistogramBuilder::new(1.1).with_max_buckets(160);
/// let latencies = Family::<Labels, NativeHistogram, _>::new_with_constructor(builder);
/// latencies.get_or_create(&Labels { method: "GET" }).observe(0.5);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct NativeHistogramBuilder {
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
            bucket_factor: factor,
            zero_threshold: 0.0,
            max_buckets: 0,
        }
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
    fn config(&self) -> NativeHistogramConfig {
        NativeHistogramConfig::new(self.bucket_factor)
            .zero_threshold(self.zero_threshold)
            .max_buckets(self.max_buckets)
    }
}

impl MetricConstructor<NativeHistogram> for NativeHistogramBuilder {
    #[track_caller]
    fn new_metric(&self) -> NativeHistogram {
        NativeHistogram {
            inner: PrometheusHistogram::new_native(self.config()),
        }
    }
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
    use serde::Serialize;

    use super::*;
    use crate::{EncodeMetric, Family, NamedMetric};

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
}
