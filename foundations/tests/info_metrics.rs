//! Info metrics reported during telemetry initialisation must reach the scrape
//! output with their bare name. Kept in its own test binary because
//! `telemetry::init` may only run once per process.

use foundations::ServiceInfo;
use foundations::telemetry::settings::{MetricsSettings, ServiceNameFormat, TelemetrySettings};
use foundations::telemetry::{TelemetryConfig, TelemetryContext, metrics};

/// Returns the value of the sample line for `series`, if it was collected.
fn sample_value(metrics: &str, series: &str) -> Option<f64> {
    metrics.lines().find_map(|line| {
        line.strip_prefix(series)
            .and_then(|rest| rest.strip_prefix(' '))
            .and_then(|value| value.parse::<f64>().ok())
    })
}

#[tokio::test]
async fn init_reports_build_and_runtime_info_unprefixed() {
    // Keeps the tracing reporter from binding real sockets during the test.
    let _ctx = TelemetryContext::test();

    let service_info = ServiceInfo {
        name: "info-svc",
        name_in_metrics: "info_svc".to_owned(),
        version: "4.5.6",
        author: "Foo Bar",
        description: "An example service",
    };
    let telemetry_settings = TelemetrySettings::default();

    foundations::telemetry::init(TelemetryConfig {
        service_info: &service_info,
        settings: &telemetry_settings,
        custom_server_routes: vec![],
    })
    .expect("telemetry init should succeed");

    let settings = MetricsSettings {
        service_name_format: ServiceNameFormat::MetricPrefix,
        report_optional: false,
    };
    let text = metrics::collect(&settings).expect("metrics should be collectable");

    assert_eq!(
        sample_value(&text, "build_info{version=\"4.5.6\"}"),
        Some(1.0),
        "build_info missing from: {text}"
    );
    assert!(
        text.contains("runtime_info{pid=\""),
        "runtime_info missing from: {text}"
    );

    // Info metrics keep their bare name even though the service name is
    // configured as a metric prefix.
    assert!(
        !text.contains("info_svc_build_info"),
        "info metrics must not be service-prefixed: {text}"
    );
}
