use crate::proto::MetricFamily;

/// A metric that can encode itself into the protobuf data model.
///
/// Encoding is best-effort: implementations skip (and internally report) any
/// metric or series that fails, so an empty `Vec` is a valid result.
///
/// This is the trait the registry stores, so an implementor controls its entire
/// output: it names its own families and owns any label set it exposes.
///
/// `foundations-metrics` also provides an `EncodeMetricValue` trait, whose
/// implementors encode only a *value* under a relative name. Pairing that with
/// `NamedMetric` supplies the name, and storing it in a `Family` supplies label
/// set storage and serialization. Prefer `EncodeMetricValue` unless a metric
/// needs to control naming or emit several families itself.
///
/// # Examples
///
/// ```
/// use foundations_metrics_registry::proto::{Gauge, LabelPair, Metric, MetricType};
/// use foundations_metrics_registry::{EncodeMetric, MetricFamily, RegistrationMetadata, register};
///
/// struct BuildRevision(&'static str);
///
/// impl EncodeMetric for BuildRevision {
///     fn encode(&self) -> Vec<MetricFamily> {
///         vec![MetricFamily {
///             // Complete producer-level name: nothing is prepended to it.
///             name: Some("build_revision".to_owned()),
///             help: Some("Revision the binary was built from.".to_owned()),
///             r#type: Some(MetricType::Gauge as i32),
///             metric: vec![Metric {
///                 // Labels are this implementation's own responsibility.
///                 label: vec![LabelPair {
///                     name: Some("revision".to_owned()),
///                     value: Some(self.0.to_owned()),
///                 }],
///                 gauge: Some(Gauge { value: Some(1.0) }),
///                 ..Default::default()
///             }],
///             unit: None,
///         }]
///     }
/// }
///
/// register(
///     Box::new(BuildRevision("9f2c1ab")) as Box<dyn EncodeMetric>,
///     RegistrationMetadata::default(),
/// );
/// ```
pub trait EncodeMetric: Send + Sync + 'static {
    /// Encodes this metric into zero or more [`MetricFamily`] messages.
    ///
    /// Every returned [`MetricFamily`] must set `name` to a complete, non-empty
    /// producer-level name.
    fn encode(&self) -> Vec<MetricFamily>;
}
