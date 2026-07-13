use foundations_metrics_registry::MetricFamily;

/// Encodes metric values before registration metadata is applied.
///
/// Each returned [`MetricFamily`] must set `name` to a relative suffix. The
/// primary series uses `Some("")`; additional series use names such as
/// `Some("_min")` and `Some("_max")`. [`NamedMetric`](crate::NamedMetric)
/// prepends the registered metric name and fills in missing help text.
pub(crate) trait EncodeMetricValue: Send + Sync + 'static {
    /// Encodes the current value into one or more relatively named families.
    fn encode_metric_value(&self) -> Vec<MetricFamily>;
}
