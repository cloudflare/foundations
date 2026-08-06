#[cfg(feature = "foundations-metrics-backend")]
use std::sync::OnceLock;

use crate::ServiceInfo;
use crate::telemetry::settings::MetricsSettings;

#[cfg(not(feature = "foundations-metrics-backend"))]
use super::internal::Registries;
use super::{info_metric, report_info};

/// Build and version information
#[info_metric(crate_path = "crate")]
struct BuildInfo {
    version: &'static str,
}

/// Information about the process runtime
#[info_metric(crate_path = "crate")]
struct RuntimeInfo {
    pid: u32,
}

/// Service name reported before [`init`] has been called.
#[cfg(feature = "foundations-metrics-backend")]
static UNINITIALISED_SERVICE_NAME: &str = "undefined";

#[cfg(feature = "foundations-metrics-backend")]
static SERVICE_NAME: OnceLock<String> = OnceLock::new();

/// Returns the service name to apply when collecting metrics.
///
/// Reads without initialising, so collecting before [`init`] falls back to
/// [`UNINITIALISED_SERVICE_NAME`] without preventing a later [`init`] from
/// taking effect.
#[cfg(feature = "foundations-metrics-backend")]
pub(super) fn service_name() -> &'static str {
    SERVICE_NAME
        .get()
        .map(String::as_str)
        .unwrap_or(UNINITIALISED_SERVICE_NAME)
}

/// Initializes the metric system with a system-wide metric prefix.
///
/// Must be called before any use of metrics defined
/// by the `metrics` proc macro attribute.
pub(crate) fn init(service_info: &ServiceInfo, settings: &MetricsSettings) {
    #[cfg(not(feature = "foundations-metrics-backend"))]
    let first_install = Registries::init(service_info, settings);

    #[cfg(feature = "foundations-metrics-backend")]
    let first_install = {
        let _ = settings; // format is read at collect time

        // `foundations-metrics` defaults to reporting non-fatal collection
        // diagnostics on stderr; route them through telemetry instead. A hook
        // installed by the service takes precedence.
        let _ = foundations_metrics::set_collect_error_hook(|args| {
            super::report_nonfatal_collect_error(&args);
        });

        SERVICE_NAME
            .set(service_info.name_in_metrics.clone())
            .is_ok()
    };

    if first_install {
        report_info(BuildInfo {
            version: service_info.version,
        });
        report_info(RuntimeInfo {
            pid: std::process::id(),
        });
    }
}
