use foundations_metrics_registry::MetricFamily;

/// Encodes metric storage independently of its registered identity.
///
/// Each returned family must set `name` to a relative suffix. The primary
/// family uses `Some("")`; additional series use suffixes such as `Some("_min")`
/// or `Some("_max")`. [`NamedMetric`](crate::NamedMetric) prepends the registered
/// base name and supplies help text when it is absent.
pub(crate) trait EncodeMetricValue: Send + Sync + 'static {
    /// Encodes the current value into one or more relatively named families.
    fn encode_metric_value(&self) -> Vec<MetricFamily>;
}
