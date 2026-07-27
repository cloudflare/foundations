use crate::MetricFamily;

/// Encodes metric values before registration metadata is applied.
///
/// Each returned [`MetricFamily`] must set `name` to a relative suffix. The
/// primary series uses `Some("")`; additional series use names such as
/// `Some("_min")` and `Some("_max")`. [`NamedMetric`](crate::NamedMetric)
/// prepends the registered metric name and fills in missing help text.
/// Implement this to define a custom metric. Pair it with
/// [`NamedMetric`](crate::NamedMetric) to attach the registered name and help
/// text, or store it in a [`Family`](crate::Family) to differentiate it by label
/// set, then hand the result to [`register`](crate::register).
///
/// # Compared with [`EncodeMetric`](crate::EncodeMetric)
///
/// Both encode into the same [`MetricFamily`] model; they differ in how much of
/// the output the implementor owns.
///
/// |  | [`EncodeMetric`](crate::EncodeMetric) | `EncodeMetricValue` |
/// |---|---|---|
/// | Family name | complete, written by the implementor | relative, prepended by [`NamedMetric`](crate::NamedMetric) |
/// | Help text | written by the implementor | filled in by [`NamedMetric`](crate::NamedMetric) when absent |
/// | Label sets | the implementor's own storage and serialization | provided by [`Family`](crate::Family) |
/// | Registered directly | yes | only once wrapped |
///
/// Implementing [`EncodeMetric`](crate::EncodeMetric) is the right choice when a
/// metric must control its own naming or emit several families; otherwise this
/// trait leaves only the value encoding to write.
///
/// ```
/// use std::sync::atomic::{AtomicU64, Ordering};
///
/// use foundations_metrics::proto::{Gauge, Metric, MetricType};
/// use foundations_metrics::{
///     EncodeMetric, EncodeMetricValue, Family, MetricFamily, NamedMetric, RegistrationMetadata,
///     register,
/// };
/// use serde::Serialize;
///
/// #[derive(Default)]
/// struct OpenFiles(AtomicU64);
///
/// impl EncodeMetricValue for OpenFiles {
///     fn encode_metric_value(&self) -> Vec<MetricFamily> {
///         vec![MetricFamily {
///             // The primary series carries an empty relative name.
///             name: Some(String::new()),
///             help: None,
///             r#type: Some(MetricType::Gauge as i32),
///             metric: vec![Metric {
///                 gauge: Some(Gauge {
///                     value: Some(self.0.load(Ordering::Relaxed) as f64),
///                 }),
///                 ..Default::default()
///             }],
///             unit: None,
///         }]
///     }
/// }
///
/// #[derive(Clone, Eq, Hash, PartialEq, Serialize)]
/// struct Labels {
///     mount: &'static str,
/// }
///
/// let open_files = Family::<Labels, OpenFiles>::default();
///
/// open_files
///     .get_or_create(&Labels { mount: "/data" })
///     .0
///     .fetch_add(1, Ordering::Relaxed);
///
/// register(
///     Box::new(NamedMetric::new(
///         "open_files",
///         "Number of open file descriptors.",
///         open_files,
///     )) as Box<dyn EncodeMetric>,
///     RegistrationMetadata::default(),
/// );
/// ```
pub trait EncodeMetricValue: Send + Sync + 'static {
    /// Encodes the current value into one or more relatively named families.
    fn encode_metric_value(&self) -> Vec<MetricFamily>;
}
