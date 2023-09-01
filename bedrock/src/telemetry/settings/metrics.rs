#[cfg(feature = "settings")]
use crate::settings::settings;

/// Metrics settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Default, Debug))]
pub struct MetricsSettings {
    /// Whether to report optional metrics in the telemetry server.
    pub report_optional: bool,
}
