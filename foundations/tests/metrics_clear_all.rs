//! End-to-end test for `clear_all` through the Foundations facade, in its own
//! binary because it changes process-wide collection state.
#![cfg(feature = "foundations-metrics-backend")]

use foundations::telemetry::metrics::{Counter, Gauge, clear_all, metrics};
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat};

mod common;
use common::collect_text;

#[metrics]
mod draining {
    /// Requests by status.
    pub fn requests_total(status: u16) -> Counter;

    /// Reports sent.
    pub fn reports_total() -> Counter;

    /// Open connections.
    pub fn connections() -> Gauge;
}

#[test]
fn clear_all_reports_only_series_that_change() {
    draining::requests_total(200).inc_by(553);
    draining::requests_total(404).inc_by(12);
    draining::reports_total().inc_by(7);
    draining::connections().set(50);

    // Resolved once and reused, as the `metrics` macro docs recommend.
    let ok_requests = draining::requests_total(200);

    let settings = MetricsSettings {
        service_name_format: ServiceNameFormat::MetricPrefix,
        report_optional: false,
    };

    clear_all();

    let text = collect_text(&settings);
    assert!(
        !text.contains("undefined_draining_"),
        "collected output was: {text}"
    );

    ok_requests.inc();
    draining::connections().dec();

    let text = collect_text(&settings);
    assert!(
        text.contains("undefined_draining_requests_total{status=\"200\"} 554"),
        "collected output was: {text}"
    );
    assert!(
        text.contains("undefined_draining_connections 49"),
        "collected output was: {text}"
    );
    assert!(
        !text.contains("status=\"404\""),
        "collected output was: {text}"
    );
    assert!(
        !text.contains("undefined_draining_reports_total"),
        "collected output was: {text}"
    );
}
