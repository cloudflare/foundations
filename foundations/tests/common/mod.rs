//! Helpers shared by the integration tests.

use foundations::telemetry::metrics::{self, ScrapeFormat};
use foundations::telemetry::settings::MetricsSettings;

/// Collects every metric as text.
///
/// `collect_format` deals in bytes because protobuf is not text, so each caller
/// wanting a string converts one back.
#[allow(dead_code)]
pub(crate) fn collect_text(settings: &MetricsSettings) -> String {
    let body = metrics::collect_format(ScrapeFormat::fallback(), settings)
        .expect("metrics should be collectable");

    String::from_utf8(body).expect("text metrics should be valid UTF-8")
}
