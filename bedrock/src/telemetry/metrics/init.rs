use super::internal::{BuildInfo, RuntimeInfo, METRIC_PREFIX, OPT_REGISTRY, REGISTRY};
use super::report_info;
use crate::ServiceInfo;
use once_cell::sync::Lazy;

/// Initializes the metric system with a system-wide metric prefix.
///
/// Must be called before any use of metrics defined
/// by the `metrics` proc macro attribute.
pub(crate) fn init(service_info: ServiceInfo) {
    report_info(BuildInfo {
        version: service_info.version,
    });

    report_info(RuntimeInfo {
        pid: std::process::id(),
    });

    *METRIC_PREFIX.write() = service_info.name;

    Lazy::force(&REGISTRY);
    Lazy::force(&OPT_REGISTRY);
}
