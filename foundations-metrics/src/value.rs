use foundations_metrics_registry::MetricFamily;

/// Internal trait for storage types
pub(crate) trait EncodeMetricValue: Send + Sync + 'static {
    fn encode_metric_value(&self) -> Vec<MetricFamily>;
}
