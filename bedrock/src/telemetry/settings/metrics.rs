#[cfg(feature = "settings")]
use crate::settings::settings;

/// Metrics settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
pub struct MetricsSettings {
    /// How the metrics service identifier defined in `ServiceInfo` is used
    /// for this service.
    pub service_identifier_format: ServiceIdentifierFormat,

    /// Whether to report optional metrics in the telemetry server.
    pub report_optional: bool,
}

/// Service identifier format.
///
/// This dictates how the [metrics server identifier](`crate::ServiceInfo::metrics_service_identifier`)
/// should be used by the metrics system.
///
/// If `MetricPrefix`, the identifier is used as a prefix. If `LabelWithName(name)`, the identifier is
/// used as the value of an additional label named `name`.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, Default))]
pub enum ServiceIdentifierFormat {
    /// Use the metrics service identifier as metric prefix.
    #[default]
    MetricPrefix,
    /// Use the metrics service identifier as the value for the given label name.
    LabelWithName(String),
}
