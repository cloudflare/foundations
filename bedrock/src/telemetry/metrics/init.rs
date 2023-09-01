use super::internal::{BuildInfo, Registries, RuntimeInfo};
use super::report_info;
use crate::telemetry::settings::MetricsSettings;
use crate::ServiceInfo;

/// Initializes the metric system with a system-wide metric prefix.
///
/// Must be called before any use of metrics defined
/// by the `metrics` proc macro attribute.
pub(crate) fn init(service_info: &ServiceInfo, settings: &MetricsSettings) {
    Registries::init(service_info, settings);

    report_info(BuildInfo {
        version: service_info.version,
    });

    report_info(RuntimeInfo {
        pid: std::process::id(),
    });
}
