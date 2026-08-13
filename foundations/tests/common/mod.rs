//! Helpers shared by the integration tests.

use foundations::telemetry::metrics;
use foundations::telemetry::settings::MetricsSettings;

/// Collects every metric as text.
///
/// Goes through `collect` rather than `collect_format` so the helper works on
/// either backend; the `allow` covers the deprecation on the new one.
#[allow(dead_code, deprecated)]
pub(crate) fn collect_text(settings: &MetricsSettings) -> String {
    metrics::collect(settings).expect("metrics should be collectable")
}
