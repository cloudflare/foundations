use crate::proto::MetricFamily;

/// A metric that can encode itself into the protobuf data model.
///
/// Encoding is best-effort: implementations skip (and internally report) any
/// metric or series that fails, so an empty `Vec` is a valid result.
pub trait EncodeMetric: Send + Sync + 'static {
    /// Encodes this metric into zero or more [`MetricFamily`] messages.
    ///
    /// Every returned [`MetricFamily`] must set `name` to a complete, non-empty
    /// producer-level name.
    fn encode(&self) -> Vec<MetricFamily>;
}
