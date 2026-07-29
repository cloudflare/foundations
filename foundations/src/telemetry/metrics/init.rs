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
///
/// # Errors
///
/// Fails if the configured service label name cannot be encoded.
pub(crate) fn init(
    service_info: &ServiceInfo,
    settings: &MetricsSettings,
) -> crate::BootstrapResult<()> {
    #[cfg(feature = "foundations-metrics-backend")]
    validate_service_name_format(settings)?;

    #[cfg(not(feature = "foundations-metrics-backend"))]
    let first_install = Registries::init(service_info, settings);

    // Only the service name is kept. How it is represented is read from the
    // settings passed to collection.
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

    Ok(())
}

/// Rejects a service label name that collection could not encode.
///
/// The setting is fixed for the process lifetime, so checking it here only
/// changes how it is discovered: collection skips every metric family, leaving a
/// scrape that reads as a healthy process exporting nothing. Failing startup
/// names the setting at fault instead.
#[cfg(feature = "foundations-metrics-backend")]
fn validate_service_name_format(settings: &MetricsSettings) -> crate::BootstrapResult<()> {
    use crate::telemetry::settings::ServiceNameFormat;

    if let ServiceNameFormat::LabelWithName(label_name) = &settings.service_name_format
        && !foundations_metrics::is_valid_name(label_name)
    {
        anyhow::bail!(
            "metrics.service_name_format label name {label_name:?} cannot be encoded; expected {}",
            foundations_metrics::NAME_REQUIREMENT,
        );
    }

    Ok(())
}

/// Tested here rather than through `telemetry::init`, which refuses to run twice
/// per process and so cannot assert the accepting and rejecting cases together.
#[cfg(all(test, feature = "foundations-metrics-backend", feature = "settings"))]
mod service_name_format_tests {
    use super::*;
    use crate::telemetry::settings::ServiceNameFormat;

    fn validate(format: ServiceNameFormat) -> crate::BootstrapResult<()> {
        validate_service_name_format(&MetricsSettings {
            service_name_format: format,
            ..Default::default()
        })
    }

    #[test]
    fn usable_label_names_are_accepted() {
        for name in ["service", "app_name", "a", "sérvice", "with space"] {
            assert!(
                validate(ServiceNameFormat::LabelWithName(name.to_owned())).is_ok(),
                "{name:?} is encodable and should be accepted"
            );
        }
    }

    #[test]
    fn unencodable_label_names_are_rejected() {
        for name in ["", "\0", "ser\0vice"] {
            assert!(
                validate(ServiceNameFormat::LabelWithName(name.to_owned())).is_err(),
                "{name:?} cannot be encoded and should be rejected"
            );
        }
    }

    /// Prefixing composes a family name that the encoders validate themselves.
    #[test]
    fn metric_prefix_format_is_not_subject_to_the_check() {
        assert!(validate(ServiceNameFormat::MetricPrefix).is_ok());
    }
}
